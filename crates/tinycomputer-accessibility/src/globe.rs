//! macOS Globe/Fn key listener helper management.
//!
//! The listener runs as a tiny Swift process that monitors `flagsChanged`
//! events globally and reports `FN_DOWN` / `FN_UP` lines over stdout.

#[cfg(target_os = "macos")]
use super::Error;
use super::{PermissionState, Result as AccessibilityResult};
#[cfg(any(target_os = "macos", test))]
#[path = "globe/compiler.rs"]
mod compiler;
#[cfg(not(target_os = "macos"))]
#[path = "globe/fallback.rs"]
mod fallback;
#[cfg(target_os = "macos")]
#[path = "globe/helper.rs"]
mod helper;
#[cfg(any(target_os = "macos", test))]
#[path = "globe/owned.rs"]
mod owned;
#[cfg(any(target_os = "macos", test))]
#[path = "globe/queue.rs"]
mod queue;
#[cfg(any(target_os = "macos", test))]
#[path = "globe/reader.rs"]
mod reader;
#[cfg(target_os = "macos")]
use command_group::CommandGroup;
#[cfg(not(target_os = "macos"))]
pub use fallback::{
    globe_listener_poll, globe_listener_read, globe_listener_read_with_cancel,
    globe_listener_start, globe_listener_start_with_cancel, globe_listener_stop,
};

#[cfg(target_os = "macos")]
use std::io::BufReader;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
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

pub use tinycomputer_bus::accessibility::{GlobeHotkeyPollResult, GlobeHotkeyStatus};

#[cfg(target_os = "macos")]
struct GlobeListenerProcess {
    child: owned::Owned,
    ready: bool,
    event_queue: Arc<StdMutex<queue::Queue>>,
    /// Most recent listener error, if any.
    last_error: Arc<StdMutex<Option<String>>>,
}

#[cfg(target_os = "macos")]
static GLOBE_LISTENER: LazyLock<StdMutex<Option<GlobeListenerProcess>>> =
    LazyLock::new(|| StdMutex::new(None));

#[cfg(target_os = "macos")]
fn push_event(queue: &Arc<StdMutex<queue::Queue>>, event: String) {
    let Ok(mut guard) = queue.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock queue for event");
        return;
    };
    guard.push(event);
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
fn drain_events(queue: &Arc<StdMutex<queue::Queue>>) -> Vec<String> {
    let Ok(mut guard) = queue.lock() else {
        log::warn!("{LOG_PREFIX} failed to lock queue for drain");
        return Vec::new();
    };
    guard.drain().0
}

#[cfg(target_os = "macos")]
fn queue_len(queue: &Arc<StdMutex<queue::Queue>>) -> usize {
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
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<GlobeHotkeyStatus, String> {
    let restarted = state.is_some();
    if let Some(process) = state.as_mut().filter(|process| !process.ready) {
        process.child.cleanup().map_err(str::to_owned)?;
        *state = None;
    }
    let input_monitoring_permission = input_monitoring_permission();
    if input_monitoring_permission != PermissionState::Granted {
        let message =
            "input monitoring permission is required for the macOS Globe/Fn listener".to_string();
        log::warn!(
            "{LOG_PREFIX} start skipped: input_monitoring_permission={input_monitoring_permission:?}"
        );
        if let Some(process) = state.as_mut() {
            process.child.cleanup().map_err(str::to_owned)?;
            let _ = drain_events(&process.event_queue);
        }
        *state = None;
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
                process.child.cleanup().map_err(str::to_owned)?;
                *state = None;
            }
            Err(err) => {
                let message = format!("failed to inspect globe listener state: {err}");
                log::warn!("{LOG_PREFIX} {message}");
                set_last_error(&process.last_error, Some(message));
                process.child.cleanup().map_err(str::to_owned)?;
                *state = None;
            }
        }
    }

    let binary_path = helper::ensure_globe_helper_binary(cancel)?;
    log::info!("{LOG_PREFIX} starting helper {}", binary_path.display());
    let child = Command::new(&binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .group_spawn()
        .map_err(|_| "native listener spawn failed".to_owned())?;

    let mut queue = queue::Queue::default();
    if restarted {
        queue.discontinuity();
    }
    let event_queue = Arc::new(StdMutex::new(queue));
    let last_error = Arc::new(StdMutex::new(None));

    *state = Some(GlobeListenerProcess {
        child: owned::Owned::new(child),
        ready: false,
        event_queue,
        last_error,
    });
    let process = state
        .as_mut()
        .ok_or_else(|| "native ownership missing".to_owned())?;
    let stdout = process
        .child
        .child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| "failed to capture globe listener stdout".to_string())?;
    let stderr = process
        .child
        .child
        .inner()
        .stderr
        .take()
        .ok_or_else(|| "failed to capture globe listener stderr".to_string())?;

    {
        let queue = process.event_queue.clone();
        let error_store = process.last_error.clone();
        let reader = std::thread::Builder::new()
            .spawn(move || {
                if let Err(reason) = reader::events(BufReader::new(stdout), |event| {
                    push_event(&queue, event.to_owned());
                }) {
                    set_last_error(&error_store, Some(reason.into()));
                    if let Ok(mut queue) = queue.lock() {
                        queue.push("invalid".into());
                    }
                }
                log::debug!("{LOG_PREFIX} stdout reader exited");
            })
            .map_err(|_| "native reader spawn failed".to_owned())?;
        process.child.reader(reader);
    }

    {
        let error_store = process.last_error.clone();
        let reader = std::thread::Builder::new()
            .spawn(move || {
                if let Err(reason) = reader::errors(stderr, || {
                    set_last_error(&error_store, Some("native helper reported an error".into()));
                }) {
                    set_last_error(&error_store, Some(reason.into()));
                }
                log::debug!("{LOG_PREFIX} stderr reader exited");
            })
            .map_err(|_| "native reader spawn failed".to_owned())?;
        process.child.reader(reader);
    }

    let process = state
        .as_mut()
        .ok_or_else(|| "globe listener process missing after spawn".to_string())?;
    process.ready = true;
    Ok(GlobeHotkeyStatus {
        supported: true,
        running: true,
        input_monitoring_permission,
        last_error: current_error(&process.last_error),
        events_pending: queue_len(&process.event_queue),
    })
}

