//! The tinycomputer agent runtime.
//!
//! This crate composes the browser and optional desktop surface adapters with
//! Jev, `TypeSafe`'s decision model, into loops an embedding host can call
//! directly or serve through the module:
//!
//! - [`resolve_intent`] grounds one described element and optionally acts on
//!   it;
//! - [`run_goal`] runs a bounded, scoped goal with visible success predicates;
//! - [`run_flow`] runs a high-level, UI-agnostic intent flow, grounding each
//!   step on the live screen with small Jev questions
//!   (`docs/technical/specs/jev-intent-flows.md`);
//! - [`validate_flow`] and [`flow_guide`] check and document flows without
//!   touching the desktop.
//!
//! Every decision-loop entry point returns a [`DesktopResponse`] envelope.
//! A [`JevRuntime`] is built once from the host's configuration and cloned per
//! call.
//!
//! [`Tasks`] is the controller behind the Agent interface: it runs a task's
//! flow in the background and reports it as a status a model can act on,
//! pausing for missing values, irreversible actions, and always at payment.
//! With a [`Rescuer`], a failed step is first handed to a reasoning model
//! for guidance, up to five times a task (`docs/technical/specs/task-rescue.md`).
//!
//! A [`Workspace`] joins desktop and browser when the default `desktop`
//! feature is enabled. A [`BrowserRunner`] runs the public [`Tasks`] manager
//! directly on browser sessions without `TinyBus` or a desktop adapter.
//!
//! The crate holds no bus: `tinycomputer` may serve these functions over
//! `TinyBus`, while embedding hosts can call the runner and task manager in
//! process.

mod agentic;
mod planner;
mod rescue;
mod shape;
mod task;
mod usage;
mod workspace;

pub(crate) use agentic::run_flow_with_host;

#[cfg(all(test, feature = "planner"))]
pub(crate) static MODEL_NETWORK_TEST_MUTEX: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

pub use agentic::{
    JOURNAL_DEFAULT_DIR, JOURNAL_ENV, JOURNAL_FILE, JevRuntime, flow_guide, run_flow, validate_flow,
};
#[cfg(feature = "desktop")]
pub use agentic::{resolve_intent, run_goal};
pub use planner::{
    Completion, LanguageModel, ModelFailure, ModelReply, ModelUse, Planner, REPAIRS,
    ReportedCompletion, ReportedUsage, Role, Turn,
};
#[cfg(feature = "planner")]
pub use planner::{
    HostModelConfig, HostModels, ModelRoute, OPEN_ROUTER_BASE_URL, OUTPUT_MODEL, PLANNER_MODEL,
    PlannerConfig, RESCUE_MODEL, TINYHUMANS_BASE_URL, open_router, open_router_rescuer,
    open_router_shaper, openai_compatible_models,
};
pub use rescue::{Briefing, Guidance, MAX_RESCUE_STEPS, MAX_RESCUES, Rescuer, SCREEN_CHARS};
pub use shape::{Harvest, RECORDS_CHARS, Shaper};
pub use task::{
    BrowserRunner, CaptureFuture, ContinuePaidAtShopRequest, ContinuePaymentRequest,
    FailedStepRecovery, FailedStepRecoveryFuture, FailedStepRecoveryResult, FlowFuture, FlowRunner,
    HostWorkGuard, MAX_AWAIT_MS, MAX_TASKS, PageOriginFuture, PausedTaskError, PlanRecoveryFuture,
    PlanRecoveryRequest, PlanRecoveryResult, PrepareFuture, SecretHost, SecretTypeRequest,
    TaskBudgetScope, TaskCallBudget, Tasks, TextFuture, capabilities,
};
pub use tinycomputer_bus::DesktopResponse;
#[cfg(feature = "desktop")]
use tinycomputer_desktop::Desktop;
pub use usage::{ModelRole, UsageRecorder};
pub use workspace::{BROWSER, Workspace};
