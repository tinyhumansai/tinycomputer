//! Tests for the planner's routes: the configuration's wire form, the
//! endpoint allow-list, and which client each selection builds.

use serde_json::json;
use tinycomputer_bus::agent::{LanguageModelConfiguration, LanguageModelProvider};

use super::super::config::{
    ModelRoute, OPEN_ROUTER_BASE_URL, PlannerConfig, TINYHUMANS_BASE_URL, resolve_route,
    trusted_base_url,
};
use super::super::hosted::chat_model;
use super::super::{open_router, open_router_rescuer, open_router_shaper};

fn config(value: serde_json::Value) -> PlannerConfig {
    serde_json::from_value(value).expect("the configuration decodes")
}

#[test]
fn a_configuration_from_before_routes_still_means_open_router() {
    let legacy = config(json!({
        "api_key": "k",
        "model": "anthropic/claude-sonnet-5",
        "rescue_model": "openai/gpt-6-luna-pro"
    }));
    assert_eq!(legacy.route.provider, LanguageModelProvider::OpenRouter);
    assert_eq!(legacy.route.endpoint_url, None);
    assert!(legacy.rescue_route.is_none());
    assert_eq!(legacy.rescuer_route().api_key, "k");
    assert_eq!(
        serde_json::to_value(LanguageModelProvider::OpenRouter).unwrap(),
        json!("open_router")
    );
    assert_eq!(
        serde_json::to_value(LanguageModelProvider::TinyHumans).unwrap(),
        json!("tiny_humans")
    );
}

#[test]
fn a_tiny_humans_configuration_decodes_with_its_own_rescue_route() {
    let decoded = config(json!({
        "api_key": "th-bearer",
        "provider": "tiny_humans",
        "endpoint_url": "https://api.tinyhumans.ai/openai/v1",
        "sdk_name": "openhuman",
        "rescue_model": "openai/gpt-6-luna",
        "rescue_route": {"api_key": "sk-or", "provider": "open_router"}
    }));
    assert_eq!(decoded.route.provider, LanguageModelProvider::TinyHumans);
    assert_eq!(decoded.route.sdk_name.as_deref(), Some("openhuman"));
    let rescue = decoded.rescuer_route();
    assert_eq!(rescue.provider, LanguageModelProvider::OpenRouter);
    assert_eq!(rescue.api_key, "sk-or");
    assert_eq!(rescue.endpoint_url, None, "a rescue route inherits nothing");

    let printed = format!("{decoded:?}");
    assert!(!printed.contains("th-bearer") && !printed.contains("sk-or"));
    assert!(printed.contains("TinyHumans"));
}

#[test]
fn only_each_providers_own_base_url_is_approved() {
    for (provider, approved) in [
        (LanguageModelProvider::OpenRouter, OPEN_ROUTER_BASE_URL),
        (LanguageModelProvider::TinyHumans, TINYHUMANS_BASE_URL),
    ] {
        assert!(trusted_base_url(provider, approved));
        assert!(trusted_base_url(provider, &format!("{approved}/")));
    }
    for (provider, refused) in [
        (LanguageModelProvider::OpenRouter, TINYHUMANS_BASE_URL),
        (LanguageModelProvider::TinyHumans, OPEN_ROUTER_BASE_URL),
        (
            LanguageModelProvider::TinyHumans,
            "https://api.tinyhumans.ai.evil.example/openai/v1",
        ),
        (
            LanguageModelProvider::TinyHumans,
            "https://api.tinyhumans.ai/openai/v1/chat/completions",
        ),
        (
            LanguageModelProvider::OpenRouter,
            "https://attacker.example/api/v1",
        ),
        (
            LanguageModelProvider::OpenRouter,
            "http://openrouter.ai/api/v1",
        ),
    ] {
        assert!(!trusted_base_url(provider, refused), "{refused}");
    }

    let refused = config(json!({
        "api_key": "k",
        "provider": "tiny_humans",
        "endpoint_url": "https://attacker.example/openai/v1"
    }));
    for error in [
        open_router(&refused).map(|_| ()).unwrap_err(),
        open_router_rescuer(&refused).map(|_| ()).unwrap_err(),
        open_router_shaper(&refused).map(|_| ()).unwrap_err(),
    ] {
        assert!(error.contains("approved"), "{error}");
    }
    let bad_rescue = config(json!({
        "api_key": "k",
        "rescue_route": {"api_key": "r", "endpoint_url": "https://attacker.example/v1"}
    }));
    assert!(
        open_router(&bad_rescue).is_ok(),
        "the planner's own route is fine"
    );
    assert!(open_router_rescuer(&bad_rescue).is_err());
    let keyless_rescue = config(json!({"api_key": "k", "rescue_route": {"api_key": " "}}));
    assert!(open_router_rescuer(&keyless_rescue).is_err());
}

#[test]
fn each_selection_builds_its_client_on_the_selected_route() {
    let open = ModelRoute {
        api_key: "k".to_owned(),
        ..ModelRoute::default()
    };
    let chat = chat_model(&open, "anthropic/claude-sonnet-5").unwrap();
    assert_eq!(chat.base_url(), OPEN_ROUTER_BASE_URL);
    assert_eq!(chat.provider(), "openrouter");
    assert_eq!(chat.model(), "anthropic/claude-sonnet-5");

    let tiny = ModelRoute {
        api_key: "k".to_owned(),
        provider: LanguageModelProvider::TinyHumans,
        endpoint_url: Some(format!("{TINYHUMANS_BASE_URL}/")),
        sdk_name: Some(" OpenHuman App! ".to_owned()),
    };
    let chat = chat_model(&tiny, "openai/gpt-6-luna").unwrap();
    assert_eq!(chat.base_url(), TINYHUMANS_BASE_URL);
    assert_eq!(chat.provider(), "tinyhumans");
    assert_eq!(chat.model(), "openai/gpt-6-luna");
    assert_eq!(
        resolve_route(&tiny).unwrap().sdk_name.as_deref(),
        Some("openhumanapp"),
        "the product name is sanitized"
    );
    let open_with_name = ModelRoute {
        sdk_name: Some("openhuman".to_owned()),
        ..open
    };
    assert_eq!(
        resolve_route(&open_with_name).unwrap().sdk_name,
        None,
        "attribution goes to the TinyHumans gateway only"
    );
}

#[test]
fn each_model_reports_its_route_and_model_for_describe() {
    let configured = config(json!({
        "api_key": "th",
        "provider": "tiny_humans",
        "model": "anthropic/claude-sonnet-5",
        "rescue_model": "openai/gpt-6-luna-pro",
        "rescue_route": {"api_key": "or"}
    }));
    let reported = |provider, model: &str| LanguageModelConfiguration {
        provider,
        model: model.to_owned(),
        endpoint_url: None,
    };
    assert_eq!(
        open_router(&configured).unwrap().configuration(),
        Some(&reported(
            LanguageModelProvider::TinyHumans,
            "anthropic/claude-sonnet-5"
        ))
    );
    assert_eq!(
        open_router_rescuer(&configured).unwrap().configuration(),
        Some(&reported(
            LanguageModelProvider::OpenRouter,
            "openai/gpt-6-luna-pro"
        ))
    );
    assert_eq!(
        open_router_shaper(&configured).unwrap().configuration(),
        Some(&reported(
            LanguageModelProvider::TinyHumans,
            super::super::OUTPUT_MODEL
        ))
    );
}
