//! macOS Globe/Fn key listener helper management.
//!
//! The listener runs as a tiny Swift process that monitors `flagsChanged`
//! events globally and reports `FN_DOWN` / `FN_UP` lines over stdout.

#[cfg(target_os = "macos")]
use super::Error;
use super::{PermissionState, Result as AccessibilityResult};
#[cfg(any(target_os = "macos", test))]
use std::collections::VecDeque;

#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::hash::{Hash, Hasher};
#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::LazyLock;
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex as StdMutex};

fn input_monitoring_permission() -> PermissionState {
    #[cfg(target_os = "macos")]
    {
        super::detect_input_monitoring_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionState::Unsupported
    }
}

#[cfg(target_os = "macos")]
const LOG_PREFIX: &str = "[globe_hotkey]";
#[cfg(any(target_os = "macos", test))]
const MAX_PENDING_EVENTS: usize = 64;

pub use tinycomputer_bus::accessibility::{GlobeHotkeyPollResult, GlobeHotkeyStatus};

#[cfg(target_os = "macos")]
struct GlobeListenerProcess {
    child: Child,
    event_queue: Arc<StdMutex<VecDeque<String>>>,
    /// Most recent listener error, if any.
    last_error: Arc<StdMutex<Option<String>>>,
}

#[cfg(target_os = "macos")]
static GLOBE_LISTENER: LazyLock<StdMutex<Option<GlobeListenerProcess>>> =
    LazyLock::new(|| StdMutex::new(None));

#[cfg(target_os = "macos")]
fn push_event(queue: &Arc<StdMutex<VecDeque<String>>>, event: String) {
    let Ok(mut guard) = queue.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock queue for event");
        return;
    };
    guard.push_back(event);
    trim_event_queue(&mut guard);
}

#[cfg(any(target_os = "macos", test))]
fn trim_event_queue(queue: &mut VecDeque<String>) {
    while queue.len() > MAX_PENDING_EVENTS {
        let _ = queue.pop_front();
    }
}

#[cfg(target_os = "macos")]
fn set_last_error(error_store: &Arc<StdMutex<Option<String>>>, message: Option<String>) {
    let Ok(mut guard) = error_store.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock last_error store");
        return;
    };
    *guard = message;
}

#[cfg(target_os = "macos")]
fn drain_events(queue: &Arc<StdMutex<VecDeque<String>>>) -> Vec<String> {
    let Ok(mut guard) = queue.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock queue for drain");
        return Vec::new();
    };
    guard.drain(..).collect()
}

#[cfg(target_os = "macos")]
fn queue_len(queue: &Arc<StdMutex<VecDeque<String>>>) -> usize {
    let Ok(guard) = queue.lock() else {
        return 0;
    };
    guard.len()
}

#[cfg(target_os = "macos")]
fn current_error(error_store: &Arc<StdMutex<Option<String>>>) -> Option<String> {
    let Ok(guard) = error_store.lock() else {
        return Some("failed to read globe listener error state".to_string());
    };
    guard.clone()
}

