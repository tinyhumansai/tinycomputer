//! Native permission, focus, paste and Globe listener vocabulary.
//! No native framework, device, process, or listener behavior is linked here.
mod error;
mod globe;
mod types;
pub use error::{Error, Result};
pub use globe::{GlobeHotkeyPollResult, GlobeHotkeyStatus};
pub use types::{
    ElementBounds, FocusedTextContext, PermissionKind, PermissionState, PermissionStatus,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

/// Focus read options.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FocusQuery {
    /// Include the helper's full focused-element context.
    #[serde(default)]
    pub verbose: bool,
}
/// Captured focus identity, checked again before text insertion.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FocusTarget {
    /// Expected application.
    pub app: Option<String>,
    /// Expected accessibility role.
    pub role: Option<String>,
    /// Expected element bounds.
    pub bounds: Option<ElementBounds>,
}
/// Text and the captured target to validate inside the module.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PasteRequest {
    /// Authorized insertion text.
    pub text: String,
    /// Captured focus identity.
    #[serde(default)]
    pub target: FocusTarget,
}
/// Opaque module-owned Globe listener lease.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct GlobeHandle(pub String);
/// Started listener and its opaque lease.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlobeStarted {
    /// Lease passed to poll and stop.
    pub handle: GlobeHandle,
    /// Current native status.
    pub status: GlobeHotkeyStatus,
}
