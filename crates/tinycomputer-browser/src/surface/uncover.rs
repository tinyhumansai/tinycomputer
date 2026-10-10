//! Pressing again when the pointer or the page's scroll left the target
//! covered, and following a link whose press did not take.

use std::time::Duration;

use serde_json::{Value, json};
use tinycomputer_bus::browser::Action;
use tinycomputer_bus::{DesktopError, DesktopResponse};
use tinycomputer_core::surface::Candidate;

use super::BrowserSurface;
use super::card::selects_on_click;
use super::envelope::covered;
use super::location;
use super::operations::target;
use super::sight;

/// How long a hover effect is given to end once the pointer has left it.
const HOVER_END_MS: u64 = 150;

/// Lets the focused text box go, so the list of suggestions it holds open
/// closes (live, a store's search dropdown stayed over its basket button,
/// and Escape left it there), unless the element is a row of such a list,
/// then brings the element to the middle of the window at once; `true` when
/// it found the element. A page that scrolls smoothly (live, a store set
/// `scroll-behavior: smooth`) would otherwise still be moving the element
/// when the press lands.
pub(crate) const CENTRE_JS: &str = r"(element => {
  if (!element) return false;
  const focused = document.activeElement;
  const row = element.closest('[role=option], [role=listbox], [role=menu], [role=menuitem], datalist');
  if (focused && focused !== element && !focused.contains(element) && !row
      && focused.matches('input, textarea, [contenteditable=true]')) {
    focused.blur();
  }
  element.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  return true;
})";

/// Brings an element whose middle lies outside the window to the window's
/// middle, and leaves one whose middle shows where it is; `true` when it
/// moved the element. A press lands on the element's middle, and the browser
/// presses there whether the window shows that point or not: live, a store's
/// "Add to cart" sat at the window's foot with its middle below it, and every
/// press went nowhere while reporting success.
pub(crate) const INTO_VIEW_JS: &str = r"(element => {
  if (!element) return false;
  const box = element.getBoundingClientRect();
  const x = box.left + box.width / 2;
  const y = box.top + box.height / 2;
  if (x >= 0 && y >= 0 && x < window.innerWidth && y < window.innerHeight) return false;
  element.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  return true;
})";

/// How long a link's press is given to start leaving the page.
const LEAVE_MS: u64 = 400;

/// The address a link the element is, or sits in, leads to, when that is
/// another page than this one: `null` for an in-page anchor, a script
/// link, a link whose page is already open, a short link such as a menu's
/// "More", which may open a menu where it is rather than a page, a link
/// that downloads a file or controls something on the page, a page already
/// leaving (`__tcLeaving`, set when it began to unload), or while a dialog
/// is shown in front.
const AWAY_JS: &str = r"(element => {
  const link = element && element.closest('a[href]');
  if (!link || window.__tcLeaving) return null;
  if ((link.innerText || '').trim().split(/\s+/).length < 4) return null;
  if (link.hasAttribute('download') || link.matches('[aria-expanded], [aria-haspopup], [aria-controls]')) {
    return null;
  }
  const href = link.getAttribute('href') || '';
  if (!href || href.startsWith('#') || /^javascript:/i.test(href)) return null;
  const away = new URL(href, location.href);
  const here = new URL(location.href);
  away.hash = '';
  here.hash = '';
  if (away.href === here.href || !/^https?:$/.test(away.protocol)) return null;
  const shown = (node) => {
    const box = node.getBoundingClientRect();
    return box.width > 1 && box.height > 1 && getComputedStyle(node).visibility !== 'hidden';
  };
  const front = [...document.querySelectorAll('dialog[open], [role=dialog], [role=alertdialog], [aria-modal=true]')]
    .some(shown);
  return front ? null : away.href;
})";