#[cfg(target_os = "macos")]
/// Start the Globe/Fn key listener process; unsupported platforms report `supported: false`.
///
/// # Errors
///
/// Returns a message when the listener helper cannot be built or started.
pub fn globe_listener_start() -> AccessibilityResult<GlobeHotkeyStatus> {
    globe_listener_start_with_cancel(&std::sync::atomic::AtomicBool::new(false))
}

#[cfg(target_os = "macos")]
/// Start native work under an owning terminal cancellation predicate.
///
/// # Errors
/// Returns native startup or cleanup failure; pending compiler ownership remains stoppable.
pub fn globe_listener_start_with_cancel(
    cancel: &std::sync::atomic::AtomicBool,
) -> AccessibilityResult<GlobeHotkeyStatus> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| Error::GlobeListener("globe listener lock poisoned".to_string()))?;
    ensure_running_locked(&mut guard, cancel).map_err(Error::GlobeListener)
}

#[cfg(target_os = "macos")]
/// Drain pending Globe/Fn key events and report listener status.
///
/// # Errors
///
/// Returns a message when the listener state cannot be read.
pub fn globe_listener_poll() -> AccessibilityResult<GlobeHotkeyPollResult> {
    globe_listener_read().map(|(result, _)| result)
}

#[cfg(target_os = "macos")]
/// Read bounded native events and whether overflow lost continuity.
///
/// # Errors
/// Returns a listener failure when native state cannot be read.
pub fn globe_listener_read() -> AccessibilityResult<(GlobeHotkeyPollResult, bool)> {
    globe_listener_read_with_cancel(&std::sync::atomic::AtomicBool::new(false))
}

#[cfg(target_os = "macos")]
/// Read native events with a terminal cancellation predicate for recovery startup.
///
/// # Errors
/// Returns native read, startup or retained cleanup failure.
pub fn globe_listener_read_with_cancel(
    cancel: &std::sync::atomic::AtomicBool,
) -> AccessibilityResult<(GlobeHotkeyPollResult, bool)> {
    let mut guard = GLOBE_LISTENER
        .lock()
        .map_err(|_| Error::GlobeListener("globe listener lock poisoned".to_string()))?;
    let status = if guard.is_some() {
        ensure_running_locked(&mut guard, cancel).map_err(Error::GlobeListener)?
    } else {
        GlobeHotkeyStatus {
            supported: true,
            running: false,
            input_monitoring_permission: input_monitoring_permission(),
            last_error: None,
            events_pending: 0,
        }
    };
    let (events, overflow) = guard
        .as_ref()
        .map(|process| {
            process
                .event_queue
                .lock()
                .map_or_else(|_| (Vec::new(), true), |mut queue| queue.drain())
        })
        .unwrap_or_default();
    Ok((
        GlobeHotkeyPollResult {
            status: GlobeHotkeyStatus {
                events_pending: 0,
                ..status
            },
            events,
        },
        overflow,
    ))
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
    compiler::stop().map_err(Error::GlobeListener)?;
    if let Some(process) = guard.as_mut() {
        process
            .child
            .cleanup()
            .map_err(|reason| Error::GlobeListener(reason.into()))?;
        let _ = drain_events(&process.event_queue);
    }
    *guard = None;

    Ok(GlobeHotkeyStatus {
        supported: true,
        running: false,
        input_monitoring_permission: input_monitoring_permission(),
        last_error: None,
        events_pending: 0,
    })
}

#[cfg(test)]
#[path = "globe_tests.rs"]
mod tests;
