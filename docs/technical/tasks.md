# Tasks: handing the module a whole job

The Agent members (`Describe`, `PlanTask`, `StartTask`, `AwaitTask`,
`ContinueTask`, `CancelTask`, `TaskReport`, `ListTasks`) are the API meant for
outside agents. A caller hands over a job such as "find the cheapest flight
from Delhi to Srinagar on 14 October and fill in my details up to payment",
and gets back a task that runs in the background and stops only when it needs
something from the caller.

The controller lives in `crates/tinycomputer-engine/src/task/`. It runs flows
through the flow runtime described in [`decision-loops.md`](decision-loops.md);
this page covers what sits on top: task state, pausing and resuming, private
values, budgets, and the planner.

## A session, end to end

A caller that has never used the module starts with `Describe`. It returns the
contract version, which surfaces are available (and why one is not), whether
Jev and a planner are configured, the step kinds, the flow guide, a JSON Schema
for every Agent member's request and reply, and worked examples.

Then it starts the task. With a planner configured, plain language is enough:

```json
{"member": "StartTask", "args": [{
  "task": "Find the cheapest one-way flight from Delhi to Srinagar on 14 October and fill in my details up to payment",
  "facts": {"first name": "Asha", "last name": "Raina", "email": "asha@example.com",
            "mobile number": "9876543210"},
  "constraints": {"surfaces": ["browser"], "origins": ["https://.goindigo.in", "https://.google.com"]},
  "budget": {"max_actions": 200, "max_model_calls": 400}
}]}
```

Without a planner, the caller passes `flow` instead of `task`, written with the
guide `Describe` returned. Either way `StartTask` returns at once with a
`TaskView`:

```json
{
  "id": "task-1",
  "status": {"state": "running"},
  "summary": "Planning the task.",
  "step": null,
  "progress": 0.0,
  "next": ["AwaitTask", "CancelTask"]
}
```

The caller then long-polls `AwaitTask({id, timeout_ms})`. It returns as soon
as the task stops running, or when the timeout passes (60 seconds at most,
`MAX_AWAIT_MS`). Each view carries a one-sentence `summary`, the current step
(index, kind, intent, surface), `progress` as the fraction of top-level steps
finished, and `next`, the calls that make sense from here. A caller that just
follows `next` cannot get the protocol wrong.

A booking like the one above usually ends like this:

```json
{
  "status": {"state": "checkpoint",
             "reason": "reached the payment step (Pay now); payment is always left to you",
             "location": "Pay now", "continuable": false,
             "summary": "done: https://www.google.com/travel/flights…; the flight results by lowest price; …"},
  "summary": "Stopped: reached the payment step (Pay now); payment is always left to you.",
  "next": ["CancelTask", "TaskReport"]
}
```

The browser session stays open on the payment page so a person can finish.
`CancelTask` releases it afterwards. `TaskReport` (confidential) returns every
step report, the records the task read or extracted, the trace, the grounding
hints it learned, and `artifacts`: the best-effort screenshot each stopped run
took before its surfaces were released, read with `BrowserReadOutput` within
five minutes. No status carries a screenshot, since views also reach the
non-confidential `AwaitTask` and `ListTasks`.

## Statuses

| State | Means | What to call next |
|---|---|---|
| `running` | the task is working | `AwaitTask`, `CancelTask` |
| `needs_input{fields}` | the flow uses a `${name}` no fact supplies, or the planner asked for a value | `ContinueTask` with `inputs` |
| `needs_approval{action, target}` | a `stop_before` found an irreversible control and the task may not press it on its own | `ContinueTask` with `approve: true` or `false` |
| `needs_human{reason}` | a step failed in front of a captcha, a one-time code, two-factor authentication, or a login wall | a person does it, then `ContinueTask` |
| `checkpoint{reason, location, summary, continuable}` | the task stopped where a person must take over; a payment checkpoint is final | `CancelTask` to release it, `TaskReport` |
| `needs_plan{guide}` | a plain-language task arrived and no planner is configured | `StartTask` again with a `flow` |
| `done{answer, records}` | every step finished | `TaskReport` |
| `failed{step, reason, hint, recoverable}` | a step failed, a budget ran out, or the flow was invalid | `TaskReport`, then a new `StartTask` |
| `cancelled` | the caller cancelled, or declined an approval | `TaskReport` |

