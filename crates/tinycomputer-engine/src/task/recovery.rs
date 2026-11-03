//! Rescuing a failed step: handing it to the task's rescuer with what the
//! run reached, recording the rescue, and turning its guidance into a run.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use tinycomputer_bus::agent::{Rescue, RescueOutcome, TaskId, TaskStatus};
use tinycomputer_bus::{Flow, FlowAction, FlowStep, StepOutcome, StepReport};
use tinycomputer_core::Facts;

use super::brief::brief;
use super::names::{fact_names, known_names};
use super::publish::publish;
use super::store::{Cell, Run};
use super::timing;
use super::{
    FailedStepRecovery, FailedStepRecoveryResult, FlowRunner, RESCUE_TIMEOUT_MS, redact_task_data,
};
use crate::rescue::{Briefing, Guidance, MAX_RESCUES, Rescuer, resumed};

/// A recoverable failure of a top-level step is first rescued: `Err` holds
/// the run the guidance makes, to run next. Anything else, or a rescue that
/// gave up or was not tried, is the status to publish.
pub(super) async fn rescued(
    cell: &Cell,
    runner: &dyn FlowRunner,
    run: &Run,
    status: TaskStatus,
    reached: Vec<StepReport>,
) -> Result<TaskStatus, Run> {
    let TaskStatus::Failed {
        step: Some(failed),
        reason,
        hint,
        recoverable: true,
    } = status
    else {
        return Ok(status);
    };
    let fallback_briefing =
        failed_step_briefing(cell, runner, run, failed, &reason, reached.clone()).await;
    let mut hint = match rescue(cell, runner, run, failed, &reason, reached).await {
        Rescued::Run(rescued) => return Err(rescued),
        Rescued::GaveUp(why) => {
            format!("the rescuer gave up: {why}; reword the step, split it, or take over")
        }
        Rescued::Skipped => hint,
    };
    if let Some(briefing) = fallback_briefing
        && exact_top_level_tail(&briefing)
    {
        match recover_failed_step(cell, runner, run, &briefing, failed, reason.clone(), hint).await
        {
            Ok(recovered) => return Err(recovered),
            Err(next_hint) => hint = next_hint,
        }
    }
    Ok(failed_status(failed, reason, hint))
}

async fn recover_failed_step(
    cell: &Cell,
    runner: &dyn FlowRunner,
    run: &Run,
    briefing: &Briefing,
    failed: usize,
    failure: String,
    hint: String,
) -> Result<Run, String> {
    let (constraints, budget, spent, task_budget) = {
        let state = cell.state.lock().map_err(|_| hint.clone())?;
        let task_budget = state.usage.task_call_budget().ok_or_else(|| hint.clone())?;
        (
            state.constraints.clone(),
            state.budget,
            state.spent,
            task_budget,
        )
    };
    let Some(work) = task_budget.begin_work() else {
        return Err(format!("{hint}; the task host-work budget is unavailable"));
    };
    let host_budget = work.scope();
    let remaining_actions = budget
        .max_actions
        .unwrap_or(120)
        .saturating_sub(spent.actions);
    let remaining_model_calls = budget
        .max_model_calls
        .unwrap_or(super::DEFAULT_MODEL_CALLS)
        .saturating_sub(spent.model_calls);
    let remaining_ms = budget
        .max_elapsed_ms
        .map(|max| max.saturating_sub(spent.elapsed_ms));
    let timeout_ms = host_budget
        .remaining_ms()
        .or(remaining_ms)
        .unwrap_or(RESCUE_TIMEOUT_MS)
        .min(RESCUE_TIMEOUT_MS);
    let request = FailedStepRecovery {
        task: cell.view.borrow().id.clone(),
        briefing: briefing.clone(),
        constraints,
        remaining_actions,
        remaining_model_calls,
        remaining_ms,
        host_budget,
    };
    let recovered = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        runner.recover_failed_step(request),
    )
    .await;
    drop(work);
    match recovered {
        Ok(FailedStepRecoveryResult::Completed {
            reason,
            actions,
            model_calls: _,
        }) => {
            let tail = resumed(briefing, Vec::new(), 0);
            if tail.steps.is_empty() {
                return Err(format!(
                    "{hint}; host recovery completed the last step but no safe tail exists"
                ));
            }
            if let Ok(mut state) = cell.state.lock() {
                state.spent.actions = state.spent.actions.saturating_add(actions);
                state.rescues.push(Rescue {
                    step: failed,
                    failure,
                    reason,
                    steps: Vec::new(),
                    covers: 0,
                    outcome: RescueOutcome::Recovered,
                });
            }
            Ok(Run {
                flow: tail,
                allow_destructive: run.allow_destructive,
                rescue: None,
            })
        }
        Ok(FailedStepRecoveryResult::GiveUp { reason }) => {
            Err(format!("{hint}; host recovery gave up: {reason}"))
        }
        Err(_) => Err(format!("{hint}; host recovery took too long")),
    }
}

fn failed_status(failed: usize, reason: String, hint: String) -> TaskStatus {
    TaskStatus::Failed {
        step: Some(failed),
        reason,
        hint,
        recoverable: true,
    }
}

