//! Trace configuration and engine-context setup for [`super::Desktop`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_desktop_core::{AppError, context::CommandContext};

use super::Desktop;

impl Desktop {
    /// Returns this `Desktop` writing a command trace to `path`.
    ///
    /// A trace records each command and its outcome, which is what makes a
    /// failed run reconstructable afterwards. `strict` additionally fails a
    /// command whose trace could not be written, rather than letting it succeed
    /// with no record — the right setting when the trace is the audit log
    /// rather than a debugging aid.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// let desktop = Desktop::new().with_trace("/tmp/run.jsonl", false);
    /// assert!(desktop.is_tracing());
    /// ```
    #[must_use]
    pub fn with_trace(mut self, path: impl Into<PathBuf>, strict: bool) -> Self {
        self.trace_path = Some(path.into());
        self.trace_strict = strict;
        self
    }

    /// Returns this `Desktop` sending desktop trace events to `sink`.
    ///
    /// The selected sink takes precedence over `trace_path` and session trace
    /// files. Its synchronous methods are called in event order; strict mode
    /// returns sink failures to the command, and best-effort mode warns and
    /// continues, matching the local trace writer. The sink's authority stays
    /// outside event data.
    #[must_use]
    pub fn with_trace_sink(mut self, sink: Arc<dyn agent_desktop_core::TraceSink>) -> Self {
        self.trace_sink = Some(sink);
        self
    }

    /// Whether commands write a trace.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// assert!(!Desktop::new().is_tracing());
    /// ```
    #[must_use]
    pub fn is_tracing(&self) -> bool {
        self.trace_path.is_some() || self.trace_sink.is_some()
    }

    /// Where commands write their trace, if anywhere.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::path::Path;
    /// # use tinycomputer_desktop::Desktop;
    /// let desktop = Desktop::new().with_trace("/tmp/run.jsonl", true);
    /// assert_eq!(desktop.trace_path(), Some(Path::new("/tmp/run.jsonl")));
    /// ```
    #[must_use]
    pub fn trace_path(&self) -> Option<&Path> {
        self.trace_path.as_deref()
    }

    /// Builds the engine context every command runs under.
    pub(super) fn context(&self) -> Result<CommandContext, AppError> {
        let context = match &self.trace_sink {
            Some(sink) => CommandContext::new_with_trace_sink(
                self.session_id.clone(),
                sink.clone(),
                self.trace_strict,
                true,
            ),
            None => CommandContext::new(
                self.session_id.clone(),
                self.trace_path.clone(),
                self.trace_strict,
            ),
        };
        context.map(|context| context.with_headed(self.headed))
    }
}
