//! Overflow recovery and bounded arrival order.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "controlled native fixtures assert completion and worker panic handling"
)]
use super::*;
#[test]
fn dropped_release_is_reported_as_continuity_loss() {
    let mut queue = Queue::default();
    queue.push("FN_UP".into());
    for _ in 0..CAPACITY {
        queue.push("FN_DOWN".into());
    }
    assert_eq!(queue.len(), CAPACITY);
    let (events, overflow) = queue.drain();
    assert!(overflow);
    assert_eq!(events.len(), CAPACITY);
    assert!(events.iter().all(|event| event == "FN_DOWN"));
    assert_eq!(queue.drain(), (Vec::new(), false));
}
#[test]
fn ordered_transitions_and_unknown_input_are_bounded() {
    let mut queue = Queue::default();
    queue.push("FN_DOWN".into());
    queue.push("FN_UP".into());
    assert_eq!(
        queue.drain(),
        (vec!["FN_DOWN".into(), "FN_UP".into()], false)
    );
    queue.discontinuity();
    assert_eq!(queue.drain(), (Vec::new(), true));
    queue.push("unexpected payload".into());
    assert_eq!(queue.drain(), (Vec::new(), true));
}
