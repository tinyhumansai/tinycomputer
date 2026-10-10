//! Tests for the launch a session starts with.

use serde_json::json;
use tinycomputer_bus::browser::{SessionOptions, Viewport};

use crate::convert::{launch, viewport};

#[test]
fn a_default_session_launches_headless_with_nothing_else() {
    assert_eq!(
        launch(&SessionOptions::default()),
        json!({"action": "launch", "headless": true, "args": []})
    );
}

#[test]
fn every_session_option_reaches_its_launch_field() {
    let options = SessionOptions {
        endpoint: Some("ws://127.0.0.1:9222".to_owned()),
        executable: Some("/usr/bin/chromium".to_owned()),
        headless: false,
        user_agent: Some("agent".to_owned()),
        user_data_dir: Some("/profiles/trip".to_owned()),
        download_dir: Some("/downloads".to_owned()),
        args: vec!["--lang=en".to_owned()],
        allowed_origins: vec!["https://example.com".to_owned()],
        viewport: Viewport::desktop(800, 600),
        ..SessionOptions::default()
    };
    assert_eq!(
        launch(&options),
        json!({
            "action": "launch",
            "headless": false,
            "args": ["--lang=en"],
            "cdpUrl": "ws://127.0.0.1:9222",
            "executablePath": "/usr/bin/chromium",
            "userAgent": "agent",
            "profile": "/profiles/trip",
            "downloadPath": "/downloads",
        }),
        "the allowed origins stay with the session, which checks pages itself"
    );
    assert_eq!(
        viewport(&options),
        json!({"action": "viewport", "width": 800, "height": 600, "deviceScaleFactor": 1.0, "mobile": false})
    );
}
