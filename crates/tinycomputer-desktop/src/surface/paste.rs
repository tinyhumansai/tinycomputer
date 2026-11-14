//! Pasting through the clipboard and putting back what it held.

use serde_json::{Value, json};
use tinycomputer_bus::{
    ClipboardFormat, ClipboardGetRequest, ClipboardSetRequest, DesktopResponse, PressRequest,
    RefRequest, WaitRequest,
};
use tinycomputer_core::surface::Candidate;

use super::act::press_at;

/// What a paste does to the machine, one call per step, so the order of the
/// steps and what each failure leaves behind can be driven by a script.
pub(super) trait PasteSteps {
    /// Reads the pasteboard.
    fn clipboard_get(&self, request: ClipboardGetRequest) -> DesktopResponse;
    /// Writes the pasteboard.
    fn clipboard_set(&self, request: ClipboardSetRequest) -> DesktopResponse;
    /// Empties the pasteboard.
    fn clipboard_clear(&self) -> DesktopResponse;
    /// Focuses an element.
    fn focus(&self, request: RefRequest) -> DesktopResponse;
    /// Clicks an element.
    fn click(&self, request: RefRequest) -> DesktopResponse;
    /// Presses a key combination.
    fn press(&self, request: PressRequest) -> DesktopResponse;
    /// Waits.
    fn wait(&self, request: WaitRequest) -> DesktopResponse;
}

/// Puts `text` in `target` through the pasteboard, then puts back what the
/// pasteboard held; a failed restoration is reported in the response.
pub(super) fn paste_through(
    steps: &impl PasteSteps,
    app: &str,
    target: &Candidate,
    text: &str,
) -> DesktopResponse {
    let previous = steps.clipboard_get(ClipboardGetRequest {
        format: Some(ClipboardFormat::Auto),
        out: None,
    });
    let focused = steps.focus(RefRequest::new(target.ref_id.clone()));
    if !focused.ok {
        let clicked = steps.click(RefRequest::new(target.ref_id.clone()));
        if !clicked.ok {
            return clicked;
        }
    }
    let staged = steps.clipboard_set(ClipboardSetRequest::text(text));
    if !staged.ok {
        return staged;
    }
    let replaces = target
        .available_actions
        .iter()
        .any(|action| action == "SetValue");
    let selected = if replaces {
        steps.press(press_at(app, "cmd+a"))
    } else {
        DesktopResponse::ok("press", json!({}))
    };
    // Without a successful select-all the paste would land beside the old
    // text instead of replacing it, so skip it and report the select failure.
    let outcome = if selected.ok {
        let pasted = steps.press(press_at(app, "cmd+v"));
        let _settled = steps.wait(WaitRequest::sleep(150));
        pasted
    } else {
        selected
    };
    let restored = restore_plan(&previous).is_none_or(|restore| {
        match restore {
            Restore::Set(request) => steps.clipboard_set(request),
            Restore::Clear => steps.clipboard_clear(),
        }
        .ok
    });
    with_restoration(outcome, restored)
}

/// What to put back on the pasteboard once a paste is done with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Restore {
    /// Write this back; it is what the pasteboard held before the paste.
    Set(ClipboardSetRequest),
    /// The pasteboard held nothing in a readable flavor before the paste, so
    /// it is emptied rather than left holding the field text the paste staged.
    Clear,
}

/// Decides how to restore the pasteboard from what a pre-paste
/// [`Desktop::clipboard_get`] read there, without touching the pasteboard
/// itself: a pure function a test can drive with a scripted reply.
///
/// A failed read (permission denied, no pasteboard service) reports nothing
/// unreadable rather than "empty", so the pasteboard is left untouched instead
/// of being cleared on a guess.
pub(crate) fn restore_plan(previous: &DesktopResponse) -> Option<Restore> {
    let data = previous.data.as_ref().filter(|_| previous.ok)?;
    if data.get("found").and_then(Value::as_bool) == Some(false) {
        return Some(Restore::Clear);
    }
    match data.get("type").and_then(Value::as_str) {
        Some("text") => data
            .get("text")
            .and_then(Value::as_str)
            .map(|text| Restore::Set(ClipboardSetRequest::text(text))),
        Some("file_urls") => data.get("file_urls").and_then(Value::as_array).map(|urls| {
            let file_urls = urls
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            Restore::Set(ClipboardSetRequest {
                file_urls,
                ..ClipboardSetRequest::default()
            })
        }),
        Some("image") => data.get("path").and_then(Value::as_str).map(|path| {
            Restore::Set(ClipboardSetRequest {
                image: Some(path.to_owned()),
                ..ClipboardSetRequest::default()
            })
        }),
        // `found: false`, or an unrecognized flavor: the pasteboard had
        // nothing readable before the paste, so it is emptied rather than
        // left holding the field text the paste just staged there.
        _ => Some(Restore::Clear),
    }
}

/// Folds a failed clipboard restoration into the response `paste` would
/// otherwise return, instead of discarding it.
///
/// The field operation's own success is left alone — the text still arrived,
/// and a caller checking `ok` must keep seeing that — but `data` gains
/// `clipboard_restored: false` so a caller that reads it can see the user's
/// prior clipboard contents were not put back. An already-failed response has
/// nothing to fold into and is returned unchanged.
pub(super) fn with_restoration(mut result: DesktopResponse, restored: bool) -> DesktopResponse {
    if !restored && result.ok {
        let mut data = result.data.take().unwrap_or_else(|| json!({}));
        if let Value::Object(map) = &mut data {
            map.insert("clipboard_restored".to_owned(), json!(false));
        }
        result.data = Some(data);
    }
    result
}