/// Builds the same redacted briefing used by Jev's rescuer for the host hook.
async fn failed_step_briefing(
    cell: &Cell,
    runner: &dyn FlowRunner,
    run: &Run,
    failed: usize,
    failure: &str,
    reached: Vec<StepReport>,
) -> Option<Briefing> {
    let id = { cell.view.borrow().id.clone() };
    let (facts, goal, earlier, rules, reads) = {
        let state = cell.state.lock().ok()?;
        (
            state.facts.clone(),
            state.goal.clone(),
            state.rescues.clone(),
            brief(&state).rules,
            state.reads.clone(),
        )
    };
    briefing(
        runner,
        RescueContext {
            id,
            facts,
            goal,
            flow: run.flow.clone(),
            failed,
            failure: failure.to_owned(),
            reached,
            earlier,
            rules,
            reads,
        },
    )
    .await
}

/// The existing resume operation can preserve this tail exactly only when
/// failure is at a top-level atomic step. A failed branch or loop might have
/// unfinished nested siblings or iterations, so the host must give up.
fn exact_top_level_tail(briefing: &Briefing) -> bool {
    let Some(step) = briefing.flow.steps.get(briefing.failed) else {
        return false;
    };
    !matches!(
        step.action(),
        FlowAction::If(_) | FlowAction::RepeatUntil(_) | FlowAction::StopBefore(_)
    )
}

/// What a rescue came to.
pub(super) enum Rescued {
    /// Run this next: the guidance, then the rest of the failed run.
    Run(Run),
    /// The rescuer gave up, failed, or gave no valid guidance, for this
    /// reason.
    GaveUp(String),
    /// No rescue was tried: none is configured, or the task's are spent.
    Skipped,
}

struct RescueContext {
    id: TaskId,
    facts: Facts,
    goal: String,
    flow: Flow,
    failed: usize,
    failure: String,
    reached: Vec<StepReport>,
    earlier: Vec<Rescue>,
    rules: Vec<String>,
    reads: BTreeMap<String, String>,
}

async fn briefing(runner: &dyn FlowRunner, context: RescueContext) -> Option<Briefing> {
    let screen = runner
        .visible_text(&context.id)
        .await
        .iter()
        .map(|line| {
            runner.redact(
                &context.id,
                &context.facts.redact(&context.facts.mask(line)),
            )
        })
        .collect();
    let (known, mut collected) = recalled(&context.flow, &context.facts, &context.reads);
    for (_, value) in &mut collected {
        *value = runner.redact(&context.id, value);
    }
    let mut steps = Vec::new();
    for step in context.reached {
        steps.push(redact_task_data(
            runner,
            &context.id,
            redacted(&context.facts, step),
        )?);
    }
    let briefing = Briefing {
        goal: runner.redact(&context.id, &context.facts.redact(&context.goal)),
        flow: context.flow,
        failed: context.failed,
        failure: runner.redact(&context.id, &context.facts.redact(&context.failure)),
        steps,
        earlier: context.earlier,
        screen,
        rules: context.rules,
        known,
        collected,
        secrets: fact_names(&context.facts),
    };
    redact_task_data(runner, &context.id, briefing)
}

/// The names a rescue's steps may use — the flow's, the facts', and what the
/// task saved — and what it saved, fact values redacted.
pub(super) fn recalled(
    flow: &Flow,
    facts: &Facts,
    reads: &BTreeMap<String, String>,
) -> (BTreeSet<String>, Vec<(String, String)>) {
    let known = known_names(flow, facts)
        .into_iter()
        .chain(reads.keys().cloned())
        .collect();
    let collected = reads
        .iter()
        .map(|(name, value)| (name.clone(), facts.redact(&facts.mask(value))))
        .collect();
    (known, collected)
}

/// `step` with every fact value in its text and note redacted.
pub(super) fn redacted(facts: &Facts, mut step: StepReport) -> StepReport {
    step.text = facts.redact(&step.text);
    step.note = facts.redact(&step.note);
    step
}

