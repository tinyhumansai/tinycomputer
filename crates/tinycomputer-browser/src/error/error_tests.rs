//! Tests for the crate-wide error type.
//!
//! The point of these is the mapping: a variant whose wire name is wrong sends a
//! host down the wrong recovery path, and nothing else in the build checks it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::Error;
use tinycomputer_bus::browser::errors;
use tinycomputer_bus::{DeliveryDisposition, RetryDisposition};

#[test]
fn every_variant_maps_to_a_published_name() {
    for error in every_variant() {
        assert!(
            errors::NAMES.contains(&error.wire_name()),
            "{error} maps to {}, which the contract does not publish",
            error.wire_name()
        );
    }
}

#[test]
fn the_variants_a_model_can_fix_are_the_recoverable_ones() {
    assert!(errors::is_agent_recoverable(
        Error::invalid_input("bad").wire_name()
    ));
    assert!(errors::is_agent_recoverable(
        Error::not_actionable("covered").wire_name()
    ));
    assert!(errors::is_agent_recoverable(
        Error::timeout("navigate", 30_000).wire_name()
    ));
    assert!(errors::is_agent_recoverable(
        Error::NoSuchElement {
            target: "@e1".to_string()
        }
        .wire_name()
    ));
}

#[test]
fn deployment_problems_are_not_offered_back_to_the_model() {
    assert!(!errors::is_agent_recoverable(
        Error::browser_unavailable("no chrome").wire_name()
    ));
    assert!(!errors::is_agent_recoverable(
        Error::BlockedByPolicy {
            url: "https://evil.test/".to_string()
        }
        .wire_name()
    ));
}

#[test]
fn a_lost_connection_tells_the_host_to_open_a_new_session() {
    // Not `ModuleFailed`: the session is gone, and `NoSuchSession` is the name
    // that makes a host reopen rather than retry into a dead socket.
    assert_eq!(
        Error::connection_lost("websocket closed").wire_name(),
        errors::NO_SUCH_SESSION
    );
}

#[test]
fn stale_ref_tells_the_caller_to_snapshot_again() {
    let error = Error::StaleRef {
        reference: "e12".to_string(),
    };

    assert_eq!(error.wire_name(), errors::STALE_REF);
    assert_eq!(
        error.to_string(),
        "ref @e12 is not in the latest snapshot, take a fresh one"
    );
}

#[test]
fn messages_are_lowercase_and_unpunctuated() {
    for error in every_variant() {
        let rendered = error.to_string();
        let first = rendered.chars().next().expect("a message");

        assert!(
            !first.is_uppercase(),
            "{rendered} starts with a capital letter"
        );
        assert!(!rendered.ends_with('.'), "{rendered} ends with a full stop");
    }
}

/// One of every variant, so the mapping tests cannot miss a new one.
fn every_variant() -> Vec<Error> {
    vec![
        Error::invalid_input("bad url"),
        Error::NoSuchSession {
            id: "s-1".to_string(),
        },
        Error::NoSuchElement {
            target: "@e1".to_string(),
        },
        Error::StaleRef {
            reference: "e1".to_string(),
        },
        Error::not_actionable("covered by a banner"),
        Error::timeout("navigate", 30_000),
        Error::BlockedByPolicy {
            url: "https://evil.test/".to_string(),
        },
        Error::LeftRefusedPage {
            url: "https://evil.test/".to_string(),
        },
        Error::browser_unavailable("no chrome on this host"),
        Error::page("ReferenceError: x is not defined"),
        Error::NoSuchOutput {
            id: "o-1".to_string(),
        },
        Error::LimitExceeded {
            message: "too many sessions".to_string(),
        },
        Error::connection_lost("websocket closed"),
        Error::failed("something else"),
    ]
}

