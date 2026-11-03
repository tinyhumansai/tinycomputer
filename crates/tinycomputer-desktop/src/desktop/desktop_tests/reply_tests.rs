//! Tests for the envelope: successes, structured errors, and delivery
//! dispositions.

use serde_json::json;
use tinycomputer_bus as bus;

use crate::desktop::{Desktop, reply};
use agent_desktop_core::{AdapterError, AppError, DeliverySemantics, ErrorCode};

#[test]
fn a_command_that_fails_still_names_itself_in_the_envelope() {
    // Whatever this machine says about the request, the command name is the
    // one field a batched caller correlates on, so it must be present on both
    // outcomes.
    let reply = Desktop::new().get(bus::GetRequest::new("", bus::ElementProperty::Value));

    assert_eq!(reply.command, "get");
    assert_eq!(reply.version, bus::ENVELOPE_VERSION);
    assert_eq!(reply.ok, reply.error.is_none());
}

#[test]
fn a_successful_result_becomes_a_successful_envelope() {
    let envelope = reply::envelope("version", Ok(json!({ "version": "0.8.3" })));

    assert!(envelope.ok);
    assert_eq!(envelope.command, "version");
    assert_eq!(envelope.data, Some(json!({ "version": "0.8.3" })));
    assert!(envelope.error.is_none());
}

#[test]
fn surface_names_in_success_data_use_the_contract_wire_spelling() {
    let envelope = reply::envelope(
        "list-surfaces",
        Ok(json!({
            "surfaces": [{
                "type": "system-tray",
                "unclassified": ["notification-center"]
            }]
        })),
    );

    assert_eq!(
        envelope.data,
        Some(json!({
            "surfaces": [{
                "type": "system_tray",
                "unclassified": ["notification_center"]
            }]
        }))
    );
}

#[test]
fn supported_surface_names_in_error_details_use_the_contract_wire_spelling() {
    let error = AppError::Adapter(
        AdapterError::new(ErrorCode::PlatformNotSupported, "surface not supported").with_details(
            json!({
                "requested_surface": "quick-settings",
                "supported_surfaces": ["system-tray", "notification-center"]
            }),
        ),
    );

    let payload = reply::envelope("snapshot", Err(error))
        .error
        .expect("a failed envelope carries an error");

    assert_eq!(
        payload.details,
        Some(json!({
            "requested_surface": "quick_settings",
            "supported_surfaces": ["system_tray", "notification_center"]
        }))
    );
}

#[test]
fn an_adapter_error_keeps_its_structured_detail_through_the_envelope() {
    let error = AppError::Adapter(
        AdapterError::new(ErrorCode::StaleRef, "Ref @s1:e2 is no longer valid")
            .with_suggestion("Take a fresh snapshot")
            .with_details(json!({ "ref": "@s1:e2" }))
            .with_disposition(DeliverySemantics::not_delivered()),
    );

    let payload = reply::envelope("click", Err(error))
        .error
        .expect("a failed envelope carries an error");

    assert_eq!(payload.code, "STALE_REF");
    assert_eq!(payload.suggestion.as_deref(), Some("Take a fresh snapshot"));
    assert_eq!(payload.details, Some(json!({ "ref": "@s1:e2" })));
    assert_eq!(payload.disposition.retry, bus::RetryDisposition::Safe);
    // A retry-safe stale ref is exactly the case that gets a recovery hint.
    let recovery = payload
        .recovery
        .expect("a stale ref carries a recovery hint");
    assert!(recovery.requires_fresh_snapshot);
}

#[test]
fn every_delivery_disposition_survives_the_envelope_with_its_retry_verdict() {
    for (semantics, expected_delivery, expected_retry) in [
        (
            DeliverySemantics::unknown(),
            bus::DeliveryDisposition::Unknown,
            bus::RetryDisposition::Unknown,
        ),
        (
            DeliverySemantics::not_delivered(),
            bus::DeliveryDisposition::NotDelivered,
            bus::RetryDisposition::Safe,
        ),
        (
            DeliverySemantics::uncertain(),
            bus::DeliveryDisposition::DeliveryUncertain,
            bus::RetryDisposition::Unsafe,
        ),
        (
            DeliverySemantics::delivered_unverified(),
            bus::DeliveryDisposition::DeliveredUnverified,
            bus::RetryDisposition::Unsafe,
        ),
        (
            DeliverySemantics::delivered_verified(),
            bus::DeliveryDisposition::DeliveredVerified,
            bus::RetryDisposition::Unsafe,
        ),
    ] {
        let error = AppError::Adapter(
            AdapterError::new(ErrorCode::ActionFailed, "the action did not take")
                .with_disposition(semantics),
        );
        let payload = reply::envelope("click", Err(error))
            .error
            .expect("a failed envelope carries an error");

        assert_eq!(payload.disposition.delivery, expected_delivery);
        assert_eq!(payload.disposition.retry, expected_retry);
        // The pair the engine reports and the pair the contract derives are the
        // same pair; a caller may trust either field.
        assert_eq!(payload.disposition, bus::Delivery::of(expected_delivery));
    }
}

#[test]
fn a_rejected_argument_is_reported_as_never_having_been_delivered() {
    // An argument the engine refuses never reached the application, so a retry
    // with a corrected argument cannot duplicate an effect. The envelope has to
    // carry that, or a caller has to guess.
    let payload = reply::envelope("find", Err(AppError::invalid_input("no mode selected")))
        .error
        .expect("a failed envelope carries an error");

    assert_eq!(payload.code, ErrorCode::InvalidArgs.as_str());
    assert!(payload.platform_detail.is_none());
    assert!(payload.details.is_none());
    assert_eq!(payload.disposition.retry, bus::RetryDisposition::Safe);
}

#[test]
fn an_error_the_engine_cannot_classify_becomes_an_internal_one() {
    // `Internal` carries no code of its own, so this is the branch of the
    // mapping that must not assume an adapter error.
    let payload = reply::envelope("status", Err(AppError::Internal("boom".to_owned())))
        .error
        .expect("a failed envelope carries an error");

    assert_eq!(payload.code, ErrorCode::Internal.as_str());
    assert_eq!(payload.message, "boom");
    assert!(payload.suggestion.is_none());
    assert!(payload.recovery.is_none());
    assert_eq!(payload.disposition, bus::Delivery::default());
}
