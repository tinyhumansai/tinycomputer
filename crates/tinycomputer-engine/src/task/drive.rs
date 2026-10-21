//! Running a task in the background: planning it when asked to, running its
//! flows in order, interpreting where each stopped, and finishing it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tinycomputer_bus::agent::{SurfaceKind, TaskConstraints, TaskId, TaskStatus};
use tinycomputer_bus::{FLOW_GUIDE, FlowAction};

use super::artifact::{capture, captured};
use super::budget::{elapsed_budget_failed, run_request, stop_task};
use super::human::human_wall;
use super::interpret::{Next, finished, run_outcome};
use super::publish::{publish, records, stopped_summary};
use super::recovery::{rescue_outcome, rescued};
use super::store::{Cell, PaymentCheckpointState, Run};
use super::{DEFAULT_VOTES, FlowRunner, SHAPE_TIMEOUT_MS, redact_task_value};
use crate::shape::Harvest;

type TaskRecords = BTreeMap<String, Vec<BTreeMap<String, String>>>;

/// Plans the task, then runs the plan — or asks for what it needs first.
pub(super) async fn plan_then_drive(
    cell: Arc<Cell>,
    runner: Arc<dyn FlowRunner>,
    planner: crate::planner::Planner,
    task: String,
    surfaces: Vec<tinycomputer_bus::agent::SurfaceKind>,
) {
    plan_then_drive_with_page(cell, runner, planner, task, surfaces, true, true).await;
}

/// Plans a payment continuation against the retained current page without
/// reopening the task's named start page.
pub(super) async fn plan_payment_then_drive(
    cell: Arc<Cell>,
    runner: Arc<dyn FlowRunner>,
    planner: crate::planner::Planner,
    task: String,
    surfaces: Vec<tinycomputer_bus::agent::SurfaceKind>,
) {
    plan_then_drive_with_page(cell, runner, planner, task, surfaces, false, false).await;
}

async fn plan_then_drive_with_page(
    cell: Arc<Cell>,
    runner: Arc<dyn FlowRunner>,
    planner: crate::planner::Planner,
    task: String,
    surfaces: Vec<tinycomputer_bus::agent::SurfaceKind>,
    open_named_page: bool,
    prepare_runner: bool,
) {
    let (names, secrets, constraints, votes, capped) = cell.state.lock().map_or_else(
        |_| {
            (
                Vec::new(),
                Vec::new(),
                TaskConstraints::default(),
                DEFAULT_VOTES,
                false,
            )
        },
        |state| {
            let owned = |names: Vec<&str>| names.into_iter().map(str::to_owned).collect::<Vec<_>>();
            (
                owned(state.facts.names()),
                owned(state.facts.secret_names()),
                state.constraints.clone(),
                state.budget.votes.unwrap_or(DEFAULT_VOTES),
                state.budget.max_model_calls.is_some(),
            )
        },
    );
    let id = cell.view.borrow().id.clone();
    let usage = task_usage(&cell);
    let planner = planner.with_usage_recorder(Arc::clone(&usage));
    // Jev's connections open while the plan is drafted, so the first
    // decision need not open them; nothing waits for this. A budget that
    // caps the task's Jev calls is not spent on calls it does not count.
    if !capped {
        tokio::spawn(runner.warm_with_usage(&id, votes, Arc::clone(&usage)));
    }
    let started = Instant::now();
    // Timed on its own: a browser slower to open than the plan is to draft
    // is not planning time.
    let planning = async {
        let drafted = planner
            .plan_measured(&task, &names, &secrets, &surfaces)
            .await;
        (drafted, started.elapsed())
    };
    // A browser-only task's browser opens while the plan is drafted, so the
    // first step need not wait for it: whatever the plan says, it runs there.
    // The one page the task names loads in it meanwhile, for a first step
    // that browses there.
    let ((outcome, used), drafting) = if constraints.surfaces == [SurfaceKind::Browser] {
        let page = open_named_page
            .then(|| super::page::named_page(&task))
            .flatten();
        let preparing = prepare_page(
            runner.clone(),
            id.clone(),
            constraints.clone(),
            page,
            prepare_runner,
        );
        tokio::join!(planning, preparing).0
    } else {
        planning.await
    };
    runner.journal(Some(&id), "plan", &|| {
        super::timing::planned(&outcome, used, drafting, planner.configuration())
    });
    let plan = match outcome {
        Ok(plan) => plan,
        Err(reason) => {
            handle_plan_failure(cell, runner, id, task, reason).await;
            return;
        }
    };
    let allow = {
        let Ok(mut state) = cell.state.lock() else {
            return;
        };
        state.flow = plan.flow.clone();
        state.constraints.allow_destructive
    };
    if plan.questions.is_empty() {
        drive(
            cell,
            runner,
            vec![Run {
                flow: plan.flow,
                allow_destructive: allow,
                rescue: None,
            }],
        )
        .await;
    } else {
        // A browser opened while planning is let go while the task waits on
        // a person, who may take long or never answer; the run that follows
        // opens one again.
        runner.release(&id);
        publish(
            &cell,
            TaskStatus::NeedsInput {
                fields: plan.questions,
            },
            "The plan needs values before it can start.",
        );
    }
}

