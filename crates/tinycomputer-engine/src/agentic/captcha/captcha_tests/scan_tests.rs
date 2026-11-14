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

/// A just-enough DOM to run the scan script under `node`: a tree of boxes
/// where an iframe is a widget frame and an input is a response field.
const FAKE_DOM: &str = r#"
const make = (tag, props = {}, kids = []) => {
  const el = { tag, ...props, parentElement: null, children: kids, scrolled: false,
    getBoundingClientRect() { return this.rect || { top: 0, left: 0, bottom: 80, right: 300, width: 300, height: 80 }; },
    scrollIntoView() { this.scrolled = true; },
    querySelectorAll(sel) { return all(this).filter((e) => matches(e, sel)); } };
  kids.forEach((k) => { k.parentElement = el; });
  return el;
};
const all = (root) => root.children.flatMap((c) => [c, ...all(c)]);
const matches = (e, sel) => sel.split(/,(?![^\[]*\])/).some((part) => {
  const attrs = [...part.matchAll(/\[(\w+)([*]?)="([^"]*)"\]/g)];
  const tag = part.match(/^[a-z]+/);
  if (tag && tag[0] !== e.tag) return false;
  return attrs.length > 0 && attrs.every(([, k, star, v]) => {
    const have = k === "src" ? e.src : e.name;
    return star ? (have || "").includes(v) : have === v;
  });
});
const frame = (extra = {}) => make("iframe", { src: "https://www.google.com/recaptcha/api2/anchor?k=1", ...extra });
const field = (value) => make("textarea", { name: "g-recaptcha-response", value });
const OFF = { top: 2000, left: 0, bottom: 2080, right: 300, width: 300, height: 80 };
const run = (body) => {
  const root = make("html", {}, body);
  global.window = { innerWidth: 1280, innerHeight: 800 };
  global.getComputedStyle = () => ({ visibility: "visible", display: "block", opacity: "1" });
  global.document = { querySelectorAll: (s) => root.querySelectorAll(s),
    getElementsByName: (n) => all(root).filter((e) => e.name === n) };
  const result = eval(process.argv[1]);
  return { result, all: all(root) };
};
"#;

/// Runs the scan script over a fake page and returns `(scan, scrolled frame
/// indexes)`; `None` when `node` is not installed.
fn run_scan(page: &str) -> Option<(serde_json::Value, Vec<usize>)> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let program = format!(
        "{FAKE_DOM}\nconst out = run({page});\n\
         const frames = out.all.filter((e) => e.tag === 'iframe');\n\
         console.log(JSON.stringify({{ scan: out.result, scrolled: frames.flatMap((f, i) => f.scrolled ? [i] : []) }}));"
    );
    let mut child = Command::new("node")
        .args([
            "-e",
            "process.argv[1] = require('fs').readFileSync(0, 'utf8'); eval(process.argv[2])",
            "x",
        ])
        .arg(&program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(SCAN_SCRIPT.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    assert!(output.status.success(), "node failed");
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let scrolled = parsed["scrolled"]
        .as_array()?
        .iter()
        .filter_map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok()))
        .collect();
    Some((parsed["scan"].clone(), scrolled))
}

#[test]
fn the_scan_scrolls_to_the_first_unanswered_checkbox_not_the_first_shown_one() {
    // An answered checkbox in view, then an unanswered one far below.
    let page = "[make('div', {}, [frame(), field('token')]), \
                make('div', {}, [frame({ rect: OFF }), field('')])]";
    let Some((scan, scrolled)) = run_scan(page) else {
        return;
    };
    assert_eq!(
        scrolled,
        [1],
        "the waiting widget is scrolled to, not the answered one"
    );
    assert_eq!(scan["widgets"][0]["answered"], true);
    assert_eq!(scan["widgets"][1]["answered"], false);
    assert_eq!(scan["solved"], false);
}

#[test]
fn a_response_field_shared_by_two_checkboxes_does_not_answer_either() {
    let page = "[make('div', {}, [frame(), frame(), field('token')])]";
    let Some((scan, _)) = run_scan(page) else {
        return;
    };
    assert_eq!(
        scan["widgets"][0]["answered"], false,
        "ownership is ambiguous"
    );
    assert_eq!(scan["widgets"][1]["answered"], false);
    assert_eq!(scan["solved"], false);
}

#[test]
fn a_lone_checkbox_beside_a_filled_field_is_answered() {
    let page = "[make('div', {}, [frame(), field('token')])]";
    let Some((scan, _)) = run_scan(page) else {
        return;
    };
    assert_eq!(scan["widgets"][0]["answered"], true);
    assert_eq!(scan["solved"], true);
}
