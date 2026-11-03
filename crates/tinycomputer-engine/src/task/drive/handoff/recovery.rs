//! Recover a failed task plan and prepare its browser runner.

use std::sync::Arc;

use tinycomputer_bus::FLOW_GUIDE;
use tinycomputer_bus::agent::{TaskConstraints, TaskId, TaskStatus};

use crate::task::FlowRunner;
use crate::task::drive::drive;
use crate::task::publish::publish;
use crate::task::store::{Cell, Run};

pub(in crate::task) async fn handle_plan_failure(
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
    let plan =
        crate::task::plan_recovery::host_plan(&cell, runner.as_ref(), task, FLOW_GUIDE.to_owned())
            .await?;
    let facts = cell
        .state
        .lock()
        .map_err(|_| "task state is unavailable".to_owned())?
        .facts
        .clone();
    let known = crate::task::names::known_names(&plan, &facts);
    let fact_names = crate::task::names::fact_names(&facts);
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
            crate::task::publish::needs_input(&missing),
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

pub(in crate::task) async fn prepare_page(
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
