//! Native accessibility execution and listener ownership inside the compiled module.
use serde::Serialize;
use std::sync::Mutex;
use tinycomputer_accessibility as native;
use tinycomputer_bus::accessibility::{
    Error, FocusQuery, FocusTarget, FocusedTextContext, GlobeHandle, GlobeHotkeyPollResult,
    GlobeHotkeyStatus, GlobeStarted, PasteRequest, PermissionKind, PermissionStatus, Result,
};
use tinycomputer_bus::{DesktopError, DesktopResponse};

pub(super) trait Backend: std::fmt::Debug + Send + Sync {
    fn permissions(&self) -> PermissionStatus;
    fn request_permission(&self, kind: PermissionKind) -> PermissionStatus;
    fn focus(&self, query: FocusQuery) -> Result<FocusedTextContext>;
    fn validate(&self, target: &FocusTarget) -> Result<()>;
    fn paste(&self, request: PasteRequest) -> Result<()>;
    fn start(&self) -> Result<GlobeHotkeyStatus>;
    fn poll(&self) -> Result<GlobeHotkeyPollResult>;
    fn stop(&self) -> Result<GlobeHotkeyStatus>;
}
#[derive(Debug)]
struct Platform;
impl Backend for Platform {
    fn permissions(&self) -> PermissionStatus {
        native::detect_permissions()
    }
    fn request_permission(&self, kind: PermissionKind) -> PermissionStatus {
        match kind {
            PermissionKind::Microphone => native::request_microphone_access(),
            PermissionKind::Accessibility => {
                #[cfg(target_os = "macos")]
                native::request_accessibility_access();
            }
            PermissionKind::InputMonitoring => {
                #[cfg(target_os = "macos")]
                native::open_macos_privacy_pane("Privacy_ListenEvent");
            }
        }
        self.permissions()
    }
    fn focus(&self, query: FocusQuery) -> Result<FocusedTextContext> {
        if query.verbose {
            native::focused_text_context_verbose()
        } else {
            native::focused_text_context()
        }
    }
    fn validate(&self, target: &FocusTarget) -> Result<()> {
        native::validate_focused_target(
            target.app.as_deref(),
            target.role.as_deref(),
            target.bounds,
        )
    }
    fn paste(&self, request: PasteRequest) -> Result<()> {
        native::paste::insert_text(&request.text, request.target.app.as_deref())
            .map_err(Error::InsertionFailed)
    }
    fn start(&self) -> Result<GlobeHotkeyStatus> {
        native::globe_listener_start()
    }
    fn poll(&self) -> Result<GlobeHotkeyPollResult> {
        native::globe_listener_poll()
    }
    fn stop(&self) -> Result<GlobeHotkeyStatus> {
        native::globe_listener_stop()
    }
}
#[derive(Debug)]
pub(super) struct Access {
    backend: std::sync::Arc<dyn Backend>,
    listener: Mutex<Option<GlobeHandle>>,
}
impl Default for Access {
    fn default() -> Self {
        Self::new(std::sync::Arc::new(Platform))
    }
}
impl Access {
    pub(super) fn new(backend: std::sync::Arc<dyn Backend>) -> Self {
        Self {
            backend,
            listener: Mutex::new(None),
        }
    }
    pub(super) fn permissions(&self) -> PermissionStatus {
        self.backend.permissions()
    }
    pub(super) fn request_permission(&self, kind: PermissionKind) -> PermissionStatus {
        self.backend.request_permission(kind)
    }
    pub(super) fn focus(&self, query: FocusQuery) -> Result<FocusedTextContext> {
        self.backend.focus(query)
    }
    pub(super) fn validate(&self, target: &FocusTarget) -> Result<()> {
        self.backend.validate(target)
    }
    pub(super) fn paste(&self, request: PasteRequest) -> Result<()> {
        self.validate(&request.target)?;
        self.backend.paste(request)
    }
    pub(super) fn start(&self) -> Result<GlobeStarted> {
        let mut guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut entropy = [0_u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|_| Error::GlobeListener("listener entropy unavailable".into()))?;
        let status = self.backend.start()?;
        if !status.supported {
            return Err(Error::UnsupportedPlatform);
        }
        let handle = guard
            .get_or_insert_with(|| GlobeHandle(format!("{:032x}", u128::from_le_bytes(entropy))))
            .clone();
        Ok(GlobeStarted { handle, status })
    }
    pub(super) fn poll(&self, handle: &GlobeHandle) -> Result<GlobeHotkeyPollResult> {
        let guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref() != Some(handle) {
            return Err(Error::UnknownListener);
        }
        self.backend.poll()
    }
    pub(super) fn stop(&self, handle: &GlobeHandle) -> Result<GlobeHotkeyStatus> {
        let mut guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref() != Some(handle) {
            return Err(Error::UnknownListener);
        }
        let status = self.backend.stop()?;
        *guard = None;
        Ok(status)
    }
}
impl Drop for Access {
    fn drop(&mut self) {
        if self
            .listener
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .is_some()
        {
            let _ = self.backend.stop();
        }
    }
}

pub(super) async fn run<T, F>(
    access: std::sync::Arc<Access>,
    command: &'static str,
    operation: F,
) -> tinybus::Result<DesktopResponse>
where
    T: Serialize + Send + 'static,
    F: FnOnce(&Access) -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || match operation(&access) {
        Ok(value) => serde_json::to_value(value)
            .map(|data| DesktopResponse::ok(command, data))
            .map_err(|_| tinybus::Error::failed("accessibility serialization failed")),
        Err(error) => {
            let code = match error {
                Error::UnsupportedPlatform => "PLATFORM_NOT_SUPPORTED",
                Error::FocusChanged { .. }
                | Error::FocusRoleChanged { .. }
                | Error::FocusTargetChanged => "FOCUS_CHANGED",
                Error::UnknownListener => "UNKNOWN_LISTENER",
                _ => "ACCESSIBILITY_FAILED",
            };
            Ok(DesktopResponse::err(
                command,
                DesktopError::new(code, error.to_string()),
            ))
        }
    })
    .await
    .map_err(|_| tinybus::Error::failed("accessibility worker failed"))?
}

#[cfg(test)]
#[path = "accessibility_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn fixture() -> std::sync::Arc<Access> {
    tests::fixture_access()
}
