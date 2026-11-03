//! Tests for the module configuration: rejection, desktop availability, the
//! planner, the browser settings, and the cursor.

use crate::tinybus_module::DesktopService;
use serde_json::json;
use tinycomputer_bus::DesktopResponse;
use tinycomputer_bus::config::ComputerConfig;

fn browser_defaults(config: serde_json::Value) -> crate::tinybus_module::config::BrowserDefaults {
    let config: ComputerConfig = serde_json::from_value(config).expect("typed computer config");
    crate::tinybus_module::config::BrowserDefaults::from_config(config.browser.as_ref(), None)
}

#[test]
fn a_malformed_configuration_is_rejected_rather_than_defaulted() {
    let error = DesktopService::from_config(&json!({ "session_id": 7 }))
        .expect_err("a numeric session id is not a session id");

    assert!(error.to_string().contains("session_id"));
}

#[test]
fn browser_egress_is_private_strict_and_requires_the_host_connection() {
    let config = json!({"browser":{"egress":{"policy_id":"policy-secret","generation":7}}});
    let scope = crate::tinybus_module::config::BrowserDefaults::egress_from_raw(&config)
        .expect("valid host scope")
        .expect("scope is present");
    assert_eq!(scope.policy_id, "policy-secret");
    assert_eq!(scope.generation, 7);
    assert!(!format!("{scope:?}").contains("policy-secret"));
    assert!(DesktopService::from_config(&config).is_err());
    assert!(crate::tinybus_module::config::BrowserDefaults::egress_from_raw(
        &json!({"browser":{"args":["--proxy-server=http://untrusted"],"egress":{"policy_id":"p","generation":7}}})
    )
    .is_err());

    for egress in [
        json!({"policy_id":"","generation":7}),
        json!({"policy_id":"p","generation":0}),
        json!({"policy_id":"p","generation":7,"extra":true}),
        json!([]),
    ] {
        assert!(
            crate::tinybus_module::config::BrowserDefaults::egress_from_raw(
                &json!({"browser":{"egress":egress}})
            )
            .is_err()
        );
    }
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
    ] {
        assert!(DesktopService::from_config(&refused).is_err(), "{refused}");
    }
}

#[test]
fn the_browser_configuration_is_read_or_refused() {
    use crate::tinybus_module::config::BrowserDefaults;
    use tinycomputer_browser::{Perception, Settle};

    assert_eq!(browser_defaults(json!({})), BrowserDefaults::default());
    assert!(DesktopService::from_config(&json!({"browser": {}})).is_ok());
    let defaults = browser_defaults(json!({"browser": {
        "executable": "/usr/bin/chromium",
        "user_agent": "Mozilla/5.0",
        "args": ["--disable-blink-features=AutomationControlled"],
        "perception": "tree",
        "settle": "steady",
        "prelaunch": false
    }}));
    assert_eq!(defaults.executable.as_deref(), Some("/usr/bin/chromium"));
    assert_eq!(defaults.user_agent.as_deref(), Some("Mozilla/5.0"));
    assert_eq!(defaults.args.len(), 1);
    assert_eq!(defaults.perception, Perception::Tree);
    assert_eq!(defaults.settle, Settle::Steady);
    assert!(!defaults.prelaunch);
    let egress = BrowserDefaults::from_config(&json!({"browser": {
        "egress": {"policy_id": "host-policy", "generation": 7}
    }}))
    .unwrap();
    assert_eq!(egress.egress.as_ref().unwrap().policy_id, "host-policy");
    assert_eq!(egress.egress.as_ref().unwrap().generation, 7);
    // Settling promptly and opening the browser early are on unless turned
    // off.
    let unset = BrowserDefaults::default();
    assert_eq!(unset.settle, Settle::Prompt);
    assert!(unset.prelaunch);
    let asked = browser_defaults(json!({"browser": {
        "settle": "prompt",
        "prelaunch": true
    }}));
    assert_eq!(asked, unset);
    for invalid in [
        json!({"browser": "chrome"}),
        json!({"browser": {"executable": 7}}),
        json!({"browser": {"user_agent": false}}),
        json!({"browser": {"args": "--headless"}}),
        json!({"browser": {"args": [1]}}),
        json!({"browser": {"perception": "vision"}}),
        json!({"browser": {"settle": "fast"}}),
        json!({"browser": {"prelaunch": "yes"}}),
        json!({"browser": {"useragent": "typo"}}),
        json!({"browser": {"egress": {"policy_id": "", "generation": 0}}}),
        json!({"browser": {"egress": {"policy_id": "p", "generation": 1, "extra": true}}}),
        json!({"browser": {"egress": {"policy_id": "p", "generation": 1}, "args": ["--proxy-server=bad"]}}),
    ] {
        assert!(
            serde_json::from_value::<ComputerConfig>(invalid.clone()).is_err(),
            "{invalid}"
        );
        assert!(DesktopService::from_config(&invalid).is_err(), "{invalid}");
    }
}

