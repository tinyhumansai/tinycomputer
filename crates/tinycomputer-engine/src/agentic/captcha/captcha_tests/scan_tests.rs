//! Reading the scan script's result.

use serde_json::json;

use super::*;
use crate::agentic::captcha::scan::SCAN_SCRIPT;

#[test]
fn a_scan_reads_widgets_their_boxes_and_a_solved_token() {
    let value = json!({
        "widgets": [
            {"vendor": "recaptcha", "role": "checkbox", "visible": true,
             "rect": {"x": 20, "y": 100, "width": 304, "height": 78}},
            {"vendor": "hcaptcha", "role": "challenge", "visible": false,
             "rect": {"x": 0, "y": -9999, "width": 400, "height": 600}},
            {"vendor": "turnstile", "role": "checkbox", "visible": true,
             "rect": {"x": 5, "y": 5, "width": 300, "height": 65}}
        ],
        "solved": false,
        "viewport": {"width": 1280, "height": 800}
    });
    let scan = CaptchaScan::parse(&value).unwrap();
    assert_eq!(scan.widgets.len(), 3);
    assert!(
        scan.visible(CaptchaRole::Challenge).is_none(),
        "hidden is not shown"
    );
    let checkbox = scan.visible(CaptchaRole::Checkbox).unwrap();
    assert_eq!(checkbox.vendor, CaptchaVendor::Recaptcha);
    assert!((checkbox.rect.width - 304.0).abs() < f64::EPSILON);
    assert!(!scan.solved);
}

#[test]
fn anything_but_a_scan_with_a_viewport_is_refused() {
    assert!(CaptchaScan::parse(&json!(42)).is_none());
    assert!(CaptchaScan::parse(&json!({"widgets": [], "solved": true})).is_none());
    assert!(
        CaptchaScan::parse(&json!({
            "widgets": [], "solved": true, "viewport": {"width": 0, "height": 800}
        }))
        .is_none()
    );
    assert!(
        CaptchaScan::parse(&json!({
            "widgets": [{"vendor": "someone", "role": "checkbox", "visible": true,
                         "rect": {"x": 0, "y": 0, "width": 1, "height": 1}}],
            "solved": false, "viewport": {"width": 10, "height": 10}
        }))
        .is_none()
    );
}

#[test]
fn the_scan_script_looks_for_every_known_widget_and_token() {
    for needle in [
        "/recaptcha/",
        "/anchor",
        "/bframe",
        "frame=checkbox",
        "frame=challenge",
        "challenges.cloudflare.com",
        "g-recaptcha-response",
        "h-captcha-response",
        "cf-turnstile-response",
    ] {
        assert!(SCAN_SCRIPT.contains(needle), "{needle}");
    }
}
