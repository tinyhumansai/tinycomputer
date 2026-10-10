//! Shared platform types for accessibility, focus, and permissions.

use serde::{Deserialize, Serialize};

/// Unified element bounds — used by autocomplete.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ElementBounds {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// Context returned by an accessibility focus query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusedTextContext {
    /// Frontmost application name.
    pub app_name: Option<String>,
    /// Accessibility role of the focused element.
    pub role: Option<String>,
    /// Text content of the element.
    pub text: String,
    /// Currently selected text, if any.
    pub selected_text: Option<String>,
    /// Diagnostic from the helper when the query partly failed.
    pub raw_error: Option<String>,
    /// Element bounds, if reported.
    pub bounds: Option<ElementBounds>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Whether a permission is granted.
pub enum PermissionState {
    /// Permission granted.
    Granted,
    /// Permission denied.
    Denied,
    /// State cannot be determined.
    Unknown,
    /// Not applicable on this platform.
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Every permission this crate reports, at one instant.
pub struct PermissionStatus {
    /// Accessibility (AX) trust.
    pub accessibility: PermissionState,
    /// Input Monitoring (global key events).
    pub input_monitoring: PermissionState,
    /// Microphone access.
    pub microphone: PermissionState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// A permission this crate can detect or request.
pub enum PermissionKind {
    /// Accessibility (AX) trust.
    Accessibility,
    /// Input Monitoring.
    InputMonitoring,
    /// Microphone access.
    Microphone,
}