/// Hands the step `failed` of `run`, which failed for `failure`, to the
/// task's rescuer, with what the run reached and the screen now — every
/// fact value redacted. Returns the run its guidance makes, or why not.
pub(super) async fn rescue(
    cell: &Cell,
    runner: &dyn FlowRunner,
    run: &Run,
    failed: usize,
    failure: &str,
    reached: Vec<StepReport>,
) -> Rescued {
    let Some(rescuer) = cell.rescuer.clone() else {
        return Rescued::Skipped;
    };
    let usage = cell
        .state
        .lock()
        .map(|state| std::sync::Arc::clone(&state.usage))
        .unwrap_or_default();
    let rescuer = rescuer.with_usage_recorder(usage);
    let (facts, goal, earlier, limit, time_left, brief_rules, reads) = {
        let Ok(state) = cell.state.lock() else {
            return Rescued::Skipped;
        };
        let limit = state
            .budget
            .max_rescues
            .unwrap_or(MAX_RESCUES)
            .min(MAX_RESCUES);
        let used = u32::try_from(state.rescues.len()).unwrap_or(u32::MAX);
        if used >= limit {
            return Rescued::Skipped;
        }
        let time_left = state
            .budget
            .max_elapsed_ms
            .map(|max| max.saturating_sub(state.spent.elapsed_ms));
        (
            state.facts.clone(),
            state.goal.clone(),
            state.rescues.clone(),
            limit,
            time_left,
            brief(&state).rules,
            state.reads.clone(),
        )
    };
    let attempt = earlier.len() + 1;
    let id = cell.view.borrow().id.clone();
    let Some(briefing) = briefing(
        runner,
        RescueContext {
            id: id.clone(),
            facts: facts.clone(),
            goal,
            flow: run.flow.clone(),
            failed,
            failure: failure.to_owned(),
            reached,
            earlier,
            rules: brief_rules,
            reads,
        },
    )
    .await
    else {
        return Rescued::GaveUp("the rescue briefing could not be safely redacted".to_owned());
    };
    publish(
        cell,
        TaskStatus::Running,
        &format!(
            "Step {} failed; asking for guidance (rescue {attempt} of {limit}).",
            failed + 1
        ),
    );
    let wait = time_left.map_or(RESCUE_TIMEOUT_MS, |left| left.min(RESCUE_TIMEOUT_MS));
    let (record, guided, spent_ms) =
        ask(runner, &id, &rescuer, &briefing, wait, (attempt, limit)).await;
    let Some(record) = redact_task_data(runner, &id, record) else {
        return Rescued::GaveUp("the rescue response could not be safely redacted".to_owned());
    };
    let guided = guided.and_then(|steps| {
        let flow = resumed(&briefing, steps, record.covers);
        redact_task_data(runner, &id, flow)
    });
    let reason = runner.redact(&id, &facts.redact(&record.reason));
    let index = {
        let Ok(mut state) = cell.state.lock() else {
            return Rescued::Skipped;
        };
        state.spent.elapsed_ms = state.spent.elapsed_ms.saturating_add(spent_ms);
        state.rescues.push(record);
        state.rescues.len() - 1
    };
    let Some(flow) = guided else {
        return Rescued::GaveUp(reason);
    };
    publish(
        cell,
        TaskStatus::Running,
        &format!("Rescue {attempt} of {limit}: {reason}"),
    );
    Rescued::Run(Run {
        flow,
        allow_destructive: run.allow_destructive,
        rescue: Some(index),
    })
}

/// Asks `rescuer` about `briefing`, waiting at most `wait` ms, journals how
/// it went as rescue `attempt` of `limit`, and records it: the record, the
/// guidance's steps when it gave any, and the milliseconds asking took.
async fn ask(
    runner: &dyn FlowRunner,
    id: &TaskId,
    rescuer: &Rescuer,
    briefing: &Briefing,
    wait: u64,
    (attempt, limit): (usize, u32),
) -> (Rescue, Option<Vec<FlowStep>>, u64) {
    let started = Instant::now();
    let (answer, used) = match tokio::time::timeout(
        Duration::from_millis(wait),
        rescuer.guide_measured(briefing),
    )
    .await
    {
        Ok((answer, used)) => (answer, Some(used)),
        Err(_) => (Err("the rescuer took too long".to_owned()), None),
    };
    let took = started.elapsed();
    let outcome = timing::answered(&answer, used.is_none());
    let (record, guided) = record(briefing.failed, briefing.failure.clone(), answer);
    runner.journal(Some(id), "rescue", &|| {
        timing::rescued(&timing::Rescued {
            attempt,
            limit,
            took,
            used,
            outcome,
            record: &record,
            model: rescuer.configuration(),
        })
    });
    let spent_ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
    (record, guided, spent_ms)
}

/// The record of a rescue of step `failed`, and the guidance's steps when
/// it gave any.
pub(super) fn record(
    failed: usize,
    failure: String,
    answer: Result<Guidance, String>,
) -> (Rescue, Option<Vec<FlowStep>>) {
    let (reason, steps, covers, outcome) = match answer {
        Ok(Guidance::Retry {
            reason,
            steps,
            covers,
        }) => (reason, steps, covers, RescueOutcome::Running),
        Ok(Guidance::GiveUp { reason }) | Err(reason) => {
            (reason, Vec::new(), 0, RescueOutcome::GaveUp)
        }
    };
    let guided = (outcome == RescueOutcome::Running).then(|| steps.clone());
    (
        Rescue {
            step: failed,
            failure,
            reason,
            steps,
            covers,
            outcome,
        },
        guided,
    )
}

/// How a rescue went, from the steps its run reached: recovered when each
/// of the `count` guidance steps (the run's first top-level steps) finished
/// or reached its approval.
pub(super) fn rescue_outcome(reached: &[StepReport], count: usize) -> RescueOutcome {
    let finished = (1..=count).all(|number| {
        reached.iter().any(|step| {
            step.path == number.to_string()
                && matches!(
                    step.outcome,
                    StepOutcome::Done | StepOutcome::AlreadyDone | StepOutcome::Gated
                )
        })
    });
    if finished {
        RescueOutcome::Recovered
    } else {
        RescueOutcome::FailedAgain
    }
}
