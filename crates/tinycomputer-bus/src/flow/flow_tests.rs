//! Wire-form tests for intent-flow payloads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    Deliberation, FLOW_GUIDE, Flow, FlowAction, FlowBrief, FlowLoop, FlowStep, FlowStopReason,
    FlowStrategy, GroundingHint, RunFlowRequest, Slot, Slots, StepOutcome,
};
use serde_json::json;

#[test]
fn a_bare_string_step_is_a_do_intent() {
    let step: FlowStep = serde_json::from_value(json!("start a new note")).unwrap();
    assert_eq!(step, FlowStep::Intent("start a new note".into()));
    assert_eq!(step.action(), FlowAction::Do("start a new note".into()));
    assert_eq!(
        serde_json::to_value(&step).unwrap(),
        json!("start a new note")
    );
}

#[test]
fn enter_keeps_its_slots_in_document_order_when_parsed_from_text() {
    let step: FlowStep = serde_json::from_str(
        r#"{"enter": {"recipient": "a@b.c", "subject": "Hi", "body": "Text"}}"#,
    )
    .unwrap();
    let FlowStep::Action(FlowAction::Enter(Slots(slots))) = &step else {
        panic!("expected an enter step, got {step:?}");
    };
    let order = slots
        .iter()
        .map(|slot| slot.slot.as_str())
        .collect::<Vec<_>>();
    assert_eq!(order, ["recipient", "subject", "body"]);
    assert_eq!(
        serde_json::to_string(&step).unwrap(),
        r#"{"enter":{"recipient":"a@b.c","subject":"Hi","body":"Text"}}"#
    );
    assert!(serde_json::from_value::<Slots>(json!(["not", "a", "map"])).is_err());
}

#[test]
fn malformed_steps_explain_what_is_wrong() {
    let unknown = serde_json::from_value::<FlowStep>(json!({"click": "Send"}))
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("unknown step kind `click`"), "{unknown}");
    let two_keys = serde_json::from_value::<FlowStep>(json!({"do": "a", "verify": "b"}))
        .unwrap_err()
        .to_string();
    assert!(two_keys.contains("exactly one key"), "{two_keys}");
    let wrong_payload = serde_json::from_value::<FlowStep>(json!({"read": "subject"}))
        .unwrap_err()
        .to_string();
    assert!(wrong_payload.contains("in `read` step"), "{wrong_payload}");
    let empty = serde_json::from_value::<FlowStep>(json!({}))
        .unwrap_err()
        .to_string();
    assert!(empty.contains("found none"), "{empty}");
    for value in [json!(null), json!(true), json!(3), json!([])] {
        let error = serde_json::from_value::<FlowStep>(value)
            .unwrap_err()
            .to_string();
        assert!(error.contains("a step: a string"), "{error}");
    }
    let borrowed: FlowStep = serde_json::from_str(r#""plain""#).unwrap();
    assert_eq!(borrowed, FlowStep::Intent("plain".into()));
}

#[test]
fn control_steps_pin_their_wire_form() {
    let flow: Flow = serde_json::from_value(json!({
        "app": "Finder",
        "steps": [
            {"if": {"condition": "c", "then": ["a"], "else": [{"verify": "v"}]}},
            {"repeat_until": {"condition": "done", "steps": ["x"]}},
            {"choose": {"what": "list", "option": "B"}},
            {"read": {"what": "title", "into": "t"}},
            {"wait_for": "ready"},
            {"stop_before": "deleting"},
            {"open": "Finder"},
            {"browse": "https://flights.test"},
            {"pick": {"from": "the flight results", "by": "lowest price", "into": "flight"}},
            {"extract": {"what": "the flight results", "into": "flights"}},
            {"do": "y"}
        ]
    }))
    .unwrap();
    let FlowStep::Action(FlowAction::RepeatUntil(repeat)) = &flow.steps[1] else {
        panic!("expected repeat_until");
    };
    assert_eq!(repeat.max, 5);
    assert_eq!(
        flow.steps[7],
        FlowStep::Action(FlowAction::Browse("https://flights.test".to_owned()))
    );
    let FlowStep::Action(FlowAction::Pick(pick)) = &flow.steps[8] else {
        panic!("expected pick");
    };
    assert_eq!(pick.into.as_deref(), Some("flight"));
    let bare: crate::PickStep =
        serde_json::from_value(json!({"from": "results", "by": "cheapest"})).unwrap();
    assert!(serde_json::to_value(&bare).unwrap().get("into").is_none());
    let round_trip: Flow = serde_json::from_value(serde_json::to_value(&flow).unwrap()).unwrap();
    assert_eq!(round_trip, flow);
    assert_eq!(
        serde_json::to_value(&flow).unwrap()["steps"][0]["if"]["else"],
        json!([{"verify": "v"}])
    );
}

