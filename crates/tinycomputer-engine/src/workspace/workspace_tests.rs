//! Tests for routing workspace calls between the desktop and the browser.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use serde_json::json;
use tinycomputer_bus::{DesktopError, DesktopResponse, JevOperation};
use tinycomputer_core::surface::{Candidate, Depth, Screen, Surface};

use super::Workspace;

/// A surface that records every call under its own name.
#[derive(Clone)]
struct Recorder {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    failing: bool,
}

impl Recorder {
    fn new(name: &'static str, calls: &Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name,
            calls: calls.clone(),
            failing: false,
        }
    }

    fn note(&self, call: &str) -> DesktopResponse {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:{call}", self.name));
        if self.failing {
            DesktopResponse::err(call, DesktopError::new("FAILED", "scripted failure"))
        } else {
            DesktopResponse::ok(call, json!({}))
        }
    }
}

impl Surface for Recorder {
    fn observe(
        &self,
        app: &str,
        _root: Option<&str>,
        _depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        let reply = self.note("observe");
        if !reply.ok {
            return Err(Box::new(reply));
        }
        Ok(Screen {
            app: app.to_owned(),
            window: None,
            surface: "window".to_owned(),
            candidates: vec![
                Candidate {
                    name: Some(format!("{} control", self.name)),
                    ..Candidate::default()
                },
                Candidate {
                    name: Some("Email".to_owned()),
                    value: Some(serde_json::json!("asha@example.com")),
                    ..Candidate::default()
                },
                Candidate {
                    name: Some("Phone".to_owned()),
                    value: Some(serde_json::json!("  ")),
                    ..Candidate::default()
                },
            ],
            context: vec![format!("{} text", self.name)],
            unexplored: Vec::new(),
            text_nodes: Vec::new(),
        })
    }

    fn execute(
        &self,
        _operation: JevOperation,
        _target: Option<Candidate>,
        _text: Option<String>,
    ) -> DesktopResponse {
        self.note("execute")
    }

    fn read_value(&self, _target: &Candidate) -> Option<String> {
        self.note("read");
        Some(self.name.to_owned())
    }

    fn paste(&self, _app: &str, _target: &Candidate, _text: &str) -> DesktopResponse {
        self.note("paste")
    }

    fn press(&self, _app: &str, _combo: &str) -> DesktopResponse {
        self.note("press")
    }

    fn launch(&self, _app: &str) -> DesktopResponse {
        self.note("launch")
    }

    fn settle(&self) {
        self.note("settle");
    }

    fn settle_briefly(&self) {
        self.note("settle_briefly");
    }

    fn await_change(&self, _ms: u64) -> bool {
        self.note("await_change");
        true
    }

    fn navigate(&self, _url: &str) -> DesktopResponse {
        self.note("navigate")
    }

    fn back(&self, _app: &str) -> DesktopResponse {
        self.note("back")
    }

    fn dismiss_cover(&self, _target: &Candidate) -> DesktopResponse {
        self.note("dismiss_cover")
    }
}

fn workspace(with_browser: bool) -> (Workspace<Recorder, Recorder>, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let workspace = Workspace::new(
        Some(Recorder::new("desktop", &calls)),
        with_browser.then(|| Recorder::new("browser", &calls)),
    );
    (workspace, calls)
}

fn drain(calls: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    std::mem::take(&mut *calls.lock().unwrap())
}

#[test]
fn named_calls_route_by_application_name() {
    let (workspace, calls) = workspace(true);
    assert!(workspace.has_browser());
    let node = Candidate::default();
    workspace.observe("Mail", None, Depth::Full).unwrap();
    workspace.observe("browser", None, Depth::Full).unwrap();
    workspace.launch("https://flights.test");
    workspace.launch("browser:trips");
    workspace.press("Mail", "cmd+n");
    workspace.paste("browser", &node, "text");
    workspace.paste("Notes", &node, "text");
    workspace.press(" Browser ", "return");
    assert_eq!(
        drain(&calls),
        [
            "desktop:observe",
            "browser:observe",
            "browser:launch",
            "browser:launch",
            "desktop:press",
            "browser:paste",
            "desktop:paste",
            "browser:press"
        ]
    );
}

#[test]
fn unnamed_calls_follow_the_side_last_observed_or_opened() {
    let (workspace, calls) = workspace(true);
    let node = Candidate::default();
    workspace.execute(JevOperation::Click, None, None);
    assert_eq!(workspace.read_value(&node).as_deref(), Some("desktop"));
    assert!(!workspace.browser_active());
    workspace.settle();
    workspace.navigate("https://flights.test");
    assert!(workspace.browser_active());
    workspace.execute(JevOperation::Click, Some(node.clone()), None);
    assert_eq!(workspace.read_value(&node).as_deref(), Some("browser"));
    workspace.settle();
    workspace.launch("Mail");
    assert!(!workspace.browser_active(), "the task moved to the desktop");
    workspace.execute(JevOperation::Click, None, None);
    assert_eq!(
        drain(&calls),
        [
            "desktop:execute",
            "desktop:read",
            "desktop:settle",
            "browser:navigate",
            "browser:execute",
            "browser:read",
            "browser:settle",
            "desktop:launch",
            "desktop:execute"
        ]
    );
}

#[test]
fn a_wait_for_a_change_watches_the_active_side() {
    let (workspace, calls) = workspace(true);
    assert!(workspace.await_change(1_000));
    workspace.navigate("https://flights.test");
    assert!(workspace.await_change(1_000));
    assert_eq!(
        drain(&calls),
        [
            "desktop:await_change",
            "browser:navigate",
            "browser:await_change"
        ]
    );
    let bare: Workspace<Recorder, Recorder> = Workspace::new(None, None);
    assert!(!bare.await_change(1_000), "nothing to change");
}

