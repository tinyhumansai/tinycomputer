//! Tests for the module configuration: rejection, desktop availability, the
//! planner, the browser settings, and the cursor.

use crate::tinybus_module::DesktopService;
use serde_json::json;
use tinycomputer_bus::DesktopResponse;

#[test]
fn a_malformed_configuration_is_rejected_rather_than_defaulted() {
    let error = DesktopService::from_config(&json!({ "session_id": 7 }))
        .expect_err("a numeric session id is not a session id");

    assert!(error.to_string().contains("session_id"));
}

#[test]
fn the_desktop_is_available_only_with_accessibility() {
    use crate::tinybus_module::dispatch::desktop_availability;

    let reply = |state: serde_json::Value| {
        DesktopResponse::ok("permissions", json!({"accessibility": state}))
    };
    assert!(desktop_availability(&reply(json!({"state": "granted"}))).available);
    assert!(desktop_availability(&reply(json!({"state": "not_required"}))).available);
    let denied = desktop_availability(&reply(
        json!({"state": "denied", "suggestion": "open Settings"}),
    ));
    assert!(!denied.available);
    assert_eq!(denied.reason.as_deref(), Some("open Settings"));
    let bare = desktop_availability(&reply(json!({"state": "denied"})));
    assert_eq!(
        bare.reason.as_deref(),
        Some("grant the accessibility permission")
    );
    let unknown = desktop_availability(&reply(json!({"state": "unknown"})));
    assert!(unknown.reason.unwrap().contains("could not be read"));
    let failed = desktop_availability(&DesktopResponse::err(
        "permissions",
        tinycomputer_bus::DesktopError::new("PLATFORM_NOT_SUPPORTED", "no surfaces here"),
    ));
    assert_eq!(failed.reason.as_deref(), Some("no surfaces here"));
}

#[test]
fn a_planner_is_configured_from_private_configuration_only_with_a_key() {
    let service = DesktopService::from_config(&json!({"planner": {"api_key": "k"}})).unwrap();
    assert!(service.tasks.planner_configured());
    assert!(
        service.tasks.rescue_configured(),
        "the planner's key brings the rescuer"
    );
    assert!(service.tasks.output_configured(), "and the shaper");
    assert!(
        DesktopService::from_config(
            &json!({"planner": {"api_key": "k", "output_model": "openai/gpt-6-luna-pro"}})
        )
        .is_ok()
    );
    assert!(
        DesktopService::from_config(
            &json!({"planner": {"api_key": "k", "rescue_model": "openai/gpt-6-luna-pro"}})
        )
        .is_ok()
    );
    assert!(
        !DesktopService::from_config(&json!({}))
            .unwrap()
            .tasks
            .rescue_configured()
    );
    assert!(
        !DesktopService::from_config(&json!({}))
            .unwrap()
            .tasks
            .output_configured()
    );
    assert!(DesktopService::from_config(&json!({"planner": {"api_key": " "}})).is_err());
    assert!(DesktopService::from_config(&json!({"planner": {"model": "m"}})).is_err());
}

#[test]
fn the_planner_rescuer_and_shaper_route_through_tiny_humans_or_open_router() {
    use tinycomputer_bus::agent::LanguageModelProvider;

    let service = DesktopService::from_config(&json!({"planner": {
        "api_key": "th-bearer",
        "provider": "tiny_humans",
        "endpoint_url": "https://api.tinyhumans.ai/openai/v1",
        "sdk_name": "openhuman",
        "rescue_model": "openai/gpt-6-luna-pro",
        "rescue_route": {"api_key": "sk-or", "provider": "open_router"}
    }}))
    .unwrap();
    let planner = service.tasks.planner_model().unwrap();
    assert_eq!(planner.provider, LanguageModelProvider::TinyHumans);
    assert_eq!(planner.model, "anthropic/claude-sonnet-5");
    let rescue = service.tasks.rescue_model().unwrap();
    assert_eq!(rescue.provider, LanguageModelProvider::OpenRouter);
    assert_eq!(rescue.model, "openai/gpt-6-luna-pro");
    let output = service.tasks.output_model().unwrap();
    assert_eq!(output.provider, LanguageModelProvider::TinyHumans);

    for refused in [
        json!({"planner": {"api_key": "k", "provider": "tiny_humans",
            "endpoint_url": "https://openrouter.ai/api/v1"}}),
        json!({"planner": {"api_key": "k", "endpoint_url": "https://attacker.example/v1"}}),
        json!({"planner": {"api_key": "k", "provider": "anthropic"}}),
        json!({"planner": {"api_key": "k", "rescue_route": {"api_key": ""}}}),
    ] {
        assert!(DesktopService::from_config(&refused).is_err(), "{refused}");
    }
}

