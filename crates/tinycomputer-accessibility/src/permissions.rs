//! Platform permission detection and requests for accessibility, input monitoring, and microphone access.
//!
//! The macOS permission checks call `ApplicationServices`, CoreFoundation and
//! `IOKit` directly, so this is the one module in the crate that allows `unsafe`.
#![allow(unsafe_code)]

use super::types::{PermissionKind, PermissionState, PermissionStatus};

#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[cfg(target_os = "macos")]
type CFAllocatorRef = *const c_void;
#[cfg(target_os = "macos")]
type CFDictionaryRef = *const c_void;
#[cfg(target_os = "macos")]
type CFBooleanRef = *const c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const c_void;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFAllocatorDefault: CFAllocatorRef;
    static kCFBooleanTrue: CFBooleanRef;
    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
    fn CFRelease(cf: *const c_void);
}

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: i32) -> isize;
}

#[cfg(target_os = "macos")]
const IOHID_REQUEST_TYPE_LISTEN_EVENT: i32 = 1;
#[cfg(target_os = "macos")]
const IOHID_ACCESS_GRANTED: isize = 0;
#[cfg(target_os = "macos")]
const IOHID_ACCESS_DENIED: isize = 1;

#[must_use]
/// The wire name of a permission kind (`accessibility`, `input_monitoring`, `microphone`).
pub fn permission_to_str(permission: PermissionKind) -> &'static str {
    match permission {
        PermissionKind::Accessibility => "accessibility",
        PermissionKind::InputMonitoring => "input_monitoring",
        PermissionKind::Microphone => "microphone",
    }
}

#[cfg(target_os = "macos")]
/// Open a pane of System Settings > Privacy & Security, e.g. `Privacy_Accessibility`.
pub fn open_macos_privacy_pane(pane: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
    let _ = std::process::Command::new("open").arg(url).status();
}

#[cfg(target_os = "macos")]
/// Ask macOS to prompt for Accessibility access.
pub fn request_accessibility_access() {
    // SAFETY: `kAXTrustedCheckOptionPrompt` and `kCFBooleanTrue` are immutable
    // CoreFoundation globals; the dictionary is created with null callbacks
    // (no retains), used once, and released here on the same thread.
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        let _ = AXIsProcessTrustedWithOptions(options);
        if !options.is_null() {
            CFRelease(options);
        }
    }
}

#[cfg(target_os = "macos")]
#[must_use]
/// Whether this process is trusted for Accessibility.
pub fn detect_accessibility_permission() -> PermissionState {
    // SAFETY: `AXIsProcessTrusted` takes no arguments and only reads TCC state.
    unsafe {
        if AXIsProcessTrusted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }
}

#[cfg(target_os = "macos")]
#[must_use]
/// Whether this process may listen for input events (Input Monitoring).
pub fn detect_input_monitoring_permission() -> PermissionState {
    // SAFETY: `IOHIDCheckAccess` takes a plain request-type integer and only
    // reads TCC state.
    let access = unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    match access {
        IOHID_ACCESS_GRANTED => PermissionState::Granted,
        IOHID_ACCESS_DENIED => PermissionState::Denied,
        // `IOHID_ACCESS_UNKNOWN` and any future value.
        _ => PermissionState::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Microphone permission — cross-platform
// ---------------------------------------------------------------------------

/// Detect whether the app has microphone permission.
///
/// Uses CPAL to detect whether an input device is present. Device enumeration
/// does not prove recording authorization, so the result remains `Unknown`
/// until the host opens an input stream and observes whether capture is allowed.
///
/// On **macOS** under hardened runtime, CPAL will fail to enumerate input
/// devices when the `com.apple.security.device.audio-input` entitlement is
/// missing or microphone permission is denied in System Settings.
///
/// On **Windows**, `None` may indicate a privacy toggle denial or no hardware.
///
/// **Linux** standard desktops don't enforce per-app permissions; Flatpak/Snap
/// sandboxes are detected separately.
#[cfg(all(
    feature = "microphone-probe",
    any(target_os = "macos", target_os = "windows")
))]
#[must_use]
pub fn detect_microphone_permission() -> PermissionState {
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    if let Some(device) = host.default_input_device() {
        let name = cpal::traits::DeviceTrait::description(&device)
            .map_or_else(|_| "<unknown>".into(), |d| d.name().to_string());
        log::debug!(
            "[permissions] input device detected; capture authorization is unverified — device: {name}"
        );
        PermissionState::Unknown
    } else {
        log::debug!(
            "[permissions] no default input device — possible permission denial or no mic connected"
        );
        PermissionState::Unknown
    }
}

