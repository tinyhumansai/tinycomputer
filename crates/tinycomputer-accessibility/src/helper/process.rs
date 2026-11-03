//! Unified helper process lifecycle: spawning and supervising the persistent
//! Swift binary, the request/response and fire-and-forget send paths, and
//! compiling/caching the helper binary from its embedded Swift source.

#[cfg(target_os = "macos")]
use std::hash::{Hash, Hasher};
#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "macos")]
use std::sync::LazyLock;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::{Mutex as StdMutex, mpsc};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
use std::{
    fs,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
};

#[cfg(target_os = "macos")]
use super::swift_source::unified_swift_source;
#[cfg(target_os = "macos")]
use serde_json::Value;

/// Process handle + stdin writer.  Held only briefly for writes.
#[cfg(target_os = "macos")]
struct UnifiedHelperProcess {
    child: Child,
    stdin: ChildStdin,
}

/// Guards the process handle and stdin.
#[cfg(target_os = "macos")]
static UNIFIED_HELPER: LazyLock<StdMutex<Option<UnifiedHelperProcess>>> =
    LazyLock::new(|| StdMutex::new(None));

/// Channel receiver fed by the background stdout-reader thread.
/// Separate from `UNIFIED_HELPER` so fire-and-forget callers never contend here.
#[cfg(target_os = "macos")]
static RESPONSE_RX: LazyLock<StdMutex<Option<mpsc::Receiver<String>>>> =
    LazyLock::new(|| StdMutex::new(None));

/// Serialises request/response pairs so two callers cannot interleave reads.
/// Fire-and-forget callers never acquire this lock.
#[cfg(target_os = "macos")]
static RECV_SERIALISER: LazyLock<StdMutex<()>> = LazyLock::new(|| StdMutex::new(()));

/// Prevents concurrent Swift compiles from `ensure_helper_binary` vs background precompile.
#[cfg(target_os = "macos")]
static HELPER_COMPILE_LOCK: LazyLock<StdMutex<()>> = LazyLock::new(|| StdMutex::new(()));

/// Monotonic ids for `helper_send_receive` so a late line cannot be consumed as the wrong reply.
#[cfg(target_os = "macos")]
static HELPER_REQ_ID: AtomicU64 = AtomicU64::new(1);

/// Timeout for a single request/response round-trip with the Swift helper.
#[cfg(target_os = "macos")]
const HELPER_RECV_TIMEOUT: Duration = Duration::from_secs(8);

/// Send a JSON request and read a JSON response (one line each).
/// Used for `focus` and `paste` commands that produce a response.
///
/// Holds `RECV_SERIALISER` for the full round-trip, but releases
/// `UNIFIED_HELPER` before blocking on the channel recv, so fire-and-forget
/// callers (`show`/`hide`) are never blocked by an in-flight focus query.
#[cfg(target_os = "macos")]
pub(crate) fn helper_send_receive(
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Serialise request/response pairs — prevents interleaved reads.
    let _rr_guard = RECV_SERIALISER
        .lock()
        .map_err(|_| "recv serialiser lock poisoned".to_string())?;

    ensure_helper_running()?;

    let id_num = HELPER_REQ_ID.fetch_add(1, Ordering::Relaxed);
    let id_str = id_num.to_string();

    let mut req = request.clone();
    let req_obj = req
        .as_object_mut()
        .ok_or_else(|| "helper request must be a JSON object".to_string())?;
    req_obj.insert("id".to_string(), Value::String(id_str.clone()));

    // Write the request, holding UNIFIED_HELPER only for this brief write.
    {
        let mut guard = UNIFIED_HELPER
            .lock()
            .map_err(|_| "unified helper lock poisoned".to_string())?;
        let helper = guard
            .as_mut()
            .ok_or_else(|| "unified helper unavailable".to_string())?;
        let line = req.to_string();
        helper
            .stdin
            .write_all(line.as_bytes())
            .and_then(|()| helper.stdin.write_all(b"\n"))
            .and_then(|()| helper.stdin.flush())
            .map_err(|e| format!("failed to write to helper stdin: {e}"))?;
    } // UNIFIED_HELPER released here — fire-and-forget callers can proceed

    // Read until the line matches `id` (discards stale lines after a timeout or reordering).
    match await_reply(&RESPONSE_RX, &id_str, HELPER_RECV_TIMEOUT)? {
        Some(value) => Ok(value),
        None => {
            reset_helper_process();
            Err(format!(
                "helper response timed out waiting for id {id_str} (stale lines may follow)"
            ))
        }
    }
}

