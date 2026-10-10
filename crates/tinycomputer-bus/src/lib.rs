//! Every type that crosses the tinycomputer module's `TinyBus` boundary, and the
//! names of the members that carry them.
//!
//! tinycomputer exposes native desktop automation — the accessibility tree of
//! any running application, and the actions that drive it — as a loadable
//! `TinyBus` module. `crates/tinycomputer` is built as a `cdylib` and serves one
//! object. A host that loads that binary can call into it but cannot `use`
//! anything out of it, so the payload vocabulary has to be published as an
//! ordinary library. This is that library.
//!
//! # What is here
//!
//! - [`names`] — the interface name, the object path, and one constant per
//!   member, plus [`names::METHODS`] listing all ninety in dispatch order.
//! - [`catalogue`] — every member with its family (task, flow, desktop,
//!   browser), a one-line summary, and whether it is confidential; `Describe`
//!   serves it so a caller can find the right member without this source.
//! - [`envelope`] — [`DesktopResponse`], the reply every member returns, and
//!   the structured [`DesktopError`] it carries on failure.
//! - [`vocabulary`] — the enumerations shared across payloads: surfaces,
//!   modifiers, mouse buttons, element properties.
//! - [`observation`], [`interaction`], [`input`], [`apps`], [`clipboard`],
//!   [`notifications`], [`waiting`], [`system`] — one module per family of
//!   members, holding that family's request payloads.
//! - [`agent`] — the Agent interface: tasks handed over in plain language or
//!   as a flow, run across the desktop and the browser, paused only for what
//!   the caller must decide.
//! - [`browser`] — the browser members' own vocabulary and names (each
//!   `Browser`-prefixed), namespaced so its snapshot and screenshot types
//!   never shadow the desktop ones. They reply with the same envelope.
//! - [`version`] — [`CONTRACT_VERSION`] and the [`is_compatible`] bind rule.
//!
//! # How the pieces fit together at a call site
//!
//! Observation allocates refs; interaction spends them. A caller snapshots an
//! application, reads the compact tree that comes back, picks the ref it wants,
//! and names that ref in the next call. Coordinates are the fallback, not the
//! interface.
//!
//! ```
//! use tinycomputer_bus::{names, DesktopResponse, RefRequest, SnapshotRequest};
//!
//! // What a host puts on the wire: a positional argument array.
//! let body = serde_json::to_value([SnapshotRequest {
//!     app: Some("Safari".to_owned()),
//!     skeleton: true,
//!     ..SnapshotRequest::default()
//! }])?;
//! assert_eq!(body[0]["app"], "Safari");
//!
//! // Then act on a ref the reply named.
//! let click = serde_json::to_value([RefRequest::new("@s8f3k2p9:e1")])?;
//! assert_eq!(click[0]["ref_id"], "@s8f3k2p9:e1");
//! assert_eq!(names::methods::CLICK, "Click");
//!
//! // Every member replies with the same envelope, success or failure.
//! let reply: DesktopResponse = serde_json::from_value(serde_json::json!({
//!     "version": "2.5",
//!     "ok": false,
//!     "command": "click",
//!     "error": { "code": "STALE_REF", "message": "ref is no longer valid" },
//! }))?;
//! assert_eq!(reply.error.as_ref().map(|error| error.code.as_str()), Some("STALE_REF"));
//! # Ok::<(), serde_json::Error>(())
//! ```
//!
//! # What is deliberately not here
//!
//! **No behavior, and no engine.** The implementation lives in
//! `crates/tinycomputer`, which wraps the vendored `agent-desktop` engine and
//! re-exports this crate. A payload type describes what a frame carries, not
//! what the module does with it — and pulling the engine in here would drag a
//! platform accessibility backend into every host that only wanted to name a
//! member.
//!
//! **No transport.** This crate does not depend on `tinybus` and holds no
//! connection, client, or codec. A host already owns its connection — its
//! reconnect policy, its timeouts, its tracing — and the useful part is the
//! vocabulary, not another wrapper around it.
//!
//! That is also a structural necessity, not only a preference: `tinybus` is
//! vendored as a submodule whose manifest inherits fields from its own nested
//! `[workspace.package]`. A crate that every workspace member can depend on has
//! to stay transport-free, and staying transport-free is what keeps this crate
//! down to two pure-Rust dependencies.
//!
//! **No session, trace, or skills members.** The engine also offers session
//! lifecycle, trace read and export, and a bundled documentation loader. They
//! are process-lifecycle concerns of a CLI rather than of a loaded module: a
//! module is configured once, at load, and its session identity comes from that
//! configuration instead of from a member a caller has to remember to invoke.
//! They are candidates for a later minor version of this contract, which is
//! why the bind rule in [`version`] treats an added member as compatible.
//!
//! # This crate sits underneath the implementation, not beside it
//!
//! `tinycomputer` **depends on this crate and re-exports all of it**, so
//! `tinycomputer::SnapshotRequest` and `tinycomputer_bus::SnapshotRequest` are the
//! *same type*, not structural twins. Defining a parallel set of payload types
//! for hosts would mean a conversion at every call site that nothing checks.
//! One definition, here, at the bottom.
//!
//! So: a module author depends on `tinycomputer` and gets behavior and
//! vocabulary. A host depends on `tinycomputer-bus` and gets vocabulary alone.
//!
//! # Staying in step with the module
//!
//! [`names::METHODS`] lists every member. `crates/tinycomputer` asserts its
//! served members against that list, in order, so a method added to the
//! interface without an entry here fails that crate's tests rather than
//! surfacing as an unknown method in a host at runtime.

