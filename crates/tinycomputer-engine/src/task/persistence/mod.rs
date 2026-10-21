//! Versioned snapshots for tasks paused for input, approval, or human help.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tinycomputer_bus::agent::{Rescue, TaskConstraints, TaskId, TaskStatus, TaskUsage, TaskView};
use tinycomputer_bus::{Flow, GroundingHint, JevExchange, StepReport};
use tinycomputer_core::{Facts, SecretFactHandle};

use super::interpret::Resume;
use super::publish::{needs_input, publish};
use super::store::{Cell, Spent, State};
use super::{MAX_TASKS, TaskCallBudget, Tasks};
use crate::rescue::Rescuer;
use crate::{UsageRecorder, shape::Shaper};

const VERSION: u32 = 2;
const OLDEST_VERSION: u32 = 1;

/// Why a paused task could not be saved or restored.
#[derive(Debug)]
pub enum PausedTaskError {
    /// The task is absent from this controller.
    NoSuchTask(TaskId),
    /// Only pauses that `ContinueTask` can resume may be saved.
    NotResumable,
    /// A secret value appears in task state outside its facts map.
    SecretValueInState,
    /// The snapshot uses a different persistence version.
    UnsupportedVersion(u32),
    /// The snapshot is malformed or its state is inconsistent.
    Invalid(String),
    /// The runner is attached through an endpoint URL which may carry
    /// credentials and cannot safely be moved to a new process.
    HostBoundEndpoint,
    /// The snapshot could not be encoded or decoded as JSON.
    Json(serde_json::Error),
}

impl std::fmt::Display for PausedTaskError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchTask(id) => write!(formatter, "no task {id}"),
            Self::NotResumable => formatter.write_str("the task is not in a resumable pause"),
            Self::SecretValueInState => formatter.write_str(
                "a secret value appears in task state; clear it from the state before exporting",
            ),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "paused-task snapshot version {version} is unsupported"
                )
            }
            Self::Invalid(reason) => write!(formatter, "invalid paused-task snapshot: {reason}"),
            Self::HostBoundEndpoint => formatter.write_str(
                "tasks attached through browser_endpoint cannot be persisted; reattach through the new host",
            ),
            Self::Json(error) => write!(formatter, "paused-task JSON: {error}"),
        }
    }
}

impl std::error::Error for PausedTaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for PausedTaskError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    task: PausedTask,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PausedTask {
    id: TaskId,
    view: TaskView,
    flow: Flow,
    #[serde(default)]
    handoff_to_planner: bool,
    goal: String,
    shared_facts: BTreeMap<String, String>,
    secret_handles: BTreeMap<String, tinycomputer_bus::agent::SecretHandle>,
    omitted_secrets: BTreeSet<String>,
    constraints: TaskConstraints,
    budget: tinycomputer_bus::agent::TaskBudget,
    memory: Vec<GroundingHint>,
    trace: bool,
    steps: Vec<StepReport>,
    exchanges: Vec<JevExchange>,
    learned: Vec<GroundingHint>,
    reads: BTreeMap<String, String>,
    dialog_left_open: bool,
    finished: usize,
    resume: Option<Resume>,
    return_after_input: Option<TaskStatus>,
    spent: Spent,
    rescues: Vec<Rescue>,
    output: Option<tinycomputer_bus::agent::TaskOutput>,
    artifacts: Vec<tinycomputer_bus::browser::OutputRef>,
    usage: TaskUsage,
}

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

    /// Restores a versioned paused task into this controller.
    ///
    /// The saved task id and browser profile are retained. A browser runner
    /// reopening that profile lets the browser restore its last page. No
    /// flow runs until the caller continues the imported pause.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed snapshots, an unsupported format
    /// version, state that cannot be resumed safely, an endpoint-bound
    /// browser session, a duplicate task id, or a full task store.
    pub fn import_paused_task(&self, bytes: &[u8]) -> Result<TaskView, PausedTaskError> {
        let mut task = decode(bytes)?;
        let number = task_number(&task)?;
        let (facts, return_after_input, waiting_for_secret_reentry) = prepare_import(&mut task)?;
        let cell = build_cell(
            task,
            facts,
            return_after_input,
            self.rescuer.clone(),
            self.shaper.clone(),
            self.planner.clone(),
        );
        let mut tasks = self
            .cells
            .lock()
            .map_err(|_| PausedTaskError::Invalid("task index is poisoned".to_owned()))?;
        if tasks.contains_key(&number) {
            return Err(PausedTaskError::Invalid(
                "task id is already registered".to_owned(),
            ));
        }
        while tasks.len() >= MAX_TASKS {
            let Some(oldest_final) = tasks
                .iter()
                .find(|(_, existing)| existing.view.borrow().status.is_final())
                .map(|(number, _)| *number)
            else {
                return Err(PausedTaskError::Invalid("task store is full".to_owned()));
            };
            tasks.remove(&oldest_final);
        }
        tasks.insert(number, cell.clone());
        self.counter.fetch_max(number, Ordering::Relaxed);
        drop(tasks);
        let status = cell.view.borrow().status.clone();
        let summary = if waiting_for_secret_reentry {
            "The task needs values before it can resume.".to_owned()
        } else {
            cell.view.borrow().summary.clone()
        };
        publish(&cell, status, &summary);
        Ok(cell.current_view())
    }
}