#[test]
fn run_requests_default_to_safe_bounded_runs() {
    let request: RunFlowRequest =
        serde_json::from_value(json!({"flow": {"app": "Mail", "steps": ["x"]}})).unwrap();
    assert!(!request.allow_destructive && !request.include_values);
    assert_eq!(
        (request.max_actions, request.max_model_calls, request.votes),
        (60, 3000, 7)
    );
    assert!(request.brief.is_empty());
    assert!(request.disabled_loops.is_empty() && request.memory.is_empty() && !request.trace);
    assert_eq!(request.strategy, FlowStrategy::Narrow);
    assert_eq!(request.deliberation, Deliberation::Deep);
    assert_eq!(
        request,
        RunFlowRequest {
            flow: request.flow.clone(),
            ..RunFlowRequest::default()
        }
    );
}

#[test]
fn run_request_facts_default_empty_and_round_trip() {
    let request: RunFlowRequest =
        serde_json::from_value(json!({"flow": {"app": "Mail", "steps": ["x"]}})).unwrap();
    assert!(request.facts.is_empty());

    let with_facts: RunFlowRequest = serde_json::from_value(json!({
        "flow": {"app": "Mail", "steps": ["x"]},
        "vars": {"email": "sam@example.com"},
        "facts": ["email"],
    }))
    .unwrap();
    assert_eq!(
        with_facts.facts,
        std::collections::BTreeSet::from(["email".to_owned()])
    );
    let round_trip: RunFlowRequest =
        serde_json::from_value(serde_json::to_value(&with_facts).unwrap()).unwrap();
    assert_eq!(round_trip, with_facts);
}

#[test]
fn run_request_carries_host_handles_and_task_context() {
    let request: RunFlowRequest = serde_json::from_value(json!({
        "flow": {"app": "browser", "steps": [{"enter": {"card number": "${card number}"}}]},
        "facts": ["card number"],
        "fact_handles": {
            "card number": {"handle": "opaque-card-reference", "mask": "•••• 1111", "kind": "payment_card"}
        },
        "task_id": "t-7"
    }))
    .unwrap();
    assert_eq!(
        request.fact_handles["card number"].handle,
        "opaque-card-reference"
    );
    assert_eq!(request.task_id.as_deref(), Some("t-7"));
    let encoded = serde_json::to_value(&request).unwrap();
    assert_eq!(encoded["fact_handles"]["card number"]["mask"], "•••• 1111");
    assert_eq!(
        serde_json::from_value::<RunFlowRequest>(encoded).unwrap(),
        request
    );
}

#[test]
fn a_brief_pins_its_wire_form_and_defaults_empty() {
    let brief: FlowBrief = serde_json::from_value(json!({
        "goal": "book the cheapest flight to Srinagar",
        "details": {"first name": "Asha", "date of birth": "2000-01-01"},
        "secrets": ["card number"],
        "rules": ["stop before paying"]
    }))
    .unwrap();
    assert!(!brief.is_empty());
    assert_eq!(brief.details["date of birth"], "2000-01-01");
    assert_eq!(
        serde_json::to_value(&brief).unwrap(),
        json!({
            "goal": "book the cheapest flight to Srinagar",
            "details": {"date of birth": "2000-01-01", "first name": "Asha"},
            "secrets": ["card number"],
            "rules": ["stop before paying"]
        })
    );
    let request: RunFlowRequest = serde_json::from_value(json!({
        "flow": {"app": "Mail", "steps": ["x"]},
        "votes": 3,
        "brief": {"goal": "g"}
    }))
    .unwrap();
    assert_eq!((request.votes, request.brief.goal.as_str()), (3, "g"));
    assert_eq!(
        request.max_do_turns, None,
        "unset keeps the module's default"
    );
    assert!(
        serde_json::to_value(&request)
            .unwrap()
            .get("max_do_turns")
            .is_none(),
        "an unset turn limit is left off the wire"
    );
    let request: RunFlowRequest = serde_json::from_value(json!({
        "flow": {"app": "Mail", "steps": ["x"]},
        "max_do_turns": 12
    }))
    .unwrap();
    assert_eq!(request.max_do_turns, Some(12));
    assert_eq!(serde_json::to_value(&request).unwrap()["max_do_turns"], 12);
    for (flow_loop, wire) in [
        (FlowLoop::Vote, "vote"),
        (FlowLoop::PageKind, "page_kind"),
        (FlowLoop::Validation, "validation"),
        (FlowLoop::Survey, "survey"),
        (FlowLoop::Digest, "digest"),
        (FlowLoop::Reflection, "reflection"),
        (FlowLoop::Evidence, "evidence"),
        (FlowLoop::Escalation, "escalation"),
        (FlowLoop::Duel, "duel"),
        (FlowLoop::TreeGrounding, "tree_grounding"),
        (FlowLoop::Denoise, "denoise"),
        (FlowLoop::Expectation, "expectation"),
        (FlowLoop::Checkpoint, "checkpoint"),
        (FlowLoop::Backtrack, "backtrack"),
        (FlowLoop::Attention, "attention"),
    ] {
        assert_eq!(serde_json::to_value(flow_loop).unwrap(), json!(wire));
    }
}