## How a task runs

`Tasks::start` validates the request, registers a `Cell` (the task's published
view plus its working state), and spawns a worker. The controller holds at most
32 tasks (`MAX_TASKS`), dropping finished ones first.

The worker runs a list of flow runs in order (`drive`). Usually that list has
one entry, the whole flow. For each run it:

1. builds a `RunFlowRequest` (`run_request`) with the facts as variables, the
   fact names marked private, `include_values` on (so Jev can check what was
   typed; secrets are masked), the task's grounding memory,
   and what is left of the budget;
2. hands it to the `FlowRunner`, which runs it on the task's workspace;
3. reads the result (`interpret::run_outcome`) and decides whether to carry
   on or stop with a status;
4. adds the run's steps, trace, learned hints, and spend to the task, and
   keeps any values the run read: every variable that is not a fact and that
   the run changed from what the flow gave it. A flow variable passed in and
   left as it was is the caller's input, while one a planner declared empty
   up front and a `read` then filled is a read. Those become the `records` in
   the final answer and in `TaskReport`.

The `FlowRunner` trait is what makes the controller testable: tests script the
runs, and the module plugs in `WorkspaceRunner`
(`crates/tinycomputer/src/tinybus_module/runner.rs`), which gives each task its
own `Workspace` of the desktop and a fresh browser session. Runs of one task
share that workspace, so a resumed task picks up on the page the last run left.

### Stopping and resuming

`task/interpret.rs` maps how a run stopped to what the task does next:

- **Completed.** On to the next run, or `done` if there is none. The answer
  lists what was read, for example `Finished all 9 steps. cheapest: IndiGo ·
  08:00 · ₹7,346`.
- **Stopped before an irreversible action.** If the control or the phrase is a
  payment (`consequence` says `Payment`), the task ends at a final
  `checkpoint`. Otherwise it pauses at `needs_approval` and remembers how to
  resume: the `stop_before` phrase, the app the flow was on, and the steps
  after it. Approving runs a one-step flow with that `stop_before` and
  `allow_destructive` on, then the rest with the task's own setting. Declining
  cancels the task. A `stop_before` nested inside an `if` or `repeat_until`
  resumes from the start of the containing top-level step, because which
  branch or round it was in cannot be recovered from the step path.
- **A step failed.** The task fails, marked recoverable, and remembers the
  failed step and everything after it. Then `human_wall` reads the surface's
  visible text. If it shows a captcha, "verify you are human", a one-time code,
  two-factor authentication, or a login wall ("sign in to continue", "log in to
  see …", "log in or sign up", "login required"; a header's bare "Log in" and
  "Sign up" links are not one), the status becomes
  `needs_human` and `ContinueTask` reruns the failed step and the rest once
  the person has got past it. If not, and a rescuer is configured, the failure
  is rescued (below). Otherwise it stays `failed`. A step that failed because
  no browser could be started (`BROWSER_UNAVAILABLE`) is never rescued: the
  task fails at once, not recoverable, with a hint to give the path of Chrome
  or Chromium. An `open` or `browse` step's failure note keeps the first line
  of the failure's own words after its code.
- **A budget ran out, or the flow was invalid.** `failed`, with a hint such as
  "raise budget.max_actions".

### Budgets across runs

A task's `TaskBudget` (`max_actions`, `max_model_calls`, `max_elapsed_ms`)
bounds the whole task, not one run. An approval or a human step splits a task
into several runs, and each run gets only what the task has not spent yet, so
resuming never refills the budget. Unset fields default to 120 actions and 6000
Jev calls, and each decision is voted seven ways. `budget.strategy` picks how
decisions are asked: `narrow` (the default) or `wide`, one request per turn
over a digest of the screen ([`specs/jev-wide-turns.md`](specs/jev-wide-turns.md)).
`budget.deliberation` picks how much each decision is deliberated before it
is acted on: `deep` (the default), `standard`, or `off`
([`specs/jev-deliberation.md`](specs/jev-deliberation.md)). Deliberation
spends calls only where the evidence is thin, and degrades rung by rung when
the budget runs short rather than failing the run.
`budget.max_rescues` caps how many failed steps are rescued (below).
The flow runtime has no clock, so `max_elapsed_ms` is enforced by
the controller, which times out a run that would go past what is left. Time
spent waiting for the caller does not count.

