//! [`BrowserSurface`]: one browser session as a decision loop's
//! [`Surface`], so the flow runtime drives a web page the way it drives a
//! desktop application.
//!
//! Surface calls block, and the flow runtime makes them off its executor
//! (`spawn_blocking`); each one here runs the async [`Browser`] call to
//! completion on the runtime handle the surface was built with. The session
//! opens lazily, on the first call that needs a page.
//!
//! When the session has a window on screen, the agent's cursor glides onto
//! each element before the surface acts on it (`cursor.rs`). It is cosmetic:
//! the actions are the same with or without it.

mod card;
mod cover;
mod cursor;
mod envelope;
mod fields;
mod location;
mod native_select;
mod operations;
mod sight;
mod tabs;
mod tree;
mod uncover;
mod watch;

#[cfg(test)]
pub(crate) use cover::{COVER_JS, SPOT_GRID};
#[cfg(test)]
pub(crate) use uncover::{CENTRE_JS, INTO_VIEW_JS};

pub use sight::Denoised;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tinycomputer_bus::DesktopResponse;
use tinycomputer_bus::browser::{
    Action, NavigateRequest, SessionId, SessionOptions, SnapshotRequest,
};
use tinycomputer_core::Platform;
use tinycomputer_core::surface::Screen;
use tinycomputer_cursor::ScreenCursor;

use crate::error::{Error, Result};
use crate::sessions::Browser;
use envelope::reply;
#[cfg(doc)]
use tinycomputer_core::surface::Surface;

/// How deep a skeleton observation reads before a flow drills in.
const SKELETON_DEPTH: u32 = 6;

/// How long the page is given to react once its network is quiet: long
/// enough for a banner or menu to finish closing.
const SETTLE_MS: u64 = 400;

/// The longest a settle waits for the page's network to go quiet; a page
/// that polls forever is never idle, so this is a cap, not an expectation.
const NETWORK_IDLE_MS: u64 = 2_000;

/// The longest [`Settle::Prompt`] waits for the requests that change the
/// page to end. Live on Amazon, each action that opened a page sent 100+
/// requests for over 2 s, while what the task needed (the results, a
/// product's title and Add to Cart, the cart's subtotal) showed after
/// 0.7–1.2 s.
const QUIET_MS: u64 = 1_000;

/// The longest one reading of the page may take, by sight or as a tree. A
/// reading sent while a page was being replaced waited out the browser's
/// own deadline live, 30 s for sight and again for the tree, so one look
/// took a minute; a reading this late is retried on the next look instead.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How a [`BrowserSurface`] reads a page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Perception {
    /// As a person looks at it: what is drawn and on top, the words on and
    /// beside each control, and which boxes take text, read from the
    /// rendered page. One shadow root's controls are read from the
    /// accessibility tree beside it. Falls back to the tree alone when it
    /// cannot reach what it sees (two shadow roots showing controls, a frame
    /// in front, a shadow root the tree cannot read) or the reading fails.
    #[default]
    Sight,
    /// Through the accessibility tree alone: roles and names as the page's
    /// markup declares them.
    Tree,
}

/// How a [`BrowserSurface`] lets the page settle after an action, before
/// the page is read again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Settle {
    /// Wait for the network to go idle — 500 ms with nothing in flight,
    /// counted only after a first quiet receive window, so at least about
    /// 1.1 s, and at most 2 s (`NETWORK_IDLE_MS`) — then pause 400 ms
    /// (`SETTLE_MS`) more.
    Steady,
    /// Wait for the network to go quiet, counting the 500 ms from the start
    /// and only the requests that can change the page (its document,
    /// scripts, stylesheets, fetched data), those the action sent included,
    /// at most 1 s (`QUIET_MS`); then only until the page stops changing:
    /// no DOM change for 120 ms (`STILL_MS`) and no finite CSS animation
    /// running, over at least two drawn frames, at most 400 ms (`SETTLE_MS`).
    /// An idle page is read again after about 0.6 s instead of 1.6 s; a busy
    /// one still waits for its requests. The default: over 44 live runs it
    /// cost no run its outcome.
    #[default]
    Prompt,
}

/// How long the page must go without a DOM change, under [`Settle::Prompt`],
/// to count as still.
const STILL_MS: u64 = 120;

/// The longest an early load ([`BrowserSurface::open_at`]) waits for its
/// page's `load` event: beyond nine in ten live first pages (6.4 s). A plan
/// drafted sooner waits for it no longer, and a page not drawn by then is
/// loaded again by the step that browses there.
const EARLY_LOAD_MS: u64 = 10_000;

