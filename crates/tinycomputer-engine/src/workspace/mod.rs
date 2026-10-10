//! [`Workspace`]: the desktop and the browser as one surface, so a single
//! flow can move between an application and a web page.
//!
//! Calls that name an application route by that name: `browser` (or an
//! address) goes to the browser, anything else to the desktop. Calls that do
//! not name one — acting on a candidate, reading it back — go to whichever
//! side was observed or opened last, which is the side the candidate came
//! from.

use std::sync::{Arc, Mutex};

use tinycomputer_bus::{DesktopError, DesktopResponse, JevOperation};
use tinycomputer_core::surface::{Candidate, Depth, Screen, Surface};

/// The application name that routes to the browser.
pub const BROWSER: &str = "browser";

/// Which side of a [`Workspace`] a call goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Desktop,
    Browser,
}

/// A desktop surface and a browser surface, each present only when the
/// task's constraints allow it: a task confined to one side must never reach
/// the other, so neither side is unconditionally available.
#[derive(Debug, Clone)]
pub struct Workspace<D, W> {
    desktop: Option<D>,
    browser: Option<W>,
    active: Arc<Mutex<Side>>,
    /// The application (or `browser`) last observed or opened.
    last_app: Arc<Mutex<String>>,
}

impl<D: Surface, W: Surface> Workspace<D, W> {
    /// A workspace with `desktop` when the desktop is available to this task,
    /// and `browser` when the browser is.
    #[must_use]
    pub fn new(desktop: Option<D>, browser: Option<W>) -> Self {
        // Default to whichever side is actually available, so a browser-only
        // task does not start by aiming an unnamed call at a desktop it was
        // never given.
        let active = if desktop.is_some() {
            Side::Desktop
        } else {
            Side::Browser
        };
        Self {
            desktop,
            browser,
            active: Arc::new(Mutex::new(active)),
            last_app: Arc::new(Mutex::new(String::new())),
        }
    }

