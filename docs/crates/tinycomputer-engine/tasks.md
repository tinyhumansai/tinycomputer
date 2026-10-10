# Tasks

`Tasks`, in `crates/tinycomputer-engine/src/task/controller.rs`, is the controller
behind the Agent interface: the thing most callers should actually use
instead of `RunGoal` or `ResolveIntent` directly. Give it a flow (or a
plain-language task, with a planner configured) and it runs that flow in the
background, reporting back a status a calling model can act on: still
running, needs a value, needs approval, reached a checkpoint, needs a
person, done, or failed.

This page covers the controller's behavior. For the flow language itself,
`browse`, `enter`, `pick`, `stop_before`, and the rest of the step kinds,
see [flow/README.md](flow/README.md) and
[writing-flows.md](../../writing-flows.md).

## The shape of a task

A task is a `Cell`: a published view (`TaskView`, readable by every caller
watching it), and private working state (facts, budget, the steps run so
far, rescues, what to resume once unpaused). Starting a task with
`StartTask` returns a `TaskView` immediately; the flow itself runs on a
spawned background worker, and a caller polls it with `AwaitTask` or reads
its latest view straight from `ListTasks` / `TaskReport`.

```
StartTask  ──> TaskView (Running)
                  │
                  │  AwaitTask (blocks until it changes, or times out)
                  v
          Running | NeedsInput | NeedsApproval | Checkpoint | NeedsHuman
                  │
                  │  ContinueTask (answers what it asked for)
                  v
          Running again, or ...
                  v
              Done | Failed | Cancelled
```

## Starting a task

`StartTaskRequest` takes either `task` (plain language; needs a planner
configured, or the task pauses immediately with `NeedsPlan`) or `flow` (a
flow you wrote yourself). It also takes:

