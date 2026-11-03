//! The task controller behind the Agent interface.
//!
//! [`Tasks`] takes a task — a flow and the facts it may type — runs it in
//! the background, and reports it as a [`TaskView`] a model can act on. It
//! pauses only for what the caller must decide:
//!
//! - **missing values** — a `${name}` the flow uses and no fact supplies
//!   becomes `needs_input` before anything runs;
//! - **irreversible actions** — a `stop_before` the task may not perform
//!   becomes `needs_approval`, and approving it performs that action and
//!   carries on with the steps after it;
//! - **payment** — the control that pays is never pressed on its own: by
//!   default reaching it is a final checkpoint, and under
//!   [`PaymentMode::FillThenApprove`] the payment form is filled and pressing
//!   it waits for `needs_approval`.
//!
//! - **a failed step** — when a rescuer is configured, a step that fails is
//!   first handed to it ([`crate::Rescuer`]), up to five times a task: its
//!   steps run in place of the failed one and the task carries on. Only when
//!   it gives up, or the rescues are spent, does the task fail.
//!
//! Facts reach the flow as variables, so they are typed locally. Shared ones
//! also brief Jev by value ([`FlowBrief`]); secret ones reach Jev only as
//! `${name}`. The flow runs with `include_values` on, so Jev reads what a
//! field holds and can check what was typed; the flow runtime masks every
//! secret value, there too, before anything reaches Jev. Every summary is
//! redacted of every fact.
//!
//! How a flow actually runs is behind [`FlowRunner`], so this controller is
//! tested with scripted runs and the module plugs in the real flow runtime.
//!
//! The controller's API is in `controller`; each task's cell and state in
//! `store`; the background run in `drive`, capped by `budget` and briefed by
//! `brief`; answering a paused task in `resume`; rescuing a failed step in
//! `recovery`; spotting a wall only a person can pass in `human`; and what
//! the caller sees in `publish`; the screenshots a stopped run leaves in
//! `artifact`.

mod artifact;
mod brief;
mod browser;
mod budget;
mod cancel;
mod controller;
mod describe;
mod drive;
mod errors;
mod human;
mod interpret;
mod names;
mod page;
mod persistence;
mod plan_recovery;
mod publish;
mod recovery;
mod resume;
mod secret_host;
mod store;
mod timing;

use std::future::Future;
use std::pin::Pin;

use tinycomputer_bus::agent::{TaskConstraints, TaskId};
use tinycomputer_bus::browser::OutputRef;
use tinycomputer_bus::{DesktopResponse, Flow, RunFlowRequest};

/// Sanitizes every textual representation in a JSON report or model payload
/// through the task runner's host secret filter.
pub(super) fn redact_task_value(
    runner: &dyn FlowRunner,
    task: &TaskId,
    value: &mut serde_json::Value,
) {
    redact_value(value, &|text| runner.redact(task, text));
}

/// Serializes, sanitizes and rebuilds a task model payload or report. Returns
/// `None` when the redacted representation no longer fits its declared type.
pub(super) fn redact_task_data<T>(runner: &dyn FlowRunner, task: &TaskId, data: T) -> Option<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut value = serde_json::to_value(data).ok()?;
    redact_task_value(runner, task, &mut value);
    serde_json::from_value(value).ok()
}

fn redact_value(value: &mut serde_json::Value, redact: &impl Fn(&str) -> String) {
    match value {
        serde_json::Value::String(text) => *text = redact(text),
        serde_json::Value::Number(number) => {
            let text = number.to_string();
            let masked = redact(&text);
            if masked != text {
                *value = serde_json::Value::String(masked);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_value(item, redact);
            }
        }
        serde_json::Value::Object(fields) => {
            let mut safe_fields = serde_json::Map::new();
            for (key, mut item) in std::mem::take(fields) {
                redact_value(&mut item, redact);
                let base = redact(&key);
                let mut safe_key = base.clone();
                let mut suffix = 2_u32;
                while safe_fields.contains_key(&safe_key) {
                    safe_key = format!("{base}#{suffix}");
                    suffix = suffix.saturating_add(1);
                }
                safe_fields.insert(safe_key, item);
            }
            *fields = safe_fields;
        }
        _ => {}
    }
}

pub use browser::BrowserRunner;
pub use budget::{HostWorkGuard, TaskBudgetScope, TaskCallBudget};
pub use controller::{ContinuePaidAtShopRequest, ContinuePaymentRequest, Tasks};
pub use describe::capabilities;
pub(crate) use names::input_kind;
pub use persistence::PausedTaskError;
pub use secret_host::{SecretHost, SecretTypeRequest};

/// The future a [`FlowRunner`] returns: the flow runtime's reply envelope.
pub type FlowFuture = Pin<Box<dyn Future<Output = DesktopResponse> + Send>>;

