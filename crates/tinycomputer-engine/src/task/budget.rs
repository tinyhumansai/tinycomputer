//! A task's budget across its runs: the request each run is given, capped at
//! what the task has not spent, and ending a task whose time ran out.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use tinycomputer_bus::RunFlowRequest;
use tinycomputer_bus::agent::{TaskConstraints, TaskStatus};

use super::artifact::captured;
use super::brief::brief;
use super::names::fact_names;
use super::publish::{publish, stopped_summary};
use super::store::{Cell, Run};
use super::{DEFAULT_MODEL_CALLS, DEFAULT_VOTES, FlowRunner};

/// A handle to reserve host model calls against this task's Jev evaluation
/// budget. Hosts must reserve before starting provider I/O; a reservation is
/// not refunded when that I/O fails or is cancelled.
#[derive(Clone)]
pub struct TaskCallBudget {
    cell: Weak<Cell>,
}

impl TaskCallBudget {
    pub(super) fn for_cell(cell: Weak<Cell>) -> Self {
        Self { cell }
    }

    /// Starts one host-work interval. Its elapsed time is charged when the
    /// returned guard is dropped, even if a callback retains its scope.
    #[must_use]
    pub fn begin_work(&self) -> Option<HostWorkGuard> {
        self.begin(false)
    }

    /// Starts a scoped payment-mapping interval on the task's one available
    /// non-continuable payment checkpoint. This is host-only authority; generic
    /// task continuation remains unavailable at that checkpoint.
    #[must_use]
    pub fn begin_payment_work(&self) -> Option<HostWorkGuard> {
        self.begin(true)
    }

    /// Reserves one planner call while no other host work is active.
    #[must_use]
    pub fn reserve_model_call(&self) -> bool {
        let Some(cell) = self.cell.upgrade() else {
            return false;
        };
        let Ok(mut state) = cell.state.lock() else {
            return false;
        };
        if state.host_work != super::store::HostWorkState::Open
            || state
                .budget
                .max_elapsed_ms
                .is_some_and(|max| state.spent.elapsed_ms >= max)
        {
            return false;
        }
        let max = state.budget.max_model_calls.unwrap_or(DEFAULT_MODEL_CALLS);
        if state.spent.model_calls >= max {
            return false;
        }
        state.spent.model_calls = state.spent.model_calls.saturating_add(1);
        true
    }

    fn begin(&self, payment_checkpoint: bool) -> Option<HostWorkGuard> {
        let cell = self.cell.upgrade()?;
        let mut state = cell.state.lock().ok()?;
        let is_payment_checkpoint = matches!(
            &cell.view.borrow().status,
            tinycomputer_bus::agent::TaskStatus::Checkpoint {
                continuable: false,
                ..
            }
        );
        let authorized_payment_checkpoint = payment_checkpoint
            && state.payment_checkpoint == super::store::PaymentCheckpointState::Available
            && is_payment_checkpoint;
        let state_allows_work = if payment_checkpoint {
            state.host_work == super::store::HostWorkState::Closed && authorized_payment_checkpoint
        } else {
            state.host_work == super::store::HostWorkState::Open
        };
        if !state_allows_work
            || state
                .budget
                .max_elapsed_ms
                .is_some_and(|max| state.spent.elapsed_ms >= max)
        {
            return None;
        }
        state.host_work = if payment_checkpoint {
            super::store::HostWorkState::PaymentActive
        } else {
            super::store::HostWorkState::Active
        };
        drop(state);
        Some(HostWorkGuard {
            inner: Arc::new(HostWork {
                cell: Arc::downgrade(&cell),
                started: Instant::now(),
                charged: AtomicBool::new(false),
                payment_checkpoint,
            }),
        })
    }
}

impl fmt::Debug for TaskCallBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskCallBudget")
            .finish_non_exhaustive()
    }
}

/// One cloneable host-work interval, shared with the request passed to the
/// host callback. Its owning guard charges active time once when dropped.
#[derive(Clone)]
pub struct TaskBudgetScope {
    inner: Arc<HostWork>,
}

/// Owns one host-work interval. The task is charged when this guard is dropped,
/// even if a callback retained a clone of its [`TaskBudgetScope`].
pub struct HostWorkGuard {
    inner: Arc<HostWork>,
}