#[cfg(target_os = "macos")]
// Spawn, wire the reader threads, and record the process as one unit so the
// lock is held across the whole transition.
#[allow(clippy::too_many_lines)]
fn ensure_running_locked(
    state: &mut Option<GlobeListenerProcess>,
) -> Result<GlobeHotkeyStatus, String> {
    let input_monitoring_permission = input_monitoring_permission();
    if input_monitoring_permission != PermissionState::Granted {
        let message =
            "input monitoring permission is required for the macOS Globe/Fn listener".to_string();
        log::warn!(
            "{LOG_PREFIX} start skipped: input_monitoring_permission={input_monitoring_permission:?}"
        );
        if let Some(mut process) = state.take() {
            set_last_error(&process.last_error, Some(message.clone()));
            let _ = process.child.kill();
            let _ = process.child.wait();
            let _ = drain_events(&process.event_queue);
        }
        return Ok(GlobeHotkeyStatus {
            supported: true,
            running: false,
            input_monitoring_permission,
            last_error: Some(message),
            events_pending: 0,
        });
    }

    if let Some(process) = state.as_mut() {
        match process.child.try_wait() {
            Ok(None) => {
                return Ok(GlobeHotkeyStatus {
                    supported: true,
                    running: true,
                    input_monitoring_permission,
                    last_error: current_error(&process.last_error),
                    events_pending: queue_len(&process.event_queue),
                });
            }
            Ok(Some(status)) => {
                let message = format!("globe listener exited unexpectedly: {status}");
                log::warn!("{LOG_PREFIX} {message}");
                set_last_error(&process.last_error, Some(message));
                *state = None;
            }
            Err(err) => {
                let message = format!("failed to inspect globe listener state: {err}");
                log::warn!("{LOG_PREFIX} {message}");
                set_last_error(&process.last_error, Some(message));
                let _ = process.child.kill();
                let _ = process.child.wait();
                *state = None;
            }
        }
    }

    let binary_path = ensure_globe_helper_binary()?;
    log::info!("{LOG_PREFIX} starting helper {}", binary_path.display());
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn globe listener helper: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture globe listener stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture globe listener stderr".to_string())?;

    let event_queue = Arc::new(StdMutex::new(VecDeque::with_capacity(MAX_PENDING_EVENTS)));
    let last_error = Arc::new(StdMutex::new(None));

    {
        let queue = event_queue.clone();
        let error_store = last_error.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        log::debug!("{LOG_PREFIX} helper event={trimmed}");
                        push_event(&queue, trimmed.to_string());
                        set_last_error(&error_store, None);
                    }
                    Err(err) => {
                        let message = format!("failed reading globe listener stdout: {err}");
                        log::warn!("{LOG_PREFIX} {message}");
                        set_last_error(&error_store, Some(message));
                        break;
                    }
                }
            }
            log::debug!("{LOG_PREFIX} stdout reader exited");
        });
    }

    {
        let error_store = last_error.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        log::warn!("{LOG_PREFIX} helper stderr={trimmed}");
                        set_last_error(&error_store, Some(trimmed.to_string()));
                    }
                    Err(err) => {
                        let message = format!("failed reading globe listener stderr: {err}");
                        log::warn!("{LOG_PREFIX} {message}");
                        set_last_error(&error_store, Some(message));
                        break;
                    }
                }
            }
            log::debug!("{LOG_PREFIX} stderr reader exited");
        });
    }

    *state = Some(GlobeListenerProcess {
        child,
        event_queue,
        last_error,
    });

    let process = state
        .as_ref()
        .ok_or_else(|| "globe listener process missing after spawn".to_string())?;
    Ok(GlobeHotkeyStatus {
        supported: true,
        running: true,
        input_monitoring_permission,
        last_error: current_error(&process.last_error),
        events_pending: queue_len(&process.event_queue),
    })
}

#[cfg(target_os = "macos")]
fn ensure_globe_helper_binary() -> Result<PathBuf, String> {
    let cache_dir = super::helper::private_cache_dir("openhuman-globe-listener")?;

    let source = globe_swift_source();
    let mut source_hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut source_hasher);
    let source_id = format!("{:016x}", source_hasher.finish());
    let source_path = cache_dir.join(format!("globe_listener_{source_id}.swift"));
    let binary_path = cache_dir.join(format!("globe_listener_{source_id}"));

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, &source)
            .map_err(|e| format!("failed to write globe helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        let temporary_binary = cache_dir.join(format!(
            "globe_listener_{source_id}.tmp-{}",
            std::process::id()
        ));
        log::debug!("{LOG_PREFIX} compiling Swift helper");
        let output = Command::new("xcrun")
            .args(["swiftc", "-O", "-framework", "Cocoa"])
            .arg(&source_path)
            .arg("-o")
            .arg(&temporary_binary)
            .output()
            .or_else(|_| {
                Command::new("swiftc")
                    .args(["-O", "-framework", "Cocoa"])
                    .arg(&source_path)
                    .arg("-o")
                    .arg(&temporary_binary)
                    .output()
            })
            .map_err(|e| format!("failed to invoke swiftc for globe listener: {e}"))?;
        if !output.status.success() {
            let _ = fs::remove_file(&temporary_binary);
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "failed to compile globe listener helper: {}",
                if stderr.is_empty() {
                    "swiftc returned non-zero exit status".to_string()
                } else {
                    stderr
                }
            ));
        }
        fs::rename(&temporary_binary, &binary_path)
            .map_err(|e| format!("failed to install compiled globe listener helper: {e}"))?;
        log::debug!("{LOG_PREFIX} Swift helper compiled successfully");
    }

    Ok(binary_path)
}

