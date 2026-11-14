//! Getting a task past a captcha with Jev's image questions, before a person
//! is asked to.
//!
//! When a failed step leaves a page whose text asks to "solve the captcha" or
//! "prove you are human" (`tinycomputer_core::human_needed`), the task asks
//! its runner to try this solver first ([`crate::FlowRunner::solve_captcha`]).
//! It knows the checkbox widgets of reCAPTCHA, hCaptcha and Cloudflare
//! Turnstile, and the image-grid challenges reCAPTCHA and hCaptcha put up
//! behind them:
//!
//! - `scan` finds those widgets' frames in the page and whether the page
//!   already holds a solved token;
//! - `pixels` cuts the widget out of a viewport screenshot and scales it
//!   into the sizes Jev reads (280 to 1568 pixels a side);
//! - `solve` runs the bounded loop: press the checkbox (a Jev Box question
//!   finds it), and for an image grid ask Jev where the grid and the verify
//!   button are, then one yes/no question per tile, press the tiles Jev is
//!   sure of, and verify;
//! - `page` is [`BrowserCaptchaPage`], the loop's eyes and hand on a real
//!   browser session.
//!
//! Every Jev call goes through [`JevRuntime::evaluate`](super::JevRuntime),
//! so it is budgeted and journaled like any other. Jev decides; it cannot
//! write, so a challenge that asks for typed characters is
//! [`CaptchaOutcome::Unsupported`] at once and stays with a person. So does
//! anything still standing after [`CaptchaSettings::max_attempts`].

mod page;
mod pixels;
mod scan;
mod solve;

#[cfg(test)]
#[path = "captcha_tests.rs"]
mod captcha_tests;

use std::future::Future;
use std::pin::Pin;

pub use page::BrowserCaptchaPage;
pub use scan::{CaptchaRole, CaptchaScan, CaptchaVendor, CaptchaWidget, Rect, Viewport};
pub use solve::solve_captcha;

/// The default number of challenges the solver submits before handing the
/// page to a person.
pub const DEFAULT_CAPTCHA_ATTEMPTS: u32 = 3;

/// The most challenges any configuration may let the solver submit.
pub const MAX_CAPTCHA_ATTEMPTS: u32 = 10;

/// How hard the solver tries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptchaSettings {
    /// Image challenges submitted before giving up, 1 to
    /// [`MAX_CAPTCHA_ATTEMPTS`]. Pressing the checkbox is not counted.
    pub max_attempts: u32,
}

impl Default for CaptchaSettings {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_CAPTCHA_ATTEMPTS,
        }
    }
}

impl CaptchaSettings {
    /// Settings submitting at most `max_attempts` challenges, clamped to 1 to
    /// [`MAX_CAPTCHA_ATTEMPTS`].
    #[must_use]
    pub fn with_attempts(max_attempts: u32) -> Self {
        Self {
            max_attempts: max_attempts.clamp(1, MAX_CAPTCHA_ATTEMPTS),
        }
    }
}

/// What trying a captcha came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptchaOutcome {
    /// The runner has no solver: none configured, no browser on the task,
    /// or a page whose pixels may show secrets.
    Unavailable,
    /// The page took an answer: it holds a solved token, or the widget is
    /// gone after the solver acted.
    Solved {
        /// Image challenges submitted.
        attempts: u32,
    },
    /// The page shows nothing this solver can answer, such as a challenge
    /// asking for typed characters.
    Unsupported {
        /// What was found instead.
        reason: String,
    },
    /// The solver tried and the captcha still stands.
    Unsolved {
        /// Image challenges submitted.
        attempts: u32,
        /// Why it stopped.
        reason: String,
    },
}

/// A boxed future, for the object-safe [`CaptchaPage`].
pub type CaptchaPageFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the solver sees and does on a page.
///
/// [`BrowserCaptchaPage`] is the real one; tests stand in a scripted page.
pub trait CaptchaPage: Send + Sync {
    /// The captcha widgets on the page now, or `None` when it cannot be read.
    fn scan(&self) -> CaptchaPageFuture<'_, Option<CaptchaScan>>;

    /// A PNG screenshot of the viewport, or `None` when one cannot be taken.
    fn shoot(&self) -> CaptchaPageFuture<'_, Option<Vec<u8>>>;

    /// Presses the viewport point (`x`, `y`), in CSS pixels; `false` when the
    /// press was refused or failed.
    fn press(&self, x: f64, y: f64) -> CaptchaPageFuture<'_, bool>;

    /// Waits `ms` milliseconds for the page to react.
    fn pause(&self, ms: u64) -> CaptchaPageFuture<'_, ()>;
}