#[test]
fn every_variant_has_an_envelope_code_and_its_wire_name() {
    for error in every_variant() {
        let envelope = error.envelope();
        assert_eq!(envelope.code, errors::code(error.wire_name()));
        assert_eq!(envelope.message, error.to_string());
        let details = envelope.details.expect("details carry the wire name");
        assert_eq!(details["name"], error.wire_name());
        assert_eq!(
            details["agent_recoverable"],
            errors::is_agent_recoverable(error.wire_name())
        );
    }
}

#[test]
fn a_stale_ref_envelope_matches_the_desktop_recovery() {
    let envelope = Error::StaleRef {
        reference: "e3".to_owned(),
    }
    .envelope();
    assert_eq!(envelope.code, "STALE_REF");
    let hint = envelope.recovery.expect("a stale ref has a way out");
    assert_eq!(hint.strategy, "refresh_snapshot_then_retry_original");
    assert!(
        envelope
            .suggestion
            .is_some_and(|s| s.contains("BrowserSnapshot"))
    );
    // agent-browser could not resolve the ref, so nothing reached the page:
    // retrying with a fresh ref cannot repeat an effect.
    assert_eq!(envelope.disposition.retry, RetryDisposition::Safe);
}

#[test]
fn a_timeout_may_have_reached_the_page() {
    let envelope = Error::timeout("navigate", 30_000).envelope();
    assert_eq!(envelope.code, "TIMEOUT");
    assert_eq!(envelope.disposition.delivery, DeliveryDisposition::Unknown);
    assert!(envelope.suggestion.is_none());
    // A click may already have landed: inspect before repeating it.
    let hint = envelope.recovery.expect("a timeout has a way out");
    assert_eq!(hint.strategy, "inspect_state_then_retry_original");
    assert!(hint.requires_fresh_snapshot);
}

#[test]
fn a_refused_navigation_says_not_to_retry() {
    let envelope = Error::BlockedByPolicy {
        url: "https://evil.test/".to_owned(),
    }
    .envelope();
    assert_eq!(envelope.code, "POLICY_DENIED");
    assert!(envelope.recovery.is_none());
    assert!(
        envelope
            .suggestion
            .is_some_and(|s| s.starts_with("do not retry"))
    );
    // The allowed origins refuse before any navigation reaches the page.
    assert_eq!(
        envelope.disposition.delivery,
        DeliveryDisposition::NotDelivered
    );
}

#[test]
fn a_page_refused_after_the_call_ran_is_the_same_refusal_but_may_have_taken_effect() {
    // A click on a listed site's "Pay" that lands on an unlisted bank page
    // paid: a host that retried "nothing was delivered" would pay twice.
    let envelope = Error::LeftRefusedPage {
        url: "https://bank.test/3ds".to_owned(),
    }
    .envelope();
    assert_eq!(envelope.code, "POLICY_DENIED");
    assert_eq!(envelope.disposition.delivery, DeliveryDisposition::Unknown);
    assert!(
        envelope
            .suggestion
            .is_some_and(|s| s.contains("check what it did"))
    );
}

#[test]
fn only_failures_decided_before_the_page_claim_nothing_was_delivered() {
    let before_the_page = |error: &Error| {
        matches!(
            error,
            Error::NoSuchSession { .. }
                | Error::NoSuchOutput { .. }
                | Error::StaleRef { .. }
                | Error::BlockedByPolicy { .. }
        )
    };
    for error in every_variant() {
        let expected = before_the_page(&error);
        let name = error.to_string();
        let delivery = error.envelope().disposition.delivery;
        assert_eq!(
            delivery == DeliveryDisposition::NotDelivered,
            expected,
            "{name}"
        );
    }
}

#[test]
fn a_lost_connection_suggests_a_new_session() {
    let envelope = Error::connection_lost("closed").envelope();
    assert_eq!(envelope.code, "SESSION_NOT_FOUND");
    assert!(
        envelope
            .suggestion
            .is_some_and(|s| s.contains("BrowserOpenSession"))
    );
}