pub mod agent;
pub mod agentic;
pub mod apps;
pub mod browser;
pub mod catalogue;
pub mod clipboard;
pub mod envelope;
pub mod flow;
pub mod input;
pub mod interaction;
pub mod names;
pub mod notifications;
pub mod observation;
pub mod system;
pub mod version;
pub mod vocabulary;
pub mod waiting;

pub use agentic::{
    GoalContinuation, JevConfig, JevConfiguration, JevDecision, JevDecisionKind, JevMetrics,
    JevObservation, JevOperation, JevPredicateResult, JevProvider, JevRunResult, JevStopReason,
    JevTarget, JevTurn, ResolveIntentRequest, RunGoalRequest, VisiblePredicate,
};
pub use apps::{
    CloseAppRequest, FocusWindowRequest, LaunchRequest, ListAppsRequest, ListSurfacesRequest,
    ListWindowsRequest, MoveWindowRequest, ResizeWindowRequest, WindowRequest,
};
pub use clipboard::{ClipboardGetRequest, ClipboardSetRequest};
pub use envelope::{
    Delivery, DeliveryDisposition, DesktopError, DesktopResponse, ENVELOPE_VERSION, RecoveryHint,
    RetryDisposition,
};
pub use flow::{
    ChooseStep, Deliberation, FLOW_GUIDE, Flow, FlowAction, FlowActionRecord, FlowBrief, FlowLoop,
    FlowRunResult, FlowStep, FlowStopReason, FlowStrategy, FlowValidation, GroundingHint, IfStep,
    JevExchange, PickStep, ReadStep, RepeatStep, RunFlowRequest, STEP_KINDS, Slot, Slots,
    StepOutcome, StepReport, ValidateFlowRequest,
};
pub use input::{
    DragEndpoint, DragRequest, HoldKeyRequest, HoldMouseRequest, HoverRequest, MouseClickRequest,
    MouseMoveRequest, MouseWheelRequest, PressRequest,
};
pub use interaction::{RefRequest, ScrollRequest, SelectRequest, SetValueRequest, TypeRequest};
pub use names::{INTERFACE, METHODS, OBJECT_PATH};
pub use notifications::{
    DismissAllNotificationsRequest, DismissNotificationRequest, ListNotificationsRequest,
    NotificationActionRequest,
};
pub use observation::{FindRequest, GetRequest, IsRequest, ScreenshotRequest, SnapshotRequest};
pub use system::PermissionsRequest;
pub use version::{CONTRACT_VERSION, is_compatible};
pub use vocabulary::{
    ClipboardFormat, Direction, ElementProperty, ElementStateProperty, Modifier, MouseButton,
    StatePredicate, Surface,
};
pub use waiting::WaitRequest;

/// Native accessibility DTOs shared with the compiled module and compatibility library.
pub mod accessibility;