#[test]
fn each_decision_model_is_selected_from_private_configuration() {
    use tinycomputer_bus::JevProvider;

    for (jev, provider, model) in [
        (json!({"api_key": "k"}), JevProvider::TypeSafe, "jev-latest"),
        (
            json!({"api_key": "k", "provider": "tiny_humans_open_router", "sdk_name": "openhuman"}),
            JevProvider::TinyHumansOpenRouter,
            "jev-latest",
        ),
        (
            json!({"api_key": "k", "provider": "open_jev"}),
            JevProvider::OpenJev,
            "openjev",
        ),
        (
            json!({"api_key": "k", "provider": "sage", "fast": true}),
            JevProvider::Sage,
            "levanto-sage",
        ),
        (
            json!({"api_key": "k", "provider": "self_hosted",
                "endpoint_url": "https://inference.internal.example/rune-26b/v1/decisions",
                "model": "ci-models-gemma__ci-rune-26b-a4b"}),
            JevProvider::SelfHosted,
            "ci-models-gemma__ci-rune-26b-a4b",
        ),
    ] {
        let service = DesktopService::from_config(&json!({ "jev": jev })).unwrap();
        let configured = service.jev_runtime().unwrap().configuration().clone();
        assert_eq!(configured.provider, provider);
        assert_eq!(configured.model, model);
    }
    for refused in [
        json!({"jev": {"api_key": "k", "provider": "sage",
            "endpoint_url": "https://api.openjev.sh/v1/systemone"}}),
        json!({"jev": {"api_key": "k", "provider": "open_jev",
            "endpoint_url": "https://attacker.example/v1/systemone"}}),
        json!({"jev": {"api_key": "", "provider": "sage"}}),
        json!({"jev": {"api_key": "k", "provider": "levanto"}}),
        json!({"jev": {"api_key": "k", "provider": "self_hosted",
            "endpoint_url": "https://inference.internal.example/v1/decisions"}}),
        json!({"jev": {"api_key": "k", "provider": "self_hosted",
            "model": "ci-models-gemma__ci-rune-26b-a4b"}}),
    ] {
        assert!(DesktopService::from_config(&refused).is_err(), "{refused}");
    }
}

#[test]
fn the_browser_configuration_is_read_or_refused() {
    use crate::tinybus_module::config::BrowserDefaults;
    use tinycomputer_browser::Perception;

    assert_eq!(
        BrowserDefaults::from_config(&json!({})).unwrap(),
        BrowserDefaults::default()
    );
    assert!(DesktopService::from_config(&json!({"browser": {}})).is_ok());
    let defaults = BrowserDefaults::from_config(&json!({"browser": {
        "executable": "/usr/bin/chromium",
        "user_agent": "Mozilla/5.0",
        "args": ["--disable-blink-features=AutomationControlled"],
        "perception": "tree"
    }}))
    .unwrap();
    assert_eq!(defaults.executable.as_deref(), Some("/usr/bin/chromium"));
    assert_eq!(defaults.user_agent.as_deref(), Some("Mozilla/5.0"));
    assert_eq!(defaults.args.len(), 1);
    assert_eq!(defaults.perception, Perception::Tree);
    for invalid in [
        json!({"browser": "chrome"}),
        json!({"browser": {"executable": 7}}),
        json!({"browser": {"user_agent": false}}),
        json!({"browser": {"args": "--headless"}}),
        json!({"browser": {"args": [1]}}),
        json!({"browser": {"perception": "vision"}}),
        json!({"browser": {"useragent": "typo"}}),
    ] {
        assert!(BrowserDefaults::from_config(&invalid).is_err(), "{invalid}");
        assert!(DesktopService::from_config(&invalid).is_err(), "{invalid}");
    }
}

#[test]
fn browser_defaults_fill_only_what_the_caller_left_unset() {
    use crate::tinybus_module::config::BrowserDefaults;
    use tinycomputer_browser::SessionOptions;

    let defaults = BrowserDefaults::from_config(&json!({"browser": {
        "executable": "/opt/chromium", "user_agent": "UA", "args": ["--a"]
    }}))
    .unwrap();
    let launched = defaults.apply(SessionOptions::default());
    assert_eq!(launched.executable.as_deref(), Some("/opt/chromium"));
    assert_eq!(launched.user_agent.as_deref(), Some("UA"));
    assert_eq!(launched.args, vec!["--a".to_owned()]);

    let own = defaults.apply(SessionOptions {
        user_agent: Some("mine".to_owned()),
        args: vec!["--b".to_owned()],
        ..SessionOptions::default()
    });
    assert_eq!(own.user_agent.as_deref(), Some("mine"));
    assert_eq!(own.args, vec!["--b".to_owned()]);

    let attached = defaults.apply(SessionOptions {
        endpoint: Some("http://127.0.0.1:9222".to_owned()),
        ..SessionOptions::default()
    });
    assert!(attached.executable.is_none() && attached.args.is_empty());
    assert_eq!(attached.user_agent.as_deref(), Some("UA"));
}

#[test]
fn the_cursor_is_configured_or_refused() {
    use crate::tinybus_module::config::cursor_config;
    use tinycomputer_browser::CursorPace;
    assert_eq!(
        cursor_config(&json!({})).unwrap().pace(),
        CursorPace::Natural
    );
    assert_eq!(
        cursor_config(&json!({"cursor": "calm"})).unwrap().pace(),
        CursorPace::Calm
    );
    assert!(
        cursor_config(&json!({"cursor": "off"}))
            .unwrap()
            .pace()
            .is_off()
    );
    let configured =
        cursor_config(&json!({"cursor": {"pace": "brisk", "overlay": "/opt/overlay"}}));
    assert_eq!(configured.unwrap().pace(), CursorPace::Brisk);
    assert_eq!(
        cursor_config(&json!({"cursor": {}})).unwrap().pace(),
        CursorPace::Natural
    );
    for wrong in [
        json!({"cursor": "frantic"}),
        json!({"cursor": true}),
        json!({"cursor": {"pace": 3}}),
        json!({"cursor": {"overlay": 3}}),
    ] {
        assert!(DesktopService::from_config(&wrong).is_err(), "{wrong}");
    }
    assert!(DesktopService::from_config(&json!({"cursor": "off"})).is_ok());
}
