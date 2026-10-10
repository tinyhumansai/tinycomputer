# The Agent and task types

Source: [`crates/tinycomputer-bus/src/agent/`](../../../crates/tinycomputer-bus/src/agent/)

This is the surface meant for an external agent (typically a language model
with tools attached) to drive. It is deliberately smaller and plainer than
the desktop and browser interfaces underneath it: hand over a job, ask how it
is going, answer what it asks for, until it is done. For the story of using
this interface (not its wire types), see
[Giving it a task](../../giving-it-a-task.md).

## Why this interface looks the way it does

The desktop and browser interfaces speak in refs, selectors, and coordinates,
because that is the right vocabulary for driving one screen. A caller
handing over a whole job does not want to think in those terms, and should
not have to know that "book a flight" turns into forty small decisions
across two applications. The Agent interface is built to be driven by a
model with only a few members, one reply shape, and state that explains
itself.

There are few members: call `Describe` once, then loop over `StartTask`,
`AwaitTask`, and `ContinueTask` until the status is final. There is one
reply shape: every member answers `AgentResponse<T>`, and a failed call
carries an `AgentError` with a stable code, a one-sentence message, a
one-sentence hint, and whether retrying could help. The state explains
itself: a `TaskView` says in one sentence what is happening, which member
calls make sense right now, and, inside `TaskStatus`, exactly what the task
needs from the caller. There are no refs, selectors, or coordinates at this
level; those stay private to the flow a task runs underneath. And shared
facts are briefed by value while secret facts stay templates: a traveller's
name briefs the decision model so it knows who it is booking for, but a card
number is only ever shown to it as `${name}`.

## The members

Source: [`crates/tinycomputer-bus/src/agent/names/mod.rs`](../../../crates/tinycomputer-bus/src/agent/names/mod.rs)

| Member | Takes | Returns |
|---|---|---|
| `Describe` | nothing | `Capabilities`: surfaces available, whether Jev and the planner are configured, the flow guide, every task member's JSON Schema, worked examples, and a `catalogue` of all 80 members |
| `PlanTask` | `PlanTaskRequest` | `TaskPlan`: a drafted flow, without touching anything |
| `StartTask` | `StartTaskRequest` | `TaskView`, at once |
| `AwaitTask` | `AwaitTaskRequest` | `TaskView`, once something changes or the timeout passes |
| `ContinueTask` | `ContinueTaskRequest` | `TaskView`, resumed |
| `CancelTask` | `TaskRef` | `TaskView`; cancelling an already-finished task succeeds |
| `TaskReport` | `TaskRef` | `TaskReport`: the full record, steps, records, artifacts, rescues |
| `ListTasks` | nothing | every `TaskView` this module holds, newest first |

`StartTask`, `ContinueTask`, and `TaskReport` are listed in
`agent::names::CONFIDENTIAL`: their frames carry facts or page data and must
travel with confidential bus delivery.

## Starting a task

```json
{
  "task": "Find the cheapest flight from Delhi to Srinagar on 14 October and fill in my details up to payment",
  "facts": {"first name": "Asha", "last name": "Raina", "email": "asha@example.com"},
  "constraints": {"surfaces": ["browser"]}
}
```

