//! Clicking through a result card's own cover, and pressing a selection again
//! when a page ignored the click.

use serde_json::{Value, json};
use tinycomputer_bus::DesktopResponse;
use tinycomputer_core::surface::Candidate;

use super::BrowserSurface;
use super::sight;

/// Whether what covers a point belongs to the same result card as the
/// element that was meant, so the click may go through it. Many result lists
/// lay a transparent click layer, or the card's own text, over each card's
/// link, so the card's own controls are always "covered" — by the card
/// itself.
///
/// The target is found by its name, *exactly*: among the elements stacked at
/// the point (`elementsFromPoint`, topmost first), or, when the card's
/// content sits over it — Google Flights puts each card's duration text
/// above its "Select flight" link — as the one element on the page whose
/// `aria-label` is that name and whose box holds the point (Google renders
/// each flight twice, once in a hidden tab). Whitespace runs count as one
/// space, as they do in an accessible name. A short name such as "Select"
/// appears in almost any card, so containment is never enough, and a label
/// two elements at the point share matches neither. A ref sight minted
/// passes its mark's selector as `exact`, and is the target itself, name or
/// no name. The click goes through only when what is
/// on top sits inside the target's own card (`li`, `listitem`, `row`,
/// `article`) and inside no dialog; a banner or dialog in front still
/// blocks it.
/// Presses `element` through the DOM when it is not selected yet; `true`
/// when it pressed.
const SELECT_JS: &str = r"(element => {
  if (!element) return false;
  const on = element.getAttribute('aria-selected') === 'true'
    || element.getAttribute('aria-checked') === 'true' || element.checked === true;
  if (on) return false;
  element.click();
  return true;
})";

/// Roles a click selects rather than toggles, and that a page marks as
/// selected once it took — or, for a list's option, closes the list over.
pub(super) fn selects_on_click(node: &Candidate) -> bool {
    ["tab", "radio", "option"].contains(&node.role.as_str())
        && !node
            .states
            .iter()
            .any(|state| state == "selected" || state == "checked")
}

const SAME_CARD_JS: &str = r#"((x, y, name, exact) => {
  if (!name && !exact) return false;
  const stack = document.elementsFromPoint(x, y);
  const top = stack[0];
  if (!top) return false;
  const squash = (text) => text.replace(/\s+/g, ' ').trim();
  name = squash(name || '');
  const shown = (element) => squash(element.getAttribute('aria-label') || element.innerText || '');
  const under = (element) => {
    const box = element.getBoundingClientRect();
    return box.width > 0 && box.height > 0
      && x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
  };
  const labelled = [...document.querySelectorAll('[aria-label]')]
    .filter((element) => squash(element.getAttribute('aria-label')) === name && under(element));
  const marked = exact ? document.querySelector(exact) : null;
  const target = exact ? (marked && under(marked) ? marked : null)
    : stack.find((element) => shown(element) === name)
      || (labelled.length === 1 ? labelled[0] : null);
  if (!target || target === top) return false;
  const card = target.closest('li,[role="listitem"],[role="row"],article,[role="article"]');
  const modal = top.closest('dialog,[role="dialog"],[role="alertdialog"],[aria-modal="true"]');
  return Boolean(card && card.contains(top) && !modal);
})"#;

impl BrowserSurface {
    /// Clicks the middle of `reference` even though something covers it,
    /// but only when the cover is part of the same result card, so a banner
    /// or dialog in front still blocks the click. `None` when it is not.
    /// Presses a tab, radio, or option again through the DOM when the click
    /// left it on screen unselected: a page can ignore a trusted click it has not yet wired up
    /// (Emirates' trip tabs, freshly loaded) while its own `click()` works.
    /// Selecting is idempotent, so pressing an already selected one is
    /// harmless; a checkbox, which toggles, is never pressed twice.
    pub(super) fn select_if_ignored(&self, reference: &str) {
        let Ok(id) = self.ensure_session() else {
            return;
        };
        let Ok(selector) = serde_json::to_string(&sight::selector(reference)) else {
            return;
        };
        let script = format!("{SELECT_JS}(document.querySelector({selector}))");
        let _pressed = self.block(
            self.browser
                .command(&id, json!({"action": "evaluate", "script": script})),
        );
    }

    pub(super) fn click_through_own_card(
        &self,
        reference: &str,
        name: &str,
    ) -> Option<DesktopResponse> {
        let id = self.ensure_session().ok()?;
        let selector = sight::selector(reference);
        let (x, y) = self.middle(&id, reference)?;
        // The JS side now requires an exact match against the target's own
        // shown text, so the name is passed through untruncated: cutting it
        // short would make an exact match against the page's full text
        // impossible for any control with a longer name.
        let name = name.trim();
        // A ref sight minted names its element exactly, by its mark; a
        // tree ref is found by its name.
        let exact = if sight::is_seen(reference) {
            serde_json::to_string(&selector).ok()?
        } else {
            "null".to_owned()
        };
        let script = format!(
            "{SAME_CARD_JS}({x}, {y}, {}, {exact})",
            serde_json::to_string(&name).ok()?
        );
        let same_card = self
            .block(
                self.browser
                    .command(&id, json!({"action": "evaluate", "script": script})),
            )
            .ok()?;
        if same_card.get("result") != Some(&Value::Bool(true)) {
            return None;
        }
        self.press_point(&id, x, y)?;
        Some(DesktopResponse::ok(
            "click",
            json!({"clicked": selector, "through": "its own card's click layer"}),
        ))
    }
}
