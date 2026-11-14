//! Tests for the clipboard round trip around a paste.

use serde_json::json;
use tinycomputer_bus::DesktopResponse;

use crate::surface::act::platform_combo;
use crate::surface::paste::{PasteSteps, Restore, paste_through, restore_plan, with_restoration};
use std::sync::Mutex;
use tinycomputer_bus::{
    ClipboardGetRequest, ClipboardSetRequest, PressRequest, RefRequest, WaitRequest,
};
use tinycomputer_core::surface::Candidate;

#[test]
fn restore_plan_writes_back_whatever_flavor_the_pasteboard_held() {
    let text = DesktopResponse::ok("clipboard-get", json!({"type": "text", "text": "hello"}));
    assert_eq!(
        restore_plan(&text),
        Some(Restore::Set(tinycomputer_bus::ClipboardSetRequest::text(
            "hello"
        )))
    );

    let file_urls = DesktopResponse::ok(
        "clipboard-get",
        json!({"type": "file_urls", "file_urls": ["file:///tmp/a.txt"]}),
    );
    assert_eq!(
        restore_plan(&file_urls),
        Some(Restore::Set(tinycomputer_bus::ClipboardSetRequest {
            file_urls: vec!["file:///tmp/a.txt".to_owned()],
            ..tinycomputer_bus::ClipboardSetRequest::default()
        }))
    );

    let image = DesktopResponse::ok(
        "clipboard-get",
        json!({"type": "image", "path": "/tmp/clip.png"}),
    );
    assert_eq!(
        restore_plan(&image),
        Some(Restore::Set(tinycomputer_bus::ClipboardSetRequest {
            image: Some("/tmp/clip.png".to_owned()),
            ..tinycomputer_bus::ClipboardSetRequest::default()
        }))
    );
}

#[test]
fn restore_plan_clears_an_originally_empty_pasteboard() {
    let empty = DesktopResponse::ok("clipboard-get", json!({"type": "text", "found": false}));
    assert_eq!(restore_plan(&empty), Some(Restore::Clear));
}

#[test]
fn restore_plan_leaves_the_pasteboard_alone_when_the_read_failed() {
    let failed = DesktopResponse::err(
        "clipboard-get",
        tinycomputer_bus::DesktopError::new("PERM_DENIED", "no automation permission"),
    );
    assert_eq!(restore_plan(&failed), None);
}

#[test]
fn platform_combo_keeps_cmd_on_macos_and_maps_it_to_ctrl_elsewhere() {
    // Flows and the paste path only ever write app shortcuts in terms of
    // `cmd`; the engine's own combo parser maps that literally to the Meta
    // key, which is the Windows/Super key everywhere but macOS.
    assert_eq!(platform_combo("cmd+n", true), "cmd+n");
    assert_eq!(platform_combo("cmd+shift+n", true), "cmd+shift+n");
    assert_eq!(platform_combo("cmd+n", false), "ctrl+n");
    assert_eq!(platform_combo("cmd+shift+n", false), "ctrl+shift+n");
    assert_eq!(platform_combo("cmd+a", false), "ctrl+a");
    assert_eq!(platform_combo("cmd+v", false), "ctrl+v");
    // Combos with no `cmd` modifier are left untouched on every platform.
    assert_eq!(platform_combo("escape", false), "escape");
    assert_eq!(platform_combo("tab", true), "tab");
}

#[test]
fn a_failed_clipboard_restoration_is_folded_into_the_response_instead_of_ignored() {
    let ok = DesktopResponse::ok("press", json!({}));
    // A successful restoration leaves the response untouched.
    assert_eq!(with_restoration(ok.clone(), true), ok);
    // A failed restoration is reported, not silently dropped, without
    // turning a delivered field's own success into a failure.
    let annotated = with_restoration(ok, false);
    assert!(annotated.ok, "the field operation itself still succeeded");
    assert_eq!(annotated.data, Some(json!({"clipboard_restored": false})));
    // A failed operation's own error is left alone: nothing to fold into.
    let failed = DesktopResponse::err(
        "press",
        tinycomputer_bus::DesktopError::new("PERM_DENIED", "no automation permission"),
    );
    assert_eq!(with_restoration(failed.clone(), false), failed);
}

/// A scripted pasteboard and keyboard that records what a paste did.
struct Steps {
    held: DesktopResponse,
    /// The steps that fail: `focus`, `click`, `stage`, `select`, `restore`.
    failing: Vec<&'static str>,
    calls: Mutex<Vec<String>>,
}

