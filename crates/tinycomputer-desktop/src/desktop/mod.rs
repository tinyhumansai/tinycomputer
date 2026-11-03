//! The engine: one method per member, and the machinery they share.
//!
//! [`Desktop`] is the whole of this crate's behavior. It holds the
//! configuration a command runs under — session identity, tracing, whether
//! physical input is permitted — and exposes one method per member of
//! [`tinycomputer_bus::names::METHODS`]. `tinybus_module` does nothing but adapt
//! those methods to the bus, which is what keeps the feature code testable
//! without a broker.
//!
//! # Why every method returns a response instead of a `Result`
//!
//! A command method returns [`DesktopResponse`] for both outcomes and never
//! fails in the Rust sense. That is deliberate, and it is the same reasoning
//! [`crate::Error`] documents: a stale ref carries a recovery hint, a denied
//! permission carries the setting to change, an ambiguous application name
//! carries the candidates. Those belong in a payload a caller can branch on,
//! not in a `Result::Err` that a bus adapter would flatten to a string.
//!
//! # What happens on each call
//!
//! 1. A platform adapter is constructed. Every adapter is a zero-sized type,
//!    so this costs nothing and avoids holding a platform handle across an
//!    `await` in the bus layer.
//! 2. A [`CommandContext`] is built from this `Desktop`'s configuration. It
//!    carries the session the refs live in and the input policy the command
//!    runs under.
//! 3. The command's [permission need](permission::Need) is checked against the
//!    platform's report, so a command that cannot possibly work fails with
//!    `PERM_DENIED` and the setting to change rather than with whatever the
//!    accessibility API returns for an unauthorized process.
//! 4. The engine runs the command, and its result — value or error — is
//!    wrapped in the envelope.
//!
//! # Files
//!
//! The command methods are split by family across sibling files, each holding
//! one `impl Desktop` block: `observation`, `interaction`, `input`, `apps`,
//! `clipboard`, `notifications`, `waiting`, and `system`. `convert` maps
//! contract payloads onto the engine's argument types, `reply` maps the
//! engine's errors onto the envelope, and `permission` holds the preflight.

mod apps;
mod clipboard;
mod convert;
mod input;
mod interaction;
mod notifications;
mod observation;
mod permission;
mod reply;
mod system;
mod trace;
mod waiting;

use std::path::PathBuf;
use std::sync::Arc;

use agent_desktop_core::{AppError, PermissionReport, PlatformAdapter, context::CommandContext};
use serde_json::Value;
use tinycomputer_bus::DesktopResponse;
use tinycomputer_cursor::ScreenCursor;

use crate::{Error, Result};
use permission::Need;

/// The desktop automation engine, configured once and called many times.
///
/// A default `Desktop` runs without a named session, without tracing, and
/// headless — meaning ref actions go through accessibility APIs and are
/// blocked from stealing focus, moving the cursor, or touching the pasteboard
/// as a side effect. That is the configuration a background agent wants.
///
/// # Examples
///
/// ```
/// use tinycomputer_desktop::Desktop;
///
/// let desktop = Desktop::new();
/// let reply = desktop.version();
///
/// assert!(reply.ok);
/// assert!(reply.data.is_some());
/// ```
#[derive(Clone, Default)]
pub struct Desktop {
    session_id: Option<String>,
    trace_path: Option<PathBuf>,
    trace_strict: bool,
    headed: bool,
    cursor: Option<Arc<ScreenCursor>>,
    trace_sink: Option<Arc<dyn agent_desktop_core::TraceSink>>,
}

impl std::fmt::Debug for Desktop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Desktop")
            .field("session_id", &self.session_id)
            .field("trace_path", &self.trace_path)
            .field("trace_strict", &self.trace_strict)
            .field("headed", &self.headed)
            .field("cursor", &self.cursor)
            .field(
                "trace_sink",
                &self.trace_sink.as_ref().map(|_| "configured"),
            )
            .finish()
    }
}

impl Desktop {
    /// Builds a `Desktop` with every setting at its default.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// assert!(Desktop::new().version().ok);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this `Desktop` bound to the named session.
    ///
    /// A session is where allocated refs live. Two `Desktop` values sharing a
    /// session id can spend each other's refs; two with different sessions
    /// cannot, which is what keeps concurrent runs from resolving each other's
    /// elements.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// let desktop = Desktop::new().with_session("run-42");
    /// assert_eq!(desktop.session_id(), Some("run-42"));
    /// ```
    #[must_use]
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Returns this `Desktop` in headed or headless mode.
    ///
    /// Headed mode lets a ref action take focus and move the real cursor when
    /// the platform needs it to. It makes some interactions work that
    /// otherwise cannot, at the cost of disturbing whoever is at the machine,
    /// so it is off by default and has to be asked for.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// assert!(Desktop::new().with_headed(true).is_headed());
    /// ```
    #[must_use]
    pub fn with_headed(mut self, headed: bool) -> Self {
        self.headed = headed;
        self
    }