    /// The visible text of whatever was last observed or opened: its
    /// context lines, control labels, and what each field holds in full
    /// (`label = "value"`). Empty when nothing has been, or it can no
    /// longer be read. Blocks, like every surface call.
    ///
    /// A field's value is never clipped here: every caller masks secrets
    /// out of this text afterward by their exact value, and a value cut
    /// short first would leave its uncut prefix unmasked. Callers that also
    /// bound the total text they send on (the rescuer's briefing, capped at
    /// [`crate::rescue::SCREEN_CHARS`]) do so after masking, not before.
    #[must_use]
    pub fn visible_text(&self) -> Vec<String>
    where
        D: Sync,
        W: Sync,
    {
        let app = self
            .last_app
            .lock()
            .map(|app| app.clone())
            .unwrap_or_default();
        if app.is_empty() {
            return Vec::new();
        }
        self.observe(&app, None, Depth::Full)
            .map(|screen| {
                screen
                    .context
                    .into_iter()
                    .chain(screen.candidates.iter().filter_map(|node| {
                        let held = node
                            .value
                            .as_ref()
                            .and_then(serde_json::Value::as_str)
                            .filter(|held| !held.trim().is_empty());
                        match (&node.name, held) {
                            (Some(name), Some(held)) => Some(format!("{name} = {held:?}")),
                            (name, _) => name.clone(),
                        }
                    }))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn remember(&self, app: &str) {
        if let Ok(mut last) = self.last_app.lock() {
            app.clone_into(&mut last);
        }
    }

    /// Whether this workspace can reach a browser.
    #[must_use]
    pub fn has_browser(&self) -> bool {
        self.browser.is_some()
    }

    /// Whether this workspace can reach the desktop.
    #[must_use]
    pub fn has_desktop(&self) -> bool {
        self.desktop.is_some()
    }

    fn side_for(app: &str) -> Side {
        let app = app.trim().to_ascii_lowercase();
        if app == BROWSER
            || app.starts_with("browser:")
            || app.starts_with("http://")
            || app.starts_with("https://")
        {
            Side::Browser
        } else {
            Side::Desktop
        }
    }

    fn activate(&self, side: Side) {
        if let Ok(mut active) = self.active.lock() {
            *active = side;
        }
    }

    fn active(&self) -> Side {
        self.active.lock().map_or(Side::Desktop, |active| *active)
    }

    /// Whether the browser is the side in use now: the surface a screenshot
    /// of the task's current screen should come from.
    #[must_use]
    pub fn browser_active(&self) -> bool {
        self.active_browser().is_some()
    }

    /// The browser, when it is the side in use. The browser only becomes
    /// active after a call on it succeeds, so it is never active when absent.
    fn active_browser(&self) -> Option<&W> {
        self.browser
            .as_ref()
            .filter(|_| self.active() == Side::Browser)
    }

    /// Runs `call` on the side named by `side`, or refuses when that side is
    /// not available to this task.
    fn on(
        &self,
        side: Side,
        command: &str,
        desktop: impl FnOnce(&D) -> DesktopResponse,
        browser: impl FnOnce(&W) -> DesktopResponse,
    ) -> DesktopResponse {
        match (side, &self.desktop, &self.browser) {
            (Side::Desktop, Some(surface), _) => desktop(surface),
            (Side::Desktop, None, _) => no_desktop(command),
            (Side::Browser, _, Some(surface)) => browser(surface),
            (Side::Browser, _, None) => no_browser(command),
        }
    }
}

fn no_browser(command: &str) -> DesktopResponse {
    DesktopResponse::err(
        command,
        DesktopError::new(
            "BROWSER_NOT_AVAILABLE",
            "this task has no browser; allow the browser surface to use web pages",
        ),
    )
}

fn no_desktop(command: &str) -> DesktopResponse {
    DesktopResponse::err(
        command,
        DesktopError::new(
            "DESKTOP_NOT_AVAILABLE",
            "this task has no desktop; allow the desktop surface to use applications",
        ),
    )
}

impl<D: Surface + Sync, W: Surface + Sync> Surface for Workspace<D, W> {
    fn observe(
        &self,
        app: &str,
        root: Option<&str>,
        depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        let side = Self::side_for(app);
        let screen = match (side, &self.desktop, &self.browser) {
            (Side::Desktop, Some(desktop), _) => desktop.observe(app, root, depth),
            (Side::Desktop, None, _) => Err(Box::new(no_desktop("snapshot"))),
            (Side::Browser, _, Some(browser)) => browser.observe(app, root, depth),
            (Side::Browser, _, None) => Err(Box::new(no_browser("snapshot"))),
        }?;
        self.activate(side);
        self.remember(app);
        Ok(screen)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.execute(operation, target, text),
            (None, Some(desktop)) => desktop.execute(operation, target, text),
            (None, None) => no_desktop("execute"),
        }
    }

    fn read_value(&self, target: &Candidate) -> Option<String> {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.read_value(target),
            (None, Some(desktop)) => desktop.read_value(target),
            (None, None) => None,
        }
    }

    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse {
        self.on(
            Self::side_for(app),
            "paste",
            |desktop| desktop.paste(app, target, text),
            |browser| browser.paste(app, target, text),
        )
    }

    fn press(&self, app: &str, combo: &str) -> DesktopResponse {
        self.on(
            Self::side_for(app),
            "press",
            |desktop| desktop.press(app, combo),
            |browser| browser.press(app, combo),
        )
    }

    fn launch(&self, app: &str) -> DesktopResponse {
        let side = Self::side_for(app);
        let reply = self.on(
            side,
            "launch",
            |desktop| desktop.launch(app),
            |browser| browser.launch(app),
        );
        if reply.ok {
            self.activate(side);
            self.remember(app);
        }
        reply
    }

    fn settle(&self) {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.settle(),
            (None, Some(desktop)) => desktop.settle(),
            (None, None) => {}
        }
    }

    fn settle_briefly(&self) {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.settle_briefly(),
            (None, Some(desktop)) => desktop.settle_briefly(),
            (None, None) => {}
        }
    }

    fn await_change(&self, ms: u64) -> bool {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.await_change(ms),
            (None, Some(desktop)) => desktop.await_change(ms),
            (None, None) => false,
        }
    }

    fn navigate(&self, url: &str) -> DesktopResponse {
        let Some(browser) = &self.browser else {
            return no_browser("navigate");
        };
        let reply = browser.navigate(url);
        if reply.ok {
            self.activate(Side::Browser);
            self.remember(BROWSER);
        }
        reply
    }
    fn back(&self, app: &str) -> DesktopResponse {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.back(app),
            (None, Some(desktop)) => desktop.back(app),
            (None, None) => no_browser("back"),
        }
    }

    fn dismiss_cover(&self, target: &Candidate) -> DesktopResponse {
        match (self.active_browser(), &self.desktop) {
            (Some(browser), _) => browser.dismiss_cover(target),
            (None, Some(desktop)) => desktop.dismiss_cover(target),
            (None, None) => no_browser("dismiss-cover"),
        }
    }
}

#[cfg(test)]
mod workspace_tests;
