//! Tests pinning the `ConsequenceHost` interface identity and wire forms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn the_interface_is_its_own_not_the_desktop_or_site_memory_one() {
    assert_ne!(INTERFACE, crate::names::INTERFACE);
    assert_ne!(INTERFACE, crate::site_memory::INTERFACE);
    assert_ne!(OBJECT_PATH, crate::site_memory::OBJECT_PATH);
    assert_eq!(METHODS, ["Review"]);
}

#[test]
fn a_review_request_keeps_its_wire_form() {
    let request: ReviewActionRequest = serde_json::from_value(serde_json::json!({
        "task": "t1", "action": "pay now", "target": "Pay", "kind": "payment",
        "host_context": "opaque-context"
    }))
    .unwrap();
    assert_eq!(request.kind, ConsequenceKind::Payment);
    assert_eq!(
        request
            .host_context
            .as_ref()
            .map(crate::agent::HostContext::as_str),
        Some("opaque-context")
    );
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["kind"], "payment");
    assert_eq!(json["host_context"], "opaque-context");
    let irreversible = serde_json::to_value(ConsequenceKind::Irreversible).unwrap();
    assert_eq!(irreversible, "irreversible");

    let legacy: ReviewActionRequest = serde_json::from_value(serde_json::json!({
        "task": "t2", "action": "send", "target": "Send", "kind": "irreversible"
    }))
    .unwrap();
    assert_eq!(legacy.host_context, None);
}

#[test]
fn a_reason_is_optional_and_left_out_when_absent() {
    let reply: ReviewActionResponse = serde_json::from_str(r#"{"allow":true}"#).unwrap();
    assert_eq!(reply.reason, None);
    let json = serde_json::to_value(&reply).unwrap();
    assert!(json.get("reason").is_none());
}