/// The future [`FlowRunner::visible_text`] returns.
pub type TextFuture = Pin<Box<dyn Future<Output = Vec<String>> + Send>>;

/// The HTTPS origin of the browser page a running task currently holds.
pub type PageOriginFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;

/// The future [`FlowRunner::capture`] returns: a held screenshot, if the
/// task's surface could take one.
pub type CaptureFuture = Pin<Box<dyn Future<Output = Option<OutputRef>> + Send>>;

/// The future [`FlowRunner::prepare`] returns, once the surfaces are ready
/// or could not be made so; a task goes on either way.
pub type PrepareFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The remaining work a host may try after Jev and its rescuer cannot finish
/// one top-level step.
#[derive(Debug, Clone)]
pub struct FailedStepRecovery {
    /// The task whose browser session is still open.
    pub task: TaskId,
    /// Redacted context for the failed step.
    pub briefing: crate::Briefing,
    /// The task's original browser and payment constraints.
    pub constraints: TaskConstraints,
    /// Actions left in the task budget.
    pub remaining_actions: u32,
    /// Jev evaluations left in the task budget.
    pub remaining_model_calls: u32,
    /// Time left in the task budget, when bounded.
    pub remaining_ms: Option<u64>,
    /// The shared task budget for host model calls and active recovery time.
    pub host_budget: TaskBudgetScope,
}

/// What a host's bounded failed-step recovery did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedStepRecoveryResult {
    /// The host completed only the failed step; `TinyComputer` may run its tail.
    Completed {
        /// Why the host considers that step complete.
        reason: String,
        /// Browser actions spent while recovering it.
        actions: u32,
        /// Model calls spent while recovering it.
        model_calls: u32,
    },
    /// The host could not safely complete the step.
    GiveUp {
        /// Why recovery stopped.
        reason: String,
    },
}

/// The future returned by [`FlowRunner::recover_failed_step`].
pub type FailedStepRecoveryFuture = Pin<Box<dyn Future<Output = FailedStepRecoveryResult> + Send>>;

/// The task and constraints a host needs to recover an unavailable plan.
#[derive(Debug, Clone)]
pub struct PlanRecoveryRequest {
    /// The task whose browser session is ready for the plan request.
    pub task: TaskId,
    /// The caller's task description.
    pub task_text: String,
    /// The guide the caller would otherwise use to supply a flow.
    pub guide: String,
    /// The task's original browser and payment constraints.
    pub constraints: TaskConstraints,
    /// Names the plan may reference; values are never included.
    pub known_names: Vec<String>,
    /// Secret names the flow may reference as handles.
    pub secret_names: Vec<String>,
    /// Actions left in the task budget.
    pub remaining_actions: u32,
    /// Jev evaluations left in the task budget.
    pub remaining_model_calls: u32,
    /// Time left in the task budget, when bounded.
    pub remaining_ms: Option<u64>,
    /// The shared task budget for host model calls and active planning time.
    pub host_budget: TaskBudgetScope,
}

/// A same-task plan the host derived, or why it could not derive one.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanRecoveryResult {
    /// A caller-validated flow to install on the existing task.
    Plan(Flow),
    /// No safe plan could be supplied.
    GiveUp(String),
}

/// The future returned by [`FlowRunner::recover_plan`].
pub type PlanRecoveryFuture = Pin<Box<dyn Future<Output = PlanRecoveryResult> + Send>>;

