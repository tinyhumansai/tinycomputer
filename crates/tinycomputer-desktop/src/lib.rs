//! Native desktop automation over the [`agent-desktop`] engine.
//!
//! [`Desktop`] is the adapter: one method per tinycomputer desktop member, each
//! taking a contract request from [`tinycomputer_bus`] and returning a
//! [`DesktopResponse`]. It holds no bus, no agent loop and no model. The
//! `tinycomputer-engine` crate composes it with the browser adapter and Jev, and
//! the `tinycomputer` crate serves both over `TinyBus`. A host that only needs
//! desktop automation can take this crate alone.
//!
//! [`agent-desktop`]: https://github.com/lahfir/agent-desktop
//!
//! Every contract type is re-exported, so `tinycomputer_desktop::SnapshotRequest`
//! is the same type as `tinycomputer_bus::SnapshotRequest`.
//!
//! # The model: observe, then act on what you observed
//!
//! A snapshot walks an application's accessibility tree and hands back a
//! compact description in which every element carries a *ref* — a qualified
//! handle like `@s8f3k2p9:e1`. Interaction members take those refs. They do not
//! take coordinates, and they do not take selectors evaluated fresh at click
//! time.
//!
//! That indirection is the whole design. A ref is bound to the snapshot it came
//! from, so acting on one either reaches the element that was described or
//! fails with `STALE_REF` and asks for a fresh snapshot. What it will not do is
//! click whatever has since moved into that position.
//!
//! ```no_run
//! use tinycomputer_desktop::{Desktop, FindRequest, RefRequest};
//!
//! let desktop = Desktop::new();
//!
//! let found = desktop.find(FindRequest {
//!     app: Some("Safari".to_owned()),
//!     role: Some("button".to_owned()),
//!     name: Some("Save".to_owned()),
//!     first: true,
//!     ..FindRequest::default()
//! });
//!
//! if found.ok {
//!     let reply = desktop.click(RefRequest::new("@s8f3k2p9:e1"));
//!     assert_eq!(reply.command, "click");
//! }
//! ```
//!
//! # Headless by default
//!
//! A ref action goes through the platform's accessibility API, not through
//! synthesized input, so it does not steal focus, move the cursor, or touch the
//! pasteboard as a side effect. A run can proceed while someone else is using
//! the machine. [`Desktop::with_headed`] relaxes that for the interactions that
//! genuinely need a real cursor, and the [`input`]
//! members bypass it entirely — both on purpose, and both the exception.
//!
//! # Errors are replies, not failures
//!
//! Every command method returns a [`DesktopResponse`] and never a `Result`. A
//! stale ref, a missing permission, an ambiguous application name — these carry
//! codes, suggestions, and recovery hints a caller can branch on, and flattening
//! them into an error string would throw that away. [`Error`] is reserved for
//! the module failing to start a command at all. See
//! [`tinycomputer_bus::envelope`] for the full reasoning.
//!
//! # Platform support
//!
//! macOS and Windows have full accessibility backends. Linux builds, loads, and
//! answers, but implements no surfaces yet: every observation there fails with
//! `PLATFORM_NOT_SUPPORTED` and lists the surfaces it does support, which is
//! none. That is inherited from the vendored engine and will follow it.

mod desktop;
mod error;
mod surface;

pub use agent_desktop_core::{
    AppError as TraceSinkError, Deadline as TraceSinkDeadline, TraceArtifact as TraceSinkArtifact,
    TraceArtifactKind as TraceSinkArtifactKind, TraceArtifactLimits as TraceSinkArtifactLimits,
    TraceSink,
};
pub use desktop::Desktop;
pub use error::{Error, Result};
pub use tinycomputer_bus;
pub use tinycomputer_bus::*;
pub use tinycomputer_cursor::{CursorPace, ScreenCursor};
