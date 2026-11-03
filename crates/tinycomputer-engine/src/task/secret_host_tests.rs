//! Tests for tests behavior.

use super::SecretTypeRequest;

#[test]
fn debug_hides_the_opaque_handle_and_keeps_safe_type_context() {
    let request = SecretTypeRequest {
        handle: "secret-value-must-not-appear".into(),
        mask: "•••• 1111".into(),
        kind: "payment_card".into(),
        field_role: "textbox".into(),
        field_name: Some("Card number".into()),
        field_ref: "field-7".into(),
        current_url: "https://shop.example/checkout".into(),
        task_id: "task-2".into(),
        actions_remaining: 3,
    };

    let debug = format!("{request:?}");

    assert!(!debug.contains(&request.handle));
    assert!(debug.contains("[opaque]"));
    assert!(debug.contains("field-7"));
    assert!(debug.contains("actions_remaining: 3"));
}
