//! Draft and validate a continuation from the current page.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tinycomputer_bus::agent::SurfaceKind;

use super::super::finish::task_usage;
use super::planner::{HandoffDraft, PlanningDebit, run_handoff_planner};
use super::validation::{ensure_handoff_budget, validate_handoff_plan};
use crate::task::FlowRunner;
use crate::task::store::Cell;

/// Plans the rest of a task after a supplied flow prefix. The runner is not
/// prepared or opened here: it still owns the page the prefix left current.
pub(in crate::task) async fn plan_handoff(
    cell: &Arc<Cell>,
    runner: &dyn FlowRunner,
) -> Result<Option<tinycomputer_bus::Flow>, String> {
    let (task, surfaces, facts, planner) = {
        let state = cell
            .state
            .lock()
            .map_err(|_| "task state is unavailable".to_owned())?;
        (
            state.goal.clone(),
            state.constraints.surfaces.clone(),
            state.facts.clone(),
            cell.planner.clone(),
        )
    };
    if task.trim().is_empty() {
        return Err("the handoff task text is empty".to_owned());
    }
    let (plan, origin) =
        draft_from_current_page(cell, runner, task, surfaces, facts.clone(), planner).await?;
    validate_handoff_plan(cell, plan, &facts, &origin)
}

async fn draft_from_current_page(
    cell: &Arc<Cell>,
    runner: &dyn FlowRunner,
    task: String,
    surfaces: Vec<SurfaceKind>,
    facts: tinycomputer_core::Facts,
    planner: Option<crate::planner::Planner>,
) -> Result<(tinycomputer_bus::agent::TaskPlan, String), String> {
    let usage = task_usage(cell);
    let call_budget = usage
        .task_call_budget()
        .ok_or_else(|| "the task model-call budget is unavailable".to_owned())?;
    let budget_left = {
        let state = cell
            .state
            .lock()
            .map_err(|_| "task state is unavailable".to_owned())?;
        let calls_left = state
            .budget
            .max_model_calls
            .unwrap_or(crate::task::DEFAULT_MODEL_CALLS)
            .saturating_sub(state.spent.model_calls);
        let elapsed_left = state
            .budget
            .max_elapsed_ms
            .map(|max| max.saturating_sub(state.spent.elapsed_ms));
        if calls_left == 0 {
            return Err(
                "the task has no model calls remaining for continuation planning".to_owned(),
            );
        }
        if elapsed_left == Some(0) {
            return Err("the task has no time remaining for continuation planning".to_owned());
        }
        elapsed_left
    };
    // Drop also charges elapsed time and started planner calls if this future
    // is cancelled while the model is still responding.
    let mut debit = PlanningDebit::new(Arc::downgrade(cell));
    let id = cell.view.borrow().id.clone();
    let origin_future = runner.page_origin(&id);
    let origin = if let Some(ms) = budget_left {
        tokio::time::timeout(Duration::from_millis(ms), origin_future)
            .await
            .map_err(|_| "reading the current page exceeded the task time budget".to_owned())?
    } else {
        origin_future.await
    }
    .ok_or_else(|| "the current browser origin is unavailable".to_owned())?;
    let planning_task = runner.redact(
        &id,
        &format!(
            "{task}\n\n{}\nContinue this same task from the current page at {origin}. The current browser session is still open. Do not open or browse to the shop start page again, and do not repeat the completed prefix actions. Inspect the current page before choosing the next action.",
            successful_prefix_summary(cell)
        ),
    );
    let plan = run_handoff_planner(
        cell,
        runner,
        HandoffDraft {
            task: planning_task.clone(),
            surfaces,
            facts,
            planner,
            budget_left,
            call_budget,
        },
        &mut debit,
    )
    .await?;
    drop(debit);
    ensure_handoff_budget(cell)?;
    Ok((plan, origin))
}

fn successful_prefix_summary(cell: &Cell) -> String {
    let Ok(state) = cell.state.lock() else {
        return "The safe prefix completed; its action structure is unavailable.".to_owned();
    };
    let mut counts = BTreeMap::<&'static str, usize>::new();
    for report in &state.steps {
        if !matches!(
            report.outcome,
            tinycomputer_bus::StepOutcome::Done | tinycomputer_bus::StepOutcome::AlreadyDone
        ) {
            continue;
        }
        let category = match report.kind.as_str() {
            "browse" => "navigation",
            "do" => "browser interaction",
            "wait_for" => "page wait",
            "verify" => "page check",
            _ => continue,
        };
        *counts.entry(category).or_default() += 1;
    }
    if counts.is_empty() {
        return "The safe recipe prefix completed; no successful replayable action classes were recorded.".to_owned();
    }
    let steps = counts
        .into_iter()
        .map(|(kind, count)| format!("{count} completed {kind} step(s)"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Completed safe-prefix structure: {steps}. Do not repeat these steps.")
}
