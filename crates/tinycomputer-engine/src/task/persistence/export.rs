//! Encoding paused tasks and validating state before it leaves the controller.

use std::collections::BTreeMap;

use tinycomputer_bus::agent::{TaskId, TaskStatus};
use tinycomputer_core::Facts;

use super::super::Tasks;
use super::PausedTaskError;
use super::VERSION;
use super::types::{Envelope, PausedTask};
use crate::task::interpret::Resume;

impl Tasks {
    /// Encodes a task paused for input, approval, or human help.
    ///
    /// Non-secret facts, opaque secret handles, the flow continuation,
    /// reports, hints, records, budget spend, and provider usage are kept in a
    /// versioned JSON envelope. Inline secret values are omitted and must be
    /// supplied again after import if the flow needs them. If a secret value
    /// leaked into another part of the task state, export fails closed.
    /// Payment checkpoints and every other non-resumable state are refused.
    ///
    /// # Errors
    ///
    /// Returns [`PausedTaskError::NoSuchTask`] when `id` is unknown,
    /// [`PausedTaskError::NotResumable`] when its status cannot be continued,
    /// [`PausedTaskError::HostBoundEndpoint`] when its runner is attached to
    /// a host endpoint, [`PausedTaskError::SecretValueInState`] when a known
    /// secret appears elsewhere in the state, or a JSON/state error if it
    /// cannot be safely encoded.
    pub fn export_paused_task(&self, id: &TaskId) -> Result<Vec<u8>, PausedTaskError> {
        let cell = self
            .find(id)
            .ok_or_else(|| PausedTaskError::NoSuchTask(id.clone()))?;
        let view = cell.view.borrow().clone();
        if !resumable(&view.status) {
            return Err(PausedTaskError::NotResumable);
        }
        let state = cell
            .state
            .lock()
            .map_err(|_| PausedTaskError::Invalid("task state is poisoned".to_owned()))?;
        validate_pause(
            &view.status,
            state.resume.as_ref(),
            state.return_after_input.as_ref(),
        )?;
        if state.constraints.browser_endpoint.is_some() {
            return Err(PausedTaskError::HostBoundEndpoint);
        }
        let mut view = view;
        clear_screenshot(&mut view.status);
        let usage = state.usage.snapshot();
        view.usage = usage;
        let shared_facts = state
            .facts
            .shared()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        let mut secret_handles = BTreeMap::new();
        let mut omitted_secrets = state.omitted_secrets.clone();
        for name in state.facts.secret_names() {
            if let Some(handle) = state.facts.handle(name) {
                secret_handles.insert(
                    name.to_owned(),
                    tinycomputer_bus::agent::SecretHandle {
                        handle: handle.handle.clone(),
                        mask: handle.mask.clone(),
                        kind: handle.kind.clone(),
                    },
                );
            } else {
                // Inline secret values are never written. A flow that needs
                // one will ask for it after import.
                omitted_secrets.insert(name.to_owned());
            }
        }
        let task = PausedTask {
            id: id.clone(),
            view,
            flow: state.flow.clone(),
            handoff_to_planner: state.handoff_to_planner,
            goal: state.goal.clone(),
            shared_facts,
            secret_handles,
            omitted_secrets,
            constraints: state.constraints.clone(),
            budget: state.budget,
            memory: state.memory.clone(),
            trace: state.trace,
            steps: state.steps.clone(),
            exchanges: state.exchanges.clone(),
            learned: state.learned.clone(),
            reads: state.reads.clone(),
            dialog_left_open: state.dialog_left_open,
            finished: state.finished,
            resume: state.resume.clone(),
            return_after_input: state.return_after_input.clone(),
            spent: state.spent,
            rescues: state.rescues.clone(),
            output: state.output.clone(),
            // Output handles belong to the old process and cannot be read
            // after import. The restored browser profile is the durable view.
            artifacts: Vec::new(),
            usage,
        };
        let facts = state.facts.clone();
        drop(state);

        // Scrub every serialized string surface, including nested flow text,
        // reports, traces, statuses, hints, and collected records. If this
        // changes task state, fail closed rather than restore altered work.
        let mut value = serde_json::to_value(Envelope {
            version: VERSION,
            task,
        })?;
        let redact = |text: &str| self.runner.redact(id, text);
        let changed = scrub_secrets(&mut value, &facts, &redact);
        if changed {
            return Err(PausedTaskError::SecretValueInState);
        }
        Ok(serde_json::to_vec(&value)?)
    }
}

pub(super) fn resumable(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::NeedsInput { .. }
            | TaskStatus::NeedsApproval { .. }
            | TaskStatus::NeedsHuman { .. }
    )
}

pub(super) fn validate_pause(
    status: &TaskStatus,
    resume: Option<&Resume>,
    return_after_input: Option<&TaskStatus>,
) -> Result<(), PausedTaskError> {
    let valid = match status {
        TaskStatus::NeedsInput { fields } => {
            !fields.is_empty()
                && match return_after_input {
                    None => resume.is_none(),
                    Some(TaskStatus::NeedsApproval { .. }) => {
                        matches!(resume, Some(Resume::Approval { .. }))
                    }
                    Some(TaskStatus::NeedsHuman { .. }) => {
                        matches!(resume, Some(Resume::Retry { .. }))
                    }
                    _ => false,
                }
        }
        TaskStatus::NeedsApproval { .. } => {
            return_after_input.is_none() && matches!(resume, Some(Resume::Approval { .. }))
        }
        TaskStatus::NeedsHuman { .. } => {
            return_after_input.is_none() && matches!(resume, Some(Resume::Retry { .. }))
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(PausedTaskError::Invalid(
            "pause status and continuation state do not match".to_owned(),
        ))
    }
}

fn clear_screenshot(status: &mut TaskStatus) {
    match status {
        TaskStatus::NeedsApproval { screenshot, .. }
        | TaskStatus::NeedsHuman { screenshot, .. } => *screenshot = None,
        _ => {}
    }
}

fn scrub_secrets(
    value: &mut serde_json::Value,
    facts: &Facts,
    redact: &dyn Fn(&str) -> String,
) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let scrubbed = redact(&facts.mask(text));
            let changed = scrubbed != *text;
            *text = scrubbed;
            changed
        }
        serde_json::Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= scrub_secrets(value, facts, redact);
            }
            changed
        }
        serde_json::Value::Object(values) => {
            let mut changed = false;
            let old = std::mem::take(values);
            for (key, mut child) in old {
                let scrubbed_key = facts.mask(&key);
                changed |= scrubbed_key != key;
                let scrubbed_key = redact(&scrubbed_key);
                changed |= scrubbed_key != key;
                changed |= scrub_secrets(&mut child, facts, redact);
                values.insert(scrubbed_key, child);
            }
            changed
        }
        _ => false,
    }
}
