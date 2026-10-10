//! Accessibility vocabulary keeps its existing JSON representations.
#![allow(
    clippy::expect_used,
    reason = "test assertions require fixture operations to succeed"
)]
use super::*;

#[test]
fn permission_states_and_kinds_keep_their_snake_case_wire() {
    for (state, wire) in [
        (PermissionState::Granted, "granted"),
        (PermissionState::Denied, "denied"),
        (PermissionState::Unknown, "unknown"),
        (PermissionState::Unsupported, "unsupported"),
    ] {
        assert_eq!(
            serde_json::to_value(&state).expect("serialize"),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<PermissionState>(serde_json::json!(wire)).expect("decode"),
            state
        );
    }
    assert_eq!(
        serde_json::to_value(PermissionKind::InputMonitoring).expect("serialize"),
        serde_json::json!("input_monitoring")
    );
}

#[test]
fn focus_context_and_globe_events_round_trip_without_native_dependencies() {
    let context = FocusedTextContext {
        app_name: Some("Editor".into()),
        role: Some("AXTextArea".into()),
        text: "text".into(),
        selected_text: None,
        raw_error: None,
        bounds: Some(ElementBounds {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        }),
    };
    assert_eq!(
        serde_json::from_value::<FocusedTextContext>(
            serde_json::to_value(&context).expect("serialize")
        )
        .expect("decode"),
        context
    );
    let batch = GlobeHotkeyPollResult {
        status: GlobeHotkeyStatus {
            supported: true,
            running: true,
            input_monitoring_permission: PermissionState::Granted,
            last_error: None,
            events_pending: 0,
        },
        events: vec!["FN_DOWN".into(), "FN_UP".into()],
    };
    let wire = serde_json::to_value(&batch).expect("serialize");
    assert_eq!(wire["events"], serde_json::json!(["FN_DOWN", "FN_UP"]));
    let decoded: GlobeHotkeyPollResult = serde_json::from_value(wire).expect("decode");
    assert_eq!(decoded.events, batch.events);
}
