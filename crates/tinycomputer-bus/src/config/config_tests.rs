//! Tests for the public module configuration's wire shape and secret handling.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::json;

use super::{BrowserConfig, BrowserPerception, BrowserSettle, ComputerConfig};

#[test]
fn computer_config_deserializes_the_host_fields_and_module_defaults() {
    let config: ComputerConfig = serde_json::from_value(json!({
        "jev": {"api_key": "th-secret", "provider": "tiny_humans_open_router"},
        "planner": {
            "api_key": "or-secret",
            "provider": "open_router",
            "model": "anthropic/claude-sonnet-5",
            "rescue_route": {"api_key": "rescue-secret", "provider": "tiny_humans"}
        },
        "browser": {"executable": "/usr/bin/chromium"},
        "trace_path": "/workspace/state/computer/desktop-trace.jsonl",
        "session_id": "legacy-module-option"
    }))
    .expect("valid host and module-only fields decode");

    assert_eq!(config.jev.as_ref().unwrap().api_key(), "th-secret");
    assert_eq!(config.planner.as_ref().unwrap().route.api_key, "or-secret");
    assert_eq!(
        config.planner.as_ref().unwrap().rescuer_route().api_key,
        "rescue-secret"
    );
    let browser = config.browser.unwrap();
    assert_eq!(browser.executable.as_deref(), Some("/usr/bin/chromium"));
    assert_eq!(browser.perception, BrowserPerception::Sight);
    assert_eq!(browser.settle, BrowserSettle::Prompt);
    assert!(browser.prelaunch);
    assert_eq!(
        config.trace_path.as_deref(),
        Some("/workspace/state/computer/desktop-trace.jsonl")
    );
}

#[test]
fn computer_config_serializes_without_empty_optional_sections() {
    let config = ComputerConfig {
        browser: Some(BrowserConfig {
            executable: Some("/usr/bin/chromium".into()),
            ..BrowserConfig::default()
        }),
        trace_path: Some("/trace.jsonl".into()),
        ..ComputerConfig::default()
    };

    assert_eq!(
        serde_json::to_value(config).unwrap(),
        json!({
            "browser": {"executable": "/usr/bin/chromium"},
            "trace_path": "/trace.jsonl"
        })
    );
}

#[test]
fn browser_config_defaults_and_serializes_non_default_fields() {
    let default = BrowserConfig::default();
    assert_eq!(default.perception, BrowserPerception::Sight);
    assert_eq!(default.settle, BrowserSettle::Prompt);
    assert!(default.prelaunch);

    let configured = BrowserConfig {
        perception: BrowserPerception::Tree,
        settle: BrowserSettle::Steady,
        prelaunch: false,
        ..default
    };
    assert_eq!(
        serde_json::to_value(configured).unwrap(),
        json!({
            "perception": "tree",
            "settle": "steady",
            "prelaunch": false
        })
    );
}

#[test]
fn browser_config_debug_redacts_sensitive_values() {
    let config = BrowserConfig {
        executable: Some("/secret/chromium".into()),
        user_agent: Some("private agent string".into()),
        args: vec!["--private-argument".into()],
        ..BrowserConfig::default()
    };
    let debug = format!("{config:?}");

    assert!(!debug.contains("/secret/chromium"));
    assert!(!debug.contains("private agent string"));
    assert!(!debug.contains("--private-argument"));
    assert!(debug.contains("args_count: 1"));
}

#[test]
fn computer_configuration_debug_redacts_all_model_credentials() {
    let config: ComputerConfig = serde_json::from_value(json!({
        "jev": {"api_key": "jev-secret"},
        "planner": {
            "api_key": "planner-secret",
            "rescue_route": {"api_key": "rescue-secret"}
        },
        "trace_path": "/private/workspace/computer-trace.jsonl"
    }))
    .unwrap();

    let debug = format!("{config:?}");
    for secret in ["jev-secret", "planner-secret", "rescue-secret"] {
        assert!(!debug.contains(secret), "{secret} leaked in {debug}");
    }
    assert!(!debug.contains("/private/workspace/computer-trace.jsonl"));
}

#[test]
fn browser_configuration_rejects_unknown_fields() {
    assert!(serde_json::from_value::<BrowserConfig>(json!({"executabel": "chrome"})).is_err());
}

#[test]
fn computer_config_rejects_null_sections_but_keeps_nullable_trace_path() {
    for field in ["jev", "planner", "browser"] {
        assert!(
            serde_json::from_value::<ComputerConfig>(json!({field: null})).is_err(),
            "explicit null must not silently disable {field}"
        );
    }
    let config: ComputerConfig =
        serde_json::from_value(json!({"trace_path": null})).expect("legacy null trace path");
    assert!(config.trace_path.is_none());
}
