//! Text insertion into the currently active text field.
//!
//! Uses the **clipboard-paste** strategy (like OpenWhispr): writes text
//! to the system clipboard then simulates Cmd+V / Ctrl+V to paste it.
//! This is atomic and instantaneous, unlike enigo's `text()` which types
//! character-by-character and causes garbled/repeated output on macOS.
//!
//! The previous clipboard contents are saved and restored after a short
//! delay so the user's clipboard is not permanently overwritten.

use std::time::Duration;

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use log::{debug, info, warn};

const LOG_PREFIX: &str = "[voice_input]";

/// Delay before sending Cmd+V, letting the clipboard write settle.
/// `OpenWhispr` uses 120ms on macOS.
const PASTE_DELAY: Duration = Duration::from_millis(120);

/// Delay after sending Cmd+V before restoring the clipboard, giving the
/// target application time to read from the clipboard.
/// `OpenWhispr` uses 450ms on macOS.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(450);
#[cfg(target_os = "macos")]
const FOCUS_RESTORE_DELAY: Duration = Duration::from_millis(100);

/// Insert text into the currently active text field via clipboard-paste.
///
/// Strategy:
/// 1. Save current clipboard contents
/// 2. Write transcribed text to clipboard
/// 3. Simulate Cmd+V (macOS) or Ctrl+V (Windows/Linux)
/// 4. Wait briefly, then restore original clipboard
///
/// This avoids the character-by-character typing issues with enigo's
/// `text()` method which causes garbled/repeated output.
///
/// `expected_app` (macOS only) is the application that should own the focused
/// field; if focus has moved, the app is re-activated before pasting.
///
/// # Errors
///
/// Returns a message when the clipboard or the synthetic keyboard cannot be
/// reached, or when a keystroke cannot be sent. Empty or whitespace-only text
/// is a successful no-op.
pub fn insert_text(text: &str, expected_app: Option<&str>) -> Result<(), String> {
    insert_with(
        &System,
        text,
        expected_app,
        PASTE_DELAY,
        CLIPBOARD_RESTORE_DELAY,
    )
    .map(|_| ())
}

/// The clipboard operations the paste flow needs.
trait Pasteboard {
    fn read(&mut self) -> Option<String>;
    fn write(&mut self, text: &str) -> Result<(), String>;
}

/// The synthetic keyboard the paste flow drives.
trait KeyOps {
    fn key(&mut self, key: Key, direction: Direction) -> Result<(), String>;
}

/// Where the clipboard and keyboard come from; faked in tests so the flow runs
/// headless.
trait Platform: Clone + Send + 'static {
    fn pasteboard(&self) -> Result<Box<dyn Pasteboard>, String>;
    fn keyboard(&self) -> Result<Box<dyn KeyOps>, String>;
}

/// The real system clipboard and keyboard.
#[derive(Clone, Copy)]
struct System;

impl Pasteboard for Clipboard {
    fn read(&mut self) -> Option<String> {
        self.get_text().ok()
    }
    fn write(&mut self, text: &str) -> Result<(), String> {
        self.set_text(text).map_err(|e| e.to_string())
    }
}

impl KeyOps for Enigo {
    fn key(&mut self, key: Key, direction: Direction) -> Result<(), String> {
        Keyboard::key(self, key, direction).map_err(|e| e.to_string())
    }
}

impl Platform for System {
    fn pasteboard(&self) -> Result<Box<dyn Pasteboard>, String> {
        Clipboard::new()
            .map(|c| Box::new(c) as Box<dyn Pasteboard>)
            .map_err(|e| e.to_string())
    }
    fn keyboard(&self) -> Result<Box<dyn KeyOps>, String> {
        Enigo::new(&Settings::default())
            .map(|e| Box::new(e) as Box<dyn KeyOps>)
            .map_err(|e| e.to_string())
    }
}