impl HostWorkGuard {
    /// A cloneable view a host callback can retain and use for live checks.
    #[must_use]
    pub fn scope(&self) -> TaskBudgetScope {
        TaskBudgetScope {
            inner: self.inner.clone(),
        }
    }
}

impl fmt::Debug for HostWorkGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostWorkGuard")
            .finish_non_exhaustive()
    }
}

impl Drop for HostWorkGuard {
    fn drop(&mut self) {
        self.inner.charge();
    }
}

impl TaskBudgetScope {
    /// Reserves one task model call before the host starts a provider request.
    /// The reservation remains spent if the request fails or is cancelled.
    #[must_use]
    pub fn try_reserve_model_call(&self) -> bool {
        let Some(cell) = self.inner.cell.upgrade() else {
            return false;
        };
        let Ok(mut state) = cell.state.lock() else {
            return false;
        };
        if !self.inner.is_active(&cell, &state) {
            return false;
        }
        if state.budget.max_elapsed_ms.is_some_and(|max| {
            state.spent.elapsed_ms.saturating_add(
                u64::try_from(self.inner.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ) >= max
        }) {
            return false;
        }
        let max = state.budget.max_model_calls.unwrap_or(DEFAULT_MODEL_CALLS);
        if state.spent.model_calls >= max {
            return false;
        }
        state.spent.model_calls = state.spent.model_calls.saturating_add(1);
        true
    }

    /// The calls remaining at this instant, including other host reservations.
    #[must_use]
    pub fn remaining_model_calls(&self) -> u32 {
        let Some(cell) = self.inner.cell.upgrade() else {
            return 0;
        };
        let Ok(state) = cell.state.lock() else {
            return 0;
        };
        if !self.inner.is_active(&cell, &state) {
            return 0;
        }
        if state.budget.max_elapsed_ms.is_some_and(|max| {
            state.spent.elapsed_ms.saturating_add(
                u64::try_from(self.inner.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ) >= max
        }) {
            return 0;
        }
        state
            .budget
            .max_model_calls
            .unwrap_or(DEFAULT_MODEL_CALLS)
            .saturating_sub(state.spent.model_calls)
    }

    /// The task's remaining active time at this instant, including this host
    /// work interval. `None` means the task has no elapsed-time cap.
    #[must_use]
    pub fn remaining_ms(&self) -> Option<u64> {
        let cell = self.inner.cell.upgrade()?;
        let state = cell.state.lock().ok()?;
        if !self.inner.is_active(&cell, &state) {
            return Some(0);
        }
        state.budget.max_elapsed_ms.map(|max| {
            max.saturating_sub(state.spent.elapsed_ms).saturating_sub(
                u64::try_from(self.inner.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            )
        })
    }
}

impl fmt::Debug for TaskBudgetScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskBudgetScope")
            .finish_non_exhaustive()
    }
}

struct HostWork {
    cell: Weak<Cell>,
    started: Instant,
    charged: AtomicBool,
    payment_checkpoint: bool,
}

impl HostWork {
    fn is_active(&self, cell: &Cell, state: &super::store::State) -> bool {
        if self.charged.load(Ordering::Acquire) {
            return false;
        }
        match state.host_work {
            super::store::HostWorkState::Active => !self.payment_checkpoint,
            super::store::HostWorkState::PaymentActive => {
                self.payment_checkpoint
                    && (state.payment_checkpoint == super::store::PaymentCheckpointState::Available
                        && matches!(
                            &cell.view.borrow().status,
                            tinycomputer_bus::agent::TaskStatus::Checkpoint {
                                continuable: false,
                                ..
                            }
                        )
                        || state.payment_checkpoint
                            == super::store::PaymentCheckpointState::Transitioned
                            && matches!(
                                &cell.view.borrow().status,
                                tinycomputer_bus::agent::TaskStatus::Running
                            ))
            }
            super::store::HostWorkState::Open | super::store::HostWorkState::Closed => false,
        }
    }