async fn handle_plan_failure(
    cell: Arc<Cell>,
    runner: Arc<dyn FlowRunner>,
    id: TaskId,
    task: String,
    reason: String,
) {
    if let Err(recovery) =
        recover_plan_after_planner_failure(cell.clone(), runner.clone(), id.clone(), task).await
    {
        runner.release(&id);
        let failure = format!("{reason}; host planning recovery failed: {recovery}");
        publish(
            &cell,
            TaskStatus::Failed {
                step: None,
                reason: failure.clone(),
                hint: "reword the task, or pass a flow written with Describe's guide".to_owned(),
                recoverable: true,
            },
            &format!("Planning failed: {failure}"),
        );
    }
}

async fn recover_plan_after_planner_failure(
    cell: Arc<Cell>,
    runner: Arc<dyn FlowRunner>,
    id: TaskId,
    task: String,
) -> Result<(), String> {
    let plan = super::plan_recovery::host_plan(&cell, runner.as_ref(), task, FLOW_GUIDE.to_owned())
        .await?;
    let facts = cell
        .state
        .lock()
        .map_err(|_| "task state is unavailable".to_owned())?
        .facts
        .clone();
    let known = super::names::known_names(&plan, &facts);
    let fact_names = super::names::fact_names(&facts);
    let missing = crate::agentic::missing_inputs(&plan, &known, &fact_names);
    let allow = {
        let mut state = cell
            .state
            .lock()
            .map_err(|_| "task state is unavailable".to_owned())?;
        state.flow = plan.clone();
        state.constraints.allow_destructive
    };
    if !missing.is_empty() {
        runner.release(&id);
        publish(
            &cell,
            super::publish::needs_input(&missing),
            "The recovered plan needs caller values before it can start.",
        );
        return Ok(());
    }
    drive(
        cell,
        runner,
        vec![Run {
            flow: plan,
            allow_destructive: allow,
            rescue: None,
        }],
    )
    .await;
    Ok(())
}

async fn prepare_page(
    runner: Arc<dyn FlowRunner>,
    id: tinycomputer_bus::agent::TaskId,
    constraints: TaskConstraints,
    page: Option<String>,
    prepare_runner: bool,
) {
    if prepare_runner {
        runner.prepare(&id, &constraints).await;
        if let Some(page) = page {
            runner.open_page(&id, &page).await;
        }
    }
}

