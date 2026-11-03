//! Awaiting, cancelling, and reporting task status.

use std::time::Duration;

use tinycomputer_bus::agent::{
    AgentError, AgentResponse, AwaitTaskRequest, TaskId, TaskReport, TaskReportRequest, TaskStatus,
    TaskView,
};

use super::Tasks;
use crate::task::MAX_AWAIT_MS;
use crate::task::errors::{no_such_task, poisoned};
use crate::task::publish::{publish, records};
use crate::task::redact_task_data;
use crate::task::store::PaymentCheckpointState;

impl Tasks {
    /// Waits until the task is no longer running, or `timeout_ms` passes.
    pub async fn await_task(&self, request: AwaitTaskRequest) -> AgentResponse<TaskView> {
        let Some(cell) = self.find(&request.id) else {
            return no_such_task(&request.id);
        };
        let mut changes = cell.view.subscribe();
        let wait = Duration::from_millis(request.timeout_ms.min(MAX_AWAIT_MS));
        let _waited = tokio::time::timeout(wait, async {
            while changes.borrow().status == TaskStatus::Running {
                if changes.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
        AgentResponse::ok(cell.current_view())
    }

    /// Stops a task, and lets go of whatever surface it still holds.
    ///
    /// Cancelling one already finished leaves its status unchanged — its
    /// view still reports how it ended — but still releases its workspace.
    /// A payment checkpoint is final without ever running to `Done`, so this
    /// is also its only path to release the browser session it left open for
    /// a person to pay in: without it, every checkout would permanently
    /// consume one of a limited number of session slots.
    #[must_use]
    pub fn cancel(&self, id: &TaskId) -> AgentResponse<TaskView> {
        let Some(cell) = self.find(id) else {
            return no_such_task(id);
        };
        if matches!(
            &cell.view.borrow().status,
            TaskStatus::Checkpoint {
                continuable: false,
                ..
            }
        ) {
            let Ok(mut state) = cell.state.lock() else {
                return poisoned();
            };
            if matches!(
                &cell.view.borrow().status,
                TaskStatus::Checkpoint {
                    continuable: false,
                    ..
                }
            ) {
                state.payment_checkpoint = PaymentCheckpointState::Closed;
                state.resume = None;
                state.handoff_to_planner = false;
                drop(state);
                self.runner.release(id);
                return AgentResponse::ok(cell.current_view());
            }
        }
        if !cell.view.borrow().status.is_final() {
            if let Ok(mut state) = cell.state.lock() {
                state.handoff_to_planner = false;
            }
            if let Ok(worker) = cell.worker.lock()
                && let Some(worker) = worker.as_ref()
            {
                worker.abort.abort();
            }
            publish(&cell, TaskStatus::Cancelled, "The task was cancelled.");
        }
        self.runner.release(id);
        AgentResponse::ok(cell.current_view())
    }

    /// Everything the task did.
    #[must_use]
    pub fn report(&self, id: &TaskId) -> AgentResponse<TaskReport> {
        let Some(cell) = self.find(id) else {
            return no_such_task(id);
        };
        let view = cell.current_view();
        let usage = view.usage;
        let Ok(state) = cell.state.lock() else {
            return poisoned();
        };
        let report = TaskReport {
            view,
            usage,
            flow: Some(state.flow.clone()),
            steps: state.steps.clone(),
            records: records(&state.reads),
            artifacts: state.artifacts.clone(),
            learned: state.learned.clone(),
            trace: state.exchanges.clone(),
            rescues: state.rescues.clone(),
        };
        match redact_task_data(self.runner.as_ref(), id, report) {
            Some(report) => AgentResponse::ok(report),
            None => AgentResponse::err(AgentError::new(
                "REPORT_REDACTION_FAILED",
                "the task report could not be safely redacted",
                "retry TaskReport",
                true,
            )),
        }
    }

    /// The report `request` asks for: [`Tasks::report`], without the Jev
    /// exchanges unless `request.trace`.
    #[must_use]
    pub fn report_for(&self, request: &TaskReportRequest) -> AgentResponse<TaskReport> {
        let mut reply = self.report(&request.id);
        if !request.trace
            && let Some(report) = reply.data.as_mut()
        {
            report.trace.clear();
        }
        reply
    }

    /// Every task held, newest first.
    #[must_use]
    pub fn list(&self) -> AgentResponse<Vec<TaskView>> {
        let Ok(tasks) = self.cells.lock() else {
            return poisoned();
        };
        AgentResponse::ok(
            tasks
                .values()
                .rev()
                .map(|cell| cell.current_view())
                .collect(),
        )
    }
}
