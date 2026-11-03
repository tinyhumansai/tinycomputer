//! Planning a task and running its first planned flow.

use super::finish::task_usage;
use super::handoff::{handle_plan_failure, prepare_page};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tinycomputer_bus::agent::{SurfaceKind, TaskConstraints, TaskStatus};

use super::drive;
use crate::task::DEFAULT_VOTES;
use crate::task::FlowRunner;
use crate::task::publish::publish;
use crate::task::store::{Cell, Run};

/// Plans the task, then runs the plan — or asks for what it needs first.
pub(in crate::task) async fn plan_then_drive(
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
pub(in crate::task) async fn plan_payment_then_drive(
    cell: Arc<Cell>,
    runner: Arc<dyn FlowRunner>,
    planner: crate::planner::Planner,
    task: String,
    surfaces: Vec<tinycomputer_bus::agent::SurfaceKind>,
    budget: crate::task::TaskBudgetScope,
    work: crate::task::HostWorkGuard,
) {
    let (names, secrets) = cell.state.lock().map_or_else(
        |_| (Vec::new(), Vec::new()),
        |state| {
            let owned = |names: Vec<&str>| names.into_iter().map(str::to_owned).collect::<Vec<_>>();
            (
                owned(state.facts.names()),
                owned(state.facts.secret_names()),
            )
        },
    );
    let id = cell.view.borrow().id.clone();
    let planner = planner.with_usage_recorder(task_usage(&cell));
    let started = Instant::now();
    let reserve_call = || budget.try_reserve_model_call();
    let planning =
        planner.plan_measured_with_budget(&task, &names, &secrets, &surfaces, Some(&reserve_call));
    let (outcome, used) = if let Some(ms) = budget.remaining_ms() {
        if let Ok(result) = tokio::time::timeout(Duration::from_millis(ms), planning).await {
            result
        } else {
            drop(work);
            handle_plan_failure(
                cell,
                runner,
                id,
                task,
                "payment continuation exceeded the task time budget".to_owned(),
            )
            .await;
            return;
        }
    } else {
        planning.await
    };
    runner.journal(Some(&id), "plan", &|| {
        crate::task::timing::planned(&outcome, used, started.elapsed(), planner.configuration())
    });
    let plan = match outcome {
        Ok(plan) => plan,
        Err(reason) => {
            drop(work);
            handle_plan_failure(cell, runner, id, task, reason).await;
            return;
        }
    };
    if !payment_plan_stays_on_current_page(&plan.flow.steps) {
        drop(work);
        runner.release(&id);
        publish(
            &cell,
            TaskStatus::Failed {
                step: None,
                reason:
                    "the payment continuation tried to navigate away from the current checkout page"
                        .to_owned(),
                hint: "continue only with a plan that inspects the current checkout page"
                    .to_owned(),
                recoverable: true,
            },
            "The payment continuation was stopped because its plan navigated away from checkout.",
        );
        return;
    }
    let allow = {
        let Ok(mut state) = cell.state.lock() else {
            return;
        };
        state.flow = plan.flow.clone();
        state.constraints.allow_destructive
    };
    drop(work);
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
        runner.release(&id);
        publish(
            &cell,
            TaskStatus::NeedsInput {
                fields: plan.questions,
            },
            "The payment continuation needs values before it can continue.",
        );
    }
}

fn payment_plan_stays_on_current_page(steps: &[tinycomputer_bus::FlowStep]) -> bool {
    use tinycomputer_bus::FlowAction;
    steps.iter().all(|step| match step.action() {
        FlowAction::Browse(_) => false,
        FlowAction::If(branch) => {
            payment_plan_stays_on_current_page(&branch.then)
                && payment_plan_stays_on_current_page(&branch.otherwise)
        }
        FlowAction::RepeatUntil(repeat) => payment_plan_stays_on_current_page(&repeat.steps),
        _ => true,
    })
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
            .then(|| crate::task::page::named_page(&task))
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
        crate::task::timing::planned(&outcome, used, drafting, planner.configuration())
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