impl BrowserSurface {
    /// Presses the element `reference` names (`node` is what the screen
    /// showed of it): in this tab, with the location allowed when the
    /// control asks for it, through its own card's cover, once more after
    /// a cover the pointer or the scroll left, following a content link
    /// whose press went nowhere, and selecting again what a page ignored.
    /// A press still refused as covered names what covers its target in
    /// the error's `details.cover` (`cover.rs`).
    pub(super) fn press_element(
        &self,
        node: Option<&Candidate>,
        reference: Option<&str>,
        follows: bool,
    ) -> DesktopResponse {
        let Some(reference) = reference else {
            return DesktopResponse::err(
                "click",
                DesktopError::new("INVALID_TARGET", "the operation needs a target"),
            );
        };
        self.keep_in_tab(reference);
        if node.is_some_and(location::asks_where) {
            self.allow_location();
        }
        let seen = sight::is_seen(reference);
        if seen {
            self.bring_into_view(reference);
        }
        // Where a link's press starts from, to tell whether it went.
        let before = (follows && seen && node.is_some_and(|node| node.role == "link"))
            .then(|| self.page_url())
            .flatten();
        let reply = self.perform(
            "click",
            Action::Click {
                target: target(reference),
                new_tab: false,
            },
        );
        let name = node.and_then(|node| node.name.as_deref());
        let reply = if covered(&reply) && (name.is_some() || seen) {
            self.click_through_own_card(reference, name.unwrap_or_default())
                .unwrap_or(reply)
        } else {
            reply
        };
        let reply = if covered(&reply) && seen {
            self.click_uncovered(reference).unwrap_or(reply)
        } else {
            reply
        };
        let reply = if covered(&reply) {
            self.name_cover(reference, reply)
        } else {
            reply
        };
        // A follow that failed leaves the press as it went: the press itself
        // landed, and reading it as failed would press it again.
        if reply.ok
            && let Some(before) = &before
            && let Some(followed) = self
                .follow_if_ignored(reference, before)
                .filter(|followed| followed.ok)
        {
            return followed;
        }
        if reply.ok && seen && node.is_some_and(selects_on_click) {
            self.select_if_ignored(reference);
        }
        reply
    }

    /// Brings the element `reference` names into the window when its middle
    /// lies outside it ([`INTO_VIEW_JS`]), before it is pressed. A ref of the
    /// tree is brought into view by the browser itself.
    fn bring_into_view(&self, reference: &str) {
        let (Ok(id), Ok(selector)) = (
            self.ensure_session(),
            serde_json::to_string(&sight::selector(reference)),
        ) else {
            return;
        };
        let script = format!("{INTO_VIEW_JS}(document.querySelector({selector}))");
        let _moved = self.block(
            self.browser
                .command(&id, json!({"action": "evaluate", "script": script})),
        );
    }

    /// Clicks `reference` once more after the browser refused because
    /// something covered its middle, with the element brought to the middle
    /// of the window and the pointer moved off to the window's corner first.
    ///
    /// Two covers go away that way: a panel the pointer itself raised (a
    /// store's product photo opens a zoom panel over the column beside it
    /// while hovered, and live, "Add to cart" in that column was refused
    /// twelve times while the pointer rested on the photo), and a sticky
    /// bar the element had scrolled under. `None` when the element is gone.
    pub(super) fn click_uncovered(&self, reference: &str) -> Option<DesktopResponse> {
        let id = self.ensure_session().ok()?;
        let selector = serde_json::to_string(&sight::selector(reference)).ok()?;
        let script = format!("{CENTRE_JS}(document.querySelector({selector}))");
        let found = self
            .block(
                self.browser
                    .command(&id, json!({"action": "evaluate", "script": script})),
            )
            .ok()
            .and_then(|data| data.get("result").and_then(Value::as_bool))
            .unwrap_or(false);
        if !found {
            return None;
        }
        let _moved = self.block(self.browser.command(
            &id,
            json!({"action": "mouse", "eventType": "mouseMoved", "x": 1, "y": 1}),
        ));
        // Surface calls block by contract (the flow makes them off its
        // executor), so a pause here is a plain sleep.
        std::thread::sleep(Duration::from_millis(HOVER_END_MS));
        Some(self.perform(
            "click",
            Action::Click {
                target: target(reference),
                new_tab: false,
            },
        ))
    }

    /// Goes to the page the link `reference` names when its press left the
    /// browser where it was (`AWAY_JS`): a card can lay a carousel or a
    /// layer of its own over its link that takes the press, while a person
    /// reading the card means the page it links to. Live, a product link
    /// pressed six times never opened its product. `None` when the press
    /// was no link's, the page moved, or a dialog it opened is in front.
    pub(super) fn follow_if_ignored(
        &self,
        reference: &str,
        before: &str,
    ) -> Option<DesktopResponse> {
        std::thread::sleep(Duration::from_millis(LEAVE_MS));
        let id = self.ensure_session().ok()?;
        let selector = serde_json::to_string(&sight::selector(reference)).ok()?;
        let script = format!("{AWAY_JS}(document.querySelector({selector}))");
        let data = self
            .block(
                self.browser
                    .command(&id, json!({"action": "evaluate", "script": script})),
            )
            .ok()?;
        let away = data.get("result").and_then(Value::as_str)?.to_owned();
        if self.page_url()? != before {
            return None;
        }
        Some(tinycomputer_core::surface::Surface::navigate(self, &away))
    }

    /// The address the session's page shows now.
    pub(super) fn page_url(&self) -> Option<String> {
        let id = self.ensure_session().ok()?;
        self.block(self.browser.command(&id, json!({"action": "url"})))
            .ok()
            .and_then(|data| data.get("url").and_then(Value::as_str).map(str::to_owned))
    }
}
