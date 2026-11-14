//! The solver's eyes and hand on a real browser session.

use std::sync::Arc;
use std::time::Duration;

use tinycomputer_browser::Browser;
use tinycomputer_bus::browser::{EvaluateRequest, ScreenshotRequest, SessionId};

use super::scan::{CaptchaScan, SCAN_SCRIPT};
use super::{CaptchaPage, CaptchaPageFuture};

/// How long the scan script may run, in milliseconds.
const SCAN_TIMEOUT_MS: u64 = 5_000;

/// A [`CaptchaPage`] over one session of a [`Browser`].
///
/// Scans evaluate a fixed script in the top document, screenshots are of the
/// viewport, and presses are raw pointer events at a point
/// ([`Browser::press_point`]); all of them pass the session's action gate
/// and allowed origins like any other call.
#[derive(Debug, Clone)]
pub struct BrowserCaptchaPage {
    browser: Arc<Browser>,
    session: SessionId,
}

impl BrowserCaptchaPage {
    /// The page `session` of `browser` shows.
    #[must_use]
    pub fn new(browser: Arc<Browser>, session: SessionId) -> Self {
        Self { browser, session }
    }
}

impl CaptchaPage for BrowserCaptchaPage {
    fn scan(&self) -> CaptchaPageFuture<'_, Option<CaptchaScan>> {
        Box::pin(async move {
            let request = EvaluateRequest {
                expression: SCAN_SCRIPT.to_owned(),
                await_promise: false,
                timeout_ms: Some(SCAN_TIMEOUT_MS),
            };
            let value = self.browser.evaluate(&self.session, request).await.ok()?;
            CaptchaScan::parse(&value)
        })
    }

    fn shoot(&self) -> CaptchaPageFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move {
            self.browser
                .screenshot_bytes(&self.session, ScreenshotRequest::default())
                .await
                .ok()
        })
    }

    fn press(&self, x: f64, y: f64) -> CaptchaPageFuture<'_, bool> {
        Box::pin(async move { self.browser.press_point(&self.session, x, y).await.is_ok() })
    }

    fn pause(&self, ms: u64) -> CaptchaPageFuture<'_, ()> {
        Box::pin(tokio::time::sleep(Duration::from_millis(ms)))
    }
}
