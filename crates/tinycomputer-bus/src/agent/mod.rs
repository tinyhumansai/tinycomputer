//! The Agent interface: hand over a task, follow it, answer what it asks.
//!
//! This is the surface an external agent is meant to use. It takes a task —
//! in plain language, or as a high-level [`Flow`](crate::Flow) — and runs it
//! end to end across the desktop and the browser, pausing only for what the
//! caller must decide: missing values, approvals, a person's hand at a captcha,
//! and always before paying.
//!
//! # Designed to be driven by a model
//!
//! - **Few members.** `Describe` once; then `StartTask`, `AwaitTask`, and
//!   `ContinueTask` until the status is final.
//! - **One reply shape.** Every member answers an [`AgentResponse`]; a failed
//!   call carries an [`AgentError`] with a stable code, a one-sentence message,
//!   a one-sentence hint, and whether retrying can help.
//! - **Self-describing state.** A [`TaskView`] says in one sentence what is
//!   happening, which member calls make sense now, and — in [`TaskStatus`] —
//!   exactly what the task needs.
//! - **No refs, selectors, or coordinates** at this level. Those belong to the
//!   desktop and browser members.
//! - **Shared facts brief, secret facts stay templates.** A traveller's name
//!   and date of birth brief the decision model, so it knows whom it books
//!   for; a card or passport number is only ever shown to a model as
//!   `${name}` and typed locally.
//!
//! ```
//! use tinycomputer_bus::agent::{StartTaskRequest, TaskStatus, TaskView, TaskId};
//!
//! let request: StartTaskRequest = serde_json::from_value(serde_json::json!({
//!     "task": "Find the cheapest flight from Delhi to Srinagar on 14 October and fill in my details up to payment",
//!     "facts": {"first name": "Asha", "last name": "Raina", "email": "asha@example.com"},
//!     "constraints": {"surfaces": ["browser"]}
//! }))?;
//! assert!(!request.constraints.allow_destructive);
//!
//! let view: TaskView = serde_json::from_value(serde_json::json!({
//!     "id": "t-1",
//!     "status": {"state": "needs_input", "fields": [
//!         {"name": "date of birth", "why": "the traveller form requires it", "kind": "date"}
//!     ]},
//!     "summary": "Filled the traveller form; it also needs a date of birth.",
//!     "progress": 0.7,
//!     "next": ["ContinueTask"]
//! }))?;
//! assert!(matches!(view.status, TaskStatus::NeedsInput { .. }));
//! assert_eq!(view.id, TaskId::new("t-1"));
//! # Ok::<(), serde_json::Error>(())
//! ```

pub mod names;
mod types;

pub use types::{
    AGENT_ENVELOPE_VERSION, AgentError, AgentResponse, AgentResponseError, AwaitTaskRequest,
    Capabilities, ContinueTaskRequest, Example, HostContext, InputField, InputKind,
    LanguageModelConfiguration, LanguageModelProvider, MemberDoc, ModelUsage, PaymentMode,
    PlanTaskRequest, Rescue, RescueOutcome, SecretHandle, StartTaskRequest, StepView,
    SurfaceAvailability, SurfaceKind, TaskBudget, TaskConstraints, TaskId, TaskOutput, TaskPlan,
    TaskRef, TaskReport, TaskReportRequest, TaskStatus, TaskUsage, TaskView,
};

#[cfg(test)]
mod agent_tests;