/// Plans the rest of a task after a supplied flow prefix. The runner is not
/// prepared or opened here: it still owns the page the prefix left current.
async fn plan_handoff(
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
    let before_calls = usage.snapshot().planner.calls;
    let budget_left = {
        let state = cell
            .state
            .lock()
            .map_err(|_| "task state is unavailable".to_owned())?;
        let calls_left = state
            .budget
            .max_model_calls
            .unwrap_or(super::DEFAULT_MODEL_CALLS)
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
    let mut debit = PlanningDebit::new(Arc::downgrade(cell), Arc::clone(&usage), before_calls);
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

async fn run_handoff_planner(
    cell: &Arc<Cell>,
    runner: &dyn FlowRunner,
    draft: HandoffDraft,
    debit: &mut PlanningDebit,
) -> Result<tinycomputer_bus::agent::TaskPlan, String> {
    let planning = draft_handoff(
        cell,
        runner,
        draft.task.clone(),
        draft.surfaces,
        draft.facts,
        draft.planner,
    );
    let result = if let Some(ms) = draft.budget_left {
        let remaining = ms
            .saturating_sub(u64::try_from(debit.started.elapsed().as_millis()).unwrap_or(u64::MAX));
        tokio::time::timeout(Duration::from_millis(remaining), planning)
            .await
            .map_err(|_| "continuation planning exceeded the task time budget".to_owned())?
    } else {
        planning.await
    };
    match result {
        Ok(plan) => Ok(plan),
        Err(reason) if runner.plan_recovery_enabled() => {
            debit.charge();
            let remaining_calls = {
                let state = cell
                    .state
                    .lock()
                    .map_err(|_| "task state is unavailable".to_owned())?;
                state
                    .budget
                    .max_model_calls
                    .unwrap_or(super::DEFAULT_MODEL_CALLS)
                    .saturating_sub(state.spent.model_calls)
            };
            if remaining_calls == 0 {
                return Err(reason);
            }
            let flow =
                super::plan_recovery::host_plan(cell, runner, draft.task, FLOW_GUIDE.to_owned())
                    .await
                    .map_err(|recovery| format!("{reason}; host recovery failed: {recovery}"))?;
            Ok(tinycomputer_bus::agent::TaskPlan {
                flow,
                questions: Vec::new(),
                notes: Vec::new(),
            })
        }
        Err(reason) => Err(reason),
    }
}

struct HandoffDraft {
    task: String,
    surfaces: Vec<SurfaceKind>,
    facts: tinycomputer_core::Facts,
    planner: Option<crate::planner::Planner>,
    budget_left: Option<u64>,
}

fn ensure_handoff_budget(cell: &Cell) -> Result<(), String> {
    let state = cell
        .state
        .lock()
        .map_err(|_| "task state is unavailable".to_owned())?;
    if state.spent.model_calls
        > state
            .budget
            .max_model_calls
            .unwrap_or(super::DEFAULT_MODEL_CALLS)
    {
        return Err("continuation planning exhausted the task model call budget".to_owned());
    }
    if state
        .budget
        .max_elapsed_ms
        .is_some_and(|max| state.spent.elapsed_ms > max)
    {
        return Err("continuation planning exhausted the task time budget".to_owned());
    }
    Ok(())
}

fn validate_handoff_plan(
    cell: &Arc<Cell>,
    plan: tinycomputer_bus::agent::TaskPlan,
    facts: &tinycomputer_core::Facts,
    origin: &str,
) -> Result<Option<tinycomputer_bus::Flow>, String> {
    let known = super::names::known_names(&plan.flow, facts);
    let fact_names = super::names::fact_names(facts);
    let errors = crate::agentic::check_flow(&plan.flow, &known, &fact_names)
        .errors
        .into_iter()
        .filter(|error| !super::names::is_undefined(error))
        .collect::<Vec<_>>();
    if plan.flow.steps.is_empty() || !errors.is_empty() {
        return Err(if plan.flow.steps.is_empty() {
            "the planner returned an empty continuation".to_owned()
        } else {
            format!(
                "the planner returned an invalid continuation: {}",
                errors.join("; ")
            )
        });
    }
    if reopens_origin(&plan.flow, origin) {
        return Err("the planner tried to reopen the current shop's start page".to_owned());
    }
    let missing = crate::agentic::missing_inputs(&plan.flow, &known, &fact_names);
    let waiting = if plan.questions.is_empty() {
        (!missing.is_empty()).then(|| super::publish::needs_input(&missing))
    } else {
        Some(TaskStatus::NeedsInput {
            fields: plan.questions,
        })
    };
    if let Ok(mut state) = cell.state.lock() {
        state.flow = plan.flow.clone();
    }
    if let Some(status) = waiting {
        publish(
            cell,
            status,
            "The remaining task needs values before it can continue.",
        );
        return Ok(None);
    }
    Ok(Some(plan.flow))
}

struct PlanningDebit {
    cell: Weak<Cell>,
    usage: Arc<crate::UsageRecorder>,
    planner_calls_before: u64,
    started: Instant,
    charged: bool,
}

impl PlanningDebit {
    fn new(cell: Weak<Cell>, usage: Arc<crate::UsageRecorder>, planner_calls_before: u64) -> Self {
        Self {
            cell,
            usage,
            planner_calls_before,
            started: Instant::now(),
            charged: false,
        }
    }

    fn charge(&mut self) {
        if self.charged {
            return;
        }
        self.charged = true;
        let calls = self
            .usage
            .snapshot()
            .planner
            .calls
            .saturating_sub(self.planner_calls_before);
        let calls = u32::try_from(calls).unwrap_or(u32::MAX);
        if let Some(cell) = self.cell.upgrade()
            && let Ok(mut state) = cell.state.lock()
        {
            state.spent.model_calls = state.spent.model_calls.saturating_add(calls);
            state.spent.elapsed_ms = state.spent.elapsed_ms.saturating_add(
                u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            );
        }
    }
}

impl Drop for PlanningDebit {
    fn drop(&mut self) {
        self.charge();
    }
}

pub(super) fn reopens_origin(flow: &tinycomputer_bus::Flow, origin: &str) -> bool {
    let Ok(origin) = url::Url::parse(origin) else {
        return true;
    };
    if !matches!(origin.scheme(), "http" | "https")
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return true;
    }
    flow.steps.iter().any(|step| {
        let FlowAction::Browse(url) = step.action() else {
            return false;
        };
        let Ok(url) = url::Url::parse(&url) else {
            return false;
        };
        matches!(url.scheme(), "http" | "https")
            && url.origin() == origin.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && is_front_page(url.path())
    })
}

fn is_front_page(path: &str) -> bool {
    let segments = path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty());
    let segments = segments.collect::<Vec<_>>();
    segments.len() <= 2
        && segments.iter().all(|segment| {
            let bytes = segment.as_bytes();
            (bytes.len() == 2 && bytes.iter().all(u8::is_ascii_alphabetic))
                || (bytes.len() == 5
                    && bytes[..2].iter().all(u8::is_ascii_alphabetic)
                    && matches!(bytes[2], b'-' | b'_')
                    && bytes[3..].iter().all(u8::is_ascii_alphabetic))
        })
}

