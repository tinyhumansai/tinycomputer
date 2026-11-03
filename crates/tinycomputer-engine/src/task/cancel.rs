//! Awaited task cancellation for hosts that must finish browser cleanup.

use tinycomputer_bus::agent::{AgentResponse, TaskId, TaskStatus, TaskView};

use super::{Tasks, errors::no_such_task};

impl Tasks {
    /// Cancels a task and waits for its worker future to be dropped before
    /// returning. The runner's release callback is repeated after the join so
    /// adapters can defer resource teardown until the worker is quiescent.
    pub async fn cancel_and_wait(&self, id: &TaskId) -> AgentResponse<TaskView> {
        let Some(cell) = self.find(id) else {
            return no_such_task(id);
        };
        if matches!(
            &cell.view.borrow().status,
            TaskStatus::Checkpoint {
                continuable: false,
                ..
            }
        ) && let Ok(mut state) = cell.state.lock()
        {
            state.payment_checkpoint = super::store::PaymentCheckpointState::Closed;
            state.resume = None;
            state.handoff_to_planner = false;
        }
        let unfinished = !cell.view.borrow().status.is_final();
        let done = if let Ok(worker) = cell.worker.lock() {
            if unfinished && let Some(worker) = worker.as_ref() {
                worker.abort.abort();
            }
            worker.as_ref().map(|worker| worker.done.clone())
        } else {
            None
        };
        if unfinished {
            super::publish::publish(&cell, TaskStatus::Cancelled, "The task was cancelled.");
        }

        if let Some(mut done) = done {
            wait_for_worker(&mut done).await;
        }

        self.runner.release(id);
        AgentResponse::ok(cell.current_view())
    }
}

async fn wait_for_worker(done: &mut tokio::sync::watch::Receiver<bool>) {
    while !*done.borrow() {
        if done.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
#[path = "cancel_tests.rs"]
mod tests;
