//! Unit tests pinning the response envelope's wire form.
//!
//! The envelope is the one payload both halves of the system parse, so its
//! field names are the contract. These tests fail if a rename would silently
//! turn a structured error into an absent one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    Delivery, DeliveryDisposition, DesktopError, DesktopResponse, DesktopResponseError,
    ENVELOPE_VERSION, RecoveryHint, RetryDisposition,
};
use serde::Deserialize;
use serde_json::json;

#[test]
fn the_envelope_version_matches_the_engine() {
    assert_eq!(ENVELOPE_VERSION, "2.5");
}

#[test]
fn a_successful_reply_serializes_without_an_error_key() {
    let value = serde_json::to_value(DesktopResponse::ok("list-apps", json!({ "apps": [] })))
        .expect("a response serializes");

    assert_eq!(
        value,
        json!({
            "version": "2.5",
            "ok": true,
            "command": "list-apps",
            "data": { "apps": [] },
        })
    );
}

#[test]
fn a_failed_reply_serializes_without_a_data_key() {
    let value = serde_json::to_value(DesktopResponse::err(
        "click",
        DesktopError::new("STALE_REF", "ref is no longer valid")
            .with_suggestion("take a fresh snapshot"),
    ))
    .expect("a response serializes");

    assert_eq!(
        value,
        json!({
            "version": "2.5",
            "ok": false,
            "command": "click",
            "error": {
                "code": "STALE_REF",
                "message": "ref is no longer valid",
                "suggestion": "take a fresh snapshot",
                "disposition": { "delivery": "unknown", "retry": "unknown" },
            },
        })
    );
}

#[test]
fn an_engine_reply_round_trips_through_the_envelope() {
    // Verbatim output of `agent-desktop click @s1:e2 --json` on a stale ref,
    // which is what a host actually has to parse.
    let wire = json!({
        "version": "2.5",
        "ok": false,
        "command": "click",
        "error": {
            "code": "STALE_REF",
            "message": "Ref @s1:e2 is no longer valid",
            "suggestion": "Take a fresh snapshot",
            "recovery": {
                "strategy": "refresh_snapshot_then_retry_original",
                "retryable": true,
                "requires_fresh_snapshot": true,
            },
            "disposition": { "delivery": "not_delivered", "retry": "safe" },
        },
    });

    let reply: DesktopResponse = serde_json::from_value(wire.clone()).expect("the reply decodes");
    let error = reply
        .error
        .as_ref()
        .expect("a failed reply carries an error");

    assert!(!reply.ok);
    assert_eq!(error.code, "STALE_REF");
    assert_eq!(
        error
            .recovery
            .as_ref()
            .map(|hint| hint.requires_fresh_snapshot),
        Some(true)
    );
    assert_eq!(error.disposition.retry, RetryDisposition::Safe);
    assert_eq!(serde_json::to_value(&reply).expect("it re-encodes"), wire);
}

#[test]
fn a_recovery_hint_omits_an_absent_retry_delay() {
    let value = serde_json::to_value(RecoveryHint {
        strategy: "refresh_snapshot_then_retry_original".to_owned(),
        retryable: true,
        requires_fresh_snapshot: true,
        retry_after_ms: None,
    })
    .expect("a hint serializes");

    assert!(value.get("retry_after_ms").is_none());
}

#[test]
fn a_delivery_pair_follows_from_its_disposition() {
    assert_eq!(
        Delivery::of(DeliveryDisposition::NotDelivered).retry,
        RetryDisposition::Safe
    );
    for delivered in [
        DeliveryDisposition::DeliveryUncertain,
        DeliveryDisposition::DeliveredUnverified,
        DeliveryDisposition::DeliveredVerified,
    ] {
        assert_eq!(Delivery::of(delivered).retry, RetryDisposition::Unsafe);
    }
    assert_eq!(
        Delivery::of(DeliveryDisposition::Unknown).retry,
        RetryDisposition::Unknown
    );
}

#[test]
fn an_absent_disposition_decodes_as_unknown() {
    let error: DesktopError =
        serde_json::from_value(json!({ "code": "INTERNAL", "message": "boom" }))
            .expect("the error decodes");

    assert_eq!(error.disposition, Delivery::default());
    assert_eq!(error.disposition.delivery, DeliveryDisposition::Unknown);
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Payload {
    value: String,
}

#[test]
fn into_data_checks_version_and_decodes_a_successful_reply() {
    let reply = DesktopResponse::ok("example", json!({"value": "done"}));
    let payload: Payload = reply.into_data().expect("success payload decodes");
    assert_eq!(
        payload,
        Payload {
            value: "done".into()
        }
    );

    let mut stale = DesktopResponse::ok("example", json!({"value": "done"}));
    stale.version = "2.4".into();
    assert!(matches!(
        stale.into_data::<Payload>(),
        Err(DesktopResponseError::Version { actual, .. }) if actual == "2.4"
    ));
}

#[test]
fn into_data_accepts_a_present_null_value_after_wire_round_trip() {
    let reply = DesktopResponse::ok("noop", json!(null));
    let wire = serde_json::to_value(reply).expect("response serializes");
    let decoded: DesktopResponse = serde_json::from_value(wire).expect("wire response decodes");

    assert!(matches!(decoded.into_data::<Option<String>>(), Ok(None)));
}

#[test]
fn into_data_still_rejects_a_missing_success_value() {
    let decoded: DesktopResponse = serde_json::from_value(json!({
        "version": "2.5",
        "ok": true,
        "command": "noop"
    }))
    .expect("wire response decodes");

    assert!(matches!(
        decoded.into_data::<()>(),
        Err(DesktopResponseError::InconsistentOutcome {
            ok: true,
            has_data: false,
            has_error: false
        })
    ));
}

#[test]
fn into_data_returns_remote_errors_and_rejects_inconsistent_outcomes() {
    let failed = DesktopResponse::err("example", DesktopError::new("DENIED", "refused"));
    assert!(matches!(
        failed.into_data::<Payload>(),
        Err(DesktopResponseError::Remote(error)) if error.code == "DENIED"
    ));

    let mut inconsistent = DesktopResponse::ok("example", json!({"value": "done"}));
    inconsistent.error = Some(DesktopError::new("DENIED", "contradictory"));
    assert!(matches!(
        inconsistent.into_data::<Payload>(),
        Err(DesktopResponseError::InconsistentOutcome {
            ok: true,
            has_data: true,
            has_error: true
        })
    ));
}

#[test]
fn into_data_rejects_data_with_the_wrong_shape() {
    let reply = DesktopResponse::ok("example", json!({"value": 4}));
    assert!(matches!(
        reply.into_data::<Payload>(),
        Err(DesktopResponseError::Decode(_))
    ));
}
