//! Unified Swift helper process: focus queries, paste, and overlay in one native binary.
//!
//! Replaces the separate osascript subprocess spawns and standalone overlay binary
//! with a single persistent Swift process communicating via stdin/stdout JSON.
//!
//! ## Mutex architecture
//!
//! Three globals prevent deadlock between fire-and-forget (show/hide) and
//! request-response (focus/paste) callers:
//!
//! - `UNIFIED_HELPER`: guards the process handle + stdin writer.
//!   Held only for the brief duration of a stdin write (~μs).
//! - `RESPONSE_RX`: guards the mpsc receiver that the background reader
//!   thread populates.  Held only for the duration of `recv_timeout`.
//! - `RECV_SERIALISER`: held for the entire send+receive round-trip so that
//!   two callers cannot interleave their reads.
//!
//! Fire-and-forget callers never touch `RESPONSE_RX` or `RECV_SERIALISER`,
//! so `show`/`hide` can proceed while a `focus` query is in-flight.
//!
//! Split by responsibility: [`process`] owns the process lifecycle and the
//! send/receive paths, and the `swift_*` modules hold the embedded Swift
//! source (assembled by [`swift_source`]) that the helper binary compiles
//! from.

mod process;
mod swift_ax_actions;
mod swift_focus;
mod swift_overlay;
mod swift_paste;
mod swift_source;

#[cfg(target_os = "macos")]
pub(crate) fn private_cache_dir(name: &str) -> Result<std::path::PathBuf, String> {
    let temp_dir = std::fs::canonicalize(std::env::temp_dir())
        .map_err(|error| format!("failed to resolve temporary directory: {error}"))?;
    private_cache_dir_in(&temp_dir, name)
}

/// [`private_cache_dir`] under `temp_dir`, an already-resolved directory.
#[cfg(target_os = "macos")]
fn private_cache_dir_in(
    temp_dir: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let temp_metadata = std::fs::symlink_metadata(temp_dir)
        .map_err(|error| format!("failed to inspect temporary directory: {error}"))?;
    if !temp_metadata.is_dir()
        || temp_metadata.mode() & 0o077 != 0
        || temp_metadata.mode() & 0o700 != 0o700
    {
        return Err("temporary directory is not private to its owner".to_string());
    }

    let cache_dir = temp_dir.join(name);
    match std::fs::create_dir(&cache_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("failed to create helper cache directory: {error}")),
    }

    let cache_metadata = std::fs::symlink_metadata(&cache_dir)
        .map_err(|error| format!("failed to inspect helper cache directory: {error}"))?;
    if cache_metadata.file_type().is_symlink()
        || !cache_metadata.is_dir()
        || cache_metadata.uid() != temp_metadata.uid()
    {
        return Err("helper cache directory has unexpected ownership or type".to_string());
    }
    std::fs::set_permissions(&cache_dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to secure helper cache directory: {error}"))?;

    Ok(cache_dir)
}

#[cfg(test)]
#[path = "helper_tests.rs"]
mod tests;

#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub(crate) use process::helper_send_receive;
pub use process::precompile_helper_background;
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub(crate) use process::{helper_quit, helper_send_fire_and_forget};
