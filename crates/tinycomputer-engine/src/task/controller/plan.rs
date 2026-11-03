//! Drafting task plans without starting a browser task.

use tinycomputer_bus::agent::{AgentError, AgentResponse, PlanTaskRequest, TaskPlan};

use super::Tasks;
use crate::task::errors::no_planner;

impl Tasks {
    /// Drafts a flow for a plain-language task without acting.
    pub async fn plan(&self, request: &PlanTaskRequest) -> AgentResponse<TaskPlan> {
        let Some(planner) = &self.planner else {
            return AgentResponse::err(no_planner());
        };
        let started = std::time::Instant::now();
        let (outcome, used) = planner
            .plan_measured(
                &request.task,
                &request.fact_names,
                &request.secret_facts,
                &request.surfaces,
            )
            .await;
        // No task exists yet, so the plan journals to a run of its own.
        let drafting = started.elapsed();
        self.runner.journal(None, "plan", &|| {
            crate::task::timing::planned(&outcome, used, drafting, planner.configuration())
        });
        match outcome {
            Ok(plan) => AgentResponse::ok(plan),
            Err(reason) => AgentResponse::err(AgentError::new(
                "PLAN_FAILED",
                reason,
                "reword the task, or write the flow yourself with Describe's guide",
                true,
            )),
        }
    }
}
