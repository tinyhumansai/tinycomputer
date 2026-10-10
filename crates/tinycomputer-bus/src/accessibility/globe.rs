//! Bounded Globe/Fn listener status and event batches.
use super::PermissionState;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// State of the Globe/Fn key listener.
pub struct GlobeHotkeyStatus {
    /// Whether this platform supports the listener.
    pub supported: bool,
    /// Whether the listener process is alive.
    pub running: bool,
    /// Input Monitoring state the listener needs.
    pub input_monitoring_permission: PermissionState,
    /// Most recent listener error, if any.
    pub last_error: Option<String>,
    /// Events queued and not yet polled.
    pub events_pending: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// Listener status plus the key events drained by one poll.
pub struct GlobeHotkeyPollResult {
    /// Listener status at poll time.
    pub status: GlobeHotkeyStatus,
    /// Drained events, oldest first (`FN_DOWN` / `FN_UP`).
    pub events: Vec<String>,
}
