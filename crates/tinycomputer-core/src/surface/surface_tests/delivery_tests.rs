//! Tests for verified text delivery over a scripted surface, and the surface
//! defaults.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use serde_json::json;
use tinycomputer_bus::{DesktopResponse, JevOperation};

use crate::surface::{Candidate, Depth, Screen, Surface, deliver_text, holds, tokenized};

/// A backend whose reads, set-values, and pastes are scripted.
#[derive(Clone, Default)]
struct TextBackend {
    fail_execute: bool,
    fail_paste: bool,
    /// Values `read_value` returns, in order; exhausted means unreadable.
    reads: Arc<Mutex<VecDeque<String>>>,
    pastes: Arc<Mutex<Vec<String>>>,
}

impl Surface for TextBackend {
    fn observe(
        &self,
        _app: &str,
        _root: Option<&str>,
        _depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        Err(Box::new(DesktopResponse::err(
            "snapshot",
            tinycomputer_bus::DesktopError::new("EMPTY", "no screen"),
        )))
    }

    fn execute(
        &self,
        _operation: JevOperation,
        _target: Option<Candidate>,
        _text: Option<String>,
    ) -> DesktopResponse {
        if self.fail_execute {
            DesktopResponse::err(
                "fake",
                tinycomputer_bus::DesktopError::new("ACTION_FAILED", "fake failure"),
            )
        } else {
            DesktopResponse::ok("fake", json!({}))
        }
    }

    fn read_value(&self, _target: &Candidate) -> Option<String> {
        self.reads.lock().unwrap().pop_front()
    }

    fn paste(&self, _app: &str, _target: &Candidate, text: &str) -> DesktopResponse {
        self.pastes.lock().unwrap().push(text.to_owned());
        if self.fail_paste {
            DesktopResponse::err(
                "paste",
                tinycomputer_bus::DesktopError::new("PASTE_FAILED", "fake paste failure"),
            )
        } else {
            DesktopResponse::ok("paste", json!({}))
        }
    }

    fn press(&self, _app: &str, _combo: &str) -> DesktopResponse {
        DesktopResponse::ok("press", json!({}))
    }

    fn launch(&self, _app: &str) -> DesktopResponse {
        DesktopResponse::ok("launch", json!({}))
    }
}

/// [`TextBackend`], counting how often it settles in full.
#[derive(Clone, Default)]
struct Settling {
    text: TextBackend,
    settled: Arc<std::sync::atomic::AtomicUsize>,
}

impl Surface for Settling {
    fn observe(
        &self,
        app: &str,
        root: Option<&str>,
        depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        self.text.observe(app, root, depth)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        self.text.execute(operation, target, text)
    }

    fn read_value(&self, target: &Candidate) -> Option<String> {
        self.text.read_value(target)
    }

    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse {
        self.text.paste(app, target, text)
    }

    fn press(&self, app: &str, combo: &str) -> DesktopResponse {
        self.text.press(app, combo)
    }

    fn launch(&self, app: &str) -> DesktopResponse {
        self.text.launch(app)
    }

