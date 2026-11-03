//! Host-supplied planning on the existing task and browser session.

use std::time::Duration;

use tinycomputer_bus::Flow;

use super::store::Cell;
use super::{
    DEFAULT_MODEL_CALLS, FlowRunner, PlanRecoveryRequest, PlanRecoveryResult, redact_task_data,
};

/// Asks the host for a plan, then checks it against the task's facts and
/// redacts it before any flow can run.
pub(super) async fn host_plan(
    cell: &Cell,
    runner: &dyn FlowRunner,
    task_text: String,
    guide: String,
) -> Result<Flow, String> {
    let id = { cell.view.borrow().id.clone() };
    let (constraints, budget, spent, known_names, secret_names, facts, task_budget) = {
        let state = cell
            .state
            .lock()
            .map_err(|_| "task state is unavailable".to_owned())?;
        let Some(task_budget) = state.usage.task_call_budget() else {
            return Err("the task model-call budget is unavailable".to_owned());
        };
        (
            state.constraints.clone(),
            state.budget,
            state.spent,
            state
                .facts
                .names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            state
                .facts
                .secret_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            state.facts.clone(),
            task_budget,
        )
    };
    let Some(work) = task_budget.begin_work() else {
        return Err("the task host-work budget is unavailable".to_owned());
    };
    let host_budget = work.scope();
    let remaining_ms = budget
        .max_elapsed_ms
        .map(|max| max.saturating_sub(spent.elapsed_ms));
    let timeout_ms = host_budget.remaining_ms().or(remaining_ms);
    let request = PlanRecoveryRequest {
        task: id.clone(),
        task_text: runner.redact(&id, &task_text),
        guide,
        constraints,
        known_names,
        secret_names,
        remaining_actions: budget
            .max_actions
            .unwrap_or(120)
            .saturating_sub(spent.actions),
        remaining_model_calls: budget
            .max_model_calls
            .unwrap_or(DEFAULT_MODEL_CALLS)
            .saturating_sub(spent.model_calls),
        remaining_ms,
        host_budget,
    };
    let recovery = runner.recover_plan(request);
    let result = if let Some(timeout_ms) = timeout_ms {
        tokio::time::timeout(Duration::from_millis(timeout_ms), recovery)
            .await
            .map_err(|_| "the host plan recovery exceeded the task time budget".to_owned())?
    } else {
        recovery.await
    };
    let flow = match result {
        PlanRecoveryResult::Plan(flow) => flow,
        PlanRecoveryResult::GiveUp(reason) => {
            return Err(runner.redact(&id, &reason));
        }
    };
    if flow.steps.is_empty() {
        return Err("the host returned an empty plan".to_owned());
    }
    let known = super::names::known_names(&flow, &facts);
    let fact_names = super::names::fact_names(&facts);
    let validation = crate::agentic::check_flow(&flow, &known, &fact_names);
    if !validation.errors.is_empty() {
        return Err(format!(
            "the host plan did not pass flow validation: {}",
            validation.errors.join("; ")
        ));
    }
    let Some(flow) = redact_task_data(runner, &id, flow) else {
        return Err("the host plan could not be safely redacted".to_owned());
    };
    drop(work);
    Ok(flow)
}
