//! Tests for Agent response version and outcome consistency.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::json;

use super::{AGENT_ENVELOPE_VERSION, AgentError, AgentResponse, AgentResponseError};

#[test]
fn agent_responses_carry_the_version_and_extract_success_data() {
    let response = AgentResponse::ok("done".to_owned());
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["version"], AGENT_ENVELOPE_VERSION);
    assert_eq!(response.into_data().unwrap(), "done");
}

#[test]
fn agent_responses_accept_unit_data_after_wire_round_trip() {
    let response = AgentResponse::ok(());
    let wire = serde_json::to_value(response).unwrap();
    let decoded: AgentResponse<()> = serde_json::from_value(wire).unwrap();

    assert!(matches!(decoded.into_data(), Ok(())));

    let response = AgentResponse::ok(None::<String>);
    let wire = serde_json::to_value(response).unwrap();
    let decoded: AgentResponse<Option<String>> = serde_json::from_value(wire).unwrap();
    assert!(matches!(decoded.into_data(), Ok(None)));
}

#[test]
fn agent_responses_still_reject_a_missing_success_value() {
    let response: AgentResponse<()> = serde_json::from_value(json!({
        "version": AGENT_ENVELOPE_VERSION,
        "ok": true
    }))
    .unwrap();

    assert!(matches!(
        response.into_data(),
        Err(AgentResponseError::InconsistentOutcome {
            ok: true,
            has_data: false,
            has_error: false
        })
    ));

    assert!(
        serde_json::from_value::<AgentResponse<String>>(json!({
            "version": AGENT_ENVELOPE_VERSION,
            "ok": true,
            "data": null
        }))
        .is_err()
    );
}

#[test]
fn agent_responses_reject_stale_versions_and_inconsistent_outcomes() {
    let stale: AgentResponse<String> = serde_json::from_value(json!({
        "ok": true,
        "data": "old module"
    }))
    .unwrap();
    assert!(matches!(
        stale.into_data(),
        Err(AgentResponseError::Version { actual, .. }) if actual.is_empty()
    ));

    let mut inconsistent = AgentResponse::ok("done".to_owned());
    inconsistent.error = Some(AgentError::new("BROKEN", "conflict", "retry", true));
    assert!(matches!(
        inconsistent.into_data(),
        Err(AgentResponseError::InconsistentOutcome {
            ok: true,
            has_data: true,
            has_error: true
        })
    ));
}

#[test]
fn agent_responses_return_structured_remote_errors() {
    let response: AgentResponse<String> = AgentResponse::err(AgentError::new(
        "NO_SUCH_TASK",
        "task is gone",
        "start it again",
        true,
    ));
    assert!(matches!(
        response.into_data(),
        Err(AgentResponseError::Remote(error)) if error.code == "NO_SUCH_TASK"
    ));
}
