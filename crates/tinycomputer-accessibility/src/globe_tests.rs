//! Unit tests for the Globe listener's bounded event queue and platform fallbacks.

#![allow(clippy::expect_used)]

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_listener_entry_points_report_unsupported() {
    let started = super::globe_listener_start().expect("fallback start returns status");
    assert!(!started.supported);
    assert!(!started.running);
    assert_eq!(
        started.input_monitoring_permission,
        super::PermissionState::Unsupported
    );
    assert_eq!(started.events_pending, 0);

    let polled = super::globe_listener_poll().expect("fallback poll returns status");
    assert!(!polled.status.supported);
    assert!(!polled.status.running);
    assert_eq!(polled.events.len(), 0);

    let stopped = super::globe_listener_stop().expect("fallback stop returns status");
    assert!(!stopped.supported);
    assert!(!stopped.running);
}