    /// Returns this `Desktop` drawing on `cursor`, the screen's one agent
    /// cursor, shared with the browser surface.
    ///
    /// Before a decision loop clicks, checks, expands, or collapses an element
    /// that has bounds, the cursor glides onto it, and the action lands as it
    /// arrives, in time with its landing pulse. The cursor is cosmetic: the
    /// action is the same accessibility action with or without it, and the
    /// user's own pointer never moves.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use tinycomputer_desktop::{Desktop, ScreenCursor};
    /// let desktop = Desktop::new().with_cursor(Arc::new(ScreenCursor::off()));
    /// assert!(desktop.cursor().is_some());
    /// assert!(Desktop::new().cursor().is_none());
    /// ```
    #[must_use]
    pub fn with_cursor(mut self, cursor: Arc<ScreenCursor>) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// The cursor this `Desktop` draws on, when it has one.
    #[must_use]
    pub fn cursor(&self) -> Option<&ScreenCursor> {
        self.cursor.as_deref()
    }

    /// Builds a `Desktop` from the configuration blob the module loader
    /// supplies.
    ///
    /// Every field is optional; `null` and `{}` both yield
    /// [`Desktop::default`]. The recognized fields are `session_id` and
    /// `trace_path` (strings), and `trace_strict` and `headed` (booleans). An
    /// unrecognized field is ignored, so a newer host configuring a field this
    /// version does not know about still loads.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// let desktop = Desktop::from_config(&serde_json::json!({
    ///     "session_id": "run-42",
    ///     "headed": true,
    /// }))?;
    ///
    /// assert_eq!(desktop.session_id(), Some("run-42"));
    /// assert!(desktop.is_headed());
    /// # Ok::<(), tinycomputer_desktop::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigNotAnObject`] when the blob is neither `null` nor
    /// an object, and [`Error::ConfigFieldType`] when a recognized field holds
    /// the wrong type.
    pub fn from_config(config: &Value) -> Result<Self> {
        if config.is_null() {
            return Ok(Self::default());
        }
        let Some(object) = config.as_object() else {
            return Err(Error::ConfigNotAnObject);
        };

        Ok(Self {
            session_id: text(object.get("session_id"), "session_id")?,
            trace_path: text(object.get("trace_path"), "trace_path")?.map(PathBuf::from),
            trace_strict: flag(object.get("trace_strict"), "trace_strict")?,
            headed: flag(object.get("headed"), "headed")?,
            cursor: None,
            trace_sink: None,
        })
    }

    /// The session refs allocated through this `Desktop` belong to.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// assert_eq!(Desktop::new().session_id(), None);
    /// ```
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Whether ref actions may take focus and move the real cursor.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// assert!(!Desktop::new().is_headed());
    /// ```
    #[must_use]
    pub fn is_headed(&self) -> bool {
        self.headed
    }

    /// Runs `command` under a fresh adapter and context, and wraps whatever it
    /// produces in the envelope.
    ///
    /// This is the one path every member takes. `need` decides whether a
    /// permission report is fetched and checked first: fetching one costs a
    /// round trip to the platform, so a command that needs no permission does
    /// not pay for it.
    fn run<F>(&self, command: &'static str, need: Need, run: F) -> DesktopResponse
    where
        F: FnOnce(&dyn PlatformAdapter, &CommandContext) -> std::result::Result<Value, AppError>,
    {
        self.run_with_report(command, need, |adapter, context, _report| {
            run(adapter, context)
        })
    }

    /// [`Desktop::run`] for the two members that also read the report itself.
    fn run_with_report<F>(&self, command: &'static str, need: Need, run: F) -> DesktopResponse
    where
        F: FnOnce(
            &dyn PlatformAdapter,
            &CommandContext,
            &PermissionReport,
        ) -> std::result::Result<Value, AppError>,
    {
        let adapter = platform_adapter();
        let adapter: &dyn PlatformAdapter = &adapter;

        let result = self.context().and_then(|context| {
            let scope = context.command_scope(command)?;
            let result = permission::report(need, adapter)
                .and_then(|report| permission::preflight(need, &report).map(|()| report))
                .and_then(|report| run(adapter, &context, &report));
            scope.complete(&result)?;
            result
        });

        reply::envelope(command, result)
    }
}

/// Reads an optional string configuration field.
fn text(value: Option<&Value>, field: &'static str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::to_owned)
            .map(Some)
            .ok_or(Error::ConfigFieldType {
                field,
                expected: "a string",
            }),
    }
}

/// Reads an optional boolean configuration field, defaulting to `false`.
fn flag(value: Option<&Value>, field: &'static str) -> Result<bool> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(value) => value.as_bool().ok_or(Error::ConfigFieldType {
            field,
            expected: "a boolean",
        }),
    }
}

/// The accessibility backend for the platform this module was built for.
///
/// Every adapter is a zero-sized type, so constructing one per command is free
/// and saves the bus layer from holding a platform handle across an `await`.
fn platform_adapter() -> impl PlatformAdapter {
    #[cfg(target_os = "macos")]
    {
        agent_desktop_macos::MacOSAdapter::new()
    }

    #[cfg(target_os = "windows")]
    {
        agent_desktop_windows::WindowsAdapter::new()
    }

    #[cfg(target_os = "linux")]
    {
        agent_desktop_linux::LinuxAdapter::new()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    compile_error!(
        "tinycomputer needs an accessibility backend; agent-desktop ships one for macOS, Windows, \
         and Linux"
    )
}

#[cfg(test)]
mod desktop_tests;

#[cfg(test)]
#[path = "trace_sink_tests.rs"]
mod trace_sink_tests;
