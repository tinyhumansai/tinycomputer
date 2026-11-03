//! A scripted engine for tests: records every command and answers the way
//! agent-browser does, writing screenshot and download files where asked.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};

use crate::engine::{Engine, Launcher, Reply};

type Script = dyn Fn(&Value) -> Option<Value> + Send + Sync;
type Stall = dyn Fn(&Value) -> bool + Send + Sync;

/// Where a page puts the focus once an element is given it, as the focus
/// check (`sessions/focus.rs`) reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PageFocus {
    /// On the element: every element named takes it. The default.
    OnTarget,
    /// On another element, by its tag.
    Elsewhere(&'static str),
    /// On nothing: the element took none, and nothing else holds it.
    Nowhere,
    /// Inside a cross-origin frame, which the page cannot see into.
    InAFrame,
    /// On the element, whose ref the engine no longer resolves when asked
    /// for its mark.
    Stale,
}

/// Records every command; answers the focus check as its page's focus says,
/// the rest from an optional override, else like the engine would, and never
/// answers a command it is told to stall on.
#[derive(Clone)]
pub(crate) struct Fake {
    sent: Arc<Mutex<Vec<Value>>>,
    script: Arc<Script>,
    stall: Arc<Stall>,
    focus: PageFocus,
    close_delay: Option<Duration>,
    fail_close: bool,
}

impl std::fmt::Debug for Fake {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Fake")
    }
}

impl Fake {
    pub(crate) fn new() -> Self {
        Self::scripted(|_| None)
    }

    pub(crate) fn scripted(
        script: impl Fn(&Value) -> Option<Value> + Send + Sync + 'static,
    ) -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            script: Arc::new(script),
            stall: Arc::new(|_| false),
            focus: PageFocus::OnTarget,
            close_delay: None,
            fail_close: false,
        }
    }

    /// The same fake, its page putting the focus where `focus` says once an
    /// element is given it.
    pub(crate) fn with_focus(mut self, focus: PageFocus) -> Self {
        self.focus = focus;
        self
    }

    /// The same fake, never answering a command `stall` picks, as a page
    /// being replaced may not.
    pub(crate) fn stalling(
        mut self,
        stall: impl Fn(&Value) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.stall = Arc::new(stall);
        self
    }

    /// Delays the engine's `close` response, as a browser that takes time to
    /// flush its profile before it exits.
    pub(crate) fn delaying_close(mut self, delay: Duration) -> Self {
        self.close_delay = Some(delay);
        self
    }

    /// Makes the engine fail when the caller asks it to close the browser.
    pub(crate) fn failing_close(mut self) -> Self {
        self.fail_close = true;
        self
    }

    pub(crate) fn actions(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|command| command["action"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Every command sent, in order.
    pub(crate) fn sent(&self) -> Vec<Value> {
        self.sent.lock().unwrap().clone()
    }

    /// Whether a page script ran besides those every press runs: the one
    /// that keeps a press in the tab, the one that brings a control sight
    /// found into the window, and the one that names what covers a press
    /// the page refused.
    pub(crate) fn evaluated_besides_every_press(&self) -> bool {
        self.sent().iter().any(|command| {
            let script = command["script"].as_str().unwrap_or_default();
            command["action"] == "evaluate"
                && !script.contains("__tcOpen")
                && !script.starts_with(crate::surface::INTO_VIEW_JS)
                && !script.starts_with(crate::surface::COVER_JS)
        })
    }

    /// Whether the pointer pressed anywhere by position, rather than only
    /// moving.
    pub(crate) fn pressed_by_position(&self) -> bool {
        self.sent()
            .iter()
            .any(|command| command["action"] == "mouse" && command["eventType"] != "mouseMoved")
    }

    pub(crate) fn last(&self, action: &str) -> Value {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|command| command["action"] == action)
            .cloned()
            .unwrap_or_else(|| panic!("{action} was never sent"))
    }
}

pub(crate) fn ok(data: &Value) -> Value {
    json!({"id": "", "success": true, "data": data})
}

