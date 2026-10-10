//! The agent's cursor over a browser window.
//!
//! There is one cursor for the whole screen ([`ScreenCursor`]), shared with
//! the desktop surface, so it glides from an application into a web page and
//! back without jumping. Before the surface acts on an element, the element's
//! box is converted from viewport pixels to screen points and the cursor
//! glides onto it; the engine performs the action as the cursor lands, in
//! time with its pulse, exactly as it would with no cursor on screen.
//! Nothing is added to the page.
//!
//! Only a visible session — headed, or attached to an existing browser — has
//! a window on screen to draw over. The conversion assumes 100% page zoom and
//! a window whose borders are split evenly left and right, with the browser's
//! own toolbars above the page; a cursor a few points off is still a cursor
//! in the right place to a person watching.

use serde_json::{Value, json};
use tinycomputer_cursor::{Rect, ScreenCursor};

use super::BrowserSurface;

/// Reads where the page's viewport is on screen.
const VIEWPORT_JS: &str = "[window.screenX, window.screenY, window.outerWidth, \
                           window.outerHeight, window.innerWidth, window.innerHeight]";

/// Where the viewport's top-left corner is on screen, from the window's
/// position and outer and inner sizes.
pub(super) fn viewport_origin(window: &[f64]) -> Option<(f64, f64)> {
    let [
        screen_x,
        screen_y,
        outer_width,
        outer_height,
        inner_width,
        inner_height,
    ] = *window
    else {
        return None;
    };
    let border = ((outer_width - inner_width) / 2.0).max(0.0);
    let chrome = (outer_height - inner_height - border).max(0.0);
    Some((screen_x + border, screen_y + chrome))
}

impl BrowserSurface {
    /// The cursor this surface draws on.
    #[must_use]
    pub fn cursor(&self) -> &ScreenCursor {
        &self.cursor
    }

    /// Whether this session has a window on screen for the cursor to be
    /// seen over.
    pub(super) fn shows_cursor(&self) -> bool {
        !self.cursor.pace().is_off() && (!self.options.headless || self.options.endpoint.is_some())
    }

    fn command(&self, command: Value) -> Option<Value> {
        let id = self.ensure_session().ok()?;
        self.block(self.browser.command(&id, command)).ok()
    }

    /// Where the page's viewport begins on screen, in points.
    fn viewport_on_screen(&self) -> Option<(f64, f64)> {
        let window = self.command(json!({"action": "evaluate", "script": VIEWPORT_JS}))?;
        let window: Vec<f64> = window
            .get("result")?
            .as_array()?
            .iter()
            .filter_map(Value::as_f64)
            .collect();
        viewport_origin(&window)
    }

    /// `reference`'s box on screen, in points.
    fn screen_bounds(&self, reference: &str) -> Option<Rect> {
        let selector = super::sight::selector(reference);
        let data = self.command(json!({"action": "boundingbox", "selector": selector}))?;
        let field = |name: &str| data.get(name).and_then(Value::as_f64);
        let (left, top) = self.viewport_on_screen()?;
        let rect = Rect::new(
            left + field("x")?,
            top + field("y")?,
            field("width")?,
            field("height")?,
        );
        (rect.is_valid() && rect.width > 0.0 && rect.height > 0.0).then_some(rect)
    }

    /// Glides the cursor onto `reference` and returns as it lands. Does
    /// nothing when no one can see the page or the element has no box; the
    /// action proceeds regardless.
    pub(super) fn show_cursor(&self, reference: &str) {
        if !self.shows_cursor() {
            return;
        }
        if let Some(target) = self.screen_bounds(reference) {
            self.cursor.arrive(target);
        }
    }

    /// Glides the cursor onto the window's point (`x`, `y`) and returns as
    /// it lands, as [`BrowserSurface::show_cursor`] does onto an element:
    /// before a press aimed at a point, such as one outside a popup.
    pub(super) fn show_cursor_at(&self, x: f64, y: f64) {
        if !self.shows_cursor() {
            return;
        }
        if let Some((left, top)) = self.viewport_on_screen() {
            let spot = Rect::new(left + x - 1.0, top + y - 1.0, 2.0, 2.0);
            if spot.is_valid() {
                self.cursor.arrive(spot);
            }
        }
    }
}