/// Runs a task's flows, on surfaces that live as long as the task.
pub trait FlowRunner: Send + Sync + 'static {
    /// Runs `request` for `task` within `constraints`, returning `RunFlow`'s
    /// reply. Runs of one task share its surfaces, so a continuation picks up
    /// on the page or window the last run left.
    fn run(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
    ) -> FlowFuture;

    /// Returns only the current browser origin, with path, query and
    /// fragment omitted. A same-task planner handoff uses this as location
    /// context without reading page contents.
    fn page_origin(&self, _task: &TaskId) -> PageOriginFuture {
        Box::pin(async { None })
    }

    /// Runs a task flow with its usage recorder attached to model calls.
    ///
    /// Runners that do not account for model usage may keep the default. A
    /// runner backed by [`JevRuntime`](crate::JevRuntime) should override
    /// this method and attach `usage` to the cloned runtime before running.
    fn run_with_usage(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
        _usage: std::sync::Arc<crate::UsageRecorder>,
    ) -> FlowFuture {
        self.run(task, constraints, request)
    }

    /// Attempts a bounded recovery of one failed top-level step. The caller
    /// invokes this only after its configured Jev rescuer gives up or is
    /// unavailable. The default preserves the task's failure.
    fn recover_failed_step(&self, _request: FailedStepRecovery) -> FailedStepRecoveryFuture {
        Box::pin(async {
            FailedStepRecoveryResult::GiveUp {
                reason: "the host has no failed-step recovery".to_owned(),
            }
        })
    }

    /// Whether the host can asynchronously write a plan for a task that has
    /// no planner configured. Defaults to `false` so existing hosts keep the
    /// immediate `needs_plan` behavior.
    fn plan_recovery_enabled(&self) -> bool {
        false
    }

    /// Writes a validated flow for the existing task. The default keeps the
    /// task at `needs_plan`.
    fn recover_plan(&self, _request: PlanRecoveryRequest) -> PlanRecoveryFuture {
        Box::pin(async {
            PlanRecoveryResult::GiveUp("the host cannot write a task plan".to_owned())
        })
    }

    /// The visible text of the task's surface now, to spot a wall only a
    /// person can pass. Empty by default.
    fn visible_text(&self, _task: &TaskId) -> TextFuture {
        Box::pin(async { Vec::new() })
    }

    /// A screenshot of the task's surface as it stands, held for the caller
    /// to read. Taken whenever a run stops on its own — at a checkpoint,
    /// before an approval, at a person's turn, finished, failed, or cut off
    /// by its time budget — before the task's surfaces are released, so a
    /// finished or failed task still leaves one. Not for `needs_input` or
    /// `needs_plan`: those are decided before a run starts, with nothing on
    /// screen yet that the task did.
    /// `CancelTask` releases at once without one: the caller chose to stop,
    /// and can take its own with `BrowserScreenshot` first. `None` by
    /// default, and whenever the surface cannot take one.
    fn capture(&self, _task: &TaskId) -> CaptureFuture {
        Box::pin(async { None })
    }

    /// Lets go of whatever the task held, once it has ended.
    fn release(&self, _task: &TaskId) {}

    /// Gets the task's surfaces ready while its plan is drafted, so its
    /// first step does not wait for them: called alongside the planner for
    /// a task that runs on the browser alone. [`FlowRunner::release`] may
    /// run while the future is in flight, or after it was dropped with a
    /// cancelled task: what it opens then must be let go too. Does nothing
    /// by default.
    fn prepare(&self, _task: &TaskId, _constraints: &TaskConstraints) -> PrepareFuture {
        Box::pin(async {})
    }

    /// Loads `url`, the one web page the task's text names, in the browser
    /// [`FlowRunner::prepare`] got ready, while the plan is drafted: a first
    /// step that browses there finds it loaded. Called after `prepare`, in
    /// the same wait, for a task that runs on the browser alone. Does
    /// nothing by default.
    fn open_page(&self, _task: &TaskId, _url: &str) -> PrepareFuture {
        Box::pin(async {})
    }

    /// Opens the connections the task's first decision, asked `votes` ways,
    /// will use, while its plan is drafted (`JevRuntime::warm`). Started
    /// with the planner for every task and never waited for: the task goes
    /// on whether it finishes or not. Does nothing by default.
    fn warm(&self, _task: &TaskId, _votes: u32) -> PrepareFuture {
        Box::pin(async {})
    }

    /// Warms Jev while the task is planned, recording those calls in the
    /// same task tally. Runners that warm the model override this method.
    fn warm_with_usage(
        &self,
        task: &TaskId,
        votes: u32,
        _usage: std::sync::Arc<crate::UsageRecorder>,
    ) -> PrepareFuture {
        self.warm(task, votes)
    }

    /// Writes an `event` of the time a task spends outside its flows
    /// (`plan`, `rescue`, `resume`) to the debug journal: the task's own,
    /// or for `PlanTask`, which plans before any task exists (`task` is
    /// `None`), a run of its own. `fields` is only called when the event is
    /// written: nothing is built by default, or when the journal is off.
    fn journal(
        &self,
        _task: Option<&TaskId>,
        _event: &str,
        _fields: &dyn Fn() -> serde_json::Value,
    ) {
    }

    /// Replaces host-held secret values in text before a task snapshot is
    /// emitted. Runners without host-side secrets may keep the default.
    fn redact(&self, _task: &TaskId, text: &str) -> String {
        text.to_owned()
    }
}

/// How many tasks the controller holds; finished ones are dropped first.
pub const MAX_TASKS: usize = 32;

/// The longest a single `AwaitTask` waits.
pub const MAX_AWAIT_MS: u64 = 60_000;

/// Jev evaluations a task may spend when its budget does not say. Jev is
/// cheap, so this is generous: every decision is voted on several ways.
pub(crate) const DEFAULT_MODEL_CALLS: u32 = 6000;

/// How many ways each decision is asked when a task's budget does not say.
pub(crate) const DEFAULT_VOTES: u32 = 7;

/// The longest one rescue may think before the task fails without it.
pub(crate) const RESCUE_TIMEOUT_MS: u64 = 120_000;

/// The longest the shaping pass may take before the task fails without its
/// result.
pub(crate) const SHAPE_TIMEOUT_MS: u64 = 120_000;

#[cfg(test)]
mod task_tests;
