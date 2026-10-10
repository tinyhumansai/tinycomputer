//! What a caller asks of a task: identity, start, limits, await, continue, and plan.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::InputField;
use crate::flow::{Flow, GroundingHint};

/// A task's identity, handed out by `StartTask`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

impl TaskId {
    /// A task id from its string form.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Where a task may act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// Desktop applications, through the accessibility tree.
    Desktop,
    /// Web pages, through a browser session.
    Browser,
}

/// `StartTask`: what to accomplish, with what, and within what limits.
///
/// Give either `task` (plain language; needs the planner configured, or the
/// task pauses with `needs_plan`) or `flow` (a high-level flow the caller
/// wrote from `Describe`'s guide). Both together means "run this flow; the
/// task text explains it".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartTaskRequest {
    /// The goal in plain language.
    pub task: Option<String>,
    /// A flow to run instead of planning one.
    pub flow: Option<Flow>,
    /// Values the task may type, by name — traveller name, date of birth,
    /// email, phone, card number.
    ///
    /// Shared ones brief Jev by value, so it knows whom it is booking for.
    /// Secret ones — those named in `secret_facts`, those whose name labels a
    /// card, a password, a one-time code, or an identity or account number,
    /// and any value that is a card number — are templates: Jev and the
    /// planner only ever see `${name}`, and the value is typed locally.
    pub facts: BTreeMap<String, String>,
    /// Names among `facts` to keep secret on top of the ones recognised as
    /// sensitive. A name here that is not in `facts` is refused, so a typo
    /// cannot leave a value shared.
    pub secret_facts: Vec<String>,
    /// Where the task may act and what it may commit to.
    pub constraints: TaskConstraints,
    /// Upper bounds on the work a task may do.
    pub budget: TaskBudget,
    /// Grounding hints from an earlier run, so this one reads less.
    pub memory: Vec<GroundingHint>,
    /// Record every Jev exchange for `TaskReport`.
    pub trace: bool,
    /// The shape the caller wants the answer in. When set, a finished task
    /// hands what its steps read to one reasoning-model pass that returns
    /// JSON in this shape, as `done.result`; absent, a task ends with its
    /// records only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<TaskOutput>,
}

/// What a finished task returns, beyond its raw records.
///
/// The result is always a JSON object. `schema` is a JSON Schema it must
/// satisfy, whose top-level `type`, if given, is `object`, from the subset
/// `type`, `properties`, `required`, `additionalProperties` (a boolean),
/// `items`, `enum`, `minItems`, `maxItems`, `description`, and `title`; a
/// schema using any other keyword is refused when the task starts, so a
/// result that comes back has been checked against every rule given.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskOutput {
    /// What to return, in plain language: which records to keep, how many,
    /// in what order, and how to name things.
    pub instructions: String,
    /// The JSON Schema of the result; absent means any JSON object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// Where a task may act and what it may commit to.
///
/// Paying is never automatic: [`PaymentMode`] says only how far a task goes
/// before handing payment back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskConstraints {
    /// How far the task goes on a payment page.
    pub payment: PaymentMode,
    /// Surfaces the task may use; empty means every available one.
    pub surfaces: Vec<SurfaceKind>,
    /// Origins the task's browser may open pages on, such as
    /// `https://.makemytrip.com` for a site and its subdomains, or `*` for
    /// any public site (private and local addresses stay refused); empty
    /// means any. Only pages are checked, never the files a page loads from
    /// other hosts.
    pub origins: Vec<String>,
    /// Perform irreversible actions (send, delete, confirm a booking) without
    /// pausing for approval.
    pub allow_destructive: bool,
    /// Attach the task's browser work to an existing browser at this `DevTools`
    /// endpoint, such as the user's own signed-in Chrome.
    pub browser_endpoint: Option<String>,
    /// Show the browser rather than running it headless.
    pub headed: bool,
    /// The absolute path of the Chrome or Chromium binary the task's browser
    /// launches, in place of the module's configured one. A host's setting:
    /// a host that relays a model's request never takes it from the model.
    /// Not with `browser_endpoint`, which launches nothing.
    pub browser_executable: Option<String>,
    /// An absolute folder the task's browser keeps its profile in between
    /// tasks, so a site signed into once stays signed in. Absent, each task
    /// starts in a fresh profile removed when it ends. A host's setting, like
    /// `browser_executable`, and not with `browser_endpoint`. One browser can
    /// hold a folder at a time: a task started on a folder another task's
    /// browser still holds fails when its browser starts, so a host cancels
    /// the earlier task first.
    pub browser_profile: Option<String>,
}

