//! Desktop accessibility middleware for hosts that need answers in-process.
//!
//! Centralises the macOS AX / `IOKit` FFI and the unified Swift helper process
//! (focus queries, paste, overlay in one persistent process), and exposes
//! focused-text inspection, system-permission detection (Accessibility, Input
//! Monitoring, Microphone), the Globe-key listener, "System Events" automation
//! denial tracking, terminal heuristics, and AX-string normalisation. A host
//! such as a voice pipeline calls these synchronously, so this is a plain
//! library with no bus and no async runtime; the `tinycomputer` module serves
//! the agent-facing desktop members separately.
//!
//! Behaviour outside macOS is limited to what the platform can answer:
//! focus queries return a typed unsupported-platform error, permission states are `Unsupported`, and
//! the microphone probe needs the `microphone-probe` feature.
//!
//! This crate reads no configuration and imports nothing from a host.

mod automation_state;
mod error;
mod focus;
mod globe;
mod helper;
#[cfg(feature = "paste")]
pub mod paste;
mod permissions;
mod terminal;
mod text_util;
mod types;

pub use automation_state::{
    clear as clear_automation_denial, mark_system_events_denied, system_events_denied,
};
pub use error::{Error, Result};
pub use focus::{focused_text_context, focused_text_context_verbose, validate_focused_target};
pub use globe::{
    GlobeHotkeyPollResult, GlobeHotkeyStatus, globe_listener_poll, globe_listener_read,
    globe_listener_read_with_cancel, globe_listener_start, globe_listener_start_with_cancel,
    globe_listener_stop,
};
pub use helper::precompile_helper_background;
#[cfg(target_os = "macos")]
pub use permissions::{
    detect_accessibility_permission, detect_input_monitoring_permission, open_macos_privacy_pane,
    request_accessibility_access,
};
pub use permissions::{
    detect_microphone_permission, detect_permissions, microphone_denied_message, permission_to_str,
    request_microphone_access,
};
pub use terminal::{
    extract_terminal_input_context, is_terminal_app, is_text_role, looks_like_terminal_buffer,
};
pub use text_util::{normalize_ax_value, parse_ax_number, truncate_tail};
pub use types::{
    ElementBounds, FocusedTextContext, PermissionKind, PermissionState, PermissionStatus,
};
