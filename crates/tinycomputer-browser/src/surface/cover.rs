//! What lies over an element a press could not reach, said as a person
//! would, and pressing an empty layer there the way a person clicks outside
//! a popup to close it.

use std::time::Duration;

use serde_json::{Value, json};
use tinycomputer_bus::browser::SessionId;
use tinycomputer_bus::{DesktopError, DesktopResponse};

use super::BrowserSurface;
use super::sight;

/// What lies over the point (`x`, `y`) of the element `exact` selects (a
/// sight mark, or `null` for a ref of the tree), said as a person would:
/// `{said, bare, spot}`; `null` when the element itself is on top there, and
/// `{unread: true}` when nothing can be read there. Open shadow roots are
/// looked into, as a page's components draw their controls inside them.
///
/// `said` names the cover: a control by its role and name (`button "Select
/// Location"`), a dialog, a layer by the text it shows or holds, a picture,
/// or an empty layer. `bare` is whether it is an empty layer a press outside
/// a popup closes: no control, dialog, text, or picture where it covers the
/// element, part of no component, placed over the page (it or its nearest
/// placed ancestor fixed or absolute), over at least half the window, and
/// over an element known by its mark, which a press there can be told
/// apart from. With `press`, `spot` is a point where that layer itself is on
/// top and nothing pressable lies beneath it (no control, nothing that shows
/// a pointer, nothing of the element), on an even grid ([`SPOT_GRID`] cells
/// a side), or `null` when there is none. The layer stops taking the
/// pointer only for the instant each point beneath it is read.
pub(crate) const COVER_JS: &str = r#"((x, y, exact, press, grid) => {
  const squash = (text) => (text || '').replace(/\s+/g, ' ').trim();
  const cut = (text) => (text.length > 60 ? `${text.slice(0, 59)}…` : text);
  // Up from a node, across shadow roots to their hosts.
  const above = (node) => node.parentElement || (node.parentNode && node.parentNode.host) || null;
  const within = (node, outer) => {
    for (let at = node; at; at = above(at)) if (at === outer) return true;
    return false;
  };
  const nearest = (node, selector) => {
    for (let at = node; at; at = above(at)) if (at.matches && at.matches(selector)) return at;
    return null;
  };
  const topAt = (px, py) => {
    let node = document.elementFromPoint(px, py);
    while (node && node.shadowRoot) {
      const inner = node.shadowRoot.elementFromPoint(px, py);
      if (!inner || inner === node) break;
      node = inner;
    }
    return node;
  };
  const hit = topAt(x, y);
  if (!hit) return { unread: true };
  const target = exact ? document.querySelector(exact) : null;
  if (target && (within(hit, target) || within(target, hit))) return null;
  const CONTROL = 'a[href], button, input, select, textarea, summary, [onclick], [contenteditable=""], '
    + '[contenteditable=true], [role=button], [role=link], [role=checkbox], [role=radio], [role=switch], '
    + '[role=tab], [role=menuitem], [role=option], [role=combobox], [role=textbox], [role=searchbox]';
  const ROLES = { a: 'link', button: 'button', input: 'textbox', select: 'combobox', textarea: 'textbox', summary: 'button' };
  const control = nearest(hit, CONTROL);
  if (control) {
    const role = control.getAttribute('role') || ROLES[control.tagName.toLowerCase()] || 'control';
    const name = cut(squash(control.getAttribute('aria-label') || control.innerText
      || control.getAttribute('title') || control.getAttribute('placeholder')));
    return { said: name ? `${role} "${name}"` : `an unnamed ${role}`, bare: false, spot: null };
  }
  const dialog = nearest(hit, 'dialog, [role=dialog], [role=alertdialog], [aria-modal=true]');
  if (dialog) {
    const name = cut(squash(dialog.getAttribute('aria-label') || dialog.innerText));
    return { said: name ? `a dialog showing "${name}"` : 'a dialog', bare: false, spot: null };
  }
  const own = squash([...hit.childNodes].filter((node) => node.nodeType === 3)
    .map((node) => node.textContent).join(' '));
  if (own) return { said: `a layer showing "${cut(own)}"`, bare: false, spot: null };
  if (nearest(hit, 'img, svg, video, canvas, iframe, picture, object, embed')) {
    return { said: 'a picture', bare: false, spot: null };
  }
  const held = cut(squash(hit.innerText));
  const said = held ? `a layer holding "${held}"` : 'an empty layer';
  const component = hit.getRootNode() !== document || Boolean(hit.shadowRoot) || hit.tagName.includes('-');
  let placed = false;
  for (let at = hit; at && at !== document.body && at !== document.documentElement; at = at.parentElement) {
    const position = getComputedStyle(at).position;
    if (position === 'static') continue;
    placed = position === 'fixed' || position === 'absolute';
    break;
  }
  const box = hit.getBoundingClientRect();
  const wide = Math.max(0, Math.min(box.right, innerWidth) - Math.max(box.left, 0));
  const tall = Math.max(0, Math.min(box.bottom, innerHeight) - Math.max(box.top, 0));
  const bare = Boolean(target) && !component && placed && wide * tall >= innerWidth * innerHeight / 2;
  if (!bare || !press) return { said, bare, spot: null };
  const pressable = (node) => !node || Boolean(nearest(node, CONTROL))
    || getComputedStyle(node).cursor === 'pointer' || within(node, target);
  const prior = [hit.style.getPropertyValue('pointer-events'), hit.style.getPropertyPriority('pointer-events')];
  for (let row = 1; row < grid; row += 1) {
    for (let column = 1; column < grid; column += 1) {
      const at = [innerWidth * column / grid, innerHeight * row / grid];
      if (topAt(...at) !== hit) continue;
      hit.style.setProperty('pointer-events', 'none', 'important');
      let beneath;
      try {
        beneath = topAt(...at);
      } finally {
        hit.style.setProperty('pointer-events', prior[0], prior[1]);
      }
      if (!pressable(beneath)) return { said, bare, spot: at };
    }
  }
  return { said, bare, spot: null };
})"#;

