//! Running task flows and recording their results.

use super::finish::{task_usage, written_names};
use super::handoff::plan_handoff;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tinycomputer_bus::agent::TaskStatus;

use super::finish::finish;
use crate::task::FlowRunner;
use crate::task::artifact::captured;
use crate::task::budget::{elapsed_budget_failed, run_request, stop_task};
use crate::task::human::human_wall;
use crate::task::interpret::{Next, finished, run_outcome};
use crate::task::publish::publish;
use crate::task::publish::stopped_summary;
use crate::task::recovery::{rescue_outcome, rescued};
use crate::task::store::{Cell, PaymentCheckpointState, Run};

/// Runs a task's flows in order until one stops it or all finish.
///
/// A task's [`TaskBudget`] bounds the whole task, not one run of it: an
/// approval or a human intervention splits a task into several runs, and
/// each is given only what the task has not already spent, so resuming can
/// never reset the budget back to full. `max_elapsed_ms` has no equivalent in
/// [`RunFlowRequest`] — a run cannot police its own wall-clock time from the
/// inside — so it is enforced here instead, by timing out a run that would
/// otherwise run past what remains of it.
pub(in crate::task) async fn drive(cell: Arc<Cell>, runner: Arc<dyn FlowRunner>, runs: Vec<Run>) {
    let mut runs = VecDeque::from(runs);
    while let Some(run) = runs.pop_front() {
        let Some((request, constraints, time_left)) = run_request(&cell, &run) else {
            return;
        };
        if time_left == Some(0) {
            stop_task(&cell, runner.as_ref(), elapsed_budget_failed()).await;
            return;
        }
        let id = cell.view.borrow().id.clone();
        let started = Instant::now();
        let run_call = runner.run_with_usage(&id, &constraints, request, task_usage(&cell));
        let reply = if let Some(ms) = time_left {
            let Ok(reply) = tokio::time::timeout(Duration::from_millis(ms), run_call).await else {
                stop_task(&cell, runner.as_ref(), elapsed_budget_failed()).await;
                return;
            };
            reply
        } else {
            run_call.await
        };
        let spent_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (next, result) = run_outcome(&run.flow, &reply, constraints.payment);
        let completed = result
            .as_ref()
            .is_some_and(|result| result.stop == tinycomputer_bus::FlowStopReason::Completed);
        let reached = result
            .as_ref()
            .map_or_else(Vec::new, |result| result.steps.clone());
        let Some((handoff, redacted)) = record_run(
            &cell,
            &run,
            &next,
            result,
            &reached,
            spent_ms,
            completed && runs.is_empty(),
        ) else {
            return;
        };
        if handoff {
            if !queue_handoff(&cell, runner.as_ref(), &mut runs).await {
                return;
            }
            continue;
        }
        if let Next::Stop { status, .. } = next {
            let status = human_wall(&cell, runner.as_ref(), *status).await;
            let status = match rescued(&cell, runner.as_ref(), &run, status, reached).await {
                Ok(status) => status,
                Err(rescued) => {
                    runs.push_front(rescued);
                    continue;
                }
            };
            let status = captured(&cell, runner.as_ref(), status).await;
            if matches!(
                status,
                TaskStatus::Checkpoint { .. } | TaskStatus::Failed { .. } | TaskStatus::Cancelled
            ) && let Ok(mut state) = cell.state.lock()
            {
                state.handoff_to_planner = false;
            }
            let summary = redacted.redact(&stopped_summary(&status));
            let ended = matches!(
                status,
                TaskStatus::Done { .. } | TaskStatus::Failed { .. } | TaskStatus::Cancelled
            );
            publish(&cell, status, &summary);
            if ended {
                runner.release(&cell.view.borrow().id);
            }
            return;
        }
    }
    finish(&cell, runner.as_ref()).await;
}

async fn queue_handoff(
    cell: &Arc<Cell>,
    runner: &dyn FlowRunner,
    runs: &mut VecDeque<Run>,
) -> bool {
    match plan_handoff(cell, runner).await {
        Ok(Some(flow)) => {
            let allow = cell
                .state
                .lock()
                .is_ok_and(|state| state.constraints.allow_destructive);
            runs.push_front(Run {
                flow,
                allow_destructive: allow,
                rescue: None,
            });
            true
        }
        Ok(None) => false,
        Err(reason) => {
            let id = cell.view.borrow().id.clone();
            runner.release(&id);
            let reason = runner.redact(&id, &reason);
            publish(
                cell,
                TaskStatus::Failed {
                    step: None,
                    reason: reason.clone(),
                    hint: "the recipe prefix ran, but the task could not be planned from the current page".to_owned(),
                    recoverable: true,
                },
                &format!("Planning the remaining task failed: {reason}"),
            );
            false
        }
    }
}

fn record_run(
    cell: &Cell,
    run: &Run,
    next: &Next,
    result: Option<tinycomputer_bus::FlowRunResult>,
    reached: &[tinycomputer_bus::StepReport],
    spent_ms: u64,
    handoff_ready: bool,
) -> Option<(bool, tinycomputer_core::Facts)> {
    let mut state = cell.state.lock().ok()?;
    state.spent.elapsed_ms = state.spent.elapsed_ms.saturating_add(spent_ms);
    if let Some(rescue) = run.rescue.and_then(|index| state.rescues.get_mut(index)) {
        rescue.outcome = rescue_outcome(reached, rescue.steps.len());
    }
    if let Some(result) = result {
        state.dialog_left_open = result.dialog_left_open;
        state.spent.actions = state.spent.actions.saturating_add(result.actions);
        state.spent.model_calls = state.spent.model_calls.saturating_add(result.metrics.calls);
        state.finished += finished(&result.steps);
        state.steps.extend(result.steps);
        state.exchanges.extend(result.trace);
        state.learned.extend(result.learned);
        let written = written_names(&run.flow.steps);
        for (name, value) in result.vars {
            let read = match run.flow.vars.get(&name) {
                None => true,
                Some(declared) => written.contains(&name) && *declared != value,
            };
            if read && state.facts.get(&name).is_none() {
                state.reads.insert(name, value);
            }
        }
    }
    let handoff = handoff_ready && state.handoff_to_planner;
    if handoff {
        state.handoff_to_planner = false;
    }
    if let Next::Stop { resume, .. } = next {
        state.resume.clone_from(resume);
        if state.payment_checkpoint == PaymentCheckpointState::Unavailable
            && matches!(
                resume,
                Some(crate::task::interpret::Resume::PaymentCheckpoint)
            )
        {
            state.payment_checkpoint = PaymentCheckpointState::Available;
        }
    }
    Some((handoff, state.facts.clone()))
}