### Constraints

`TaskConstraints` narrows what a task may touch:

- `surfaces`: `browser`, `desktop`, or both (empty means both). A task confined
  to one side gets a workspace without the other, so it cannot reach it even by
  mistake.
- `origins`: the sites a browser session may open pages on, such as
  `https://.goindigo.in` for a site and its subdomains, or `*` for any public
  site (private and local addresses and names stay refused; a name is never
  resolved, so a public name that leads to a local address is admitted). The
  session checks pages itself: a navigation outside the list is refused
  before it is sent, nothing is done on a page outside it, and a page a
  click or redirect lands on outside it is left before it is read. The
  files a page loads from other hosts are not checked; agent-browser's domain
  filter is not used, as it refuses those files too and breaks the page. It
  is a guard rail, not a sandbox. `fill_then_approve` refuses `*`.
- `payment`: `stop_at_payment` (the default) makes the control that pays a
  final checkpoint; `fill_then_approve` fills the payment form from secret
  facts and waits at `needs_approval` before pressing it. The latter needs
  `origins` (`ORIGINS_REQUIRED` otherwise).
- `allow_destructive`: press irreversible controls including payments without
  pausing or asking for approval. Use with caution; payment controls are never
  gated when this is enabled.
- `browser_endpoint`: attach to a running Chrome at this DevTools address
  instead of launching one. Booking sites often turn away a fresh headless
  browser but serve a person's own. Closing an attached session only
  disconnects; it never closes the person's browser.
- `headed`: show the browser.
- `browser_executable`: the absolute path of the Chrome or Chromium binary to
  launch, in place of the module's configured `browser.executable`
  (`INVALID_REQUEST` for a bare name, a relative path, or no executable file
  there).
- `browser_profile`: an absolute folder to keep the browser's profile in, so
  sign-ins last between tasks (`INVALID_REQUEST` for a relative folder). One
  browser can hold a folder at a time: cancel a task still holding the folder
  before starting another on it, or the new task fails as its browser starts.
  A task's own profile is a browser the module launched, so a press of a
  page's "use my current location" grants it the location while it runs.
- `browser_executable` and `browser_profile` are the host's settings: a host
  that relays a model's request never takes them from the model, and neither
  `Describe` nor the skill offers them to one. Neither goes with
  `browser_endpoint` (`INVALID_REQUEST`), which launches nothing.

## Private values

Facts are the caller's own details: names, email, phone, date of birth. The
controller keeps them in a `Facts` store (`tinycomputer-core/src/facts/`) and
follows three rules:

1. Secret values never leave: Jev sees a secret only as `${name}`,
   including when a field on screen holds it, and the planner and the
   rescuer see fact names only. Shared values brief Jev, and Jev reads them
   back from the fields they were typed into (`include_values` is on), so a
   `verify` of typed details can pass. A value is looked up at the moment it
   is typed into a field. Summaries, step intents
   in the view, and the final answer go through `Facts::redact`, which
   replaces each value with `‹name›`.
2. Secrets are only typed: a flow may use a secret `${fact}` only as an
   `enter` step's value. In any other position (step text, a condition, a
   slot's name, an `open` app, a `browse` address) validation rejects the
   flow before anything runs. `open` and `browse` count because the app or
   address stays visible to every later question. Shared facts may appear
   anywhere. When `ContinueTask` supplies a new value, the flow is checked
   again, since a name that looked undefined at the start may now be a fact
   used somewhere it must not be.
3. Card data is always secret: a fact is secret when `secret_facts` names
   it, when its name labels a card (card number, CVV, card expiry, card PIN,
   and the like), a password, a one-time code, or an identity or account
   number, or when its value is a plausible card number
   (`tinycomputer-core/src/facts/`). A caller can make any fact secret but
   never make one of these shared, and naming a secret that is not a fact is
   refused with `UNKNOWN_SECRET`. Card details are typed only under
   `payment: "fill_then_approve"`, which requires `origins` (below).

## Payment is a checkpoint, unless the caller opted out