#[test]
fn a_brief_settle_settles_the_active_side() {
    let (workspace, calls) = workspace(true);
    workspace.settle_briefly();
    workspace.navigate("https://flights.test");
    workspace.settle_briefly();
    assert_eq!(
        drain(&calls),
        [
            "desktop:settle_briefly",
            "browser:navigate",
            "browser:settle_briefly"
        ]
    );
    let bare: Workspace<Recorder, Recorder> = Workspace::new(None, None);
    bare.settle_briefly();
}

#[test]
fn going_back_follows_the_active_side() {
    let (workspace, calls) = workspace(true);
    workspace.back("Mail");
    workspace.navigate("https://flights.test");
    workspace.back("browser");
    assert_eq!(
        drain(&calls),
        ["desktop:back", "browser:navigate", "browser:back"]
    );
    let bare: Workspace<Recorder, Recorder> = Workspace::new(None, None);
    assert_eq!(
        bare.back("browser").error.unwrap().code,
        "BROWSER_NOT_AVAILABLE"
    );
}

#[test]
fn a_cover_is_dismissed_on_the_active_side() {
    let (workspace, calls) = workspace(true);
    workspace.dismiss_cover(&Candidate::default());
    workspace.navigate("https://flights.test");
    workspace.dismiss_cover(&Candidate::default());
    assert_eq!(
        drain(&calls),
        [
            "desktop:dismiss_cover",
            "browser:navigate",
            "browser:dismiss_cover"
        ]
    );
    let bare: Workspace<Recorder, Recorder> = Workspace::new(None, None);
    assert_eq!(
        bare.dismiss_cover(&Candidate::default())
            .error
            .unwrap()
            .code,
        "BROWSER_NOT_AVAILABLE"
    );
}

#[test]
fn a_failed_open_leaves_the_active_side_alone() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut browser = Recorder::new("browser", &calls);
    browser.failing = true;
    let workspace = Workspace::new(Some(Recorder::new("desktop", &calls)), Some(browser));
    assert!(!workspace.launch("browser").ok);
    assert!(!workspace.navigate("https://flights.test").ok);
    assert!(workspace.observe("browser", None, Depth::Full).is_err());
    workspace.execute(JevOperation::Click, None, None);
    assert_eq!(drain(&calls).last().unwrap(), "desktop:execute");
}

#[test]
fn without_a_browser_web_calls_are_refused_and_nothing_else_changes() {
    let (workspace, calls) = workspace(false);
    assert!(!workspace.has_browser());
    let code = |reply: DesktopResponse| reply.error.unwrap().code;
    assert_eq!(
        code(workspace.navigate("https://flights.test")),
        "BROWSER_NOT_AVAILABLE"
    );
    assert_eq!(code(workspace.launch("browser")), "BROWSER_NOT_AVAILABLE");
    assert_eq!(
        code(workspace.press("browser", "return")),
        "BROWSER_NOT_AVAILABLE"
    );
    let refused = workspace.observe("browser", None, Depth::Full).unwrap_err();
    assert_eq!(refused.error.unwrap().code, "BROWSER_NOT_AVAILABLE");
    // With no browser, nothing can make it the active side, so unnamed calls
    // keep going to the desktop.
    workspace.execute(JevOperation::Click, None, None);
    assert_eq!(drain(&calls), ["desktop:execute"]);
}

#[test]
fn without_a_desktop_application_calls_are_refused_and_the_browser_stays_active() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let workspace: Workspace<Recorder, Recorder> =
        Workspace::new(None, Some(Recorder::new("browser", &calls)));
    assert!(!workspace.has_desktop());
    let code = |reply: DesktopResponse| reply.error.unwrap().code;
    assert_eq!(code(workspace.launch("Mail")), "DESKTOP_NOT_AVAILABLE");
    assert_eq!(
        code(workspace.press("Mail", "cmd+n")),
        "DESKTOP_NOT_AVAILABLE"
    );
    let refused = workspace.observe("Mail", None, Depth::Full).unwrap_err();
    assert_eq!(refused.error.unwrap().code, "DESKTOP_NOT_AVAILABLE");
    // With no desktop, a task confined to the browser still works, and
    // unnamed calls default to the only side it has.
    workspace.execute(JevOperation::Click, None, None);
    assert_eq!(drain(&calls), ["browser:execute"]);
}

#[test]
fn visible_text_rereads_whatever_was_last_looked_at() {
    let (workspace, calls) = workspace(true);
    assert!(workspace.visible_text().is_empty(), "nothing observed yet");
    workspace.launch("Mail");
    assert_eq!(
        workspace.visible_text(),
        [
            "desktop text",
            "desktop control",
            "Email = \"asha@example.com\"",
            "Phone"
        ],
        "a field reads with what it holds"
    );
    workspace.navigate("https://flights.test");
    assert_eq!(
        workspace.visible_text(),
        [
            "browser text",
            "browser control",
            "Email = \"asha@example.com\"",
            "Phone"
        ]
    );
    workspace.observe("Notes", None, Depth::Full).unwrap();
    assert_eq!(workspace.visible_text()[0], "desktop text");
    drain(&calls);

    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut browser = Recorder::new("browser", &calls);
    let broken = Workspace::new(
        Some(Recorder::new("desktop", &calls)),
        Some(browser.clone()),
    );
    broken.navigate("https://flights.test");
    browser.failing = true;
    let broken = Workspace {
        browser: Some(browser),
        ..broken
    };
    assert!(
        broken.visible_text().is_empty(),
        "an unreadable screen shows nothing"
    );
}