/// How far a task goes on a payment page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMode {
    /// Stop at the step that pays and hand it back as a final checkpoint.
    #[default]
    StopAtPayment,
    /// Fill the payment form from secret facts, then pause as
    /// `needs_approval` in front of the control that pays: only
    /// `ContinueTask.approve` presses it. Needs `origins`, so card details
    /// are only typed on sites the caller named.
    FillThenApprove,
}

/// Upper bounds on a task. Unset fields take the module's defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskBudget {
    /// Actions across every surface.
    pub max_actions: Option<u32>,
    /// Jev evaluations. Every framing of a voted decision counts as one. A
    /// task given this cap does not warm Jev's connections while it is
    /// planned, as warming them would spend calls the cap does not count.
    pub max_model_calls: Option<u32>,
    /// How many ways each decision is asked before its answers are averaged;
    /// the module's default when unset.
    pub votes: Option<u32>,
    /// How each flow run asks its decisions; the narrow strategy when unset.
    pub strategy: Option<crate::FlowStrategy>,
    /// How much each flow run deliberates before acting; the deep level
    /// when unset.
    pub deliberation: Option<crate::Deliberation>,
    /// Wall-clock time, excluding time spent waiting for the caller.
    pub max_elapsed_ms: Option<u64>,
    /// How many times a failed step may be rescued by the reasoning model
    /// before the task fails; the module's default (5, also the most) when
    /// unset, and `0` turns rescues off.
    pub max_rescues: Option<u32>,
}

/// `AwaitTask`: wait for a task to need something or finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwaitTaskRequest {
    /// The task.
    pub id: TaskId,
    /// The longest to wait; a view comes back sooner when the task changes
    /// state. Capped by the module.
    #[serde(default = "default_await_ms")]
    pub timeout_ms: u64,
}

const fn default_await_ms() -> u64 {
    30_000
}

/// `ContinueTask`: answer what a paused task asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContinueTaskRequest {
    /// The task.
    pub id: TaskId,
    /// Values for a `needs_input` pause, by the field names it listed. Added
    /// to the task's facts.
    pub inputs: BTreeMap<String, String>,
    /// The decision on a `needs_approval` pause, or `true` to go past a
    /// non-payment `checkpoint`.
    pub approve: Option<bool>,
    /// A free-text answer, for a pause that asked a question — or, for
    /// `needs_human`, `"done"` once the person has finished.
    pub answer: Option<String>,
}

/// A task named in a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRef {
    /// The task.
    pub id: TaskId,
}

/// `TaskReport`: which task, and whether to include its Jev exchanges.
///
/// Not a [`TaskRef`], for a reason on the wire: `TaskReport` is confidential,
/// and a `TinyBus` client refuses to send a confidential body holding an
/// object whose only field is a string `id`, because that is the shape of a
/// stream handle, whose bytes would travel unprotected. `trace` is always
/// serialized, so the request never has that shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReportRequest {
    /// The task.
    pub id: TaskId,
    /// Include every Jev exchange, when `StartTask.trace` recorded them. A
    /// traced report can run to megabytes; `false` leaves them out.
    #[serde(default = "include_trace")]
    pub trace: bool,
}

impl TaskReportRequest {
    /// The full report of `id`, trace included.
    #[must_use]
    pub fn new(id: TaskId) -> Self {
        Self { id, trace: true }
    }
}

const fn include_trace() -> bool {
    true
}

/// `PlanTask`: draft a flow without acting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanTaskRequest {
    /// The goal in plain language.
    pub task: String,
    /// The names of the facts the caller can supply — values are not needed
    /// to plan.
    pub fact_names: Vec<String>,
    /// Names among `fact_names` to keep secret, on top of the ones
    /// recognised as sensitive: the plan may only type them.
    pub secret_facts: Vec<String>,
    /// Surfaces to plan for; empty means every available one.
    pub surfaces: Vec<SurfaceKind>,
}

/// A drafted plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskPlan {
    /// The flow the task would run.
    pub flow: Flow,
    /// Facts the flow uses that the caller did not name, to collect first.
    pub questions: Vec<InputField>,
    /// Assumptions the planner made, in plain words.
    pub notes: Vec<String>,
}
