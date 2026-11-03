//! The member that blocks until a condition holds.

use agent_desktop_core::{
    AppError,
    commands::{wait, wait_surface::SurfaceWait},
};
use tinycomputer_bus::{DesktopResponse, WaitRequest};

use super::{Desktop, Need};

/// The engine's own default wait timeout, applied when a request leaves
/// `timeout_ms` absent.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

impl Desktop {
    /// Blocks until a condition holds, or fails with `TIMEOUT`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, WaitRequest};
    /// let reply = Desktop::new().wait(WaitRequest::sleep(1));
    ///
    /// assert!(reply.ok);
    /// assert_eq!(reply.data.as_ref().and_then(|data| data.get("waited_ms")), Some(&1.into()));
    /// ```
    #[must_use]
    pub fn wait(&self, request: WaitRequest) -> DesktopResponse {
        // A plain sleep touches no other application, so it needs nothing
        // granted. Every other mode watches one, and an unpermitted watch would
        // wait out its full timeout observing an empty tree.
        let need = if request.ms.is_some() {
            Need::Nothing
        } else {
            Need::Accessibility
        };

        self.run("wait", need, |adapter, context| {
            wait::execute(
                wait::WaitArgs {
                    mode: wait::WaitModeArgs {
                        ms: request.ms,
                        element: request.element.map(|ref_id| {
                            super::convert::qualified_ref(ref_id, request.snapshot_id.as_deref())
                        }),
                        window: request.window,
                        text: request.text,
                        surface: surface_wait(request.surface.as_deref())?,
                        event: request.event,
                        window_id: request.window_id,
                    },
                    predicate: wait::WaitPredicateArgs {
                        predicate: request.predicate,
                        value: request.value,
                        action: request.action,
                        count: request.count,
                    },
                    timeout_ms: request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
                    app: request.app,
                },
                adapter,
                context,
            )
        })
    }
}

/// Parses the surface wait mode.
///
/// The engine names these three through flags rather than a string, so the
/// mapping lives here. An unrecognized name is rejected by name instead of
/// being dropped, which would otherwise turn a typo into a wait with no mode
/// selected and a confusing "choose a mode" error.
pub(super) fn surface_wait(surface: Option<&str>) -> Result<Option<SurfaceWait>, AppError> {
    match surface {
        None => Ok(None),
        Some("menu") => Ok(Some(SurfaceWait::Menu)),
        Some("menu_closed") => Ok(Some(SurfaceWait::MenuClosed)),
        Some("notification") => Ok(Some(SurfaceWait::Notification)),
        Some(other) => Err(AppError::invalid_input_with_suggestion(
            format!("unknown surface wait `{other}`"),
            "Use one of: menu, menu_closed, notification",
        )),
    }
}