async fn draft_handoff(
    cell: &Arc<Cell>,
    runner: &dyn FlowRunner,
    task: String,
    surfaces: Vec<SurfaceKind>,
    facts: tinycomputer_core::Facts,
    planner: Option<crate::planner::Planner>,
) -> Result<tinycomputer_bus::agent::TaskPlan, String> {
    if let Some(planner) = planner {
        let planner = planner.with_usage_recorder(task_usage(cell));
        let started = Instant::now();
        let names = facts
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let secrets = facts
            .secret_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let (outcome, used) = planner
            .plan_measured(&task, &names, &secrets, &surfaces)
            .await;
        let id = cell.view.borrow().id.clone();
        runner.journal(Some(&id), "plan", &|| {
            super::timing::planned(&outcome, used, started.elapsed(), planner.configuration())
        });
        match outcome {
            Ok(plan) => Ok(plan),
            Err(reason) => Err(reason),
        }
    } else if runner.plan_recovery_enabled() {
        let flow =
            super::plan_recovery::host_plan(cell, runner, task, FLOW_GUIDE.to_owned()).await?;
        Ok(tinycomputer_bus::agent::TaskPlan {
            flow,
            questions: Vec::new(),
            notes: Vec::new(),
        })
    } else {
        Err("no planner or host plan-recovery hook is configured".to_owned())
    }
}

/// Runs a task's flows in order until one stops it or all finish.
///
/// A task's [`TaskBudget`] bounds the whole task, not one run of it: an
/// approval or a human intervention splits a task into several runs, and
/// each is given only what the task has not already spent, so resuming can
/// never reset the budget back to full. `max_elapsed_ms` has no equivalent in
/// [`RunFlowRequest`] — a run cannot police its own wall-clock time from the
/// inside — so it is enforced here instead, by timing out a run that would
/// otherwise run past what remains of it.
pub(super) async fn drive(cell: Arc<Cell>, runner: Arc<dyn FlowRunner>, runs: Vec<Run>) {
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
            && matches!(resume, Some(super::interpret::Resume::PaymentCheckpoint))
        {
            state.payment_checkpoint = PaymentCheckpointState::Available;
        }
    }
    Some((handoff, state.facts.clone()))
}