- `facts`: values the flow may type, by name (`${first name}`, `${card
  number}`). See [Facts and secrets](#facts-and-secrets) below.
- `secret_facts`: names among `facts` to keep secret beyond what is
  recognised automatically.
- `constraints`: where the task may act and how far it may go (see below).
- `budget`: upper bounds on the work it may do (see below).
- `memory`: grounding hints from an earlier run, so this one reads less of
  the screen to find the same elements again.
- `trace`: record every Jev exchange for `TaskReport`.
- `output`: instructions and an optional JSON Schema for the answer the
  caller wants back, instead of reading raw `done.records` by hand. Needs
  the planner configured (the shaper it builds alongside the planner and
  the rescuer), or `StartTask` fails with `OUTPUT_UNAVAILABLE`; a schema
  outside the supported subset fails with `INVALID_OUTPUT`. See
  [output.md](output.md).

Before anything runs, the flow is validated with the same checker a flow
author's `Describe` call uses. A flow that references an undefined
`${name}` is not rejected outright; that becomes a pause instead (see
below). Any other structural problem (an unknown step kind, a
`stop_before` guidance that does not itself guard, and so on) fails
`StartTask` immediately with `INVALID_FLOW`.

### Missing values become a pause, not an error

If the flow uses a `${name}` that neither `facts` nor the flow's own `vars`
supplies, the task starts in `NeedsInput` rather than failing:

```json
{
  "id": "t-7",
  "status": {
    "state": "needs_input",
    "fields": [
      { "name": "phone", "why": "the flow uses ${phone} and no fact supplies it", "kind": "phone", "options": [] }
    ]
  },
  "summary": "The task needs values before it can start.",
  "progress": 0.0,
  "next": ["ContinueTask", "CancelTask"]
}
```

`kind` is a best guess from the field's name (`input_kind` in `task/names.rs`):
a name containing "email" becomes `email`, "phone" or "mobile" becomes
`phone`, "date"/"birth"/"dob" becomes `date`, "count"/"number of"/"travellers"
becomes `number`, and anything else is plain `text`. Answer with
`ContinueTask { id, inputs: {"phone": "+91 98765 43210"} }`; the task
re-checks for anything still missing and either asks again or starts
running.

## Task statuses

Every `TaskView.status` is one of nine states (`TaskStatus` in
`tinycomputer-bus/src/agent/types/status.rs`):

| State | What it means | Answer with |
|---|---|---|
| `Running` | Working. | `AwaitTask` |
| `NeedsInput` | Missing one or more `${name}` values. | `ContinueTask.inputs` |
| `NeedsApproval` | Stopped in front of an irreversible action (a `stop_before`). | `ContinueTask.approve` |
| `Checkpoint` | Stopped at a checkpoint, always on reaching a payment page, when the task is not allowed to fill the payment form. | `ContinueTask.approve` only if `continuable`; a payment checkpoint never is |
| `NeedsHuman` | Blocked on something only a person can do: a captcha, a login page, a one-time code. | `ContinueTask { answer: "done" }` once the person has |
| `NeedsPlan` | A plain-language `task` arrived and no planner is configured. | Write a flow from the returned `guide` and call `StartTask` again with it |
| `Done` | Finished, with an `answer`, any `records` from `extract`/`pick` steps, and, when `output` was asked for, a `result` shaped to its schema. | (final) |
| `Failed` | Could not finish, with `reason`, `hint`, and whether `recoverable`. | (final, unless recoverable; see below) |
| `Cancelled` | Stopped by `CancelTask`. | (final) |

`TaskStatus::is_final()` is true for `Done`, `Failed`, `Cancelled`, and a
non-continuable `Checkpoint`. `next_calls` (in `task/publish.rs`) computes which
member calls make sense for the current state and is echoed back as
`TaskView.next`, so a caller does not have to hardcode the state machine:
it can just try one of the calls the last view offered.

## Pausing and resuming

Every run of a task, whichever kind of pause split it from the last one,
starts with what earlier runs already saved: `run_request` (in
`task/budget.rs`) fills `RunFlowRequest.collected` from the task's own state
before each new run, so a step such as "open the next chat not yet read"
still knows what earlier runs already read. See [output.md](output.md)
for the whole run-memory picture, including what the rescuer is shown.

Three kinds of pause exist, and each resumes differently:

- **`NeedsInput`.** `ContinueTask.inputs` merges into the task's facts, the
  flow is re-checked now that the values are known (a `${name}` that looked
  merely unused at `StartTask` time can turn out to be one that must never
  reach a model, once its value is finally known, so the check runs again
  in full), and if nothing is still missing the flow starts.
- **`NeedsApproval`.** The paused `stop_before` and everything after it were
  captured as a `Resume::Approval` when the flow stopped. `ContinueTask
  { approve: true }` runs the guarded action first, on its own
  (`allow_destructive: true`, for just that one step), then the rest of the
  flow, if any, as a second run. `approve: false` cancels the task outright:
  the action was declined, so there is nothing left to do, and releases
  whatever surface it held.
- **`NeedsHuman`.** The failed step and everything after it were captured as
  a `Resume::Retry`. Any `ContinueTask` call while paused here re-runs from
  that step; there is no separate "yes I did it" signal beyond calling
  `ContinueTask` at all (the `answer` field is accepted for a person's own
  bookkeeping but is not otherwise interpreted here).

### How a `NeedsHuman` pause is detected

A step failure only becomes `NeedsHuman` when it looks recoverable *and* the
run has something to retry (a `Resume::Retry` was captured; see
`human_wall` in `task/human.rs`). Whether a person, not a rescue, is actually
needed is decided by reading the screen's visible text for the signs of a
wall only a person can pass (`tinycomputer_core::human_needed`: a captcha, a
login form, a one-time-code prompt). If no such wall is detected, the
failure is left alone (and may be handed to the rescuer instead; see
[rescue.md](rescue.md)).

## Budgets, across every run of a task

A `TaskBudget` bounds a whole *task*, not one flow run of it. That
distinction matters because a task rarely finishes in exactly one flow run:
an approval, a missing value, or a rescue each splits it into another run,
and each of those runs must be charged against what the task has already
spent: never given a fresh budget just because it happens to be a new
`RunFlow` call underneath.

```rust
pub struct TaskBudget {
    pub max_actions: Option<u32>,       // default 120
    pub max_model_calls: Option<u32>,   // default 6000
    pub votes: Option<u32>,             // default 7
    pub strategy: Option<FlowStrategy>,
    pub deliberation: Option<Deliberation>,
    pub max_elapsed_ms: Option<u64>,    // default: unbounded
    pub max_rescues: Option<u32>,       // default 5, also the ceiling
}
```

The controller tracks cumulative `Spent { actions, model_calls, elapsed_ms }`
on the task's `State`, and `run_request` (in `task/budget.rs`) computes each new
run's request with what remains: `max_actions - spent.actions`,
`max_model_calls - spent.model_calls`, and so on. `max_elapsed_ms` has no
equivalent inside a single `RunFlowRequest` (a flow run cannot police its
own wall-clock time from the inside), so it is enforced by the task
controller itself, wrapping the call to the flow runner in a
`tokio::time::timeout` for whatever time remains. Time spent waiting for the
caller to answer a pause is not counted: the clock only runs while a flow is
actually running.

`votes` (how many independent framings each decision is asked, then
averaged; see [`docs/technical/jev-harness.md`](../../technical/jev-harness.md))
defaults to 7 for a task, deliberately generous: Jev is cheap, so a task
would rather ask several ways and average than risk one bad framing costing
the whole run. `RunGoal` and `ResolveIntent`, called directly rather than
through a task, do not get this default: it is specific to the task
controller's own defaults, not the flow runtime's.

## Constraints

`TaskConstraints` is where a caller says where a task may act and how far it
may go, independent of the budget:

```rust
pub struct TaskConstraints {
    pub payment: PaymentMode,
    pub surfaces: Vec<SurfaceKind>,       // empty = every available surface
    pub origins: Vec<String>,             // empty = any origin
    pub allow_destructive: bool,
    pub browser_endpoint: Option<String>,
    pub headed: bool,
    pub browser_executable: Option<String>,
    pub browser_profile: Option<String>,
}
```

- `surfaces` restricts a task to `desktop`, `browser`, or both; see
  [workspace.md](workspace.md) for how a flow step then routes to the right
  one.
- `origins` restricts which sites a browser session may open pages on: the
  sites card details may be typed on, for instance. `*` admits any public
  site, but never names one card details may be typed on. Only pages are
  checked, never the files a page loads from its CDN or APIs.
- `allow_destructive` lets a task perform irreversible actions without
  pausing at all. Off by default: a fresh task always pauses at a
  `stop_before` unless told otherwise.
- `browser_endpoint` and `headed` control how the browser surface is
  attached (an existing signed-in Chrome, shown rather than headless).
- `browser_executable` and `browser_profile` choose the binary the task's
  browser launches and an absolute folder its profile is kept in, so a site
  signed into once stays signed in for later tasks (`INVALID_REQUEST` for a
  relative folder).

## Payment modes

Paying is never automatic, in either `PaymentMode`. The mode only changes
*how far* a task goes on its way to the control that pays:

- **`StopAtPayment`** (the default). The task stops at the step that pays
  and hands it back as a **non-continuable** `Checkpoint`: a dead end for
  this task. A person finishes the payment themselves, and the task's
  browser session is only released by `CancelTask`.
- **`FillThenApprove`**. The task fills the payment form from the caller's
  secret facts, then pauses as an ordinary `NeedsApproval`, the same as any
  other irreversible action. Only `ContinueTask.approve` presses the control
  that pays. This mode requires `constraints.origins` to be set; `StartTask`
  refuses with `ORIGINS_REQUIRED` otherwise, so card details are typed only
  on sites the caller explicitly named.

Whichever mode is in force, the task's Jev brief always carries the matching
rule ("Never pay: stop in front of the control that pays." or "Fill the
payment form from the secrets, then stop in front of the control that
pays."), so the decision model is never left to infer it from context.

## Facts and secrets

Facts are how a task's caller supplies values without exposing them to a
decision model. `tinycomputer-core::Facts` (not this crate, but what it
builds on) splits every fact into shared or secret:

- **Shared** facts (a traveller's name, an email, a date of birth) are part
  of the brief Jev is given by value, so it can, say, pick "Female" from a
  list or tell "Mr" from "Ms".
- **Secret** facts (a card number, a passport number, a password, a one-time
  code) reach a model only as `${name}`, never the value. A field is
  automatically secret when its name matches a recognised sensitive term
  (`card number`, `cvv`, `otp`, `passport`, `aadhaar`, `pan`, `iban`, and
  more; see `is_sensitive_name` in `tinycomputer-core`) or when its value
  looks like a card number, and a caller can add more names to `secret_facts`
  but can never make a recognised-sensitive one shared.

Facts run with `include_values: true` at the flow layer, so Jev can read
what a field currently holds and check that what was typed matches. But the
flow runtime masks every secret value in anything it builds for Jev before
it leaves the machine, so a secret fact's value is never actually visible to
the model even though the field it landed in is. Every summary, every
`TaskReport`, and everything shown to the rescuer is redacted the same way
(`Facts::redact` replaces a value with `‹name›`; `Facts::mask` replaces a
secret's value, or a long run of its digits found some other way on the
page, with `${name}`).

## Reading a task's report

`TaskReport` (from `Tasks::report`) is everything a task did: its current
view, the flow it ran, one `StepReport` per step reached, `records` (rows
`extract` and `pick` steps collected, keyed by variable name), `learned`
grounding hints, the Jev `trace` if `StartTask.trace` was set, and every
`Rescue` attempted, in order. `records` is always the raw values the flow
saved; when `StartTask.output` was set, `view.status.result` holds the
shaped answer built from them (see [output.md](output.md)), alongside
`records`, not instead of it.

### Example: a `Done` task's records

An `extract` step that pulled rows of text ends up here as parsed JSON rows,
one map per row (`records`, built in `task/publish.rs`); anything else that was
read (a `pick` step's chosen item, say) is a single-field map:

```json
{
  "view": {
    "id": "t-7",
    "status": { "state": "done", "answer": "Finished all 8 steps. cheapest: Play 09:10 → 11:40, ₹4,321", "records": {} },
    "summary": "Finished all 8 steps. cheapest: Play 09:10 → 11:40, ₹4,321",
    "progress": 1.0,
    "next": ["TaskReport"]
  },
  "records": {
    "cheapest": [ { "value": "Play 09:10 → 11:40, ₹4,321" } ]
  },
  "rescues": []
}
```

## Failure, and the boundary with rescue

When a top-level step fails and the failure is `recoverable`, the task
controller first checks whether it looks like a captcha or login wall
(`NeedsHuman`, above); failing that, and if a rescuer is configured and the
task's rescue budget is not spent, it is handed to the rescuer for guidance
before the task is allowed to fail outright. See [rescue.md](rescue.md) for
that whole path: retry, skip, or give up, and what the rescuer is and is
not shown.

Only once a rescue is not attempted (no rescuer configured, or the budget is
spent) or the rescuer itself gives up does the task actually move to
`Failed`. A `Failed` task's `recoverable` flag tells a caller whether trying
again, after applying `hint`, has a real chance, versus a structural
problem (an invalid flow, a budget set too low structurally) that retrying
alone will not fix.

## Cancelling

`CancelTask` stops a task that is not already final and releases whatever
surface (desktop window, browser session) it was holding, unconditionally,
even on a task that already finished. That unconditional release matters
most for a payment `Checkpoint`: since it is never continuable, `CancelTask`
is the *only* way its browser session is ever released, so a caller must
call it once a person has finished paying (or decided not to), or that
session's slot stays consumed.

## Capacity

The controller holds at most `MAX_TASKS` (32) tasks. Starting a 33rd task
first evicts the oldest task whose status is already final; if every held
task is still running or waiting, `StartTask` fails with `TOO_MANY_TASKS`
rather than silently dropping one that is still in progress.

## Source

- `crates/tinycomputer-engine/src/task/`, `Tasks` (`controller.rs`), `Cell`
  and `State` (`store.rs`), `drive` (`drive.rs`), `rescued` (`recovery.rs`),
  `human_wall` (`human.rs`), budgets (`budget.rs`).
- `crates/tinycomputer-engine/src/task/interpret.rs`, turning a flow run's
  result into `Next` (continue or stop-with-status) and `Resume`.
- `crates/tinycomputer-engine/src/task/describe.rs`, `Describe`'s
  capabilities reply: members, schemas, worked examples.
- `crates/tinycomputer-bus/src/agent/types/`, every payload type named on
  this page.
- `crates/tinycomputer-core/src/facts/mod.rs`, `Facts`, `is_sensitive_name`,
  `redact`, `mask`.
- [`docs/technical/tasks.md`](../../technical/tasks.md), the formal contract
  (note: check the code, not that document, for exact card-data and
  `PaymentMode` behavior, this page and the code are current).
- [`docs/technical/specs/task-rescue.md`](../../technical/specs/task-rescue.md).
- [`docs/technical/specs/task-output.md`](../../technical/specs/task-output.md),
  the shaper and the run memory carried across a task's runs.
