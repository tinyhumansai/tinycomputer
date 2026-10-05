//! Wire-form tests for Jev desktop-control payloads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    GoalContinuation, JevConfig, JevDecisionKind, JevOperation, JevProvider, JevStopReason,
    ResolveIntentRequest, RunGoalRequest, VisiblePredicate,
};
use serde_json::json;

#[test]
fn configuration_serializes_the_key_but_never_debug_prints_it() {
    let mut request = JevConfig::new("openrouter-secret");
    request.provider = JevProvider::OpenRouter;
    request.endpoint_url = Some("https://openrouter.ai/api/alpha/decisions".into());
    request.sdk_name = Some("openhuman".into());
    let value = serde_json::to_value(&request).expect("configuration serializes");

    assert_eq!(value["api_key"], json!("openrouter-secret"));
    assert!(!format!("{request:?}").contains("openrouter-secret"));
    assert_eq!(value["sdk_name"], json!("openhuman"));
}

#[test]
fn decision_model_selection_round_trips_and_stays_additive() {
    use super::JevConfiguration;

    // A 2.7 configuration decodes unchanged: TypeSafe Jev, no `fast`.
    let legacy: JevConfig = serde_json::from_value(json!({"api_key": "k"})).unwrap();
    assert_eq!(legacy.provider, JevProvider::TypeSafe);
    assert_eq!(legacy.fast, None);

    let sage: JevConfig =
        serde_json::from_value(json!({"api_key": "k", "provider": "sage", "fast": true})).unwrap();
    assert_eq!(sage.provider, JevProvider::Sage);
    assert_eq!(sage.fast, Some(true));
    let again: JevConfig = serde_json::from_value(serde_json::to_value(&sage).unwrap()).unwrap();
    assert_eq!(again, sage);
    assert!(format!("{sage:?}").contains("fast"));

    let alias: JevConfig =
        serde_json::from_value(json!({"api_key": "k", "provider": "openjev"})).unwrap();
    assert_eq!(alias.provider, JevProvider::OpenJev);
    assert!(serde_json::from_value::<JevConfig>(json!({"provider": "levanto"})).is_err());

    // Contract 2.9: a self-hosted decision model decodes under both
    // spellings and round-trips with its declared endpoint and model.
    let self_hosted: JevConfig = serde_json::from_value(json!({
        "api_key": "k",
        "provider": "self_hosted",
        "endpoint_url": "https://inference.internal.example/rune-26b/v1/decisions",
        "model": "ci-models-gemma__ci-rune-26b-a4b"
    }))
    .unwrap();
    assert_eq!(self_hosted.provider, JevProvider::SelfHosted);
    assert_eq!(
        self_hosted.endpoint_url.as_deref(),
        Some("https://inference.internal.example/rune-26b/v1/decisions")
    );
    let again: JevConfig =
        serde_json::from_value(serde_json::to_value(&self_hosted).unwrap()).unwrap();
    assert_eq!(again, self_hosted);
    let alias: JevConfig =
        serde_json::from_value(json!({"api_key": "k", "provider": "selfhosted"})).unwrap();
    assert_eq!(alias.provider, JevProvider::SelfHosted);

    assert_eq!(JevProvider::TypeSafe.default_model(), "jev-latest");
    assert_eq!(JevProvider::OpenRouter.default_model(), "jev-latest");
    assert_eq!(
        JevProvider::TinyHumansOpenRouter.default_model(),
        "jev-latest"
    );
    assert_eq!(JevProvider::OpenJev.default_model(), "openjev");
    assert_eq!(JevProvider::Sage.default_model(), "levanto-sage");
    assert_eq!(JevProvider::SelfHosted.default_model(), "");

    // The summary leaves `fast` out when false, so a 2.7 reader sees the
    // same object it always did.
    let summary = JevConfiguration {
        provider: JevProvider::OpenJev,
        model: "openjev".into(),
        endpoint_url: None,
        fast: false,
    };
    let value = serde_json::to_value(&summary).unwrap();
    assert_eq!(
        value,
        json!({"provider": "open_jev", "model": "openjev", "endpoint_url": null})
    );
    let decoded: JevConfiguration = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, summary);
    let fast = JevConfiguration {
        provider: JevProvider::Sage,
        fast: true,
        ..summary
    };
    assert_eq!(serde_json::to_value(&fast).unwrap()["fast"], json!(true));
}