pub(crate) fn failure(message: &str) -> Value {
    json!({"id": "", "success": false, "error": message})
}

pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes
}

pub(crate) fn default_reply(command: &Value) -> Value {
    let path = command["path"].as_str().map(str::to_owned);
    match command["action"].as_str().unwrap() {
        "url" => ok(&json!({"url": "https://flights.test/"})),
        "title" => ok(&json!({"title": "Flights"})),
        "navigate" => ok(&json!({"url": command["url"], "title": "Loaded"})),
        "snapshot" => ok(&json!({
            "snapshot": "- button \"Search\" [ref=e1]",
            "refs": {"e1": {"role": "button", "name": "Search"}}
        })),
        "gettext" => ok(&json!({"text": "IndiGo ₹6,840"})),
        "getattribute" => ok(&json!({"value": "/book"})),
        "isvisible" => ok(&json!({"visible": true})),
        "read" => ok(&json!({"content": "# Flights\nIndiGo", "truncated": false})),
        "content" => ok(&json!({"html": "<h1>Flights</h1>"})),
        "evaluate" => ok(&json!({"result": 42})),
        "screenshot" => {
            std::fs::write(path.as_ref().unwrap(), png(1280, 800)).unwrap();
            ok(&json!({"path": path}))
        }
        "waitfordownload" => {
            std::fs::write(path.as_ref().unwrap(), b"itinerary").unwrap();
            ok(&json!({"path": path}))
        }
        _ => ok(&json!({})),
    }
}

impl Engine for Fake {
    fn execute(&mut self, command: Value) -> Reply<'_> {
        self.sent.lock().unwrap().push(command.clone());
        if (self.stall)(&command) {
            return Box::pin(std::future::pending());
        }
        let reply = if command["action"] == "close" && self.fail_close {
            failure("browser close failed")
        } else {
            self.focus_reply(&command)
                .or_else(|| (self.script)(&command))
                .unwrap_or_else(|| default_reply(&command))
        };
        let delay = (command["action"] == "close")
            .then_some(self.close_delay)
            .flatten();
        Box::pin(async move {
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            reply
        })
    }
}

impl Fake {
    /// The page's answer to the focus check (`sessions/focus.rs`): where
    /// the marking script finds the focus, and the target's mark.
    fn focus_reply(&self, command: &Value) -> Option<Value> {
        let script = command["script"].as_str().unwrap_or_default();
        if command["action"] == "evaluate" && script.contains("setAttribute('data-tc-focus'") {
            let found = match self.focus {
                PageFocus::OnTarget | PageFocus::Stale => {
                    json!({"focus": "element", "on": "input"})
                }
                PageFocus::Elsewhere(tag) => json!({"focus": "element", "on": tag}),
                PageFocus::Nowhere => json!({"focus": "none", "on": "nothing"}),
                PageFocus::InAFrame => json!({"focus": "frame", "on": "a frame"}),
            };
            return Some(ok(&json!({"result": found})));
        }
        if command["action"] == "getattribute" && command["attribute"] == "data-tc-focus" {
            let target = command["selector"].as_str().unwrap_or_default();
            return Some(match self.focus {
                PageFocus::OnTarget => {
                    let sent = self.sent.lock().unwrap();
                    let mark = sent
                        .iter()
                        .rev()
                        .find_map(|earlier| focus_mark(earlier["script"].as_str()?));
                    ok(&json!({"value": mark}))
                }
                PageFocus::Stale => {
                    failure(&format!("Unknown ref: {}", target.trim_start_matches('@')))
                }
                _ => ok(&json!({"value": null})),
            });
        }
        None
    }
}

/// The mark value a focus-marking script was called with, if `script` is one.
pub(crate) fn focus_mark(script: &str) -> Option<String> {
    if !script.contains("setAttribute('data-tc-focus'") {
        return None;
    }
    let start = script.rfind("(\"tc-")? + 2;
    let end = start + script[start..].find('"')?;
    Some(script[start..end].to_owned())
}

impl Launcher for Fake {
    fn open(&self, _session: &str) -> Box<dyn Engine> {
        Box::new(self.clone())
    }
}