/// One browser session, lazily opened, as a [`Surface`].
#[derive(Clone)]
pub struct BrowserSurface {
    browser: Arc<Browser>,
    options: SessionOptions,
    session: Arc<Mutex<Option<SessionId>>>,
    handle: tokio::runtime::Handle,
    platform: Platform,
    cursor: Arc<ScreenCursor>,
    perception: Perception,
    settle: Settle,
    denoised: Arc<Mutex<Denoised>>,
    /// Set once the surface is let go ([`BrowserSurface::close`]): it is
    /// then not opened early again.
    closed: Arc<AtomicBool>,
    /// The session opened early ([`BrowserSurface::open_at`]) and the
    /// address loaded in it, until the page is first read or another address
    /// is loaded: a navigation there, in that session, finds it loaded.
    opened_at: Arc<Mutex<Option<(SessionId, String)>>>,
}

impl std::fmt::Debug for BrowserSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserSurface")
            .field("session", &self.session)
            .field("platform", &self.platform)
            .field("cursor", &self.cursor)
            .field("perception", &self.perception)
            .field("settle", &self.settle)
            .field("opened_at", &self.opened_at)
            .finish_non_exhaustive()
    }
}

impl BrowserSurface {
    /// A surface that opens its session on `browser` with `options`, and
    /// runs browser calls on `handle`. It draws no cursor until given one
    /// with [`BrowserSurface::with_cursor`].
    #[must_use]
    pub fn new(
        browser: Arc<Browser>,
        options: SessionOptions,
        handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            browser,
            options,
            session: Arc::new(Mutex::new(None)),
            handle,
            platform: Platform::current(),
            cursor: Arc::new(ScreenCursor::off()),
            perception: Perception::default(),
            settle: Settle::default(),
            denoised: Arc::new(Mutex::new(Denoised::default())),
            closed: Arc::new(AtomicBool::new(false)),
            opened_at: Arc::default(),
        }
    }

    /// The same surface, settling after an action with `settle`
    /// ([`Settle::Prompt`] unless told otherwise).
    #[must_use]
    pub fn with_settle(mut self, settle: Settle) -> Self {
        self.settle = settle;
        self
    }

    /// The same surface, reading pages with `perception`
    /// ([`Perception::Sight`] unless told otherwise).
    #[must_use]
    pub fn with_perception(mut self, perception: Perception) -> Self {
        self.perception = perception;
        self
    }

    /// The same surface, drawing on `cursor` — the screen's one agent
    /// cursor, shared with the desktop surface — whenever its session has a
    /// window on screen. The cursor is cosmetic: every action is performed
    /// the same way with or without it.
    #[must_use]
    pub fn with_cursor(mut self, cursor: Arc<ScreenCursor>) -> Self {
        self.cursor = cursor;
        self
    }

    /// Opens the session now, if it is not open yet, rather than at the first
    /// call that needs it: a task that will run on the browser can open it
    /// while its plan is drafted. Whether the session is open. A surface
    /// already let go ([`BrowserSurface::close`]) is not opened: a task
    /// cancelled while its browser was being opened early is left holding
    /// none.
    #[must_use]
    pub fn open(&self) -> bool {
        self.session_slot(true).is_ok()
    }

    /// Opens the session early, as [`BrowserSurface::open`] does, and loads
    /// `url` in it: the page a task names, loaded while its plan is drafted,
    /// waiting for its `load` at most `EARLY_LOAD_MS`. Until the page is
    /// first read, a navigation to the same place (one page's two
    /// addresses: `https://` or not, `www.` or not, a trailing slash or not)
    /// in the same session finds it loaded and loads nothing. Whether the
    /// page loaded; a surface already let go opens nothing, and one let go
    /// meanwhile keeps no page.
    #[must_use]
    pub fn open_at(&self, url: &str) -> bool {
        let Ok(id) = self.session_slot(true) else {
            return false;
        };
        let request = NavigateRequest {
            timeout_ms: Some(EARLY_LOAD_MS),
            ..NavigateRequest::new(url)
        };
        let loaded = self.navigate_in(&id, request).ok;
        if loaded
            && !self.closed.load(Ordering::Acquire)
            && let Ok(mut opened) = self.opened_at.lock()
        {
            *opened = Some((id, url.to_owned()));
        }
        loaded
    }

    /// The session this surface drives, once one is open.
    #[must_use]
    pub fn session(&self) -> Option<SessionId> {
        self.session.lock().ok().and_then(|session| session.clone())
    }

    /// What the last observation left out as noise: ads, blank boxes, and
    /// hidden content. Zero until a page is read, and when the last one was
    /// read through the accessibility tree rather than by sight.
    #[must_use]
    pub fn denoised(&self) -> Denoised {
        self.denoised
            .lock()
            .map(|denoised| *denoised)
            .unwrap_or_default()
    }

    /// Closes the session, if one is open, without waiting for it: safe to
    /// call from async code, where a blocking surface call is not.
    pub fn close(&self) {
        // Before the slot is taken: an early open that has not begun yet
        // finds it set, and one under way finishes first and is closed.
        self.closed.store(true, Ordering::Release);
        let _opened = self.early_page();
        let Some(id) = self
            .session
            .lock()
            .ok()
            .and_then(|mut session| session.take())
        else {
            return;
        };
        let browser = self.browser.clone();
        self.handle.spawn(async move {
            let _closed = browser.close_session(&id).await;
        });
    }

    fn block<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.handle.block_on(future)
    }

    fn ensure_session(&self) -> Result<SessionId> {
        self.session_slot(false)
    }

    /// The open session, opening one if there is none; when `early`, not on
    /// a surface already let go. `closed` is read under the slot's lock, so
    /// an early open and a close cannot both miss each other.
    fn session_slot(&self, early: bool) -> Result<SessionId> {
        let mut slot = self
            .session
            .lock()
            .map_err(|_| Error::failed("the browser surface was poisoned by a panic"))?;
        if let Some(id) = slot.as_ref() {
            return Ok(id.clone());
        }
        if early && self.closed.load(Ordering::Acquire) {
            return Err(Error::failed("the browser surface was let go"));
        }
        let info = self.block(self.browser.open_session(self.options.clone()))?;
        *slot = Some(info.id.clone());
        Ok(info.id)
    }

    /// The page, or the part of it under `root`, read by sight; `None` when
    /// the reading fails or sees what it cannot reach, and the tree is read
    /// instead.
    fn see(&self, root: Option<&str>) -> Option<Screen> {
        self.keep_denoised(Denoised::default());
        let id = self.ensure_session().ok()?;
        // The timer is made inside the runtime `block` enters, not before.
        let reading = self.browser.command(
            &id,
            json!({"action": "evaluate", "script": sight::script(root)}),
        );
        let reply = self
            .block(async { tokio::time::timeout(READ_TIMEOUT, reading).await })
            .ok()?
            .ok()?;
        let result = reply.get("result")?;
        let mut screen = sight::screen(result)?;
        match sight::shadows(result).as_slice() {
            [] => {}
            [shadow] => self.read_shadow(&id, shadow, &mut screen)?,
            // A tree snapshot's refs last until the next one: one host's
            // subtree can be read beside sight, not two.
            _ => return None,
        }
        self.keep_denoised(sight::denoised(result));
        Some(screen)
    }

    /// Adds to `screen` what the tree reads under `shadow`'s host: the
    /// controls a selector cannot reach, and the host and what the page puts
    /// in its slots, which sight leaves to the tree, under the label of the
    /// layer they draw, after everything sight read. `None` when the subtree
    /// cannot be read, so the tree reads the whole page instead.
    fn read_shadow(
        &self,
        id: &SessionId,
        shadow: &sight::Shadow,
        screen: &mut Screen,
    ) -> Option<()> {
        let request = SnapshotRequest {
            selector: Some(sight::selector(&shadow.host)),
            ..SnapshotRequest::default()
        };
        let reading = self.browser.snapshot(id, request);
        let snapshot = self
            .block(async { tokio::time::timeout(READ_TIMEOUT, reading).await })
            .ok()?
            .ok()?;
        let part = tree::screen(&snapshot.tree, &snapshot.title);
        let after = screen
            .candidates
            .iter()
            .chain(&screen.text_nodes)
            .map(|node| node.order + 1)
            .max()
            .unwrap_or_default();
        let placed = |mut node: tinycomputer_core::surface::Candidate| {
            if let Some(label) = &shadow.label {
                node.path.insert(0, label.clone());
            }
            node.order += after;
            node
        };
        screen
            .candidates
            .extend(part.candidates.into_iter().map(placed));
        screen
            .text_nodes
            .extend(part.text_nodes.into_iter().map(placed));
        for line in part.context {
            if !screen.context.contains(&line) {
                screen.context.push(line);
            }
        }
        Some(())
    }

    fn keep_denoised(&self, denoised: Denoised) {
        if let Ok(mut kept) = self.denoised.lock() {
            *kept = denoised;
        }
    }

    fn perform(&self, command: &str, action: Action) -> DesktopResponse {
        let outcome = self.ensure_session().and_then(|id| {
            self.block(self.browser.perform(&id, action))
                .map(|outcome| json!({"value": outcome.value, "url": outcome.page.url}))
        });
        reply(command, outcome)
    }
}

#[cfg(test)]
mod surface_tests;
