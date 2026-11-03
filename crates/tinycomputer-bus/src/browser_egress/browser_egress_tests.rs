//! Browser egress callbacks carry destinations and opaque grants only.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn browser_callback_contract_has_no_http_payload_or_caller_selected_addresses() {
    let target = BrowserEgressTarget {
        policy_id: "private-policy".into(),
        expected_generation: 8,
        session_id: "private-session".into(),
        scheme: "https".into(),
        hostname: "example.test".into(),
        port: 443,
    };
    assert_eq!(
        serde_json::to_value(&target).unwrap(),
        serde_json::json!({
            "policy_id": "private-policy",
            "expected_generation": 8,
            "session_id": "private-session",
            "scheme": "https",
            "hostname": "example.test",
            "port": 443
        })
    );
    let grant = BrowserEgressGrant {
        generation: 8,
        authorization: "opaque serialized TinySecurity grant".into(),
        addresses: vec!["203.0.113.8:443".into()],
    };
    assert_eq!(
        serde_json::to_value(&grant).unwrap(),
        serde_json::json!({
            "generation": 8,
            "authorization": "opaque serialized TinySecurity grant",
            "addresses": ["203.0.113.8:443"]
        })
    );
    let consume = BrowserEgressConsumeRequest {
        target: target.clone(),
        authorization: "opaque serialized TinySecurity grant".into(),
        address: "203.0.113.8:443".into(),
    };
    assert_eq!(
        serde_json::to_value(&consume).unwrap(),
        serde_json::json!({
            "target": {
                "policy_id": "private-policy",
                "expected_generation": 8,
                "session_id": "private-session",
                "scheme": "https",
                "hostname": "example.test",
                "port": 443
            },
            "authorization": "opaque serialized TinySecurity grant",
            "address": "203.0.113.8:443"
        })
    );
    let validate = BrowserEgressValidateRequest {
        authorization: "opaque serialized TinySecurity grant".into(),
    };
    assert_eq!(
        serde_json::to_value(&validate).unwrap(),
        serde_json::json!({ "authorization": "opaque serialized TinySecurity grant" })
    );
    let decision = BrowserEgressDecision { allowed: true };
    assert_eq!(
        serde_json::to_value(decision).unwrap(),
        serde_json::json!({ "allowed": true })
    );
    assert_eq!(
        serde_json::from_value::<BrowserEgressTarget>(serde_json::to_value(&target).unwrap())
            .unwrap(),
        target
    );
    assert_eq!(
        serde_json::from_value::<BrowserEgressGrant>(serde_json::to_value(&grant).unwrap())
            .unwrap(),
        grant
    );
    assert_eq!(
        serde_json::from_value::<BrowserEgressConsumeRequest>(
            serde_json::to_value(&consume).unwrap()
        )
        .unwrap(),
        consume
    );
    assert_eq!(
        serde_json::from_value::<BrowserEgressValidateRequest>(
            serde_json::to_value(&validate).unwrap()
        )
        .unwrap(),
        validate
    );
    assert_eq!(
        serde_json::from_value::<BrowserEgressDecision>(serde_json::to_value(decision).unwrap())
            .unwrap(),
        decision
    );
    assert_eq!(METHODS, ["Authorize", "Consume", "Validate"]);
}

#[test]
fn callback_types_reject_unrecognized_fields() {
    let malformed = serde_json::json!({
        "policy_id": "p", "expected_generation": 1, "session_id": "s",
        "scheme": "https", "hostname": "example.test", "port": 443,
        "headers": ["secret"]
    });
    assert!(serde_json::from_value::<BrowserEgressTarget>(malformed).is_err());
}

#[test]
fn callback_debug_redacts_policy_destination_and_grant_material() {
    let target = BrowserEgressTarget {
        policy_id: "secret-subject-policy".into(),
        expected_generation: 9,
        session_id: "private-browser-session".into(),
        scheme: "https".into(),
        hostname: "private.example.test".into(),
        port: 443,
    };
    let grant = BrowserEgressGrant {
        generation: 9,
        authorization: "private-one-use-nonce".into(),
        addresses: vec!["198.51.100.99:443".into()],
    };
    assert!(format!("{target:?}").contains("REDACTED"));
    let consume = BrowserEgressConsumeRequest {
        target,
        authorization: grant.authorization.clone(),
        address: grant.addresses[0].clone(),
    };
    let validate = BrowserEgressValidateRequest {
        authorization: grant.authorization.clone(),
    };
    for debug in [
        format!("{consume:?}"),
        format!("{validate:?}"),
        format!("{grant:?}"),
    ] {
        for private in [
            "secret-subject-policy",
            "private-browser-session",
            "private.example.test",
            "private-one-use-nonce",
            "198.51.100.99",
        ] {
            assert!(!debug.contains(private));
        }
        assert!(debug.contains("REDACTED"));
    }
}
