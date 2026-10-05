//! Accessibility focus queries.
//!
//! Primary path: unified Swift helper (native AX API, fast, persistent process).
//! Fallback: osascript subprocess (slower, but works without compiled helper).

#[cfg(target_os = "macos")]
use super::terminal::{is_terminal_app, is_text_role};
#[cfg(target_os = "macos")]
use super::text_util::{normalize_ax_value, parse_ax_number};
#[cfg(target_os = "macos")]
use super::types::ElementBounds;
use super::types::FocusedTextContext;
use super::{Error, Result as AccessibilityResult};
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::{
    io::Read,
    process::{Command, ExitStatus, Output, Stdio},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
const FOCUS_COMMAND_TIMEOUT: Duration = Duration::from_millis(1_500);
#[cfg(any(target_os = "macos", all(test, unix)))]
const COMMAND_TIMEOUT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(any(target_os = "macos", all(test, unix)))]
fn command_output_with_timeout(
    command_name: &str,
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to run {command_name}: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_pipe(stdout));
    let stderr_reader = std::thread::spawn(move || read_pipe(stderr));
    let started_at = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return collect_command_output(command_name, status, stdout_reader, stderr_reader);
            }
            Ok(None) if started_at.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{command_name} timed out after {}ms",
                    timeout.as_millis()
                ));
            }
            Ok(None) => std::thread::sleep(COMMAND_TIMEOUT_POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to wait for {command_name}: {error}"));
            }
        }
    }
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn read_pipe(pipe: Option<impl Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    bytes
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn collect_command_output(
    command_name: &str,
    status: ExitStatus,
    stdout_reader: std::thread::JoinHandle<Vec<u8>>,
    stderr_reader: std::thread::JoinHandle<Vec<u8>>,
) -> Result<Output, String> {
    let stdout = stdout_reader
        .join()
        .map_err(|_| format!("failed to collect {command_name} stdout"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| format!("failed to collect {command_name} stderr"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

// ---------------------------------------------------------------------------
// Focus query: unified helper → osascript fallback
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
/// Query the OS for the focused text element, without verbose diagnostics.
///
/// # Errors
///
/// Returns a message when the focus query fails or is unsupported on this platform.
pub fn focused_text_context() -> AccessibilityResult<FocusedTextContext> {
    let ctx = focused_text_context_verbose()?;
    if let Some(err) = ctx.raw_error.as_ref() {
        return Err(Error::FocusQuery(format!(
            "focused text unavailable via accessibility api: {err}"
        )));
    }
    Ok(ctx)
}

/// Query the focused text element. Tries the unified Swift helper first (native AX, ~5-15ms),
/// falls back to osascript (~50-100ms) if the helper is unavailable.
///
/// # Errors
///
/// Returns a message when both the helper and the osascript fallback fail.
#[cfg(target_os = "macos")]
pub fn focused_text_context_verbose() -> AccessibilityResult<FocusedTextContext> {
    match focused_text_via_helper() {
        Ok(mut ctx) if ctx.raw_error.is_some() => {
            log::debug!(
                "[accessibility] helper returned raw_error={:?}, falling back to osascript",
                ctx.raw_error
            );
            match focused_text_via_osascript() {
                Ok(fallback) => Ok(fallback),
                Err(fallback_err) => {
                    log::debug!(
                        "[accessibility] osascript fallback failed ({fallback_err}); keeping helper context"
                    );
                    if let Some(helper_error) = ctx.raw_error.as_mut() {
                        use std::fmt::Write as _;
                        let _ = write!(helper_error, "; osascript fallback failed: {fallback_err}");
                    } else {
                        ctx.raw_error = Some(fallback_err);
                    }
                    Ok(ctx)
                }
            }
        }
        Ok(ctx) => Ok(ctx),
        Err(helper_err) => {
            log::debug!(
                "[accessibility] helper focus query failed ({helper_err}), falling back to osascript"
            );
            focused_text_via_osascript().map_err(classify_focus_error)
        }
    }
}

#[cfg(target_os = "macos")]
fn classify_focus_error(message: String) -> Error {
    if message.contains("timed out") {
        Error::HelperTimeout(message)
    } else {
        Error::FocusQuery(message)
    }
}

/// Focus query via the unified Swift helper.
#[cfg(target_os = "macos")]
// AX coordinates are screen points, well inside `i32`.
#[allow(clippy::cast_possible_truncation)]
fn focused_text_via_helper() -> Result<FocusedTextContext, String> {
    let request = serde_json::json!({"type": "focus"});
    let resp = super::helper::helper_send_receive(&request)?;

    let app_name = resp
        .get("app_name")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let role = resp
        .get("role")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let text = resp
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let selected_text = resp
        .get("selected_text")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let raw_error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let x = resp
        .get("x")
        .and_then(serde_json::Value::as_i64)
        .map(|v| v as i32);
    let y = resp
        .get("y")
        .and_then(serde_json::Value::as_i64)
        .map(|v| v as i32);
    let w = resp
        .get("w")
        .and_then(serde_json::Value::as_i64)
        .map(|v| v as i32);
    let h = resp
        .get("h")
        .and_then(serde_json::Value::as_i64)
        .map(|v| v as i32);

    Ok(FocusedTextContext {
        app_name,
        role,
        text,
        selected_text,
        raw_error,
        bounds: match (x, y, w, h) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(ElementBounds {
                    x,
                    y,
                    width,
                    height,
                })
            }
            _ => None,
        },
    })
}

/// Focus query via osascript (fallback when helper is unavailable).
///
/// Short-circuits when `automation_state::system_events_denied()` is set
/// (the autocomplete refresh loop captured `(-1743)` from a prior
/// osascript invocation). This stops re-firing osascript — and the
/// macOS Apple Events consent popup — once we've observed the denial
/// within the current session. The flag clears on
/// `autocomplete::start_if_enabled` so a user-initiated re-engagement
/// after granting via System Settings re-probes naturally.
#[cfg(target_os = "macos")]
// One linear AppleScript round-trip with its error mapping; splitting it would
// only scatter the shared context.
#[allow(clippy::too_many_lines)]
fn focused_text_via_osascript() -> Result<FocusedTextContext, String> {
    if super::automation_state::system_events_denied() {
        return Err(
            "focused_text_via_osascript skipped: System Events automation previously denied (-1743)"
                .to_string(),
        );
    }

    let script = r##"
      tell application "System Events"
        set sep to character id 31
        set frontApp to first application process whose frontmost is true
        set appName to name of frontApp
        set roleValue to "unknown"
        set textValue to ""
        set selectedValue to ""
        set errValue to ""
        set posX to ""
        set posY to ""
        set sizeW to ""
        set sizeH to ""
        set targetRoles to {"AXTextArea", "AXTextField", "AXSearchField", "AXComboBox", "AXEditableText"}

        try
          set value of attribute "AXEnhancedUserInterface" of frontApp to true
        end try

        try
          set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
          try
            set roleValue to value of attribute "AXRole" of focusedElement as text
          end try
          try
            set textValue to value of attribute "AXValue" of focusedElement as text
          end try
          try
            set p to value of attribute "AXPosition" of focusedElement
            set posX to item 1 of p as text
            set posY to item 2 of p as text
          end try
          try
            set s to value of attribute "AXSize" of focusedElement
            set sizeW to item 1 of s as text
            set sizeH to item 2 of s as text
          end try
          if textValue is "missing value" then set textValue to ""
          if textValue is "" then
            try
              set selectedValue to value of attribute "AXSelectedText" of focusedElement as text
            end try
            if selectedValue is "missing value" then set selectedValue to ""
            if selectedValue is not "" then set textValue to selectedValue
          end if
          if textValue is "" then
            try
              set textValue to value of attribute "AXTitle" of focusedElement as text
            end try
            if textValue is "missing value" then set textValue to ""
          end if
        on error errMsg number errNum
          set errValue to "ERROR:" & errNum & ":" & errMsg
        end try

        if textValue is "" then
          try
            set focusedWindow to value of attribute "AXFocusedWindow" of frontApp
            set childElems to entire contents of focusedWindow
            set staticPromptValue to ""
            set staticFallbackValue to ""
            repeat with childElem in childElems
              set childRole to ""
              set childValue to ""
              set childSelectedValue to ""
              try
                set childRole to value of attribute "AXRole" of childElem as text
              end try
              if childRole is in targetRoles then
                try
                  set childValue to value of attribute "AXValue" of childElem as text
                end try
                set childPosX to ""
                set childPosY to ""
                set childSizeW to ""
                set childSizeH to ""
                try
                  set cp to value of attribute "AXPosition" of childElem
                  set childPosX to item 1 of cp as text
                  set childPosY to item 2 of cp as text
                end try
                try
                  set cs to value of attribute "AXSize" of childElem
                  set childSizeW to item 1 of cs as text
                  set childSizeH to item 2 of cs as text
                end try
                if childValue is "missing value" then set childValue to ""
                if childValue is "" then
                  try
                    set childSelectedValue to value of attribute "AXSelectedText" of childElem as text
                  end try
                  if childSelectedValue is "missing value" then set childSelectedValue to ""
                  if childSelectedValue is not "" then set childValue to childSelectedValue
                end if
                if childValue is not "" then
                  set roleValue to childRole
                  set textValue to childValue
                  if childPosX is not "" then set posX to childPosX
                  if childPosY is not "" then set posY to childPosY
                  if childSizeW is not "" then set sizeW to childSizeW
                  if childSizeH is not "" then set sizeH to childSizeH
                  exit repeat
                end if
              end if
            end repeat
            if textValue is "" then
              repeat with childElem in childElems
                set childRole to ""
                set childValue to ""
                try
                  set childRole to value of attribute "AXRole" of childElem as text
                end try
                if childRole is "AXStaticText" then
                  try
                    set childValue to value of attribute "AXValue" of childElem as text
                  end try
                  if childValue is "missing value" then set childValue to ""
                  if childValue is not "" then
                    set staticFallbackValue to childValue
                    if childValue contains "$ " or childValue contains "# " or childValue contains "> " then
                      set staticPromptValue to childValue
                    end if
                  end if
                end if
              end repeat
              if staticPromptValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticPromptValue
              else if staticFallbackValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticFallbackValue
              end if
            end if
          on error errMsg2 number errNum2
            if errValue is "" then set errValue to "ERROR:" & errNum2 & ":" & errMsg2
          end try
        end if

        if textValue is "" and errValue is "" then
          set errValue to "ERROR:no_text_candidate_found"
        end if

        return appName & sep & roleValue & sep & textValue & sep & selectedValue & sep & errValue & sep & posX & sep & posY & sep & sizeW & sep & sizeH
      end tell
    "##;

    let mut command = Command::new("osascript");
    command.arg("-e").arg(script);
    let output = command_output_with_timeout(
        "osascript focused_text_via_osascript",
        &mut command,
        FOCUS_COMMAND_TIMEOUT,
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("unable to query focused text context".to_string());
        }
        return Err(format!("unable to query focused text context: {stderr}"));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let mut segments = trimmed.splitn(9, '\u{1f}');
    let app_name = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let role = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let mut value = segments.next().map(normalize_ax_value).unwrap_or_default();
    let mut selected_text = segments
        .next()
        .map(normalize_ax_value)
        .filter(|s| !s.is_empty());
    let mut raw_error = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let pos_x = segments.next().and_then(parse_ax_number);
    let pos_y = segments.next().and_then(parse_ax_number);
    let size_w = segments.next().and_then(parse_ax_number);
    let size_h = segments.next().and_then(parse_ax_number);

    let allow_terminal_text_value =
        is_terminal_app(app_name.as_deref()) && !value.trim().is_empty();
    if !is_text_role(role.as_deref()) && !allow_terminal_text_value {
        value.clear();
        selected_text = None;
        if raw_error.is_none() {
            raw_error = Some("ERROR:no_text_candidate_found".to_string());
        }
    }

    Ok(FocusedTextContext {
        app_name,
        role,
        text: value,
        selected_text,
        raw_error,
        bounds: match (pos_x, pos_y, size_w, size_h) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(ElementBounds {
                    x,
                    y,
                    width,
                    height,
                })
            }
            _ => None,
        },
    })
}

#[cfg(not(target_os = "macos"))]
/// Query the OS for the focused text element, without verbose diagnostics.
///
/// # Errors
///
/// Returns a message when the focus query fails or is unsupported on this platform.
pub fn focused_text_context() -> AccessibilityResult<FocusedTextContext> {
    Err(Error::UnsupportedPlatform)
}

#[cfg(not(target_os = "macos"))]
/// Query the OS for the focused text element, keeping raw diagnostics.
///
/// # Errors
///
/// Returns a message when the focus query fails or is unsupported on this platform.
pub fn focused_text_context_verbose() -> AccessibilityResult<FocusedTextContext> {
    Err(Error::UnsupportedPlatform)
}

// ---------------------------------------------------------------------------
// Focus target validation
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn is_text_editable_role(role: &str) -> bool {
    matches!(role, "AXTextArea" | "AXTextField")
}

#[cfg(target_os = "macos")]
/// Validate that the currently focused element still matches the target the
/// caller captured (`expected_app`, `expected_role`, and `expected_bounds` when given).
///
/// Inconclusive checks pass: no expected app, a failed focus query, or an
/// unsupported platform.
///
/// # Errors
///
/// Returns a message when focus moved to a different application, or to a
/// role that is not an interchangeable text-editable role.
pub fn validate_focused_target(
    expected_app: Option<&str>,
    expected_role: Option<&str>,
    expected_bounds: Option<super::types::ElementBounds>,
) -> AccessibilityResult<()> {
    if expected_app.is_none() && expected_role.is_none() && expected_bounds.is_none() {
        return Ok(());
    }
    let current = focused_text_context_verbose();
    match current {
        Ok(ctx) => {
            if let (Some(expected), Some(actual)) = (expected_app, ctx.app_name.as_deref())
                && expected.to_lowercase() != actual.to_lowercase()
            {
                return Err(Error::FocusChanged {
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
            if let (Some(expected), Some(actual)) = (expected_role, ctx.role.as_deref())
                && expected != actual
            {
                if is_text_editable_role(expected) && is_text_editable_role(actual) {
                    log::debug!(
                        "[accessibility] validate_focused_target: role changed '{expected}' -> '{actual}'; proceeding"
                    );
                } else {
                    return Err(Error::FocusRoleChanged {
                        expected: expected.to_string(),
                        actual: actual.to_string(),
                    });
                }
            }
            if let (Some(expected), Some(actual)) = (expected_bounds, ctx.bounds)
                && expected.x == actual.x
                && expected.y == actual.y
                && expected.width == actual.width
                && expected.height == actual.height
            {
                // The captured and current element occupy the same bounds.
            } else if expected_bounds.is_some() {
                return Err(Error::FocusTargetChanged);
            }
            Ok(())
        }
        Err(_) => Ok(()),
    }
}

#[cfg(not(target_os = "macos"))]
/// Validate that the currently focused element still matches the target the
/// caller captured (`expected_app`, and `expected_role` when given).
///
/// Inconclusive checks pass: no expected app, a failed focus query, or an
/// unsupported platform.
///
/// # Errors
///
/// Returns a message when focus moved to a different application, or to a
/// role that is not an interchangeable text-editable role.
pub fn validate_focused_target(
    _expected_app: Option<&str>,
    _expected_role: Option<&str>,
    _expected_bounds: Option<super::types::ElementBounds>,
) -> AccessibilityResult<()> {
    Ok(())
}

#[cfg(test)]
#[path = "focus_tests.rs"]
mod tests;