Give either `task` (plain language, needs the planner configured, or the
task immediately pauses with `needs_plan`) or `flow` (a `Flow` the caller
wrote itself, from `Describe`'s guide). Giving both means "run this flow; the
text explains what it is for."

`facts` are values the task may type: name, date of birth, email, card
number. A shared fact briefs the decision model by its actual value.
`secret_facts` names which of those `facts` to keep secret on top of the ones
the module already recognizes as sensitive by name (a field called
"password," a card number by shape). Naming a secret fact that is not in
`facts` is refused outright, so a typo can never leave a value shared by
accident. A secret's value only ever reaches the decision model as
`${name}`, exactly as described for flows in
[Writing flows](flows.md#secrets-named-everywhere-except-where-it-counts):
this is the same rule, enforced by the same masking, one level up.

### Constraints: where a task may act, and how far it goes on payment

```rust,ignore
pub struct TaskConstraints {
    pub payment: PaymentMode,
    pub surfaces: Vec<SurfaceKind>,       // empty means every available one
    pub origins: Vec<String>,             // pages only; `*` any public site; empty means any
    pub allow_destructive: bool,
    pub browser_endpoint: Option<String>, // attach to the caller's own Chrome
    pub headed: bool,
    pub browser_executable: Option<String>, // the binary to launch (2.10)
    pub browser_profile: Option<String>,    // an absolute profile folder kept between tasks (2.10)
}
```

`PaymentMode` is the one place this crate makes an explicit promise about
money: `StopAtPayment` (the default) always stops at the step that pays and
hands it back as a final checkpoint; `FillThenApprove` will fill a payment
form from secret facts and then pause as `needs_approval` in front of the
control that actually pays, and only an explicit `ContinueTask.approve`
presses it. `FillThenApprove` requires `origins` to be set, so card details
are only ever typed on sites the caller has named. Paying is never automatic
under either mode; the mode only changes how far the task gets before
handing control back. See [Safety and privacy](../../safety-and-privacy.md)
for the fuller picture of what does and does not get automated here.

### Budgets

`TaskBudget` mirrors the levers `RunFlowRequest` exposes (see
[Writing flows](flows.md)) but every field is `Option`, so an unset one takes
the module's own default rather than the crate baking one in twice:
`max_actions`, `max_model_calls`, `votes`, `strategy`, `deliberation`,
`max_elapsed_ms` (excludes time spent waiting on the caller), and
`max_rescues`, how many times a failed step may be handed to a reasoning
model for guidance before the task gives up on it. Absent means the module's
default, currently 5 (also the maximum); `0` turns rescues off. See
[Rescue](../../rescue.md) for what a rescue actually does.

### Asking for a shaped answer

`StartTaskRequest.output`, when set, changes what a finished task hands
back. Without it, a task ends with only its raw `records`, whatever
`extract` and `pick` steps collected along the way. With it:

```rust,ignore
pub struct TaskOutput {
    pub instructions: String,   // plain language: which records, how many, in what order
    pub schema: Option<Value>,  // a JSON Schema the result must satisfy
}
```

a finished task hands what its steps read to one extra reasoning-model pass,
and that pass's answer comes back as `TaskStatus::Done.result`, checked
against `schema` if one was given. The schema is deliberately restricted to a
small, checkable subset of JSON Schema, `type`, `properties`, `required`,
`additionalProperties` (must be a boolean), `items`, `enum`, `minItems`,
`maxItems`, `description`, `title`, and a schema using any keyword outside
that list is refused at `StartTask` time, before the task ever runs, rather
than discovered as a broken result at the end. `Capabilities.output_configured`
says whether this feature is available at all on the loaded module.

## Following a task

`AwaitTaskRequest { id, timeout_ms }` (default 30 seconds, capped by the
module) waits until the task's state changes or the timeout passes, whichever
is first, so a caller polls without either hammering the module or blocking
indefinitely.

Every `TaskView` answers three questions:

```rust,ignore
pub struct TaskView {
    pub id: TaskId,
    pub status: TaskStatus,
    pub summary: String,      // one sentence, for a model or a person
    pub step: Option<StepView>,
    pub progress: f32,        // 0 to 1
    pub next: Vec<String>,    // the member calls that make sense right now
}
```

`next` is worth building around: a caller that only ever calls what `next`
suggests cannot get the protocol wrong, because the module is the one
deciding what a legal next move is, not a client-side state machine that has
to be kept in sync with this crate by hand.

### `TaskStatus`: everything a task can be waiting on

```rust,ignore
pub enum TaskStatus {
    Running,
    NeedsInput { fields: Vec<InputField> },
    NeedsApproval { action: String, target: String, screenshot: Option<OutputRef> },
    Checkpoint { reason: String, location: String, screenshot: Option<OutputRef>, summary: String, continuable: bool },
    NeedsHuman { reason: String, screenshot: Option<OutputRef> },
    NeedsPlan { guide: String },
    Done { answer: String, records: BTreeMap<String, Vec<BTreeMap<String, String>>>, result: Option<Value> },
    Failed { step: Option<usize>, reason: String, hint: String, recoverable: bool },
    Cancelled,
}
```

Each variant pairs with one clear response:

- **`NeedsInput`** lists `InputField`s (`name`, `why`, `kind`, and `options`
  for a choice), answer with `ContinueTask.inputs`, keyed by `name`.
- **`NeedsApproval`** describes one pending irreversible action and the
  control that would trigger it, answer with `ContinueTask.approve`.
- **`Checkpoint`** is where a task always stops on reaching a payment page,
  and `continuable` says whether `approve` can take it any further (a
  payment checkpoint cannot be). A screenshot lets a person check the state
  before deciding.
- **`NeedsHuman`** is for anything only a person can do, a captcha, a
  login, a one-time code, answered with `ContinueTask.answer = "done"` once
  it is handled.
- **`NeedsPlan`** means a plain-language `task` arrived with no planner
  configured; it hands back the flow guide so the caller can write one and
  start again with `StartTask.flow`.
- **`Done`** carries `answer` (plain words), `records` (raw extracted rows),
  and `result` (the shaped answer from `StartTaskRequest.output`, when one
  was asked for).
- **`Failed`** names the step, the reason, a `hint` about what to change, and
  whether trying again with that change could succeed.

`TaskStatus::is_final()` is `true` for `Done`, `Failed`, `Cancelled`, and a
non-continuable `Checkpoint`, the states a caller should stop polling on.

## Answering a pause

```rust,ignore
pub struct ContinueTaskRequest {
    pub id: TaskId,
    pub inputs: BTreeMap<String, String>, // for NeedsInput
    pub approve: Option<bool>,            // for NeedsApproval, or true past a non-payment Checkpoint
    pub answer: Option<String>,           // free text, or "done" for NeedsHuman
}
```

## Planning without running

`PlanTaskRequest { task, fact_names, secret_facts, surfaces }` drafts a
`TaskPlan`, a `Flow` plus `questions` (facts the flow needs that the caller
did not name) and `notes` (assumptions the planner made), without acting on
anything. `fact_names` alone is enough to plan; values are not needed until
the task actually starts.

## The full record: `TaskReport`

```rust,ignore
pub struct TaskReport {
    pub view: TaskView,
    pub flow: Option<Flow>,
    pub steps: Vec<StepReport>,           // see Writing flows
    pub records: BTreeMap<String, Vec<BTreeMap<String, String>>>,
    pub artifacts: Vec<OutputRef>,        // best-effort screenshots from stopped runs; read with BrowserReadOutput
    pub learned: Vec<GroundingHint>,      // pass back as StartTask.memory next time
    pub trace: Vec<JevExchange>,          // only when StartTask.trace was set
    pub rescues: Vec<Rescue>,
}
```

When a run stops on its own — at a checkpoint, before an approval, at a
person's turn, at the end, or cut off by its time budget — the task tries to
take a screenshot of its surface before letting it go (best effort, within
ten seconds). `CancelTask` releases at once without one; take a
`BrowserScreenshot` first if you want the screen. It lands in `artifacts`
only — never on a status's `screenshot`, because a `TaskView` also travels
through `AwaitTask` and `ListTasks`, which are not confidential, and a held
output's id is all `BrowserReadOutput` needs. Read it with
`BrowserReadOutput` within five minutes, before it expires.

Each `Rescue` records one time a failed step was handed to a reasoning model
for a second opinion:

```rust,ignore
pub struct Rescue {
    pub step: usize,
    pub failure: String,
    pub reason: String,
    pub steps: Vec<FlowStep>,   // what the model put in place of the failed step; empty if it gave up
    pub covers: usize,          // how many of the following steps this guidance also handles
    pub outcome: RescueOutcome, // Running, Recovered, FailedAgain, GaveUp
}
```

`covers` matters for reading a report correctly: a rescue's replacement steps
can stand in for more than just the one step that failed, dropping some
number of steps that immediately followed it from the flow (never one
holding a `stop_before`, which always needs its own decision). See
[Rescue](../../rescue.md) for the fuller story of when a rescue is
attempted and how it decides to give up.

## `AgentResponse` and `AgentError`

Every member above answers `AgentResponse<T>`, structurally simple on
purpose:

```rust,ignore
pub struct AgentResponse<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<AgentError>,
}

pub struct AgentError {
    pub code: String,      // SCREAMING_SNAKE_CASE, e.g. NO_SUCH_TASK
    pub message: String,   // one sentence
    pub hint: String,      // one sentence, what to do about it
    pub recoverable: bool,
}
```

This is a different, plainer shape than `DesktopResponse`/`DesktopError` (see
[The envelope and errors](envelope-and-errors.md)): a task-level failure is
phrased for a model deciding whether to retry with a changed input, not for a
caller reasoning about delivery and retry-safety at the level of one desktop
command. Do not mix the two envelopes up; which one a member uses follows
directly from which interface it is on.

## `Describe`'s `Capabilities`

`Capabilities` is meant to be the only thing an agent reads before it starts
using this interface: which `SurfaceKind`s are available right now (and why
not, when one is missing, a permission, a browser), whether Jev and the
planner are configured, whether rescue is configured, whether shaped output
is configured, the flow step kinds, the flow guide itself, and, per member,
a `MemberDoc` with its JSON Schema for input and output plus a one-sentence
summary and whether it needs confidential delivery. `examples` are worked
requests ready to adapt rather than write from scratch.

`Capabilities.catalogue` is a `Vec<MemberSummary>` (source:
[`crates/tinycomputer-bus/src/catalogue/`](../../../crates/tinycomputer-bus/src/catalogue/)):
every one of the module's 80 members, task, flow, desktop, and browser alike,
each with its `Family`, a one-line summary of what it is for, and whether it
needs confidential delivery, in `names::METHODS` dispatch order. `members`
above already has the task members' full schemas; `catalogue` is the
lightweight map over everything else too, so a caller that has only ever
called `Describe` knows the desktop and browser primitives exist, and which
family to reach for, without a second round trip.