#[test]
fn a_deliberation_level_pins_its_wire_spelling_and_round_trips() {
    for (level, wire) in [
        (Deliberation::Off, "off"),
        (Deliberation::Standard, "standard"),
        (Deliberation::Deep, "deep"),
    ] {
        assert_eq!(serde_json::to_value(level).unwrap(), json!(wire));
        let request: RunFlowRequest = serde_json::from_value(json!({
            "flow": {"app": "Mail", "steps": ["x"]},
            "deliberation": wire
        }))
        .unwrap();
        assert_eq!(request.deliberation, level);
    }
    assert!(
        serde_json::from_value::<RunFlowRequest>(json!({
            "flow": {"app": "Mail", "steps": ["x"]},
            "deliberation": "maximum"
        }))
        .is_err(),
        "an unknown deliberation level is refused, never read as a default"
    );
}

#[test]
fn a_strategy_pins_its_wire_spelling_and_round_trips() {
    assert_eq!(
        serde_json::to_value(FlowStrategy::Wide).unwrap(),
        json!("wide")
    );
    assert_eq!(
        serde_json::to_value(FlowStrategy::Narrow).unwrap(),
        json!("narrow")
    );
    let request: RunFlowRequest = serde_json::from_value(json!({
        "flow": {"app": "Mail", "steps": ["x"]},
        "strategy": "wide"
    }))
    .unwrap();
    assert_eq!(request.strategy, FlowStrategy::Wide);
    assert_eq!(
        serde_json::to_value(&request).unwrap()["strategy"],
        json!("wide")
    );
    assert!(
        serde_json::from_value::<RunFlowRequest>(json!({
            "flow": {"app": "Mail", "steps": ["x"]},
            "strategy": "fastest"
        }))
        .is_err(),
        "an unknown strategy is refused, never read as a default"
    );
}

#[test]
fn flow_enums_and_hints_pin_their_wire_spelling() {
    assert_eq!(
        serde_json::to_value(FlowStopReason::StoppedBeforeDestructive).unwrap(),
        json!("stopped_before_destructive")
    );
    assert_eq!(
        serde_json::to_value(StepOutcome::AlreadyDone).unwrap(),
        json!("already_done")
    );
    assert_eq!(
        serde_json::to_value(FlowLoop::Corroboration).unwrap(),
        json!("corroboration")
    );
    let hint: GroundingHint =
        serde_json::from_value(json!({"app": "Mail", "key": "subject"})).unwrap();
    assert!(hint.name.is_none() && hint.path.is_empty());
    let slot = Slot {
        slot: "a".into(),
        text: "b".into(),
    };
    assert_eq!(Slots(vec![slot]).0.len(), 1);
}

#[test]
fn every_example_in_the_guide_is_a_valid_flow() {
    let mut examples = 0;
    for block in FLOW_GUIDE.split("```json").skip(1) {
        let body = block.split("```").next().expect("closed fence");
        let flow: Flow = serde_json::from_str(body)
            .unwrap_or_else(|error| panic!("guide example fails to parse: {error}\n{body}"));
        assert!(!flow.app.is_empty() && !flow.steps.is_empty());
        examples += 1;
    }
    assert!(examples >= 4, "the guide should keep its worked examples");
}
