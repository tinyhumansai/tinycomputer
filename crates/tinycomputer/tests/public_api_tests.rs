//! Integration tests against this crate's public API.
//!
//! These exercise the crate the way a dependent would: only what `src/lib.rs`
//! exports, with no reach into private items. They are the regression suite for
//! the crate's public contract — the surface a host binds to and the surface
//! that cannot change without a version bump.
//!
//! Nothing here drives a real application. Every assertion holds on a machine
//! with no permission granted, no display server, and nothing running, because
//! CI is such a machine and a test that only passes on a developer's desktop is
//! a test nobody runs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinycomputer::{
    CONTRACT_VERSION, Desktop, ENVELOPE_VERSION, ElementProperty, GetRequest, INTERFACE,
    ListAppsRequest, METHODS, OBJECT_PATH, PermissionsRequest, RefRequest, SnapshotRequest,
    WaitRequest, is_compatible, names,
};

#[test]
fn the_crate_re_exports_the_contract_rather_than_redefining_it() {
    // The same type by both paths, not a structural twin: a host may depend on
    // `tinycomputer-bus` alone and hand its values straight to this crate.
    let request: tinycomputer_bus::RefRequest = RefRequest::new("@s1:e2");
    let reply = Desktop::new().click(request);

    assert_eq!(reply.command, "click");
    assert_eq!(INTERFACE, tinycomputer_bus::INTERFACE);
    assert_eq!(OBJECT_PATH, tinycomputer_bus::OBJECT_PATH);
    assert_eq!(METHODS, tinycomputer_bus::METHODS);
}

#[test]
fn the_shipped_contract_binds_to_itself() {
    assert!(is_compatible(CONTRACT_VERSION));
    assert_eq!(CONTRACT_VERSION, (2, 9));
}

#[test]
fn the_module_serves_every_member_the_contract_names() {
    assert_eq!(METHODS.len(), 80);
    assert!(METHODS.contains(&names::methods::SNAPSHOT));
    assert!(METHODS.contains(&names::methods::START_TASK));
    assert!(METHODS.contains(&names::methods::PERMISSIONS));
}

#[test]
fn version_succeeds_on_an_unconfigured_machine() {
    let reply = Desktop::new().version();

    assert!(reply.ok);
    assert_eq!(reply.version, ENVELOPE_VERSION);
    assert_eq!(reply.command, "version");
    let data = reply.data.expect("a successful reply carries data");
    assert_eq!(data["os"], serde_json::json!(std::env::consts::OS));
}

#[test]
fn a_reply_carries_data_or_an_error_but_never_both_and_never_neither() {
    // Every member on the same envelope contract, whatever this machine
    // happens to allow.
    let replies = [
        Desktop::new().version(),
        Desktop::new().list_apps(ListAppsRequest::default()),
        Desktop::new().permissions(PermissionsRequest::default()),
        Desktop::new().snapshot(SnapshotRequest::default()),
        Desktop::new().get(GetRequest::new("@s1:e2", ElementProperty::Role)),
    ];

    for reply in replies {
        assert_eq!(reply.version, ENVELOPE_VERSION);
        assert_ne!(reply.command, "");
        assert_eq!(reply.ok, reply.data.is_some());
        assert_eq!(!reply.ok, reply.error.is_some());
    }
}

#[test]
fn a_failed_reply_carries_a_machine_readable_code() {
    // An empty ref cannot resolve anywhere, so this fails on every platform —
    // with a code rather than only a sentence.
    let reply = Desktop::new().get(GetRequest::new("", ElementProperty::Role));

    let error = reply.error.expect("an empty ref cannot succeed");
    assert_ne!(error.code, "");
    assert!(
        error
            .code
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_')
    );
    assert_ne!(error.message, "");
}

#[test]
fn a_sleep_is_the_one_wait_that_needs_nothing_granted() {
    let reply = Desktop::new().wait(WaitRequest::sleep(1));

    assert!(reply.ok, "a sleep touches no other application");
    assert_eq!(reply.data.expect("data")["waited_ms"], serde_json::json!(1));
}

#[test]
fn a_configured_desktop_reports_what_it_was_configured_with() {
    let desktop = Desktop::from_config(&serde_json::json!({
        "session_id": "public-api-test",
        "headed": true,
    }))
    .expect("a valid configuration");

    assert_eq!(desktop.session_id(), Some("public-api-test"));
    assert!(desktop.is_headed());
    assert!(!desktop.is_tracing());
}

#[test]
fn an_invalid_configuration_is_an_error_a_caller_can_match_on() {
    let error = Desktop::from_config(&serde_json::json!({ "headed": 1 }))
        .expect_err("a number is not a boolean");

    assert!(matches!(
        error,
        tinycomputer::Error::ConfigFieldType {
            field: "headed",
            ..
        }
    ));
}

#[test]
fn the_reserved_hold_members_fail_closed_and_say_what_to_use_instead() {
    let reply = Desktop::new().mouse_down(tinycomputer::HoldMouseRequest::default());

    assert!(!reply.ok, "a stateless module cannot hold a button down");
    let error = reply.error.expect("a refusal carries an error");
    assert!(
        error.suggestion.is_some() || !error.message.is_empty(),
        "the refusal must say what to do instead"
    );
}
