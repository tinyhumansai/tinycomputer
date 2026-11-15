//! Tests for one-step intent resolution, the public wrappers, runtime
//! configuration, desktop dispatch, and the reply helpers.

use super::*;
use crate::agentic::runtime::{RETRY, client_config};

#[test]
fn runtime_configuration_covers_all_providers_and_rejects_empty_keys() {
    for provider in [
        JevProvider::TypeSafe,
        JevProvider::OpenRouter,
        JevProvider::TinyHumansOpenRouter,
        JevProvider::OpenJev,
        JevProvider::Sage,
        JevProvider::Surplus,
    ] {
        let mut request = JevConfig::new("key");
        request.provider = provider;
        request.model = Some("jev-test".to_owned());
        request.timeout_ms = Some(500);
        request.max_retries = Some(0);
        request.endpoint_url = Some("http://127.0.0.1:1/decisions".to_owned());
        let runtime = JevRuntime::configure(&request).expect("configuration is valid");
        assert_eq!(runtime.configuration().provider, provider);
        let mut keyless = JevConfig::default();
        keyless.provider = provider;
        assert!(JevRuntime::configure(&keyless).is_err(), "{provider:?}");
    }
    assert!(JevRuntime::configure(&JevConfig::default()).is_err());
    let mut untrusted = JevConfig::new("key");
    untrusted.provider = JevProvider::OpenRouter;
    untrusted.endpoint_url = Some("https://attacker.example/decisions".to_owned());
    assert!(JevRuntime::configure(&untrusted).is_err());
}

#[test]
fn jev_calls_ride_out_a_provider_outage_unless_told_otherwise() {
    // Live, the client's own retries, 100 ms and then 200 ms apart, gave a
    // gateway's 502s 2.5 s before they ended the task.
    let mut request = JevConfig::new("key");
    request.provider = JevProvider::TinyHumansOpenRouter;
    assert_eq!(client_config(&request).retry, RETRY);
    request.max_retries = Some(0);
    let told = client_config(&request).retry;
    assert_eq!(told.max_retries, 0, "a configured number of retries wins");
    assert_eq!(told.initial_backoff, RETRY.initial_backoff);
}

#[test]
fn a_jev_attempt_gives_up_after_ten_seconds_unless_told_otherwise() {
    // Live, a request nothing came back for waited the client's own 30 s
    // before its retry answered in under a second.
    let mut request = JevConfig::new("key");
    assert_eq!(
        client_config(&request).timeout,
        std::time::Duration::from_secs(10)
    );
    request.timeout_ms = Some(30_000);
    assert_eq!(
        client_config(&request).timeout,
        std::time::Duration::from_secs(30),
        "a configured timeout wins"
    );
}

#[test]
fn each_provider_selects_its_decision_model() {
    let configured = |value: serde_json::Value| {
        let request: JevConfig = serde_json::from_value(value).expect("the configuration decodes");
        JevRuntime::configure(&request)
            .expect("configuration is valid")
            .configuration()
            .clone()
    };
    let legacy = configured(serde_json::json!({"api_key": "k"}));
    assert_eq!(legacy.provider, JevProvider::TypeSafe);
    assert_eq!(legacy.model, "jev-latest");
    assert!(!legacy.fast);

    let openjev = configured(serde_json::json!({"api_key": "k", "provider": "open_jev"}));
    assert_eq!(openjev.provider, JevProvider::OpenJev);
    assert_eq!(openjev.model, "openjev", "OpenJEV answers as its own model");
    let alias = configured(
        serde_json::json!({"api_key": "k", "provider": "openjev", "model": "openjev-2"}),
    );
    assert_eq!(alias.provider, JevProvider::OpenJev);
    assert_eq!(alias.model, "openjev-2");

    let sage = configured(serde_json::json!({
        "api_key": "k", "provider": "sage", "fast": true, "model": "ignored"
    }));
    assert_eq!(sage.provider, JevProvider::Sage);
    assert_eq!(sage.model, "levanto-sage", "Sage takes no model selection");
    assert!(sage.fast);
    assert!(!configured(serde_json::json!({"api_key": "k", "provider": "sage"})).fast);
    assert!(
        !configured(serde_json::json!({"api_key": "k", "provider": "open_router", "fast": true}))
            .fast,
        "`fast` is Sage's alone"
    );
    // The constructor the examples use builds the same runtime.
    assert_eq!(JevRuntime::sage("k", true).unwrap().configuration(), &sage);
}

