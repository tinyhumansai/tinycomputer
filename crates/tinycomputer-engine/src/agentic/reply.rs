//! The reply envelopes `RunGoal` and `ResolveIntent` return, the metrics
//! they carry, and the errors Jev and the desktop map to.

#[cfg(feature = "desktop")]
use tinycomputer_bus::{
    DeliveryDisposition, JevDecision, JevObservation, JevRunResult, JevStopReason, JevTurn,
};
use tinycomputer_bus::{DesktopError, DesktopResponse, JevMetrics};
use tinyinference_decisions::{Error as JevError, EvaluationResult};

#[cfg(feature = "desktop")]
use super::verify::satisfied;

#[cfg(feature = "desktop")]
pub(super) fn action_failed_response(
    mut turns: Vec<JevTurn>,
    decision: JevDecision,
    metrics: JevMetrics,
    failure: &DesktopResponse,
) -> DesktopResponse {
    turns.push(failed_turn(&turns, &decision));
    let stop = match failure
        .error
        .as_ref()
        .map(|error| error.disposition.delivery)
    {
        Some(DeliveryDisposition::NotDelivered) => JevStopReason::ActionFailed,
        _ => JevStopReason::ActionUncertain,
    };
    run_response(stop, turns, Some(decision), metrics)
}

#[cfg(feature = "desktop")]
pub(super) fn failed_turn(turns: &[JevTurn], decision: &JevDecision) -> JevTurn {
    JevTurn {
        step: u32::try_from(turns.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1),
        operation: decision.operation,
        target: decision.target.clone(),
        confidence: decision.confidence,
        ok: false,
        changed: false,
    }
}

#[cfg(feature = "desktop")]
pub(super) fn run_response(
    stop: JevStopReason,
    turns: Vec<JevTurn>,
    pending: Option<JevDecision>,
    metrics: JevMetrics,
) -> DesktopResponse {
    run_response_with_id(stop, turns, pending, metrics, None)
}

#[cfg(feature = "desktop")]
pub(super) fn run_response_observed(
    stop: JevStopReason,
    turns: Vec<JevTurn>,
    pending: Option<JevDecision>,
    metrics: JevMetrics,
    final_observation: Option<JevObservation>,
) -> DesktopResponse {
    response(
        "run-goal",
        &JevRunResult {
            verified: final_observation.as_ref().is_some_and(satisfied),
            final_observation,
            stop,
            turns,
            pending,
            confirmation_id: None,
            metrics,
        },
    )
}

#[cfg(feature = "desktop")]
pub(super) fn run_response_with_id(
    stop: JevStopReason,
    turns: Vec<JevTurn>,
    pending: Option<JevDecision>,
    metrics: JevMetrics,
    confirmation_id: Option<String>,
) -> DesktopResponse {
    response(
        "run-goal",
        &JevRunResult {
            stop,
            verified: stop == JevStopReason::Done,
            final_observation: None,
            turns,
            pending,
            confirmation_id,
            metrics,
        },
    )
}

pub(super) fn merge_metrics(metrics: &mut JevMetrics, evaluation: &EvaluationResult) {
    metrics.calls = metrics.calls.saturating_add(1);
    metrics.attempts = metrics.attempts.saturating_add(evaluation.attempts);
    metrics.latency_ms = metrics.latency_ms.saturating_add(
        evaluation
            .latency
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
    );
    metrics.input_tokens = metrics
        .input_tokens
        .saturating_add(evaluation.response.usage.input_tokens.unwrap_or_default());
    metrics.output_tokens = metrics
        .output_tokens
        .saturating_add(evaluation.response.usage.output_tokens.unwrap_or_default());
    metrics.model = Some(evaluation.response.model.clone());
}

pub(super) fn response<T: serde::Serialize>(command: &str, value: &T) -> DesktopResponse {
    match serde_json::to_value(value) {
        Ok(data) => DesktopResponse::ok(command, data),
        Err(error) => internal_error(&format!("cannot encode Jev result: {error}")),
    }
}

pub(super) fn provider_error(
    error: &tinyinference_decisions::EvaluationFailure,
) -> Box<DesktopResponse> {
    let code = match error.error.as_ref() {
        JevError::Authentication => "JEV_AUTHENTICATION",
        JevError::RateLimited => "JEV_RATE_LIMITED",
        JevError::Timeout => "JEV_TIMEOUT",
        JevError::InvalidResponse { .. } | JevError::Decode { .. } => "JEV_INVALID_RESPONSE",
        _ => "JEV_PROVIDER_FAILED",
    };
    Box::new(DesktopResponse::err(
        "jev-evaluate",
        DesktopError::new(code, error.to_string()),
    ))
}

#[cfg(feature = "desktop")]
pub(super) fn invalid_response(message: &str) -> Box<DesktopResponse> {
    Box::new(DesktopResponse::err(
        "jev-evaluate",
        DesktopError::new("JEV_INVALID_RESPONSE", message),
    ))
}

pub(super) fn internal_error(message: &str) -> DesktopResponse {
    DesktopResponse::err("jev-desktop", DesktopError::new("INTERNAL", message))
}
