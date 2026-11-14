//! Finding captcha widgets in a page, from the top document.
//!
//! The widgets live in cross-origin frames the page cannot look into, so
//! the scan reads only what the top document knows: each widget frame's
//! address and box, and the hidden response field the vendor fills with a
//! token once the captcha is passed.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The script the scan evaluates in the top document. It scrolls a widget
/// that is off screen into view, only when no challenge is showing, and
/// otherwise changes nothing.
pub(super) const SCAN_SCRIPT: &str = r#"(() => {
  const kinds = [
    ["recaptcha", "checkbox", 'iframe[src*="/recaptcha/"][src*="/anchor"]'],
    ["recaptcha", "challenge", 'iframe[src*="/recaptcha/"][src*="/bframe"]'],
    ["hcaptcha", "checkbox", 'iframe[src*="hcaptcha"][src*="frame=checkbox"]'],
    ["hcaptcha", "challenge", 'iframe[src*="hcaptcha"][src*="frame=challenge"]'],
    ["turnstile", "checkbox", 'iframe[src*="challenges.cloudflare.com"]'],
  ];
  const vw = window.innerWidth, vh = window.innerHeight;
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return r.width > 20 && r.height > 20 && style.visibility !== "hidden"
      && style.display !== "none" && Number(style.opacity || 1) > 0.1;
  };
  const inView = (r) => r.top >= 0 && r.left >= 0 && r.bottom <= vh && r.right <= vw;
  const found = [];
  for (const [vendor, role, selector] of kinds) {
    for (const el of document.querySelectorAll(selector)) found.push({ vendor, role, el });
  }
  const challenge = found.some((w) => w.role === "challenge" && shown(w.el)
    && inView(w.el.getBoundingClientRect()));
  if (!challenge) {
    const box = found.find((w) => w.role === "checkbox" && shown(w.el));
    if (box && !inView(box.el.getBoundingClientRect())) box.el.scrollIntoView({ block: "center" });
  }
  const widgets = found.map(({ vendor, role, el }) => {
    const r = el.getBoundingClientRect();
    return { vendor, role, visible: shown(el) && inView(r),
      rect: { x: r.left, y: r.top, width: r.width, height: r.height } };
  });
  const names = ["g-recaptcha-response", "h-captcha-response", "cf-turnstile-response"];
  const solved = names.some((name) => Array.from(document.getElementsByName(name))
    .some((field) => typeof field.value === "string" && field.value.length > 0));
  return { widgets, solved, viewport: { width: vw, height: vh } };
})()"#;

/// Whose captcha a widget is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptchaVendor {
    /// Google reCAPTCHA v2.
    Recaptcha,
    /// hCaptcha.
    Hcaptcha,
    /// Cloudflare Turnstile, which has a checkbox and no image challenge.
    Turnstile,
}

/// Which part of a captcha a widget frame holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptchaRole {
    /// The "I'm not a robot" checkbox.
    Checkbox,
    /// The image challenge that may open after it.
    Challenge,
}

/// A box in viewport CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// One captcha widget frame in the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptchaWidget {
    /// Whose widget it is.
    pub vendor: CaptchaVendor,
    /// Which part it is.
    pub role: CaptchaRole,
    /// Whether it is shown, whole, inside the viewport.
    pub visible: bool,
    /// Where it is.
    pub rect: Rect,
}

/// The size of the viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// What a scan of the page found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptchaScan {
    /// Every widget frame, shown or not.
    pub widgets: Vec<CaptchaWidget>,
    /// Whether a vendor's response field already holds a token.
    pub solved: bool,
    /// The viewport the boxes are measured in.
    pub viewport: Viewport,
}

impl CaptchaScan {
    /// Reads the scan script's result; `None` when it is not one.
    pub(super) fn parse(value: &Value) -> Option<Self> {
        let scan: Self = serde_json::from_value(value.clone()).ok()?;
        (scan.viewport.width > 0.0 && scan.viewport.height > 0.0).then_some(scan)
    }

    /// The first shown widget playing `role`.
    #[must_use]
    pub fn visible(&self, role: CaptchaRole) -> Option<&CaptchaWidget> {
        self.widgets
            .iter()
            .find(|widget| widget.role == role && widget.visible)
    }
}
