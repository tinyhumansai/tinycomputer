//! Redacting browser observations and responses that can contain host-held values.

use tinycomputer_bus::{DesktopError, DesktopResponse};
use tinycomputer_core::surface::{Candidate, Screen};

use crate::SecretHost;

pub(super) fn redact_observation_error(
    host: &dyn SecretHost,
    task_id: &str,
    response: Box<DesktopResponse>,
) -> Box<DesktopResponse> {
    Box::new(redact_response(host, task_id, *response))
}

pub(super) fn redact_response(
    host: &dyn SecretHost,
    task_id: &str,
    response: DesktopResponse,
) -> DesktopResponse {
    let redact = |text: &str| host.redact(task_id, text);
    let Ok(mut value) = serde_json::to_value(response) else {
        return redaction_failed_response();
    };

    // These fields can be used to address or replay an operation. Rewriting
    // them into a placeholder would make an apparently valid response point
    // at a different target, so fail closed on a collision.
    if operational_collision(&value, &redact) {
        return redaction_failed_response();
    }
    crate::task::redact_value(&mut value, &redact);
    serde_json::from_value(value).unwrap_or_else(|_| redaction_failed_response())
}

fn operational_collision(value: &serde_json::Value, redact: &impl Fn(&str) -> String) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            let operational = matches!(
                key.as_str(),
                "command" | "code" | "ref" | "ref_id" | "target" | "action" | "available_actions"
            );
            (operational && value.as_str().is_some_and(|text| redact(text) != text))
                || operational_collision(value, redact)
        }),
        serde_json::Value::Array(items) => {
            items.iter().any(|item| operational_collision(item, redact))
        }
        _ => false,
    }
}

fn redaction_failed_response() -> DesktopResponse {
    DesktopResponse::err(
        "redaction",
        DesktopError::new(
            "REDACTION_FAILED",
            "the browser response could not be safely redacted",
        ),
    )
}

pub(super) fn redact_screen(host: &dyn SecretHost, task_id: &str, screen: &mut Screen) {
    fn candidate(host: &dyn SecretHost, task_id: &str, node: &mut Candidate) -> bool {
        let redact = |text: &str| host.redact(task_id, text);
        let reference = redact(&node.ref_id);
        let role = redact(&node.role);
        // Refs and actions are executable metadata. If redaction finds a
        // collision, omit the candidate instead of manufacturing an invalid
        // target that could address a different control.
        if reference != node.ref_id || role != node.role {
            return false;
        }
        node.available_actions
            .retain(|action| redact(action) == *action);
        node.name = node.name.take().map(|text| redact(&text));
        node.description = node.description.take().map(|text| redact(&text));
        node.states.retain(|state| redact(state) == *state);
        for path in &mut node.path {
            *path = redact(path);
        }
        if let Some(value) = &mut node.value {
            crate::task::redact_value(value, &redact);
        }
        if let Some(bounds) = &mut node.bounds {
            let original = bounds.clone();
            crate::task::redact_value(bounds, &redact);
            if *bounds != original {
                node.bounds = None;
            }
        }
        node.children
            .retain_mut(|child| candidate(host, task_id, child));
        true
    }
    let redact = |text: &str| host.redact(task_id, text);
    screen.app = redact(&screen.app);
    screen.window = screen.window.take().map(|text| redact(&text));
    screen.surface = redact(&screen.surface);
    for text in &mut screen.context {
        *text = redact(text);
    }
    screen
        .unexplored
        .retain(|reference| redact(reference) == *reference);
    screen
        .candidates
        .retain_mut(|node| candidate(host, task_id, node));
    screen
        .text_nodes
        .retain_mut(|node| candidate(host, task_id, node));
}
