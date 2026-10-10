//! Native accessibility execution and listener ownership inside the compiled module.
use serde::Serialize;
use std::sync::Mutex;
use tinycomputer_accessibility as native;
use tinycomputer_bus::accessibility::{
    Error, FocusQuery, FocusTarget, FocusedTextContext, GlobeBatch, GlobeEvent, GlobeHandle,
    GlobeHotkeyPollResult, GlobeHotkeyStatus, GlobeRead, GlobeStarted, PasteRequest,
    PermissionKind, PermissionStatus, Result,
};
use tinycomputer_bus::{DesktopError, DesktopResponse};

pub(super) trait Backend: std::fmt::Debug + Send + Sync {
    fn permissions(&self) -> PermissionStatus;
    fn request_permission(&self, kind: PermissionKind) -> PermissionStatus;
    fn focus(&self, query: FocusQuery) -> Result<FocusedTextContext>;
    fn validate(&self, target: &FocusTarget) -> Result<()>;
    fn paste(&self, request: PasteRequest) -> Result<()>;
    fn start(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<GlobeHotkeyStatus>;
    fn poll(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<GlobeHotkeyPollResult>;
    fn read(&self, cancel: &std::sync::atomic::AtomicBool)
    -> Result<(GlobeHotkeyPollResult, bool)>;
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
    fn start(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<GlobeHotkeyStatus> {
        native::globe_listener_start_with_cancel(cancel)
    }
    fn poll(&self, cancel: &std::sync::atomic::AtomicBool) -> Result<GlobeHotkeyPollResult> {
        native::globe_listener_read_with_cancel(cancel).map(|(result, _)| result)
    }
    fn read(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(GlobeHotkeyPollResult, bool)> {
        native::globe_listener_read_with_cancel(cancel)
    }
    fn stop(&self) -> Result<GlobeHotkeyStatus> {
        native::globe_listener_stop()
    }
}
#[derive(Debug)]
struct Listener {
    handle: GlobeHandle,
    batch: u64,
    snapshot: Option<GlobeBatch>,
    legacy_gap: bool,
}
#[derive(Debug)]
pub(super) struct Access {
    backend: std::sync::Arc<dyn Backend>,
    listener: Mutex<Option<Listener>>,
    terminal: std::sync::atomic::AtomicBool,
    native_owned: std::sync::atomic::AtomicBool,
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
            terminal: std::sync::atomic::AtomicBool::new(false),
            native_owned: std::sync::atomic::AtomicBool::new(false),
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
        if self.terminal.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::GlobeListener("listener shutdown".into()));
        }
        let mut entropy = [0_u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|_| Error::GlobeListener("listener entropy unavailable".into()))?;
        self.native_owned
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let status = match self.backend.start(&self.terminal) {
            Ok(status) => status,
            Err(error) => {
                self.backend.stop()?;
                self.native_owned
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Err(error);
            }
        };
        if !status.supported {
            self.native_owned
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Err(Error::UnsupportedPlatform);
        }
        if self.terminal.load(std::sync::atomic::Ordering::SeqCst) {
            self.backend.stop()?;
            return Err(Error::GlobeListener("listener shutdown".into()));
        }
        let handle = guard
            .get_or_insert_with(|| Listener {
                handle: GlobeHandle(format!("{:032x}", u128::from_le_bytes(entropy))),
                batch: 0,
                snapshot: None,
                legacy_gap: false,
            })
            .handle
            .clone();
        Ok(GlobeStarted { handle, status })
    }
    pub(super) fn poll(&self, handle: &GlobeHandle) -> Result<GlobeHotkeyPollResult> {
        let mut guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref().map(|listener| &listener.handle) != Some(handle) {
            return Err(Error::UnknownListener);
        }
        let result = self.backend.poll(&self.terminal)?;
        if let Some(listener) = guard.as_mut() {
            listener.legacy_gap = true;
        }
        Ok(result)
    }
    pub(super) fn read(&self, request: &GlobeRead) -> Result<GlobeBatch> {
        let mut guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(listener) = guard.as_mut().filter(|l| l.handle == request.handle) else {
            return Err(Error::UnknownListener);
        };
        if request
            .acknowledged_batch
            .is_some_and(|ack| ack > listener.batch)
        {
            return Err(Error::GlobeListener("invalid batch acknowledgement".into()));
        }
        if let Some(snapshot) = &listener.snapshot
            && request.acknowledged_batch != Some(snapshot.batch)
        {
            return Ok(snapshot.clone());
        }
        let batch = listener
            .batch
            .checked_add(1)
            .ok_or_else(|| Error::GlobeListener("batch identity exhausted".into()))?;
        let (result, overflow) = self.backend.read(&self.terminal)?;
        let mut gap =
            overflow || listener.legacy_gap || !result.status.running || result.events.len() > 64;
        let events = result
            .events
            .into_iter()
            .take(64)
            .filter_map(|event| match event.as_str() {
                "FN_DOWN" => Some(GlobeEvent::Down),
                "FN_UP" => Some(GlobeEvent::Up),
                _ => {
                    gap = true;
                    None
                }
            })
            .collect();
        let snapshot = GlobeBatch {
            handle: listener.handle.clone(),
            batch,
            status: result.status,
            events,
            overflow: gap,
        };
        listener.batch = batch;
        listener.legacy_gap = false;
        listener.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }
    pub(super) fn close_admission(&self) {
        self.terminal
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    pub(super) fn shutdown(&self) -> Result<GlobeHotkeyStatus> {
        self.terminal
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let status = self.backend.stop()?;
        self.native_owned
            .store(false, std::sync::atomic::Ordering::SeqCst);
        *guard = None;
        Ok(status)
    }
    pub(super) fn stop(&self, handle: &GlobeHandle) -> Result<GlobeHotkeyStatus> {
        let mut guard = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref().map(|listener| &listener.handle) != Some(handle) {
            return Err(Error::UnknownListener);
        }
        let status = self.backend.stop()?;
        self.native_owned
            .store(false, std::sync::atomic::Ordering::SeqCst);
        *guard = None;
        Ok(status)
    }
}
impl Drop for Access {
    fn drop(&mut self) {
        // Explicit GlobeShutdown is the fallible unload barrier. This also
        // covers partial failed startup when no public lease was published.
        if self.native_owned.load(std::sync::atomic::Ordering::SeqCst) {
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

#[cfg(test)]
#[path = "accessibility_fixture.rs"]
mod fixture;
#[cfg(test)]
#[path = "accessibility_replay_tests.rs"]
mod replay_tests;