Two independent checks stop a task before money moves, whatever a model
decided — as long as the task has not set `allow_destructive` (see
[Constraints](#constraints)). With `allow_destructive` on, `stop_before`
clicks the control it finds instead of stopping in front of it, and neither
check below gates that click: the caller has explicitly asked to press
irreversible controls, payments included, without a checkpoint.

- `consequence(label)` in `tinycomputer-core/src/safety/` classifies a control
  by its words. "Pay", "Pay now", "Place order", "Checkout", "Buy now",
  "Confirm and pay" and similar are `Payment`. "Send", "Delete", "Publish",
  "Confirm booking" are `Irreversible`. "Book", "Select", and "Continue" are
  `Reversible` on purpose: they lead to more forms, and the payment check
  stops the run before anything is charged.
- `screen_payment_evidence(screen)` looks at the page itself for card-form
  wording (card number, CVV, expiry, cardholder). When it finds any, every
  click on that screen counts as destructive, so a "Continue" button on a card
  form is refused too.

A planned booking flow ends with `{"stop_before": "paying for the booking"}`.
The runtime finds the pay control, stops in front of it, and the controller
turns that into the final checkpoint.

## The planner

The planner (`crates/tinycomputer-engine/src/planner/`) is an optional language
model that turns a plain-language task into a flow. It never acts and never
sees the screen.

It is compiled into the module (the `planner` feature) and stays off until the
host sends a `planner` object in the module's private configuration, with an
`api_key` and an optional `model` (default `anthropic/claude-sonnet-5`), on
the `open_router` route by default or `provider: "tiny_humans"` for Tiny
Humans' OpenAI-compatible gateway (contract 2.8;
[`../crates/tinycomputer/configuration.md`](../crates/tinycomputer/configuration.md)). Without it, a plain-language `StartTask` returns
`needs_plan` with the guide, and `PlanTask` returns an error saying no planner
is configured.

`Planner::plan` sends the model:

- a protocol: reply with one JSON flow; use `browse` for the web and `open` for
  applications; refer to personal details only as `${name}`; never invent
  them; never enter payment details; end any purchase with a `stop_before` for
  paying; guard sending, deleting, publishing, and submitting the same way;
- the full flow guide;
- the task, the available surfaces, and the fact names.

The reply is parsed and checked with the same validator `RunFlow` uses. An
invalid flow goes back to the model with the errors, up to two times
(`REPAIRS`). Variables the flow uses that no fact supplies come back as
questions, which the task reports as `needs_input` before anything runs.

`PlanTask` is the dry run: it returns the flow and the questions without
starting anything, so a caller can inspect or edit the plan first.

A `StartTask` with a `task` and no `flow` plans inside the task. When the
task may run only on the browser, its runner is asked to get the browser
ready meanwhile (`FlowRunner::prepare`); unless the module's
`browser.prelaunch` is off, the session opens while the plan is drafted, so
the first step does not wait for Chrome to start. When the task's text sends
its browser to one web address, written out with `https://` or `http://`
right after words such as "go to", "open", "visit", "start at" or "on", and
carrying no query or fragment, the page loads in it meanwhile
(`FlowRunner::open_page`, journaled as `open_page`), in the same wait, for
at most 10 s: a first step that browses there finds it loaded, and loads
nothing, as long as nothing has read the page first. Over 57 live plans, 56
started by browsing the page their task named, and that first page took 1.9
s to load (p90 6.4 s) and 1.1 s to settle; a plan takes 10–30 s. An address
the task only mentions (one to check, read out, or pass on), one whose query
could hold a token a load would spend, and a task naming several addresses
or none load nothing. A plan that asks for values lets the browser go while
the task waits (`needs_input`), and the run that follows opens it again; a
task cancelled while its browser was still opening is left holding none.

Every task planned this way also has its runner warm Jev while the plan is
drafted (`FlowRunner::warm`): one small evaluation for each call its first
turn will make, its judging and grounding's opening in every framing, so that
turn finds its connections open
([`jev-runtime.md`](../crates/tinycomputer-engine/jev-runtime.md#warming-connections)).
The task never waits for the warm-up, and a task whose budget caps its Jev
calls is not warmed.

## Rescues

When a top-level step fails and no person is needed, the task asks a
reasoning model for guidance before it fails
([`specs/task-rescue.md`](specs/task-rescue.md)). The rescuer
(`crates/tinycomputer-engine/src/rescue/`) is briefed with the goal, the flow
with the failed step marked, what the run reached, earlier rescues, the
screen's visible text as untrusted data, and the fact names — every fact value
redacted. It answers with up to six steps to run in place of the failed one,
checked by the flow validator, with a skip when the screen is already past the
failed step, or gives up. Its `covers` count drops as many
of the steps right after the failed one when its steps already do them, but
never a step holding a `stop_before`, and guidance for a failed `stop_before`
must hold one itself. The task then runs the guidance and
every remaining step unchanged, `stop_before` included, from what the budget
has left.

A task gets five rescues at most (`budget.max_rescues`, 0 to 5; 0 turns them
off), and one rescue may think for two minutes. A rescue that gives up, fails,
or gives no valid guidance leaves the task `failed`, with the rescuer's reason
in the `hint`. `TaskReport.rescues` lists each one with its outcome:
`recovered` when its steps all finished, `failed_again`, or `gave_up`.

The planner's configuration brings the rescuer, on the same key: an optional
`rescue_model` (default `openai/gpt-6-luna`, asked with low reasoning effort).
`Describe` reports it as `rescue_configured`. A plain `RunFlow` is not
rescued: rescues belong to the task controller, and the flow runtime asks
only Jev.

## Output shapes

A finished task's `records` are raw: whatever the screen showed, in its
order, with its duplicates and chrome. A caller that needs a fixed shape
passes `output` with `StartTask`
([`specs/task-output.md`](specs/task-output.md)): `instructions` in plain
language and, optionally, a JSON `schema`. When every step has finished,
the shaper (`crates/tinycomputer-engine/src/shape/`) sends the goal, the
instructions, the schema, and what the steps saved — every fact value
redacted, wrapped as untrusted data — to a reasoning model in one pass. Its
JSON object is checked against the schema, sent back with what is wrong up
to twice, and returned as `done.result` beside the records. A result that
never fits fails the task, not recoverable, with the records still in
`TaskReport`.

The schema is a subset of JSON Schema: `type`, `properties`, `required`,
`additionalProperties` (a boolean), `items`, `enum`, `minItems`, `maxItems`,
`description`, and `title`, with an `object` at the top. `StartTask` refuses
anything else (`INVALID_OUTPUT`), so every rule given is checked, and refuses
`output` without a planner configured (`OUTPUT_UNAVAILABLE`). The shaping
model is the planner configuration's `output_model` (default
`openai/gpt-6-luna`); `Describe` reports it as `output_configured`.

## Where the code is

| File | Holds |
|---|---|
| `task/mod.rs` | the `FlowRunner` trait and the controller's limits |
| `task/controller.rs` | `Tasks` and the calls it answers: plan, start, await, continue, cancel, report |
| `task/store.rs` | the task store: each task's cell and working state |
| `task/drive.rs` | `drive`: running a task's flows in the background and finishing it |
| `task/interpret.rs` | what a finished run means: continue, pause, or stop, and how to resume |
| `task/resume.rs` | answering a paused task: values, approval, a person past a wall |
| `task/budget.rs` | a task's budget across its runs |
| `task/human.rs` | `human_wall`: a wall only a person can pass |
| `task/recovery.rs` | handing a failed step to the rescuer and running its guidance |
| `task/publish.rs` | publishing views, summaries, next calls, and records |
| `task/brief.rs`, `task/names.rs`, `task/errors.rs` | the task's brief, the names its flow may use, the call errors |
| `task/describe.rs` | `Describe`: capabilities, schemas, and examples |
| `planner/mod.rs` | the planning protocol, validation, and repairs |
| `planner/config.rs` | `PlannerConfig`, `ModelRoute`, the model defaults, and the route allow-list (feature `planner`) |
| `planner/hosted.rs` | the hosted `LanguageModel`s, on OpenRouter or Tiny Humans, for the planner, the rescuer, and the shaper (feature `planner`) |
| `rescue/mod.rs` | the rescue protocol and the briefing |
| `rescue/render.rs`, `rescue/judge.rs` | the briefing as the model reads it; judging its answer and building the resumed flow |
| `shape/mod.rs`, `shape/schema.rs` | the output pass: its protocol, repairs, and the JSON Schema subset it checks |
| `workspace/mod.rs` | the desktop and the browser as one surface |
| `tinycomputer/src/tinybus_module/runner.rs` | the module's `FlowRunner`: one workspace and browser session per task |