impl Steps {
    fn new() -> Self {
        Self {
            held: DesktopResponse::ok("clipboard-get", json!({"type": "text", "text": "before"})),
            failing: Vec::new(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, call: &str) {
        self.calls.lock().unwrap().push(call.to_owned());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn works(&self, step: &str) -> DesktopResponse {
        Self::reply(!self.failing.contains(&step))
    }

    fn reply(ok: bool) -> DesktopResponse {
        if ok {
            DesktopResponse::ok("step", json!({}))
        } else {
            DesktopResponse::err(
                "step",
                tinycomputer_bus::DesktopError::new("FAILED", "scripted"),
            )
        }
    }
}

impl PasteSteps for Steps {
    fn clipboard_get(&self, _request: ClipboardGetRequest) -> DesktopResponse {
        self.note("get");
        self.held.clone()
    }
    fn clipboard_set(&self, request: ClipboardSetRequest) -> DesktopResponse {
        let staging = request.text.as_deref() == Some("typed");
        let step = if staging { "stage" } else { "restore" };
        self.note(step);
        self.works(step)
    }
    fn clipboard_clear(&self) -> DesktopResponse {
        self.note("clear");
        self.works("restore")
    }
    fn focus(&self, _request: RefRequest) -> DesktopResponse {
        self.note("focus");
        self.works("focus")
    }
    fn click(&self, _request: RefRequest) -> DesktopResponse {
        self.note("click");
        self.works("click")
    }
    fn press(&self, request: PressRequest) -> DesktopResponse {
        let select = request.combo.ends_with("+a");
        self.note(if select { "select" } else { "paste" });
        if select {
            self.works("select")
        } else {
            Self::reply(true)
        }
    }
    fn wait(&self, _request: WaitRequest) -> DesktopResponse {
        self.note("wait");
        Self::reply(true)
    }
}

fn field(actions: &[&str]) -> Candidate {
    Candidate {
        ref_id: "@e1".to_owned(),
        available_actions: actions.iter().map(|action| (*action).to_owned()).collect(),
        ..Candidate::default()
    }
}

#[test]
fn a_paste_stages_selects_pastes_and_puts_back_what_the_pasteboard_held() {
    let steps = Steps::new();
    let reply = paste_through(&steps, "Notes", &field(&["SetValue"]), "typed");
    assert!(reply.ok);
    assert_eq!(reply.data, Some(json!({})), "restored: nothing to report");
    assert_eq!(
        steps.calls(),
        [
            "get", "focus", "stage", "select", "paste", "wait", "restore"
        ]
    );
}

#[test]
fn a_field_that_takes_no_value_is_pasted_into_without_selecting_first() {
    let steps = Steps::new();
    assert!(paste_through(&steps, "Notes", &field(&[]), "typed").ok);
    assert!(!steps.calls().contains(&"select".to_owned()));
}

#[test]
fn a_field_that_will_not_focus_is_clicked_and_one_that_will_not_click_stops_the_paste() {
    let mut steps = Steps::new();
    steps.failing.push("focus");
    assert!(paste_through(&steps, "Notes", &field(&[]), "typed").ok);
    assert!(steps.calls().contains(&"click".to_owned()));

    steps.failing.push("click");
    let steps = Steps {
        calls: Mutex::new(Vec::new()),
        ..steps
    };
    assert!(!paste_through(&steps, "Notes", &field(&[]), "typed").ok);
    assert_eq!(steps.calls(), ["get", "focus", "click"], "nothing staged");
}

#[test]
fn a_pasteboard_that_will_not_take_the_text_stops_the_paste_before_any_key() {
    let mut steps = Steps::new();
    steps.failing.push("stage");
    assert!(!paste_through(&steps, "Notes", &field(&["SetValue"]), "typed").ok);
    assert_eq!(steps.calls(), ["get", "focus", "stage"]);
}

#[test]
fn a_failed_select_all_is_the_pastes_failure_and_an_empty_pasteboard_is_cleared() {
    let mut steps = Steps::new();
    steps.failing.push("select");
    steps.held = DesktopResponse::ok("clipboard-get", json!({"found": false}));
    assert!(!paste_through(&steps, "Notes", &field(&["SetValue"]), "typed").ok);
    assert_eq!(steps.calls().last().map(String::as_str), Some("clear"));
}

#[test]
fn a_pasteboard_that_cannot_be_put_back_is_reported_beside_the_delivered_text() {
    let mut steps = Steps::new();
    steps.failing.push("restore");
    let reply = paste_through(&steps, "Notes", &field(&[]), "typed");
    assert!(reply.ok);
    assert_eq!(reply.data, Some(json!({"clipboard_restored": false})));
}