/// Reads lines from the helper's reply channel until one carries `id_str`,
/// discarding blank, stale, and unnumbered lines. `None` when `timeout`
/// passes first.
#[cfg(target_os = "macos")]
fn await_reply(
    rx_slot: &StdMutex<Option<mpsc::Receiver<String>>>,
    id_str: &str,
    timeout: Duration,
) -> Result<Option<Value>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let chunk = remaining.min(Duration::from_millis(500));
        let response_line = {
            let rx_guard = rx_slot
                .lock()
                .map_err(|_| "response rx lock poisoned".to_string())?;
            let rx = rx_guard
                .as_ref()
                .ok_or_else(|| "response channel unavailable".to_string())?;
            match rx.recv_timeout(chunk) {
                Ok(line) => line,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Non-fatal: the outer loop will check `remaining` and
                    // either retry or surface the top-level timeout.
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("helper response channel disconnected".to_string());
                }
            }
        };

        if response_line.trim().is_empty() {
            log::debug!(
                "[accessibility] helper skipped empty stdout line while waiting for id {id_str}"
            );
            continue;
        }

        let value: Value = serde_json::from_str(response_line.trim())
            .map_err(|e| format!("failed to parse helper response: {e}"))?;

        let matches = value
            .get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|rid| rid == id_str);
        if matches {
            return Ok(Some(value));
        }
        log::debug!(
            "[accessibility] discarding helper response with mismatched or missing id (want id={id_str})"
        );
    }
}

#[cfg(target_os = "macos")]
fn reset_helper_process() {
    if let Ok(mut guard) = UNIFIED_HELPER.lock() {
        if let Some(mut helper) = guard.take() {
            let _ = helper.child.kill();
            let _ = helper.child.wait();
        }
        if let Ok(mut rx_guard) = RESPONSE_RX.lock() {
            rx_guard.take();
        }
    }
}

/// Send a JSON request without waiting for a response.
/// Used for `show`, `hide`, and `quit` commands.
/// Only acquires `UNIFIED_HELPER` (for the stdin write) — never blocks on I/O.
#[cfg(target_os = "macos")]
// Part of the helper protocol (`show`/`hide`/`quit`); the overlay is not driven
// from this crate yet.
#[allow(dead_code)]
pub(crate) fn helper_send_fire_and_forget(request: &serde_json::Value) -> Result<(), String> {
    ensure_helper_running()?;
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;
    let helper = guard
        .as_mut()
        .ok_or_else(|| "unified helper unavailable".to_string())?;

    let line = request.to_string();
    helper
        .stdin
        .write_all(line.as_bytes())
        .and_then(|()| helper.stdin.write_all(b"\n"))
        .and_then(|()| helper.stdin.flush())
        .map_err(|e| format!("failed to write to helper stdin: {e}"))?;
    Ok(())
}

/// Quit and clean up the helper process.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub(crate) fn helper_quit() -> Result<(), String> {
    // Drop the response channel first so the reader thread exits cleanly.
    {
        let mut rx_guard = RESPONSE_RX
            .lock()
            .map_err(|_| "response rx lock poisoned".to_string())?;
        rx_guard.take();
    }
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;
    if let Some(mut helper) = guard.take() {
        let _ = helper.stdin.write_all(br#"{"type":"quit"}"#);
        let _ = helper.stdin.write_all(b"\n");
        let _ = helper.stdin.flush();
        let _ = helper.child.kill();
        let _ = helper.child.wait();
    }
    Ok(())
}

