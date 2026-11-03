//! Run the continuation planner and account for its task budget.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tinycomputer_bus::FLOW_GUIDE;
use tinycomputer_bus::agent::SurfaceKind;

use super::draft::draft_handoff;
use crate::task::FlowRunner;
use crate::task::store::Cell;

pub(super) async fn run_handoff_planner(
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
        draft.call_budget.clone(),
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
                    .unwrap_or(crate::task::DEFAULT_MODEL_CALLS)
                    .saturating_sub(state.spent.model_calls)
            };
            if remaining_calls == 0 {
                return Err(reason);
            }
            let flow = crate::task::plan_recovery::host_plan(
                cell,
                runner,
                draft.task,
                FLOW_GUIDE.to_owned(),
            )
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

pub(super) struct HandoffDraft {
    pub(super) task: String,
    pub(super) surfaces: Vec<SurfaceKind>,
    pub(super) facts: tinycomputer_core::Facts,
    pub(super) planner: Option<crate::planner::Planner>,
    pub(super) budget_left: Option<u64>,
    pub(super) call_budget: crate::task::TaskCallBudget,
}

pub(super) struct PlanningDebit {
    cell: Weak<Cell>,
    started: Instant,
    charged: bool,
}

impl PlanningDebit {
    pub(super) fn new(cell: Weak<Cell>) -> Self {
        Self {
            cell,
            started: Instant::now(),
            charged: false,
        }
    }

    pub(super) fn charge(&mut self) {
        if self.charged {
            return;
        }
        self.charged = true;
        if let Some(cell) = self.cell.upgrade()
            && let Ok(mut state) = cell.state.lock()
        {
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