/// Detect microphone permission. See the `microphone-probe` feature; without it the answer is `Unknown` on desktop platforms.
#[cfg(all(feature = "microphone-probe", target_os = "linux"))]
#[must_use]
pub fn detect_microphone_permission() -> PermissionState {
    // Standard Linux desktops (PulseAudio/PipeWire) don't enforce app-level mic permissions.
    // Detect Flatpak sandbox — input device presence cannot confirm capture access.
    let is_sandboxed = std::env::var("FLATPAK_ID").is_ok()
        || std::path::Path::new("/run/flatpak").exists()
        || std::env::var("SNAP").is_ok()
        || std::env::var("SNAP_NAME").is_ok()
        || std::env::var("SNAP_INSTANCE_NAME").is_ok();
    linux_microphone_permission(is_sandboxed, || {
        use cpal::traits::HostTrait;
        cpal::default_host().default_input_device().is_some()
    })
}

#[cfg(all(feature = "microphone-probe", target_os = "linux"))]
fn linux_microphone_permission(
    is_sandboxed: bool,
    has_default_input_device: impl FnOnce() -> bool,
) -> PermissionState {
    if !is_sandboxed {
        PermissionState::Granted
    } else if has_default_input_device() {
        PermissionState::Unknown
    } else {
        log::debug!(
            "[permissions] Linux (Flatpak): no default input device — possible sandbox restriction"
        );
        PermissionState::Denied
    }
}

/// With the `microphone-probe` feature off, `cpal` is not compiled in, so there is no
/// audio-device API to probe and the microphone cannot be inspected. Report
/// `Unknown` on otherwise-supported desktop platforms rather than a misleading
/// `Granted`/`Denied`.
#[cfg(all(
    not(feature = "microphone-probe"),
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
#[must_use]
/// Detect microphone permission. See the `microphone-probe` feature; without it the answer is `Unknown` on desktop platforms.
pub fn detect_microphone_permission() -> PermissionState {
    log::debug!(
        "[permissions] microphone probe unavailable (built without the `microphone-probe` feature)"
    );
    PermissionState::Unknown
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
/// Detect microphone permission. See the `microphone-probe` feature; without it the answer is `Unknown` on desktop platforms.
pub fn detect_microphone_permission() -> PermissionState {
    PermissionState::Unsupported
}

/// Open the operating system's microphone privacy settings where available.
///
/// - **macOS**: Opens System Settings > Privacy & Security > Microphone. It does not
///   request authorization or trigger a system permission prompt.
/// - **Windows**: Opens the Privacy > Microphone settings page.
/// - **Linux**: No-op; sandbox guidance is included in error messages.
#[cfg(target_os = "macos")]
pub fn request_microphone_access() {
    log::debug!("[permissions] requesting macOS microphone access via Privacy pane");
    open_macos_privacy_pane("Privacy_Microphone");
}

/// Send the user to where microphone access is granted, where the OS has one.
#[cfg(target_os = "windows")]
pub fn request_microphone_access() {
    log::debug!("[permissions] opening Windows Privacy > Microphone settings");
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "ms-settings:privacy-microphone"])
        .status();
}

/// Send the user to where microphone access is granted, where the OS has one.
#[cfg(target_os = "linux")]
pub fn request_microphone_access() {
    log::debug!("[permissions] Linux: no programmatic mic permission request available");
    // No-op — standard Linux desktops don't have an app-level permission gate.
    // For Flatpak, the XDG Portal API (ashpd crate) could be used in the future.
}

/// Send the user to where microphone access is granted, where the OS has one.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn request_microphone_access() {
    // Unsupported platform — no-op.
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;

/// Returns a platform-specific user-facing message when microphone permission is denied.
#[must_use]
pub fn microphone_denied_message() -> String {
    #[cfg(target_os = "macos")]
    {
        "Microphone permission denied. Grant access in System Settings > Privacy & Security > Microphone, then restart the app.".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "Microphone access unavailable. Check Settings > Privacy & Security > Microphone and ensure the app is allowed. If no microphone is connected, plug one in.".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        "No microphone device available. Check your audio settings and ensure a microphone is connected. If running in a Flatpak sandbox, grant microphone access via Flatseal or system settings.".to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "Microphone access is not supported on this platform.".to_string()
    }
}

#[cfg(target_os = "macos")]
#[must_use]
/// Snapshot every permission this crate reports.
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        accessibility: detect_accessibility_permission(),
        input_monitoring: detect_input_monitoring_permission(),
        microphone: detect_microphone_permission(),
    }
}

#[cfg(not(target_os = "macos"))]
#[must_use]
/// Snapshot every permission this crate reports.
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        accessibility: PermissionState::Unsupported,
        input_monitoring: PermissionState::Unsupported,
        microphone: detect_microphone_permission(),
    }
}