    fn charge(&self) {
        let Some(cell) = self.cell.upgrade() else {
            self.charged.store(true, Ordering::Release);
            return;
        };
        let Ok(mut state) = cell.state.lock() else {
            self.charged.store(true, Ordering::Release);
            return;
        };
        if self.charged.swap(true, Ordering::AcqRel) {
            return;
        }
        state.host_work = match state.host_work {
            super::store::HostWorkState::PaymentActive | super::store::HostWorkState::Closed => {
                super::store::HostWorkState::Closed
            }
            super::store::HostWorkState::Active | super::store::HostWorkState::Open => {
                super::store::HostWorkState::Open
            }
        };
        state.spent.elapsed_ms = state
            .spent
            .elapsed_ms
            .saturating_add(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX));
    }
}

/// Builds the next `RunFlowRequest` for `run`, capped at what the task's
/// budget has not already spent, along with the constraints to run it under
/// and how much of `max_elapsed_ms` remains (`None` when it is unbounded).
/// `None` overall only when the task's state was poisoned by a panic.
pub(super) fn run_request(
    cell: &Cell,
    run: &Run,
) -> Option<(RunFlowRequest, TaskConstraints, Option<u64>)> {
    let state = cell.state.lock().ok()?;
    // Only the caller's values: the flow's own definitions travel with the
    // flow, and the runtime expands them against these. The secret ones are
    // named in `facts`, so they never reach Jev but as `${name}`.
    let vars = state
        .facts
        .names()
        .into_iter()
        .filter_map(|name| Some((name.to_owned(), state.facts.get(name)?.to_owned())))
        .collect::<BTreeMap<_, _>>();
    let facts = fact_names(&state.facts);
    let max_actions = state
        .budget
        .max_actions
        .unwrap_or(120)
        .saturating_sub(state.spent.actions);
    let max_model_calls = state
        .budget
        .max_model_calls
        .unwrap_or(DEFAULT_MODEL_CALLS)
        .saturating_sub(state.spent.model_calls);
    let time_left = state
        .budget
        .max_elapsed_ms
        .map(|max| max.saturating_sub(state.spent.elapsed_ms));
    Some((
        RunFlowRequest {
            flow: run.flow.clone(),
            vars,
            facts,
            fact_handles: state
                .facts
                .secret_names()
                .into_iter()
                .filter_map(|name| {
                    state.facts.handle(name).map(|handle| {
                        (
                            name.to_owned(),
                            tinycomputer_bus::agent::SecretHandle {
                                handle: handle.handle.clone(),
                                mask: handle.mask.clone(),
                                kind: handle.kind.clone(),
                            },
                        )
                    })
                })
                .collect(),
            task_id: Some(cell.view.borrow().id.0.clone()),
            allow_destructive: run.allow_destructive,
            // Jev must read what the fields hold to check what was typed.
            // Secret values are masked in everything the runtime sends Jev.
            include_values: true,
            max_actions,
            max_model_calls,
            votes: state.budget.votes.unwrap_or(DEFAULT_VOTES),
            max_do_turns: state.budget.max_do_turns,
            strategy: state.budget.strategy.unwrap_or_default(),
            deliberation: state.budget.deliberation.unwrap_or_default(),
            brief: brief(&state),
            memory: state.memory.clone(),
            trace: state.trace,
            // What earlier runs saved, so a resumed run remembers it.
            collected: state.reads.clone(),
            dialog_left_open: state.dialog_left_open,
            ..RunFlowRequest::default()
        },
        state.constraints.clone(),
        time_left,
    ))
}

/// Ends a task outright with `status`, without a flow run to interpret: its
/// time budget ran out before a run of it could even start, or a run of it
/// had to be cut off mid-flight to keep from spending past what remains.
pub(super) async fn stop_task(cell: &Cell, runner: &dyn FlowRunner, status: TaskStatus) {
    // A task cut off by its time budget still leaves its last screen.
    let status = captured(cell, runner, status).await;
    let summary = stopped_summary(&status);
    publish(cell, status, &summary);
    runner.release(&cell.view.borrow().id);
}

/// The task-level failure `stop_task` reports when `budget.max_elapsed_ms`
/// is spent: never recoverable by a resume, the same as an action or model
/// budget running out inside a run.
pub(super) fn elapsed_budget_failed() -> TaskStatus {
    TaskStatus::Failed {
        step: None,
        reason: "the task's time budget ran out".to_owned(),
        hint: "raise budget.max_elapsed_ms".to_owned(),
        recoverable: true,
    }
}
