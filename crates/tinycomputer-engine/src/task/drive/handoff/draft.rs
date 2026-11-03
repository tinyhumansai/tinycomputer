//! Ask the configured planner or host recovery hook for a continuation.

use std::sync::Arc;
use std::time::Instant;

use tinycomputer_bus::FLOW_GUIDE;
use tinycomputer_bus::agent::SurfaceKind;

use super::super::finish::task_usage;
use crate::task::FlowRunner;
use crate::task::store::Cell;

pub(super) async fn draft_handoff(
    cell: &Arc<Cell>,
    runner: &dyn FlowRunner,
    task: String,
    surfaces: Vec<SurfaceKind>,
    facts: tinycomputer_core::Facts,
    planner: Option<crate::planner::Planner>,
    call_budget: crate::task::TaskCallBudget,
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
        let reserve_call = || call_budget.reserve_model_call();
        let (outcome, used) = planner
            .plan_measured_with_budget(&task, &names, &secrets, &surfaces, Some(&reserve_call))
            .await;
        let id = cell.view.borrow().id.clone();
        runner.journal(Some(&id), "plan", &|| {
            crate::task::timing::planned(&outcome, used, started.elapsed(), planner.configuration())
        });
        match outcome {
            Ok(plan) => Ok(plan),
            Err(reason) => Err(reason),
        }
    } else if runner.plan_recovery_enabled() {
        let flow = crate::task::plan_recovery::host_plan(cell, runner, task, FLOW_GUIDE.to_owned())
            .await?;
        Ok(tinycomputer_bus::agent::TaskPlan {
            flow,
            questions: Vec::new(),
            notes: Vec::new(),
        })
    } else {
        Err("no planner or host plan-recovery hook is configured".to_owned())
    }
}
