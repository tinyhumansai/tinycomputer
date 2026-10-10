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

/// A physical Globe/Fn transition, in native arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GlobeEvent {
    /// The physical key went down.
    #[serde(rename = "FN_DOWN")]
    Down,
    /// The physical key went up.
    #[serde(rename = "FN_UP")]
    Up,
}

/// Read or acknowledge the one retained reliable snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlobeRead {
    /// Existing native listener lease.
    pub handle: super::GlobeHandle,
    /// Last completely processed batch; omit to replay the retained batch.
    pub acknowledged_batch: Option<u64>,
}

/// A bounded replayable native event snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlobeBatch {
    /// Lease whose events these are.
    pub handle: super::GlobeHandle,
    /// Monotonic snapshot identity within this lease.
    pub batch: u64,
    /// Native listener status captured with this snapshot.
    pub status: GlobeHotkeyStatus,
    /// At most64 oldest-first physical transitions.
    pub events: Vec<GlobeEvent>,
    /// Continuity was lost; reset activation inactive and await physical release.
    pub overflow: bool,
}