#[cfg(target_os = "macos")]
fn globe_swift_source() -> String {
    r#"import Cocoa
import Darwin

var fnIsDown = false
func emit(_ message: String) {
    FileHandle.standardOutput.write((message + "\n").data(using: .utf8)!)
    fflush(stdout)
}

guard let monitor = NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged, handler: { event in
    let flags = event.modifierFlags
    let containsFn = flags.contains(.function)

    if containsFn && !fnIsDown {
        fnIsDown = true
        emit("FN_DOWN")
    } else if !containsFn && fnIsDown {
        fnIsDown = false
        emit("FN_UP")
    }

}) else {
    FileHandle.standardError.write("Failed to create event monitor\n".data(using: .utf8)!)
    exit(1)
}

let signalSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
signal(SIGTERM, SIG_IGN)
signalSource.setEventHandler {
    NSEvent.removeMonitor(monitor)
    exit(0)
}
signalSource.resume()

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
app.run()
"#
    .to_string()
}

#[cfg(target_os = "macos")]
/// Start the Globe/Fn key listener process; unsupported platforms report `supported: false`.
///
/// # Errors
///
/// Returns a message when the listener helper cannot be built or started.
pub fn globe_listener_start() -> AccessibilityResult<GlobeHotkeyStatus> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| Error::GlobeListener("globe listener lock poisoned".to_string()))?;
    ensure_running_locked(&mut guard).map_err(Error::GlobeListener)
}

#[cfg(target_os = "macos")]
/// Drain pending Globe/Fn key events and report listener status.
///
/// # Errors
///
/// Returns a message when the listener state cannot be read.
pub fn globe_listener_poll() -> AccessibilityResult<GlobeHotkeyPollResult> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| Error::GlobeListener("globe listener lock poisoned".to_string()))?;
    let status = if guard.is_some() {
        ensure_running_locked(&mut guard).map_err(Error::GlobeListener)?
    } else {
        GlobeHotkeyStatus {
            supported: true,
            running: false,
            input_monitoring_permission: input_monitoring_permission(),
            last_error: None,
            events_pending: 0,
        }
    };
    let events = guard
        .as_ref()
        .map(|process| drain_events(&process.event_queue))
        .unwrap_or_default();
    Ok(GlobeHotkeyPollResult {
        status: GlobeHotkeyStatus {
            events_pending: 0,
            ..status
        },
        events,
    })
}

#[cfg(target_os = "macos")]
/// Stop the Globe/Fn key listener process.
///
/// # Errors
///
/// Returns a message when the listener state cannot be read.
pub fn globe_listener_stop() -> AccessibilityResult<GlobeHotkeyStatus> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| Error::GlobeListener("globe listener lock poisoned".to_string()))?;
    if let Some(mut process) = guard.take() {
        log::info!("{LOG_PREFIX} stopping helper pid={}", process.child.id());
        let _ = process.child.kill();
        let _ = process.child.wait();
        let events = drain_events(&process.event_queue);
        log::debug!(
            "{LOG_PREFIX} drained {} queued events on stop",
            events.len()
        );
    }

    Ok(GlobeHotkeyStatus {
        supported: true,
        running: false,
        input_monitoring_permission: input_monitoring_permission(),
        last_error: None,
        events_pending: 0,
    })
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
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

#[cfg(test)]
#[path = "globe_tests.rs"]
mod tests;