#[test]
fn every_agentic_enum_pins_its_wire_spelling() {
    assert_eq!(
        serde_json::to_value(JevProvider::TypeSafe).unwrap(),
        json!("type_safe")
    );
    assert_eq!(
        serde_json::to_value(JevProvider::OpenRouter).unwrap(),
        json!("open_router")
    );
    assert_eq!(
        serde_json::to_value(JevProvider::TinyHumansOpenRouter).unwrap(),
        json!("tiny_humans_open_router")
    );
    assert_eq!(
        serde_json::to_value(JevProvider::OpenJev).unwrap(),
        json!("open_jev")
    );
    assert_eq!(
        serde_json::to_value(JevProvider::Sage).unwrap(),
        json!("sage")
    );
    for (operation, wire) in [
        (JevOperation::Click, "CLICK"),
        (JevOperation::TypeText, "TYPE_TEXT"),
        (JevOperation::Check, "CHECK"),
        (JevOperation::Uncheck, "UNCHECK"),
        (JevOperation::Expand, "EXPAND"),
        (JevOperation::Collapse, "COLLAPSE"),
        (JevOperation::Scroll, "SCROLL"),
        (JevOperation::Drill, "DRILL"),
        (JevOperation::Widen, "WIDEN"),
        (JevOperation::Wait, "WAIT"),
        (JevOperation::Done, "DONE"),
        (JevOperation::Blocked, "BLOCKED"),
    ] {
        assert_eq!(serde_json::to_value(operation).unwrap(), json!(wire));
    }
    assert_eq!(
        serde_json::to_value(JevDecisionKind::ConfirmationRequired).unwrap(),
        json!("confirmation_required")
    );
    assert_eq!(
        serde_json::to_value(JevStopReason::ActionFailed).unwrap(),
        json!("action_failed")
    );
}

#[test]
fn agentic_requests_default_to_not_sharing_field_values() {
    let resolve: ResolveIntentRequest = serde_json::from_value(json!({
        "app": "Spotify",
        "intent": "open Search"
    }))
    .expect("resolve request decodes");
    let run: RunGoalRequest = serde_json::from_value(json!({
        "app": "Spotify",
        "goal": "open Search"
    }))
    .expect("run request decodes");

    assert!(!resolve.include_values);
    assert!(!run.include_values);
    assert!(run.window_id.is_none());
    assert_eq!((run.max_steps, run.max_model_calls), (40, 80));
    assert!(run.continuation.is_none());
}

#[test]
fn confirmation_payload_has_explicit_approval_and_one_use_handle() {
    let request: RunGoalRequest = serde_json::from_value(json!({
        "continuation": {"id": "opaque-handle", "approve": false}
    }))
    .expect("continuation decodes");
    assert_eq!(
        request.continuation,
        Some(GoalContinuation {
            id: "opaque-handle".to_owned(),
            approve: false,
        })
    );
    assert_eq!(
        serde_json::to_value(JevStopReason::Cancelled).unwrap(),
        json!("cancelled")
    );
    assert_eq!(
        serde_json::to_value(JevStopReason::StaleTarget).unwrap(),
        json!("stale_target")
    );
}

#[test]
fn scoped_goal_additions_are_backward_compatible_and_have_stable_wire_names() {
    let old: RunGoalRequest =
        serde_json::from_value(json!({"app":"TextEdit","goal":"type"})).unwrap();
    assert!(old.require_confirmations);
    assert_eq!(old.success, []);
    assert_eq!(old.max_elapsed_ms, 120_000);
    let scoped: RunGoalRequest = serde_json::from_value(json!({
        "app":"TextEdit", "goal":"type", "window":"Untitled",
        "window_id":"w-515619",
        "allowed_operations":["TYPE_TEXT"],
        "allowed_targets":["Document"],
        "text_slots":{"Document":"marker"},
        "success":[{"kind":"value_contains","name":"Document","value":"marker"}],
        "max_elapsed_ms":30000,
        "require_confirmations":false
    }))
    .unwrap();
    assert_eq!(scoped.allowed_operations, vec![JevOperation::TypeText]);
    assert_eq!(scoped.window_id.as_deref(), Some("w-515619"));
    assert_eq!(
        scoped.success,
        vec![VisiblePredicate::ValueContains {
            name: "Document".into(),
            value: "marker".into()
        }]
    );
    assert!(!scoped.require_confirmations);
    assert_eq!(
        serde_json::to_value(&scoped).unwrap()["success"][0]["kind"],
        json!("value_contains")
    );
}

#[test]
fn bounded_snapshot_cannot_claim_an_element_is_absent() {
    let unsupported = serde_json::from_value::<RunGoalRequest>(json!({
        "app": "TextEdit",
        "goal": "close dialog",
        "success": [{"kind": "name_absent", "name": "Dialog"}]
    }));
    assert!(unsupported.is_err());
}

#[test]
fn contained_name_fragment_has_a_stable_wire_shape() {
    let predicate = VisiblePredicate::NameContains {
        fragment: "Your message, Hello from OpenHuman".into(),
        within: "Messages in chat with Alex Rivera".into(),
    };
    let wire = json!({
        "kind": "name_contains",
        "fragment": "Your message, Hello from OpenHuman",
        "within": "Messages in chat with Alex Rivera"
    });
    assert_eq!(serde_json::to_value(&predicate).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<VisiblePredicate>(wire).unwrap(),
        predicate
    );
}