/// The paste flow over an injectable platform. Returns the clipboard-restore
/// thread, when one was started, so tests can wait for it.
fn insert_with<P: Platform>(
    platform: &P,
    text: &str,
    expected_app: Option<&str>,
    paste_delay: Duration,
    restore_delay: Duration,
) -> Result<Option<std::thread::JoinHandle<()>>, String> {
    if text.trim().is_empty() {
        warn!("{LOG_PREFIX} transcription was empty/whitespace, skipping insertion");
        return Ok(None);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = expected_app;

    info!(
        "{LOG_PREFIX} inserting {} chars via clipboard-paste",
        text.len()
    );

    // Step 1: Save current clipboard.
    let mut clipboard = platform
        .pasteboard()
        .map_err(|e| format!("failed to access clipboard: {e}"))?;
    let saved_clipboard = clipboard.read();
    debug!(
        "{LOG_PREFIX} saved clipboard ({} chars)",
        saved_clipboard.as_ref().map_or(0, String::len)
    );

    // Step 2: Write transcription to clipboard.
    clipboard
        .write(text)
        .map_err(|e| format!("failed to write text to clipboard: {e}"))?;
    debug!("{LOG_PREFIX} transcription written to clipboard");

    // Step 3: Brief delay to let clipboard write settle, then simulate paste.
    std::thread::sleep(paste_delay);

    #[cfg(target_os = "macos")]
    if let Some(app_name) = expected_app {
        debug!("{LOG_PREFIX} validating focus before paste; expected_app='{app_name}'");
        if let Err(validation_err) = crate::validate_focused_target(Some(app_name), None, None) {
            warn!("{LOG_PREFIX} focus changed before paste: {validation_err}");
            // Always try to restore focus — even if the user hasn't clicked a
            // text field yet, activating the app brings it to front and most
            // apps will accept Cmd+V into their last-focused element.
            if let Err(restore_err) = restore_focus_to_app(app_name) {
                warn!(
                    "{LOG_PREFIX} focus restore failed: {restore_err} — will attempt paste anyway"
                );
            } else {
                info!("{LOG_PREFIX} focus restored to '{app_name}' before paste");
            }
        }
    }

    let mut keys = platform
        .keyboard()
        .map_err(|e| format!("failed to create enigo instance: {e}"))?;

    let modifier = paste_modifier_key();
    keys.key(modifier, Direction::Press)
        .map_err(|e| format!("failed to press modifier: {e}"))?;
    keys.key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| format!("failed to press 'v': {e}"))?;
    keys.key(modifier, Direction::Release)
        .map_err(|e| format!("failed to release modifier: {e}"))?;

    debug!("{LOG_PREFIX} paste keystroke sent");

    // Step 4: Restore clipboard after a delay (non-blocking).
    let restore = saved_clipboard.map(|original| {
        let platform = platform.clone();
        std::thread::spawn(move || {
            std::thread::sleep(restore_delay);
            match platform.pasteboard() {
                Ok(mut cb) => {
                    if let Err(e) = cb.write(&original) {
                        warn!("{LOG_PREFIX} failed to restore clipboard: {e}");
                    } else {
                        debug!("{LOG_PREFIX} clipboard restored");
                    }
                }
                Err(e) => warn!("{LOG_PREFIX} failed to re-open clipboard for restore: {e}"),
            }
        })
    });

    info!("{LOG_PREFIX} text inserted successfully via paste");
    Ok(restore)
}

#[cfg(target_os = "macos")]
fn restore_focus_to_app(app_name: &str) -> Result<(), String> {
    let script = format!(
        r#"tell application "{}" to activate"#,
        escape_applescript_string(app_name)
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript for focus restore: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "unknown osascript error".to_string()
        } else {
            stderr
        };
        return Err(format!("failed to restore focus to '{app_name}': {detail}"));
    }
    std::thread::sleep(FOCUS_RESTORE_DELAY);
    Ok(())
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Returns the platform-appropriate paste modifier key.
fn paste_modifier_key() -> Key {
    if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    }
}

#[cfg(test)]
#[path = "paste_tests.rs"]
mod tests;