#[test]
fn a_surplus_request_builds_a_surplus_client_config() {
    use tinyinference_decisions::Provider;

    let mut request = JevConfig::new("key");
    request.provider = JevProvider::Surplus;
    let config = client_config(&request);
    assert_eq!(config.provider, Provider::Surplus);
    assert_eq!(
        config.endpoint_url,
        "https://api.surplusintelligence.ai/v1/decisions"
    );
}

#[test]
fn only_each_providers_own_endpoint_is_approved() {
    use crate::agentic::runtime::{approved_endpoint, trusted_endpoint};

    let providers = [
        JevProvider::TypeSafe,
        JevProvider::OpenRouter,
        JevProvider::TinyHumansOpenRouter,
        JevProvider::OpenJev,
        JevProvider::Sage,
        JevProvider::Surplus,
    ];
    for provider in providers {
        let mut request = JevConfig::new("key");
        request.provider = provider;
        request.endpoint_url = Some(approved_endpoint(provider).to_owned());
        let runtime = JevRuntime::configure(&request).expect("the provider's own route");
        assert_eq!(
            runtime.configuration().endpoint_url.as_deref(),
            Some(approved_endpoint(provider))
        );
        for other in providers.into_iter().filter(|other| *other != provider) {
            assert!(
                !trusted_endpoint(provider, approved_endpoint(other)),
                "{provider:?} refuses {other:?}'s route"
            );
        }
    }
    assert_eq!(
        approved_endpoint(JevProvider::OpenJev),
        "https://api.openjev.sh/v1/systemone"
    );
    assert_eq!(
        approved_endpoint(JevProvider::Surplus),
        "https://api.surplusintelligence.ai/v1/decisions"
    );
    assert!(trusted_endpoint(
        JevProvider::Surplus,
        "https://api.surplusintelligence.ai/v1/decisions"
    ));
    assert!(!trusted_endpoint(
        JevProvider::Surplus,
        "https://api.surplusintelligence.ai/v1/decisions/"
    ));
    assert!(trusted_endpoint(
        JevProvider::Sage,
        "https://sage.levanto.ai"
    ));
    for refused in [
        "https://sage.levanto.ai.evil.example/",
        "https://sage.levanto.ai/v1/decide",
        "http://sage.levanto.ai/",
    ] {
        assert!(!trusted_endpoint(JevProvider::Sage, refused), "{refused}");
    }
    let mut untrusted = JevConfig::new("key");
    untrusted.provider = JevProvider::OpenJev;
    untrusted.endpoint_url = Some("https://api.openjev.sh.evil.example/v1/systemone".to_owned());
    let error = JevRuntime::configure(&untrusted).unwrap_err();
    assert_eq!(error.code, "JEV_INVALID_CONFIG");
}

#[test]
fn desktop_execution_dispatches_every_closed_operation_without_panicking() {
    let desktop = crate::Desktop::new();
    let candidate = Candidate {
        ref_id: String::new(),
        ..Candidate::default()
    };
    for operation in [
        JevOperation::Click,
        JevOperation::TypeText,
        JevOperation::Check,
        JevOperation::Uncheck,
        JevOperation::Expand,
        JevOperation::Collapse,
        JevOperation::Scroll,
        JevOperation::Wait,
        JevOperation::Drill,
        JevOperation::Widen,
        JevOperation::Done,
        JevOperation::Blocked,
    ] {
        let reply = execute_desktop(
            &desktop,
            operation,
            Some(&candidate),
            Some("text".to_owned()),
        );
        assert_ne!(reply.command, "");
    }
}