/// Cells a side of the grid [`COVER_JS`] looks for a spot to press an
/// empty layer on: its inner corners, 49 points evenly over the window.
pub(crate) const SPOT_GRID: u32 = 8;

/// How long an empty layer just pressed is given to go away before the
/// element under it is pressed again.
const LAYER_GONE_MS: u64 = 150;

impl BrowserSurface {
    /// The middle of the element `reference` names, in the window's
    /// coordinates, as the browser measures its box.
    pub(super) fn middle(&self, id: &SessionId, reference: &str) -> Option<(f64, f64)> {
        let bounds = self
            .block(self.browser.command(
                id,
                json!({"action": "boundingbox", "selector": sight::selector(reference)}),
            ))
            .ok()?;
        let middle = |start: &str, size: &str| {
            Some(bounds.get(start)?.as_f64()? + bounds.get(size)?.as_f64()? / 2.0)
        };
        Some((middle("x", "width")?, middle("y", "height")?))
    }

    /// What lies over the middle of the element `reference` names, by
    /// [`COVER_JS`] (with a spot to press it on when `press`): `Ok(None)`
    /// when nothing does, and `Err` saying why when it cannot be read.
    fn cover(&self, reference: &str, press: bool) -> Result<Option<Value>, String> {
        let id = self.ensure_session().map_err(|error| error.to_string())?;
        let (x, y) = self
            .middle(&id, reference)
            .ok_or_else(|| "its box could not be read".to_owned())?;
        // A ref sight minted is its element exactly, by its mark.
        let exact = if sight::is_seen(reference) {
            serde_json::to_string(&sight::selector(reference)).map_err(|error| error.to_string())?
        } else {
            "null".to_owned()
        };
        let script = format!("{COVER_JS}({x}, {y}, {exact}, {press}, {SPOT_GRID})");
        let data = self
            .block(
                self.browser
                    .command(&id, json!({"action": "evaluate", "script": script})),
            )
            .map_err(|error| error.to_string())?;
        match data.get("result") {
            Some(cover) if cover.get("unread") == Some(&Value::Bool(true)) => {
                Err("nothing shows at its middle".to_owned())
            }
            Some(cover) if cover.is_object() => Ok(Some(cover.clone())),
            _ => Ok(None),
        }
    }

