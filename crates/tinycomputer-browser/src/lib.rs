//! Browser automation over the [agent-browser] engine.
//!
//! This crate is the browser counterpart of `tinycomputer-desktop`: an adapter
//! that turns the typed requests in [`tinycomputer_bus::browser`] into engine
//! commands and the engine's replies into typed results. It holds no bus, no
//! agent loop, and no model; `tinycomputer-engine` drives it, and `tinycomputer`
//! serves it over `TinyBus` as the `Browser…` members of the module's one
//! interface.
//!
//! [agent-browser]: https://github.com/vercel-labs/agent-browser
//!
//! # What is here
//!
//! - [`Browser`] — sessions and every call on them: navigate, snapshot,
//!   perform, read, evaluate, screenshot, downloads, and held outputs.
//! - [`Engine`] and [`Launcher`] — the seam to agent-browser: one JSON command
//!   in, one reply out, one engine per session. Tests script it; the linked
//!   engine implements it over agent-browser's dispatcher.
//! - [`BrowserSurface`] — one session as a `tinycomputer_core` surface, so the
//!   engine's decision loops drive a web page as they drive a desktop app.
//!   When the session has a window on screen it glides the agent's
//!   [`ScreenCursor`] onto each target; the cursor is cosmetic.
//! - `AgentBrowser` (feature `agent-browser`) — the [`Launcher`] for
//!   agent-browser linked in-process.
//! - [`Error`] — what can go wrong, as a taxonomy of what a caller should do
//!   next, each variant mapped to one published wire name, and to the
//!   envelope error a bus member replies with ([`Error::envelope`]).
//!
//! Every browser contract type is re-exported, so `tinycomputer_browser::Action`
//! is the same type as `tinycomputer_bus::browser::Action`.

mod convert;
mod engine;
mod error;
#[cfg(test)]
mod fake;
#[cfg(feature = "agent-browser")]
mod linked;
mod origins;
mod outputs;
mod reply;
mod sessions;
mod surface;

pub use engine::{Engine, Launcher, Reply};
pub use error::{Error, Result};
#[cfg(feature = "agent-browser")]
pub use linked::AgentBrowser;
pub use outputs::SWEEP_INTERVAL;
pub use sessions::{Browser, MAX_SESSIONS};
pub use surface::{BrowserSurface, Denoised, Perception, Settle};
pub use tinycomputer_bus::browser::*;
pub use tinycomputer_cursor::{CursorPace, ProcessOverlay, ScreenCursor};