    fn settle(&self) {
        self.settled
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn field() -> Candidate {
    Candidate {
        ref_id: "@s:e1".to_owned(),
        role: "textfield".to_owned(),
        name: Some("Subject".to_owned()),
        available_actions: vec!["SetValue".to_owned()],
        ..Candidate::default()
    }
}

fn reading(reads: &[&str]) -> TextBackend {
    TextBackend {
        reads: Arc::new(Mutex::new(
            reads.iter().map(|read| (*read).to_owned()).collect(),
        )),
        ..TextBackend::default()
    }
}

#[test]
fn text_verified_by_read_back_is_not_pasted() {
    let backend = reading(&["Hello   there"]);
    let reply = deliver_text(&backend, "Mail", &field(), "Hello there");
    assert_eq!(reply.data.unwrap()["path"], json!("set_value"));
    assert!(backend.pastes.lock().unwrap().is_empty());
}

#[test]
fn a_field_that_commits_late_is_verified_on_the_settled_re_read() {
    let backend = reading(&["sam@exa", "sam@example.com"]);
    let reply = deliver_text(&backend, "Mail", &field(), "sam@example.com");
    assert_eq!(reply.data.unwrap()["path"], json!("set_value"));
    assert!(backend.pastes.lock().unwrap().is_empty());
}

#[test]
fn a_token_field_is_delivered_unverified_rather_than_pasted_over() {
    let backend = reading(&["\u{fffc}", "\u{fffc}, \u{fffc}"]);
    let data = deliver_text(&backend, "Mail", &field(), "sam@example.com")
        .data
        .unwrap();
    assert_eq!(
        (data["path"].clone(), data["verified"].clone()),
        (json!("set_value"), json!(false))
    );
    assert!(backend.pastes.lock().unwrap().is_empty());
    assert!(!tokenized("plain"));
}

#[test]
fn a_silently_ignored_set_value_falls_back_to_paste() {
    let backend = reading(&["", "", "Dear Sam, see you Friday"]);
    let data = deliver_text(&backend, "Mail", &field(), "Dear Sam, see you Friday")
        .data
        .unwrap();
    assert_eq!(
        (data["path"].clone(), data["verified"].clone()),
        (json!("paste"), json!(true))
    );
    assert_eq!(backend.pastes.lock().unwrap().len(), 1);
}

#[test]
fn text_that_never_arrives_is_reported_as_not_delivered() {
    let backend = reading(&["", "", "still empty", "still empty"]);
    assert_eq!(
        deliver_text(&backend, "Mail", &field(), "Body")
            .error
            .unwrap()
            .code,
        "TEXT_NOT_DELIVERED"
    );
    let unreadable = reading(&[]);
    assert_eq!(
        deliver_text(&unreadable, "Mail", &field(), "Body")
            .data
            .unwrap()["verified"],
        json!(false)
    );
    let failing = TextBackend {
        fail_execute: true,
        fail_paste: true,
        ..TextBackend::default()
    };
    assert_eq!(
        deliver_text(&failing, "Mail", &field(), "Body")
            .error
            .unwrap()
            .code,
        "ACTION_FAILED"
    );
    let paste_after_set = TextBackend {
        reads: Arc::new(Mutex::new(VecDeque::from(["x".to_owned()]))),
        fail_paste: true,
        ..TextBackend::default()
    };
    assert_eq!(
        deliver_text(&paste_after_set, "Mail", &field(), "Body")
            .error
            .unwrap()
            .code,
        "PASTE_FAILED"
    );
    let set_failed_paste_unverified = TextBackend {
        fail_execute: true,
        ..TextBackend::default()
    };
    assert!(deliver_text(&set_failed_paste_unverified, "Mail", &field(), "Body").ok);
    assert!(!holds("anything", "   "));
}

#[test]
fn a_surface_settles_instantly_and_has_no_addresses_unless_it_says_otherwise() {
    Surface::settle(&TextBackend::default());
    // Settling briefly is settling in full, unless the surface can tell them
    // apart: the desktop's own settle runs after a launch or Escape.
    let settling = Settling::default();
    settling.settle_briefly();
    assert_eq!(
        settling.settled.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(
        Surface::await_change(&TextBackend::default(), 1_000),
        "one that cannot watch pauses and says it may have changed"
    );
    let refused = Surface::navigate(&TextBackend::default(), "https://example.com");
    assert_eq!(refused.error.unwrap().code, "ACTION_NOT_SUPPORTED");
    let refused = Surface::back(&TextBackend::default(), "Mail");
    assert_eq!(refused.error.unwrap().code, "ACTION_NOT_SUPPORTED");
    let refused = Surface::dismiss_cover(&TextBackend::default(), &Candidate::default());
    assert_eq!(refused.error.unwrap().code, "ACTION_NOT_SUPPORTED");
}