/// Every run finished: the task is done, with what it read, shaped as its
/// `output` asks when it asks.
pub(super) async fn finish(cell: &Cell, runner: &dyn FlowRunner) {
    // The last look at the surface, before shaping and before release.
    let _ = capture(cell, runner).await;
    let id = cell.view.borrow().id.clone();
    let Some((answer, records, harvest)) = finish_content(cell, runner, &id) else {
        return;
    };
    let result = match (harvest, cell.shaper.clone()) {
        (Some(harvest), Some(shaper)) => {
            let usage = task_usage(cell);
            let shaper = shaper.with_usage_recorder(usage);
            publish(
                cell,
                TaskStatus::Running,
                "Every step finished; shaping the answer.",
            );
            let answered = tokio::time::timeout(
                Duration::from_millis(SHAPE_TIMEOUT_MS),
                shaper.shape(&harvest),
            )
            .await
            .unwrap_or_else(|_| Err("shaping the answer took too long".to_owned()));
            match answered {
                Ok(result) => Some(result),
                Err(why) => {
                    let steps = cell.state.lock().map_or(0, |state| state.flow.steps.len());
                    let reason =
                        runner.redact(&id, &format!("the answer could not be shaped: {why}"));
                    publish(
                        cell,
                        TaskStatus::Failed {
                            step: Some(steps),
                            reason: reason.clone(),
                            hint: "every step finished; TaskReport holds the records they read"
                                .to_owned(),
                            recoverable: false,
                        },
                        &format!("The task failed: {reason}"),
                    );
                    runner.release(&cell.view.borrow().id);
                    return;
                }
            }
        }
        _ => None,
    };
    publish(
        cell,
        TaskStatus::Done {
            answer: answer.clone(),
            records,
            result: result.map(|mut value| {
                redact_task_value(runner, &id, &mut value);
                value
            }),
        },
        &answer,
    );
    runner.release(&cell.view.borrow().id);
}

fn finish_content(
    cell: &Cell,
    runner: &dyn FlowRunner,
    id: &TaskId,
) -> Option<(String, TaskRecords, Option<Harvest>)> {
    let state = cell.state.lock().ok()?;
    let reads = state
        .reads
        .iter()
        .map(|(name, value)| {
            let name = runner.redact(id, name);
            let value = runner.redact(id, value);
            format!("{name}: {value}")
        })
        .collect::<Vec<_>>();
    let answer = if reads.is_empty() {
        format!("Finished all {} steps.", state.flow.steps.len())
    } else {
        format!(
            "Finished all {} steps. {}",
            state.flow.steps.len(),
            reads.join("; ")
        )
    };
    let harvest = state.output.clone().map(|mut output| {
        output.instructions = runner.redact(id, &output.instructions);
        if let Some(schema) = &mut output.schema {
            redact_task_value(runner, id, schema);
        }
        Harvest {
            goal: runner.redact(id, &state.facts.redact(&state.goal)),
            output,
            reads: state
                .reads
                .iter()
                .map(|(name, value)| {
                    (
                        runner.redact(id, name),
                        runner.redact(id, &state.facts.redact(value)),
                    )
                })
                .collect(),
        }
    });
    let reads = state
        .reads
        .iter()
        .map(|(name, value)| (runner.redact(id, name), runner.redact(id, value)))
        .collect::<std::collections::BTreeMap<_, _>>();
    Some((
        runner.redact(id, &state.facts.redact(&answer)),
        records(&reads),
        harvest,
    ))
}

fn task_usage(cell: &Cell) -> Arc<crate::UsageRecorder> {
    cell.state
        .lock()
        .map(|state| Arc::clone(&state.usage))
        .unwrap_or_default()
}

/// Every variable a step of `steps` writes: a `read`'s, `extract`'s, or
/// `pick`'s `into`, at any depth.
fn written_names(steps: &[tinycomputer_bus::FlowStep]) -> std::collections::BTreeSet<String> {
    use tinycomputer_bus::FlowAction;
    let mut names = std::collections::BTreeSet::new();
    for step in steps {
        match step.action() {
            FlowAction::Read(read) | FlowAction::Extract(read) => {
                names.insert(read.into);
            }
            FlowAction::Pick(pick) => names.extend(pick.into),
            FlowAction::If(branch) => {
                names.extend(written_names(&branch.then));
                names.extend(written_names(&branch.otherwise));
            }
            FlowAction::RepeatUntil(repeat) => names.extend(written_names(&repeat.steps)),
            _ => {}
        }
    }
    names
}