fn decode(bytes: &[u8]) -> Result<PausedTask, PausedTaskError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| PausedTaskError::Invalid("missing integer version".to_owned()))?;
    if !(OLDEST_VERSION..=VERSION).contains(&version) {
        return Err(PausedTaskError::UnsupportedVersion(version));
    }
    Ok(serde_json::from_value::<Envelope>(value)?.task)
}

fn task_number(task: &PausedTask) -> Result<u64, PausedTaskError> {
    let digits = task.id.0.strip_prefix("t-").ok_or_else(|| {
        PausedTaskError::Invalid("task id must be t-<positive integer>".to_owned())
    })?;
    let number = digits
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            PausedTaskError::Invalid("task id must be t-<positive integer>".to_owned())
        })?;
    if task.id.0 != format!("t-{number}") {
        return Err(PausedTaskError::Invalid(
            "task id must use its canonical decimal form".to_owned(),
        ));
    }
    if number == u64::MAX {
        return Err(PausedTaskError::Invalid(
            "task id leaves no space for the controller's next id".to_owned(),
        ));
    }
    if task.view.id != task.id {
        return Err(PausedTaskError::Invalid(
            "view id does not match task id".to_owned(),
        ));
    }
    Ok(number)
}

fn prepare_import(
    task: &mut PausedTask,
) -> Result<(Facts, Option<TaskStatus>, bool), PausedTaskError> {
    if !resumable(&task.view.status) {
        return Err(PausedTaskError::NotResumable);
    }
    validate_pause(
        &task.view.status,
        task.resume.as_ref(),
        task.return_after_input.as_ref(),
    )?;
    if task.constraints.browser_endpoint.is_some() {
        return Err(PausedTaskError::HostBoundEndpoint);
    }
    if task
        .omitted_secrets
        .iter()
        .any(|name| task.shared_facts.contains_key(name) || task.secret_handles.contains_key(name))
    {
        return Err(PausedTaskError::Invalid(
            "an omitted secret is also present as a persisted fact".to_owned(),
        ));
    }
    let handles = task.secret_handles.iter().map(|(name, handle)| {
        (
            name.clone(),
            SecretFactHandle {
                handle: handle.handle.clone(),
                mask: handle.mask.clone(),
                kind: handle.kind.clone(),
            },
        )
    });
    let facts = Facts::with_handles(
        task.shared_facts.clone(),
        handles,
        std::iter::empty::<String>(),
    )
    .map_err(|error| PausedTaskError::Invalid(error.to_string()))?;
    let missing = missing_inputs(
        &task.flow,
        task.resume.as_ref(),
        &facts,
        &task.omitted_secrets,
    )?;
    let return_after_input = match (&task.view.status, &task.return_after_input) {
        (TaskStatus::NeedsInput { .. }, Some(status)) => Some(status.clone()),
        (TaskStatus::NeedsApproval { .. } | TaskStatus::NeedsHuman { .. }, _)
            if !missing.is_empty() =>
        {
            Some(task.view.status.clone())
        }
        _ => None,
    };
    if !missing.is_empty() {
        let mut fields = match &task.view.status {
            TaskStatus::NeedsInput { fields } => fields.clone(),
            _ => Vec::new(),
        };
        if let TaskStatus::NeedsInput {
            fields: missing_fields,
        } = needs_input(&missing)
        {
            for field in missing_fields {
                if !fields.iter().any(|existing| existing.name == field.name) {
                    fields.push(field);
                }
            }
        }
        task.view.status = TaskStatus::NeedsInput { fields };
    }
    if matches!(task.view.status, TaskStatus::NeedsInput { ref fields } if fields.is_empty()) {
        return Err(PausedTaskError::Invalid(
            "needs_input status has no pending fields".to_owned(),
        ));
    }
    let waiting_for_secret_reentry = return_after_input.is_some();
    Ok((facts, return_after_input, waiting_for_secret_reentry))
}

