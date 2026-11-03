//! Turning an engine result into the response envelope.
//!
//! The mapping is deliberately lossless in the direction that matters. An
//! `AppError::Adapter` already carries a code, a platform detail, structured
//! details, and a delivery disposition; those are copied across rather than
//! rendered into the message, because the whole reason the envelope has fields
//! for them is that a caller acts on them. An error of any other shape has only
//! a message and a code, so that is what it becomes.

use agent_desktop_core::{AppError, DeliverySemantics, RetryDisposition, output::ErrorPayload};
use serde_json::Value;
use tinycomputer_bus::{
    Delivery, DeliveryDisposition, DesktopError, DesktopResponse, RecoveryHint,
    RetryDisposition as ContractRetry,
};

/// Wraps `result` in the envelope, naming it `command`.
pub(super) fn envelope(command: &str, result: Result<Value, AppError>) -> DesktopResponse {
    match result {
        Ok(mut data) => {
            normalize_surface_fields(&mut data);
            DesktopResponse::ok(command, data)
        }
        Err(error) => DesktopResponse::err(command, desktop_error(&error)),
    }
}

/// Converts an engine error into the contract's error payload.
///
/// The engine's own [`ErrorPayload`] does this work already — including
/// choosing the recovery strategy for a retry-safe code — so this reuses it
/// rather than reimplementing the mapping and drifting from it.
fn desktop_error(error: &AppError) -> DesktopError {
    let mut payload = ErrorPayload::from_app_error(error);
    if let Some(details) = &mut payload.details {
        normalize_surface_fields(details);
    }

    DesktopError {
        code: payload.code,
        message: payload.message,
        suggestion: payload.suggestion,
        recovery: payload.recovery.map(|hint| RecoveryHint {
            strategy: hint.strategy,
            retryable: hint.retryable,
            requires_fresh_snapshot: hint.requires_fresh_snapshot,
            retry_after_ms: hint.retry_after_ms,
        }),
        platform_detail: payload.platform_detail,
        details: payload.details,
        disposition: delivery(payload.disposition),
    }
}

/// Restores the contract's `snake_case` spellings where the engine uses
/// `SnapshotSurface::as_str()` values in structured output.
fn normalize_surface_fields(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                normalize_surface_fields(value);
            }
        }
        Value::Object(fields) => {
            for (field, value) in fields {
                match field.as_str() {
                    "surface" | "requested_surface" | "type" => {
                        if let Some(surface) = value.as_str().and_then(contract_surface_name) {
                            *value = Value::String(surface.to_owned());
                        }
                    }
                    "supported_surfaces" | "unclassified" => {
                        if let Value::Array(surfaces) = value {
                            for value in surfaces {
                                if let Some(surface) =
                                    value.as_str().and_then(contract_surface_name)
                                {
                                    *value = Value::String(surface.to_owned());
                                }
                            }
                        }
                    }
                    _ => normalize_surface_fields(value),
                }
            }
        }
        _ => {}
    }
}

/// Returns the contract spelling for an engine surface alias.
fn contract_surface_name(surface: &str) -> Option<&'static str> {
    match surface {
        "system-tray" => Some("system_tray"),
        "quick-settings" => Some("quick_settings"),
        "notification-center" => Some("notification_center"),
        "menu-bar-extras" => Some("menu_bar_extras"),
        "system-tray-overflow" => Some("system_tray_overflow"),
        "start-menu" => Some("start_menu"),
        "action-center" => Some("action_center"),
        _ => None,
    }
}

/// Converts the engine's delivery semantics into the contract's pair.
fn delivery(semantics: DeliverySemantics) -> Delivery {
    let disposition = match semantics {
        DeliverySemantics::Unknown => DeliveryDisposition::Unknown,
        DeliverySemantics::NotDelivered => DeliveryDisposition::NotDelivered,
        DeliverySemantics::DeliveryUncertain => DeliveryDisposition::DeliveryUncertain,
        DeliverySemantics::DeliveredUnverified => DeliveryDisposition::DeliveredUnverified,
        DeliverySemantics::DeliveredVerified => DeliveryDisposition::DeliveredVerified,
    };
    let retry = match semantics.retry() {
        RetryDisposition::Unknown => ContractRetry::Unknown,
        RetryDisposition::Safe => ContractRetry::Safe,
        RetryDisposition::Unsafe => ContractRetry::Unsafe,
    };

    Delivery {
        delivery: disposition,
        retry,
    }
}