/// Ensure the helper process is running.  Spawns it (and the stdout reader
/// thread) if not yet started or if it has exited unexpectedly.
#[cfg(target_os = "macos")]
fn ensure_helper_running() -> Result<(), String> {
    let mut guard = UNIFIED_HELPER
        .lock()
        .map_err(|_| "unified helper lock poisoned".to_string())?;

    if let Some(helper) = guard.as_mut() {
        if helper
            .child
            .try_wait()
            .map_err(|e| format!("failed to query helper state: {e}"))?
            .is_none()
        {
            return Ok(()); // Still running
        }
        log::debug!("[accessibility] unified helper exited, restarting");
        *guard = None;
        // Also drop the stale receiver so a new one will be created below.
        if let Ok(mut rx_guard) = RESPONSE_RX.lock() {
            rx_guard.take();
        }
    }

    let binary_path = ensure_helper_binary()?;
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn unified helper: {e}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to capture helper stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture helper stdout".to_string())?;

    // Spawn a background thread that continuously reads lines from the helper's
    // stdout and forwards them into the channel.  The thread exits when the
    // sender is dropped (i.e. when helper_quit drops RESPONSE_RX) or when the
    // process closes its stdout.
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF — helper exited
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() && tx.send(trimmed).is_err() {
                        break; // Receiver dropped — time to exit
                    }
                }
                Err(e) => {
                    log::debug!("[accessibility] helper stdout reader error: {e}");
                    break;
                }
            }
        }
        log::debug!("[accessibility] helper stdout reader thread exiting");
    });

    // Store the new receiver.
    if let Ok(mut rx_guard) = RESPONSE_RX.lock() {
        *rx_guard = Some(rx);
    }

    *guard = Some(UnifiedHelperProcess { child, stdin });
    log::debug!("[accessibility] unified helper started");
    Ok(())
}

/// Compile the Swift helper binary in the background so the first overlay
/// request does not incur the compile latency.  Safe to call multiple times;
/// subsequent calls are no-ops (the binary is cached by `ensure_helper_binary`).
#[cfg(target_os = "macos")]
pub fn precompile_helper_background() {
    std::thread::spawn(|| {
        log::debug!("[accessibility] precompile_helper_background: starting");
        match ensure_helper_binary() {
            Ok(path) => log::debug!("[accessibility] helper binary ready: {}", path.display()),
            Err(e) => log::warn!(
                "[accessibility] helper precompile failed (will retry on first use): {e}"
            ),
        }
    });
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn precompile_helper_background() {}

#[cfg(target_os = "macos")]
fn ensure_helper_binary() -> Result<PathBuf, String> {
    let _compile_guard = HELPER_COMPILE_LOCK
        .lock()
        .map_err(|_| "helper compile lock poisoned".to_string())?;

    let cache_dir = super::private_cache_dir("openhuman-accessibility-helper")?;
    let source = unified_swift_source();
    let mut source_hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut source_hasher);
    let source_id = format!("{:016x}", source_hasher.finish());
    let source_path = cache_dir.join(format!("unified_helper_{source_id}.swift"));
    let binary_path = cache_dir.join(format!("unified_helper_{source_id}"));

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, &source)
            .map_err(|e| format!("failed to write helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        let temporary_binary = cache_dir.join(format!(
            "unified_helper_{source_id}.tmp-{}",
            std::process::id()
        ));
        log::debug!("[accessibility] compiling unified Swift helper");
        let output = Command::new("xcrun")
            .args([
                "swiftc",
                "-O",
                "-framework",
                "Cocoa",
                "-framework",
                "ApplicationServices",
            ])
            .arg(&source_path)
            .arg("-o")
            .arg(&temporary_binary)
            .output()
            .or_else(|_| {
                Command::new("swiftc")
                    .args([
                        "-O",
                        "-framework",
                        "Cocoa",
                        "-framework",
                        "ApplicationServices",
                    ])
                    .arg(&source_path)
                    .arg("-o")
                    .arg(&temporary_binary)
                    .output()
            })
            .map_err(|e| format!("failed to invoke swiftc: {e}"))?;
        if !output.status.success() {
            let _ = fs::remove_file(&temporary_binary);
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "failed to compile unified helper: {}",
                if stderr.is_empty() {
                    "swiftc returned non-zero exit status".to_string()
                } else {
                    stderr
                }
            ));
        }
        fs::rename(&temporary_binary, &binary_path)
            .map_err(|e| format!("failed to install compiled unified helper: {e}"))?;
        log::debug!("[accessibility] unified helper compiled successfully");
    }

    Ok(binary_path)
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
