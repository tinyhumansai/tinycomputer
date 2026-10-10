//! The [`Surface`] implementation: observing, acting, reading, pasting,
//! pressing, and navigating, with the engine targets and key spellings they use.

use serde_json::{Value, json};
use tinycomputer_bus::browser::{
    Action, NavigateRequest, ScrollDirection, SessionId, SnapshotRequest, Target, WaitState,
};
use tinycomputer_bus::{DesktopError, DesktopResponse, JevOperation};
use tinycomputer_core::surface::{Candidate, Depth, Screen, Surface, uses_pointer};
use tinycomputer_core::{Key, Platform};

use crate::error::Error;

use super::envelope::{failure, not_a_text_field, reply};
use super::{BrowserSurface, Perception};
use super::{NETWORK_IDLE_MS, QUIET_MS, READ_TIMEOUT, SETTLE_MS, SKELETON_DEPTH, Settle, tree};
use super::{sight, tabs::place, watch};

impl Surface for BrowserSurface {
    fn observe(
        &self,
        app: &str,
        root: Option<&str>,
        depth: Depth,
    ) -> std::result::Result<Screen, Box<DesktopResponse>> {
        // A page once read is no longer the one opened early.
        let _opened = self.early_page();
        // Under a list, a page the allowed origins refuse, however the task
        // came to it, is left before it is read or acted on, and a page whose
        // address cannot be read is not read either.
        if !self.options.allowed_origins.is_empty() {
            let id = self
                .ensure_session()
                .map_err(|error| Box::new(failure("observe", &error)))?;
            let checking = self.browser.check_page(&id);
            self.block(async { tokio::time::timeout(READ_TIMEOUT, checking).await })
                .unwrap_or_else(|_| {
                    Err(Error::timeout(
                        "observe",
                        u64::try_from(READ_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                    ))
                })
                .map_err(|error| Box::new(failure("observe", &error)))?;
        }
        if self.perception == Perception::Sight
            && let Some(mut screen) = self.see(root)
        {
            if !app.is_empty() {
                app.clone_into(&mut screen.app);
            }
            return Ok(screen);
        }
        let request = SnapshotRequest {
            selector: root.map(sight::selector),
            depth: (depth == Depth::Skeleton && root.is_none()).then_some(SKELETON_DEPTH),
            ..SnapshotRequest::default()
        };
        let snapshot = self
            .ensure_session()
            .and_then(|id| {
                let reading = self.browser.snapshot(&id, request);
                self.block(async { tokio::time::timeout(READ_TIMEOUT, reading).await })
                    .unwrap_or_else(|_| {
                        Err(Error::timeout(
                            "snapshot",
                            u64::try_from(READ_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                        ))
                    })
            })
            .map_err(|error| Box::new(failure("snapshot", &error)))?;
        let mut screen = tree::screen(&snapshot.tree, &snapshot.title);
        if !app.is_empty() {
            app.clone_into(&mut screen.app);
        }
        Ok(screen)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        let reference = target
            .as_ref()
            .map(|node| node.ref_id.clone())
            .filter(|reference| !reference.is_empty());
        let targeted = |command: &str, action: fn(Target, Option<String>) -> Action| {
            reference.clone().map_or_else(
                || {
                    DesktopResponse::err(
                        command,
                        DesktopError::new("INVALID_TARGET", "the operation needs a target"),
                    )
                },
                |reference| self.perform(command, action(self::target(&reference), text.clone())),
            )
        };
        if let Some(reference) = reference
            .as_deref()
            .filter(|_| uses_pointer(operation) || operation == JevOperation::TypeText)
        {
            self.show_cursor(reference);
        }
        // A native option is chosen by value: a click opens an unseen menu.
        if let Some(reply) = self.choose_if_native(operation, target.as_ref()) {
            return reply;
        }
        match operation {
            JevOperation::Click => self.press_element(target.as_ref(), reference.as_deref(), true),
            // Opening or closing in place goes nowhere by design: never
            // followed as a link that ignored its press.
            JevOperation::Expand | JevOperation::Collapse => {
                self.press_element(target.as_ref(), reference.as_deref(), false)
            }
            // Without a target the text goes where the focus is, as into an
            // autocomplete's unnamed input once it has been opened — but
            // only once the focused element is verified to actually take
            // typed text; a page that moved focus elsewhere (or nowhere)
            // must refuse rather than silently deliver the text to whatever
            // it finds, which could otherwise leak a private value into an
            // unrelated field.
            JevOperation::TypeText if reference.is_none() => {
                if !self.focused_field_is_editable() {
                    return DesktopResponse::err(
                        "type-text",
                        DesktopError::new("INVALID_TARGET", "no editable field has focus"),
                    );
                }
                self.perform(
                    "type-text",
                    Action::Type {
                        target: None,
                        text: text.unwrap_or_default(),
                        delay_ms: None,
                    },
                )
            }
            JevOperation::TypeText => {
                if let Some(reference) = reference.as_deref()
                    && !self.takes_text(reference)
                {
                    return not_a_text_field();
                }
                targeted("type-text", |target, text| Action::Fill {
                    target,
                    value: text.unwrap_or_default(),
                })
            }
            JevOperation::Check => targeted("check", |target, _| Action::Check {
                target,
                checked: true,
            }),
            JevOperation::Uncheck => targeted("uncheck", |target, _| Action::Check {
                target,
                checked: false,
            }),
            JevOperation::Scroll => self.perform(
                "scroll",
                Action::Scroll {
                    direction: ScrollDirection::Down,
                    pixels: None,
                    target: reference.as_deref().map(self::target),
                },
            ),
            JevOperation::Wait => self.perform("wait", pause(500)),
            JevOperation::Drill | JevOperation::Widen => {
                DesktopResponse::ok("look", json!({"root": reference}))
            }
            JevOperation::Done | JevOperation::Blocked => {
                DesktopResponse::ok("resolve-intent", json!({}))
            }
        }
    }

    fn read_value(&self, target: &Candidate) -> Option<String> {
        if target.ref_id.is_empty() {
            return None;
        }
        let id = self.ensure_session().ok()?;
        let selector = sight::selector(&target.ref_id);
        ["inputvalue", "gettext"].into_iter().find_map(|action| {
            let data = self
                .block(
                    self.browser
                        .command(&id, json!({"action": action, "selector": selector})),
                )
                .ok()?;
            data.get("value")
                .or_else(|| data.get("text"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        })
    }

    /// Types at the field's caret: focus it, select what it holds when it is
    /// a plain field, and insert the text. A page needs no clipboard for this.
    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse {
        if target.ref_id.is_empty() {
            return DesktopResponse::err(
                "paste",
                DesktopError::new("INVALID_TARGET", "paste needs a target"),
            );
        }
        let focused = self.perform(
            "focus",
            Action::Focus {
                target: self::target(&target.ref_id),
            },
        );
        if !focused.ok {
            return focused;
        }
        if !self.focused_field_is_editable() {
            return not_a_text_field();
        }
        if target
            .available_actions
            .iter()
            .any(|action| action == "SetValue")
        {
            let selected = self.press(app, "cmd+a");
            if !selected.ok {
                return selected;
            }
        }
        self.perform(
            "paste",
            Action::Type {
                target: None,
                text: text.to_owned(),
                delay_ms: None,
            },
        )
    }

    fn press(&self, _app: &str, combo: &str) -> DesktopResponse {
        let key = browser_key(combo, self.platform);
        if key.is_empty() {
            return DesktopResponse::err(
                "press",
                DesktopError::new("INVALID_KEY", "press needs a key"),
            );
        }
        self.perform("press", Action::Press { key })
    }

    fn launch(&self, _app: &str) -> DesktopResponse {
        reply(
            "launch",
            self.ensure_session()
                .map(|id| json!({"running": true, "session": id})),
        )
    }

    fn settle(&self) {
        if self.settle == Settle::Prompt {
            if let Ok(id) = self.ensure_session() {
                // Each within a deadline: a call sent while a page is being
                // replaced can wait out the browser's own 30 s.
                let quiet = self.browser.command(
                    &id,
                    json!({"action": "waitforloadstate", "state": "networkquiet", "timeout": QUIET_MS}),
                );
                let _quiet = self
                    .block(async { tokio::time::timeout(watch::deadline(QUIET_MS), quiet).await });
                self.await_still(&id);
            }
            return;
        }
        if let Ok(id) = self.ensure_session() {
            let _idle = self.block(self.browser.command(
                &id,
                json!({"action": "waitforloadstate", "state": "networkidle", "timeout": NETWORK_IDLE_MS}),
            ));
        }
        let _settled = self.perform("wait", pause(SETTLE_MS));
    }

    fn settle_briefly(&self) {
        // Nothing was fetched to wait for: only the page's own movement,
        // under prompt settling. Steady settling stays as it always was.
        if self.settle != Settle::Prompt {
            self.settle();
            return;
        }
        if let Ok(id) = self.ensure_session() {
            self.await_still(&id);
        }
    }

    fn await_change(&self, ms: u64) -> bool {
        // With no page open there is nothing to watch, and nothing to open.
        let Some(id) = self.session() else {
            return true;
        };
        let watching = self.browser.command(
            &id,
            json!({"action": "evaluate", "script": watch::change_script(ms)}),
        );
        self.block(async { tokio::time::timeout(watch::deadline(ms), watching).await })
            .ok()
            .and_then(Result::ok)
            .and_then(|data| data.get("result").and_then(Value::as_bool))
            .unwrap_or(true)
    }

    fn navigate(&self, url: &str) -> DesktopResponse {
        // Loaded there while the plan was drafted, in this session, and not
        // read since.
        if let Some((opened_in, opened)) = self.early_page()
            && place(&opened) == place(url)
            && self.session().as_ref() == Some(&opened_in)
            && let Some((shown, title)) = self.shown_page(&opened_in)
        {
            return reply("navigate", Ok(json!({"url": shown, "title": title})));
        }
        match self.ensure_session() {
            Ok(id) => self.navigate_in(&id, NavigateRequest::new(url)),
            Err(error) => reply("navigate", Err(error)),
        }
    }

    fn back(&self, _app: &str) -> DesktopResponse {
        self.perform("back", Action::Back)
    }

    fn dismiss_cover(&self, target: &Candidate) -> DesktopResponse {
        self.press_cover(&target.ref_id)
    }
}

impl BrowserSurface {
    /// Loads `request`'s address in session `id`. A heavy page can be read
    /// long before its `load` event fires.
    pub(super) fn navigate_in(&self, id: &SessionId, request: NavigateRequest) -> DesktopResponse {
        let url = request.url.clone();
        let page = self
            .block(self.browser.navigate(id, request))
            .map(|page| (page.url, page.title));
        let page = match page {
            Err(error @ Error::Timeout { .. }) => self.drawn_page(id, &url).ok_or(error),
            other => other,
        };
        reply(
            "navigate",
            page.map(|(url, title)| json!({"url": url, "title": title})),
        )
    }

    /// Takes the session opened early and the address loaded in it: the
    /// first read or navigation leaves that page as it was opened.
    pub(super) fn early_page(&self) -> Option<(SessionId, String)> {
        self.opened_at
            .lock()
            .ok()
            .and_then(|mut opened| opened.take())
    }

    /// Waits until the page stops changing (`watch::still_script`), within
    /// a deadline: a call sent while a page is being replaced can wait out
    /// the browser's own 30 s.
    fn await_still(&self, id: &SessionId) {
        let still = self.browser.command(
            id,
            json!({"action": "evaluate", "script": watch::still_script()}),
        );
        let _still =
            self.block(async { tokio::time::timeout(watch::deadline(SETTLE_MS), still).await });
    }
}

/// How the engine addresses `reference`: a ref sight minted by its mark's
/// CSS selector, a tree ref as itself.
pub(super) fn target(reference: &str) -> Target {
    if sight::is_seen(reference) {
        Target::Selector {
            value: sight::selector(reference),
        }
    } else {
        Target::reference(reference)
    }
}

fn pause(ms: u64) -> Action {
    Action::WaitFor {
        target: None,
        text: None,
        state: WaitState::Visible,
        ms: Some(ms),
        timeout_ms: None,
    }
}

/// A flow's key combination (`cmd+a`, `return`) as agent-browser spells it
/// (`Meta+a`, `Enter`). The logical `cmd` becomes the platform's command key.
#[must_use]
pub(crate) fn browser_key(combo: &str, platform: Platform) -> String {
    let command = Key::SelectAll
        .browser(platform)
        .and_then(|select_all| select_all.split('+').next().map(str::to_owned))
        .unwrap_or_else(|| "Control".to_owned());
    combo
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" => command.clone(),
            "ctrl" | "control" => "Control".to_owned(),
            "shift" => "Shift".to_owned(),
            "alt" | "option" => "Alt".to_owned(),
            "return" | "enter" => "Enter".to_owned(),
            "escape" | "esc" => "Escape".to_owned(),
            "tab" => "Tab".to_owned(),
            "space" => "Space".to_owned(),
            "backspace" => "Backspace".to_owned(),
            "delete" => "Delete".to_owned(),
            "left" | "right" | "up" | "down" => {
                let mut arrow = "Arrow".to_owned();
                let mut direction = part.to_ascii_lowercase();
                direction[..1].make_ascii_uppercase();
                arrow.push_str(&direction);
                arrow
            }
            other if other.len() == 1 => other.to_owned(),
            other => {
                let mut named = other.to_owned();
                named[..1].make_ascii_uppercase();
                named
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}
