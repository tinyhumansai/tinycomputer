//! Redaction tests for browser task observations.

use std::sync::Arc;

use serde_json::json;
use tinycomputer_browser::{Browser, BrowserSurface, SessionOptions};
use tinycomputer_bus::{DesktopError, DesktopResponse, JevOperation};
use tinycomputer_core::surface::{Candidate, Depth, Screen, Surface, describe};

use super::browser_tests::BrowserScript;
use super::{redact_observation_error, redact_response, redact_screen};
use crate::{SecretHost, SecretTypeRequest};

struct Host;

impl SecretHost for Host {
    fn type_secret(&self, _request: &SecretTypeRequest) -> Result<(), String> {
        Ok(())
    }

    fn redact(&self, _task_id: &str, text: &str) -> String {
        text.replace("private-canary", "[hidden]")
            .replace("123456789", "[hidden]")
    }
}

#[test]
fn jev_screen_context_and_candidate_labels_are_redacted_before_model_use() {
    let host: Arc<dyn SecretHost> = Arc::new(Host);
    let mut screen = Screen {
        app: "browser".into(),
        window: Some("private-canary page".into()),
        surface: "window".into(),
        candidates: vec![
            Candidate {
                ref_id: "e1".into(),
                role: "button".into(),
                name: Some("private-canary action".into()),
                value: Some(json!({"private-canary": 123_456_789, "[hidden]": "private-canary"})),
                path: vec!["private-canary parent".into()],
                states: vec!["private-canary selected".into()],
                available_actions: vec!["Click".into(), "private-canary action".into()],
                bounds: Some(json!({"x": 10, "y": 2})),
                ..Candidate::default()
            },
            Candidate {
                ref_id: "private-canary-ref".into(),
                role: "button".into(),
                name: Some("Unsafe ref".into()),
                available_actions: vec!["Click".into()],
                ..Candidate::default()
            },
        ],
        context: vec!["Restored private-canary value".into()],
        unexplored: vec!["private-canary-ref".into(), "e2".into()],
        text_nodes: Vec::new(),
    };

    redact_screen(host.as_ref(), "task-1", &mut screen);

    let candidate = &screen.candidates[0];
    assert_eq!(candidate.name.as_deref(), Some("[hidden] action"));
    assert_eq!(candidate.path, ["[hidden] parent"]);
    assert_eq!(candidate.states, Vec::<String>::new());
    assert!(candidate.value.as_ref().is_some_and(|value| {
        value["[hidden]"] == "[hidden]" && value["[hidden]#2"] == "[hidden]"
    }));
    assert!(
        candidate
            .bounds
            .as_ref()
            .is_some_and(|bounds| bounds["x"] == 10)
    );
    assert_eq!(candidate.available_actions, ["Click"]);
    assert!(
        !screen
            .candidates
            .iter()
            .any(|node| node.ref_id.contains("private-canary"))
    );
    assert_eq!(screen.unexplored, ["e2"]);
    assert_eq!(screen.context[0], "Restored [hidden] value");
    assert_eq!(screen.candidates[0].ref_id, "e1");
}

#[test]
fn observation_errors_are_redacted_before_they_are_returned() {
    let host = Host;
    let error = Box::new(DesktopResponse::err(
        "observe",
        DesktopError::new("private-canary-code", "page contained private-canary"),
    ));

    let safe = redact_observation_error(&host, "task-1", error);
    assert!(safe.error.as_ref().is_some_and(|error| {
        error.code == "REDACTION_FAILED"
            && error.message == "the browser response could not be safely redacted"
    }));
}

#[test]
fn response_redaction_covers_numeric_values_and_object_keys() {
    let response = DesktopResponse::ok(
        "snapshot",
        json!({
            "metadata": {"private-canary": "123456789", "[hidden]": 123_456_789},
            "message": "private-canary",
        }),
    );

    let safe = redact_response(&Host, "task-1", response);
    assert!(safe.data.as_ref().is_some_and(|data| {
        data["metadata"]["[hidden]"] == "[hidden]"
            && data["metadata"]["[hidden]#2"] == "[hidden]"
            && data["message"] == "[hidden]"
    }));
}

#[test]
fn response_with_secret_in_an_operational_ref_fails_closed() {
    let response = DesktopResponse::ok(
        "snapshot",
        json!({"target": {"ref_id": "private-canary-ref"}}),
    );

    let safe = redact_response(&Host, "task-1", response);

    assert!(!safe.ok);
    assert!(
        safe.error
            .as_ref()
            .is_some_and(|error| error.code == "REDACTION_FAILED")
    );
}

#[test]
fn secret_shaped_bounds_are_dropped_before_model_description() {
    let host: Arc<dyn SecretHost> = Arc::new(Host);
    let mut screen = Screen {
        app: "browser".into(),
        window: None,
        surface: "window".into(),
        candidates: vec![
            Candidate {
                ref_id: "e1".into(),
                role: "button".into(),
                bounds: Some(json!({"x": 20, "y": 30, "width": 100, "height": 40})),
                ..Candidate::default()
            },
            Candidate {
                ref_id: "e2".into(),
                role: "group".into(),
                bounds: Some(json!({"label": "private-canary", "offset": 123_456_789})),
                ..Candidate::default()
            },
        ],
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    };

    redact_screen(host.as_ref(), "task-1", &mut screen);

    assert_eq!(
        screen.candidates[0].bounds,
        Some(json!({"x": 20, "y": 30, "width": 100, "height": 40}))
    );
    assert!(screen.candidates[1].bounds.is_none());
    let model_description =
        serde_json::to_string(&describe(&screen.candidates[1], true)).unwrap_or_default();
    assert!(!model_description.contains("private-canary"));
    assert!(!model_description.contains("123456789"));
}

#[test]
fn secret_redacting_surface_wraps_browser_surface_operations()
-> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    let browser = Arc::new(Browser::new(Arc::new(BrowserScript::default())));
    let inner = BrowserSurface::new(browser, SessionOptions::default(), runtime.handle().clone());
    let surface = super::SecretRedactingSurface {
        inner,
        host: Arc::new(Host),
        task_id: "task-1".into(),
    };
    let target = Candidate::default();

    assert!(surface.observe("browser", None, Depth::Full).is_ok());
    let _ = surface.execute(JevOperation::Done, Some(target.clone()), None);
    let _ = surface.authorize_secret_type(&target);
    let _ = surface.read_value(&target);
    let _ = surface.paste("browser", &target, "private-canary");
    let _ = surface.press("browser", "ENTER");
    let _ = surface.launch("browser");
    surface.settle();
    surface.settle_briefly();
    let _ = surface.await_change(0);
    let _ = surface.navigate("https://shop.example/");
    let _ = surface.back("browser");
    let _ = surface.dismiss_cover(&target);
    Ok(())
}