    /// `reply`, a press refused because something covers its target, with
    /// what covers it said in its error's `details.cover`, so the flow can
    /// say what is in the way rather than "something", and whether that is
    /// an empty layer [`BrowserSurface::press_cover`] may press in
    /// `details.empty_layer`.
    pub(super) fn name_cover(
        &self,
        reference: &str,
        mut reply: DesktopResponse,
    ) -> DesktopResponse {
        let Ok(Some(cover)) = self.cover(reference, false) else {
            return reply;
        };
        let Some(said) = cover.get("said").and_then(Value::as_str) else {
            return reply;
        };
        if let Some(error) = reply.error.as_mut() {
            let mut details = error
                .details
                .take()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            details["cover"] = Value::String(said.to_owned());
            details["empty_layer"] = Value::Bool(cover.get("bare") == Some(&Value::Bool(true)));
            error.details = Some(details);
        }
        reply
    }

    /// Presses the empty layer over the element `reference` names where
    /// nothing pressable lies beneath it ([`COVER_JS`]), as a person clicks
    /// outside a popup to close it, then gives the layer a moment to go.
    /// Refused (`NOT_ACTIONABLE`, the cover in `details.cover`) when what
    /// covers the element is no empty layer, has no such spot, or cannot be
    /// read; `dismissed` is `null` when nothing covers the element now.
    pub(super) fn press_cover(&self, reference: &str) -> DesktopResponse {
        const COMMAND: &str = "dismiss-cover";
        if reference.is_empty() {
            return DesktopResponse::err(
                COMMAND,
                DesktopError::new("INVALID_TARGET", "the operation needs a target"),
            );
        }
        let cover = match self.cover(reference, true) {
            Ok(Some(cover)) => cover,
            Ok(None) => return DesktopResponse::ok(COMMAND, json!({"dismissed": null})),
            Err(why) => {
                return DesktopResponse::err(
                    COMMAND,
                    DesktopError::new(
                        "NOT_ACTIONABLE",
                        format!(
                            "what lies over the element could not be read ({why}), so nothing was pressed"
                        ),
                    ),
                );
            }
        };
        let said = cover
            .get("said")
            .and_then(Value::as_str)
            .unwrap_or("something")
            .to_owned();
        let spot = cover
            .get("spot")
            .and_then(Value::as_array)
            .and_then(|at| Some((at.first()?.as_f64()?, at.get(1)?.as_f64()?)));
        let pressed = spot.and_then(|(x, y)| {
            let id = self.ensure_session().ok()?;
            self.show_cursor_at(x, y);
            self.press_point(&id, x, y)
        });
        if pressed.is_none() {
            let why = if spot.is_some() {
                "the press on it did not go through"
            } else if cover.get("bare") == Some(&Value::Bool(true)) {
                "every spot on it lies over something pressable"
            } else {
                "it is no empty layer"
            };
            let mut error = DesktopError::new(
                "NOT_ACTIONABLE",
                format!("{said} lies over the element, and {why}, so it was not pressed"),
            );
            error.details = Some(json!({"cover": said}));
            return DesktopResponse::err(COMMAND, error);
        }
        // Surface calls block by contract, so a pause here is a plain sleep.
        std::thread::sleep(Duration::from_millis(LAYER_GONE_MS));
        DesktopResponse::ok(COMMAND, json!({"dismissed": said}))
    }

    /// Presses the window at (`x`, `y`) as a person's pointer does, with
    /// raw mouse events: whatever is on top there takes it.
    pub(super) fn press_point(&self, id: &SessionId, x: f64, y: f64) -> Option<()> {
        for event in ["mouseMoved", "mousePressed", "mouseReleased"] {
            let pressed = event != "mouseMoved";
            self.block(self.browser.command(
                id,
                json!({
                    "action": "mouse",
                    "eventType": event,
                    "x": x,
                    "y": y,
                    "button": if pressed { "left" } else { "none" },
                    "clickCount": i32::from(pressed),
                }),
            ))
            .ok()?;
        }
        Some(())
    }
}