#[test]
fn browser_defaults_fill_only_what_the_caller_left_unset() {
    use tinycomputer_browser::SessionOptions;

    let defaults = browser_defaults(json!({"browser": {
        "executable": "/opt/chromium", "user_agent": "UA", "args": ["--a"]
    }}));
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

#[test]
fn trace_sink_requires_the_host_prepared_unique_destination() {
    use crate::tinybus_module::config::trace_sink_destination;

    assert!(trace_sink_destination(&json!({})).unwrap().is_none());
    assert!(
        trace_sink_destination(&json!({"trace_sink": false}))
            .unwrap()
            .is_none()
    );
    assert!(trace_sink_destination(&json!({"trace_sink": true})).is_err());
    assert!(trace_sink_destination(&json!({"trace_sink": {}})).is_err());
    assert!(
        trace_sink_destination(&json!({
            "trace_sink": {"destination": ":1.42", "scope": "caller-selected"}
        }))
        .is_err()
    );
    assert!(
        trace_sink_destination(&json!({"trace_sink": {"destination": "org.example.Host"}}))
            .is_err()
    );
    assert_eq!(
        trace_sink_destination(&json!({"trace_sink": {"destination": ":1.42"}}))
            .unwrap()
            .unwrap()
            .as_str(),
        ":1.42"
    );
}

#[test]
fn consequence_host_requires_the_host_prepared_unique_destination() {
    use crate::tinybus_module::config::consequence_host_destination;

    assert!(consequence_host_destination(&json!({})).unwrap().is_none());
    assert!(
        consequence_host_destination(&json!({"consequence_host": false}))
            .unwrap()
            .is_none()
    );
    for invalid in [
        json!({"consequence_host": true}),
        json!({"consequence_host": {}}),
        json!({"consequence_host": {"destination": "ai.tinyhumans.host"}}),
        json!({"consequence_host": {"destination": ":1.42", "scope": "model"}}),
    ] {
        assert!(consequence_host_destination(&invalid).is_err(), "{invalid}");
    }
    assert_eq!(
        consequence_host_destination(&json!({
            "consequence_host": {"destination": ":1.42"}
        }))
        .unwrap()
        .unwrap()
        .as_str(),
        ":1.42"
    );
}

#[test]
fn connectionless_service_construction_rejects_consequence_host_configuration() {
    let error = DesktopService::from_config(&json!({
        "consequence_host": {"destination": ":1.42"}
    }))
    .expect_err("a host callback needs the module's bus connection");

    assert!(error.to_string().contains("consequence_host"));
}

#[test]
fn connectionless_service_construction_rejects_trace_sink_configuration() {
    let error = DesktopService::from_config(&json!({
        "trace_sink": {"destination": ":1.42"}
    }))
    .expect_err("a host callback needs the module's bus connection");

    assert!(error.to_string().contains("trace_sink"));
}
