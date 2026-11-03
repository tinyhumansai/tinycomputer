//! Tests for the safe debug view of browser action observations.

use super::{ActionObservation, ObservedAction, ObservedTarget};

#[test]
fn debug_output_omits_page_urls_and_accessible_names() {
    let target = ObservedTarget {
        role: "button".to_owned(),
        name: Some("private account name".to_owned()),
        reference: Some("@s1:e2".to_owned()),
        unique: true,
    };
    let target_debug = format!("{target:?}");
    assert!(target_debug.contains("button"));
    assert!(target_debug.contains("@s1:e2"));
    assert!(!target_debug.contains("private account name"));

    let observation = ActionObservation {
        task_id: Some("task-42".to_owned()),
        session_id: "session-7".to_owned(),
        action: ObservedAction::Click,
        before_url: "https://shop.test/private".to_owned(),
        after_url: "https://shop.test/receipt".to_owned(),
        target: Some(target),
    };
    let observation_debug = format!("{observation:?}");
    assert!(observation_debug.contains("Click"));
    assert!(observation_debug.contains("task-42"));
    assert!(observation_debug.contains("session-7"));
    assert!(!observation_debug.contains("shop.test"));
}