fn build_cell(
    mut task: PausedTask,
    facts: Facts,
    return_after_input: Option<TaskStatus>,
    rescuer: Option<Rescuer>,
    shaper: Option<Shaper>,
    planner: Option<crate::planner::Planner>,
) -> Arc<Cell> {
    let usage = Arc::new(UsageRecorder::from_snapshot(task.usage));
    task.view.usage = usage.snapshot();
    let cell = Arc::new(Cell {
        view: tokio::sync::watch::Sender::new(task.view.clone()),
        state: std::sync::Mutex::new(State {
            usage: usage.clone(),
            flow: task.flow,
            handoff_to_planner: task.handoff_to_planner,
            goal: task.goal,
            facts,
            omitted_secrets: task.omitted_secrets,
            constraints: task.constraints,
            budget: task.budget,
            memory: task.memory,
            trace: task.trace,
            steps: task.steps,
            exchanges: task.exchanges,
            learned: task.learned,
            reads: task.reads,
            dialog_left_open: task.dialog_left_open,
            finished: task.finished,
            resume: task.resume,
            payment_checkpoint: super::store::PaymentCheckpointState::Unavailable,
            spent: task.spent,
            host_work: super::store::HostWorkState::Open,
            rescues: task.rescues,
            output: task.output,
            artifacts: task.artifacts,
            waiting_since: None,
            return_after_input: return_after_input.or(task.return_after_input),
        }),
        worker: std::sync::Mutex::new(None),
        rescuer,
        shaper,
        planner,
    });
    usage.set_task_call_budget(TaskCallBudget::for_cell(Arc::downgrade(&cell)));
    cell
}

fn resumable(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::NeedsInput { .. }
            | TaskStatus::NeedsApproval { .. }
            | TaskStatus::NeedsHuman { .. }
    )
}

fn validate_pause(
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

fn missing_inputs(
    flow: &Flow,
    resume: Option<&Resume>,
    facts: &Facts,
    omitted_secrets: &BTreeSet<String>,
) -> Result<Vec<String>, PausedTaskError> {
    let missing_for = |flow: &Flow| {
        let known = super::names::known_names(flow, facts);
        let fact_names = super::names::fact_names(facts);
        crate::agentic::missing_inputs(flow, &known, &fact_names)
    };
    let mut missing = missing_for(flow);
    if let Some(Resume::Retry { app, steps }) = resume {
        let resumed = Flow {
            app: app.clone(),
            vars: flow.vars.clone(),
            steps: steps.clone(),
        };
        missing.extend(missing_for(&resumed));
    }
    let mut referenced = BTreeSet::new();
    collect_references(&serde_json::to_value(flow)?, &mut referenced);
    if let Some(Resume::Retry { app, steps }) = resume {
        collect_references(
            &serde_json::to_value(Flow {
                app: app.clone(),
                vars: flow.vars.clone(),
                steps: steps.clone(),
            })?,
            &mut referenced,
        );
    }
    missing.extend(omitted_secrets.intersection(&referenced).cloned());
    missing.sort();
    missing.dedup();
    Ok(missing)
}

fn collect_references(value: &serde_json::Value, names: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            let mut remaining = text.as_str();
            while let Some(start) = remaining.find("${") {
                remaining = &remaining[start + 2..];
                let Some(end) = remaining.find('}') else {
                    break;
                };
                if end > 0 {
                    names.insert(remaining[..end].to_owned());
                }
                remaining = &remaining[end + 1..];
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_references(value, names);
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                collect_references(&serde_json::Value::String(key.clone()), names);
                collect_references(value, names);
            }
        }
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