#[tokio::test]
async fn one_step_resolution_and_public_wrappers_cover_success_and_observation_failure() {
    let runtime = runtime(vec![response("CLICK", 0.9, "1")]);
    assert!(format!("{runtime:?}").contains("JevRuntime"));
    let (backend, _) = backend(1);
    let reply = resolve_intent_with(
        backend,
        runtime.clone(),
        tinycomputer_bus::ResolveIntentRequest {
            app: "Spotify".to_owned(),
            intent: "play the topmost song".to_owned(),
            execute: false,
            ..tinycomputer_bus::ResolveIntentRequest::default()
        },
    )
    .await;
    assert!(reply.ok);

    let missing = tinycomputer_bus::ResolveIntentRequest {
        app: "__tinycomputer_missing__".to_owned(),
        intent: "click".to_owned(),
        ..tinycomputer_bus::ResolveIntentRequest::default()
    };
    assert!(
        !resolve_intent(crate::Desktop::new(), runtime.clone(), missing)
            .await
            .ok
    );
    assert!(
        !run_goal(
            crate::Desktop::new(),
            runtime,
            RunGoalRequest {
                app: "__tinycomputer_missing__".to_owned(),
                goal: "finish".to_owned(),
                ..RunGoalRequest::default()
            }
        )
        .await
        .ok
    );
}

#[tokio::test]
async fn one_step_resolution_reranks_a_close_target_shortlist() {
    let first = evaluation(json!({
        "model": "typesafe/jev-1.13-20260917",
        "answers": {
            "operation": {"type": "choice", "choice": "CLICK", "confidence": 0.4,
                "probabilities": {"CLICK": 0.9, "WAIT": 0.033_333_333_333, "DONE": 0.033_333_333_333, "BLOCKED": 0.033_333_333_334}},
            "click_target": {"type": "choice", "choice": "1", "confidence": 0.3,
                "probabilities": {"1": 0.5, "2": 0.4, "none": 0.1}},
            "destructive": {"type": "noul", "noul": 0.05}
        },
        "usage": {}
    }));
    let reranked = evaluation(json!({
        "model": "typesafe/jev-1.13-20260917",
        "answers": {
            "target": {"type": "choice", "choice": "2", "confidence": 0.6,
                "probabilities": {"1": 0.1, "2": 0.85, "none": 0.05}}
        },
        "usage": {}
    }));
    let runtime = runtime(vec![first, reranked]);
    let backend = FakeBackend {
        screens: Arc::new(Mutex::new(VecDeque::from([two_candidate_screen()]))),
        operations: Arc::new(Mutex::new(Vec::new())),
        fail_execute: false,
    };
    let reply = resolve_intent_with(
        backend,
        runtime,
        tinycomputer_bus::ResolveIntentRequest {
            app: "Spotify".to_owned(),
            intent: "activate the second song".to_owned(),
            ..tinycomputer_bus::ResolveIntentRequest::default()
        },
    )
    .await;
    assert!(reply.ok);
    let decision: tinycomputer_bus::JevDecision =
        serde_json::from_value(reply.data.expect("decision data")).expect("decision decodes");
    assert_eq!(decision.target.expect("target").ref_id, "@s1:e2");
}

#[test]
fn response_helpers_classify_provider_failures_and_policy_reasons() {
    for (error, code) in [
        (
            tinyinference_decisions::Error::Authentication,
            "JEV_AUTHENTICATION",
        ),
        (
            tinyinference_decisions::Error::RateLimited,
            "JEV_RATE_LIMITED",
        ),
        (tinyinference_decisions::Error::Timeout, "JEV_TIMEOUT"),
        (
            tinyinference_decisions::Error::InvalidResponse {
                reason: "bad".to_owned(),
            },
            "JEV_INVALID_RESPONSE",
        ),
        (
            tinyinference_decisions::Error::HttpStatus { status: 500 },
            "JEV_PROVIDER_FAILED",
        ),
    ] {
        let failure = tinyinference_decisions::EvaluationFailure {
            error: Box::new(error),
            attempts: 1,
            latency: Duration::ZERO,
        };
        assert_eq!(
            provider_error(&failure)
                .error
                .as_ref()
                .expect("provider error payload")
                .code,
            code
        );
    }
    for decision in [
        JevDecisionKind::Act,
        JevDecisionKind::ConfirmationRequired,
        JevDecisionKind::Abstain,
        JevDecisionKind::NeedsText,
        JevDecisionKind::Done,
        JevDecisionKind::Blocked,
    ] {
        assert_ne!(reason(decision, 0.5, 0.6), "");
    }
    let target = target_payload(&Candidate {
        ref_id: "@s:e1".to_owned(),
        role: "button".to_owned(),
        description: Some("described".to_owned()),
        ..Candidate::default()
    });
    assert_eq!(target.name.as_deref(), Some("described"));
    assert!(!internal_error("broken").ok);
    assert!(agent_response("test", &json!({"ok": true})).ok);
}
