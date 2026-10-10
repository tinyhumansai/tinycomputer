//! Unsupported-platform compatibility entrypoints allocate no native resource.
use super::{
    AccessibilityResult, GlobeHotkeyPollResult, GlobeHotkeyStatus, input_monitoring_permission,
};

/// Start the Globe/Fn key listener process; unsupported platforms report `supported: false`.
///
/// # Errors
///
/// Returns a message when the listener helper cannot be built or started.
pub fn globe_listener_start() -> AccessibilityResult<GlobeHotkeyStatus> {
    Ok(GlobeHotkeyStatus {
        supported: false,
        running: false,
        input_monitoring_permission: input_monitoring_permission(),
        last_error: Some("Globe/Fn hotkey listener is only supported on macOS".to_string()),
        events_pending: 0,
    })
}

/// Drain pending Globe/Fn key events and report listener status.
///
/// # Errors
///
/// Returns a message when the listener state cannot be read.
pub fn globe_listener_poll() -> AccessibilityResult<GlobeHotkeyPollResult> {
    Ok(GlobeHotkeyPollResult {
        status: GlobeHotkeyStatus {
            supported: false,
            running: false,
            input_monitoring_permission: input_monitoring_permission(),
            last_error: Some("Globe/Fn hotkey listener is only supported on macOS".to_string()),
            events_pending: 0,
        },
        events: Vec::new(),
    })
}

/// Stop the Globe/Fn key listener process.
///
/// # Errors
///
/// Returns a message when the listener state cannot be read.
pub fn globe_listener_stop() -> AccessibilityResult<GlobeHotkeyStatus> {
    Ok(GlobeHotkeyStatus {
        supported: false,
        running: false,
        input_monitoring_permission: input_monitoring_permission(),
        last_error: Some("Globe/Fn hotkey listener is only supported on macOS".to_string()),
        events_pending: 0,
    })
}

/// Read native events and overflow; unavailable platforms remain unsupported.
///
/// # Errors
/// Returns the same platform error as the compatibility poll entrypoint.
pub fn globe_listener_read() -> AccessibilityResult<(GlobeHotkeyPollResult, bool)> {
    globe_listener_poll().map(|result| (result, false))
}

/// Start under terminal cancellation; unsupported platforms allocate no resource.
///
/// # Errors
/// Returns cancellation or the existing platform failure.
pub fn globe_listener_start_with_cancel(
    cancel: &std::sync::atomic::AtomicBool,
) -> AccessibilityResult<GlobeHotkeyStatus> {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(crate::Error::GlobeListener("native_start_canceled".into()));
    }
    globe_listener_start()
}

/// Read under terminal cancellation; unsupported platforms allocate no resource.
///
/// # Errors
/// Returns cancellation or the existing platform failure.
pub fn globe_listener_read_with_cancel(
    cancel: &std::sync::atomic::AtomicBool,
) -> AccessibilityResult<(GlobeHotkeyPollResult, bool)> {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(crate::Error::GlobeListener("native_start_canceled".into()));
    }
    globe_listener_read()
}
