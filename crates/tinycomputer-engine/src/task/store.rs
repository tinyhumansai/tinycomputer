//! Where tasks live: each task's cell, its working state, and registering,
//! spawning, and finding one.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tinycomputer_bus::agent::{
    AgentResponse, Rescue, StartTaskRequest, TaskBudget, TaskConstraints, TaskId, TaskOutput,
    TaskStatus, TaskUsage, TaskView,
};
use tinycomputer_bus::browser::OutputRef;
use tinycomputer_bus::{FLOW_GUIDE, Flow, GroundingHint, JevExchange, StepReport};
use tinycomputer_core::Facts;
use tokio::sync::watch;

use super::drive::{drive, plan_then_drive};
use super::errors::too_many;
use super::interpret::Resume;
use super::publish::{next_calls, publish};
use super::{MAX_TASKS, TaskCallBudget, Tasks};
use crate::UsageRecorder;
use crate::rescue::Rescuer;
use crate::shape::Shaper;

/// One task: its view, published to waiters, and its working state.
pub(super) struct Cell {
    pub(super) view: watch::Sender<TaskView>,
    pub(super) state: Mutex<State>,
    pub(super) worker: Mutex<Option<WorkerSlot>>,
    /// Who a failed step is handed to before the task fails.
    pub(super) rescuer: Option<Rescuer>,
    /// Who turns what a finished task read into the shape it asked for.
    pub(super) shaper: Option<Shaper>,
    pub(super) planner: Option<crate::planner::Planner>,
}

pub(super) struct WorkerSlot {
    pub(super) abort: tokio::task::AbortHandle,
    pub(super) done: watch::Receiver<bool>,
}

impl Cell {
    /// The latest published state with usage accumulated since that update.
    pub(super) fn current_view(&self) -> TaskView {
        let mut view = self.view.borrow().clone();
        if let Ok(state) = self.state.lock() {
            view.usage = state.usage.snapshot();
        }
        view
    }
}

pub(super) struct State {
    pub(super) usage: Arc<UsageRecorder>,
    pub(super) flow: Flow,
    /// Run the supplied prefix once, then plan the original goal against the
    /// surface it left open.
    pub(super) handoff_to_planner: bool,
    /// The task in the caller's words, for Jev's brief; empty when only a
    /// flow was given.
    pub(super) goal: String,
    pub(super) facts: Facts,
    /// Secret fact names whose inline values were deliberately not saved.
    pub(super) omitted_secrets: std::collections::BTreeSet<String>,
    pub(super) constraints: TaskConstraints,
    pub(super) budget: TaskBudget,
    pub(super) memory: Vec<GroundingHint>,
    pub(super) trace: bool,
    pub(super) steps: Vec<StepReport>,
    pub(super) exchanges: Vec<JevExchange>,
    pub(super) learned: Vec<GroundingHint>,
    pub(super) reads: BTreeMap<String, String>,
    /// Whether the task's last run left its own dialog in front, for the
    /// next run to work within ([`RunFlowRequest::dialog_left_open`]).
    ///
    /// [`RunFlowRequest::dialog_left_open`]: tinycomputer_bus::RunFlowRequest::dialog_left_open
    pub(super) dialog_left_open: bool,
    pub(super) finished: usize,
    pub(super) resume: Option<Resume>,
    /// Host-only lifecycle for the task's one payment checkpoint grant.
    pub(super) payment_checkpoint: PaymentCheckpointState,
    /// What every run of this task has spent so far, so an approval or a
    /// human intervention that splits a task into several runs still cannot
    /// exceed its declared budget by starting each run with a fresh one.
    pub(super) spent: Spent,
    /// Host model work is single-flight and closed after terminal task status.
    pub(super) host_work: HostWorkState,
    /// Every rescue so far, in order.
    pub(super) rescues: Vec<Rescue>,
    /// The shape the caller wants the answer in, if any.
    pub(super) output: Option<TaskOutput>,
    /// Screenshots taken each time a run stopped, oldest first.
    pub(super) artifacts: Vec<OutputRef>,
    /// When the task began waiting for an answer it is still waiting for,
    /// to journal how long the person took.
    pub(super) waiting_since: Option<std::time::Instant>,
    /// A pause to return to after a caller re-enters an omitted secret fact
    /// while restoring a task.
    pub(super) return_after_input: Option<TaskStatus>,
}

/// A task's cumulative spend against its [`TaskBudget`], across every run.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub(super) struct Spent {
    pub(super) actions: u32,
    pub(super) model_calls: u32,
    pub(super) elapsed_ms: u64,
}

/// One flow run in a task's sequence.
pub(super) struct Run {
    pub(super) flow: Flow,
    pub(super) allow_destructive: bool,
    /// The rescue whose guidance opens this run, by its index in
    /// `State::rescues`.
    pub(super) rescue: Option<usize>,
}

impl Tasks {
    pub(super) fn register(
        &self,
        flow: &Flow,
        facts: Facts,
        request: &StartTaskRequest,
    ) -> Option<Arc<Cell>> {
        // Imports reserve their restored id while holding this same index
        // lock. Allocate under it so a start cannot reserve that id before
        // blocking on the index and then replace the imported cell.
        let mut tasks = self.cells.lock().ok()?;
        let number = loop {
            let current = self.counter.load(Ordering::Relaxed);
            let number = current.checked_add(1)?;
            if self
                .counter
                .compare_exchange_weak(current, number, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break number;
            }
        };
        let id = TaskId::new(format!("t-{number}"));
        let usage = Arc::new(UsageRecorder::default());
        let view = TaskView {
            id,
            status: TaskStatus::Running,
            summary: "The task is starting.".to_owned(),
            step: None,
            progress: 0.0,
            next: next_calls(&TaskStatus::Running),
            usage: TaskUsage::default(),
        };
        let cell = Arc::new(Cell {
            view: watch::Sender::new(view),
            state: Mutex::new(State {
                usage: usage.clone(),
                flow: flow.clone(),
                handoff_to_planner: request.handoff_to_planner,
                goal: request.task.clone().unwrap_or_default(),
                facts,
                omitted_secrets: BTreeSet::new(),
                constraints: request.constraints.clone(),
                budget: request.budget,
                memory: request.memory.clone(),
                trace: request.trace,
                steps: Vec::new(),
                exchanges: Vec::new(),
                learned: Vec::new(),
                reads: BTreeMap::new(),
                dialog_left_open: false,
                finished: 0,
                resume: None,
                payment_checkpoint: PaymentCheckpointState::Unavailable,
                spent: Spent::default(),
                host_work: HostWorkState::Open,
                rescues: Vec::new(),
                output: request.output.clone(),
                artifacts: Vec::new(),
                waiting_since: None,
                return_after_input: None,
            }),
            worker: Mutex::new(None),
            rescuer: self.rescuer.clone(),
            shaper: self.shaper.clone(),
            planner: self.planner.clone(),
        });
        usage.set_task_call_budget(TaskCallBudget::for_cell(Arc::downgrade(&cell)));
        while tasks.len() >= MAX_TASKS {
            let oldest_final = tasks
                .iter()
                .find(|(_, cell)| cell.view.borrow().status.is_final())
                .map(|(number, _)| *number)?;
            tasks.remove(&oldest_final);
        }
        tasks.insert(number, cell.clone());
        Some(cell)
    }

    pub(super) fn register_planless(
        &self,
        request: &StartTaskRequest,
        facts: Facts,
    ) -> AgentResponse<TaskView> {
        let flow = Flow {
            app: String::new(),
            vars: BTreeMap::new(),
            steps: Vec::new(),
        };
        let Some(cell) = self.register(&flow, facts, request) else {
            return too_many();
        };
        if self.runner.plan_recovery_enabled() {
            self.start_planless_recovery(&cell, request.task.as_deref().unwrap_or_default());
            return AgentResponse::ok(cell.current_view());
        }
        publish(
            &cell,
            TaskStatus::NeedsPlan {
                guide: FLOW_GUIDE.to_owned(),
            },
            "No planner is configured: write a flow for this task with the guide and start it again.",
        );
        AgentResponse::ok(cell.current_view())
    }

    fn start_planless_recovery(&self, cell: &Arc<Cell>, task: &str) {
        publish(
            cell,
            TaskStatus::Running,
            "Writing a safe plan for the task.",
        );
        let (done, done_rx) = watch::channel(false);
        let running_cell = cell.clone();
        let runner = self.runner.clone();
        let task = task.to_owned();
        let worker = tokio::spawn(async move {
            let _completion = WorkerCompletion(done);
            let id = running_cell.view.borrow().id.clone();
            let constraints = {
                let Ok(state) = running_cell.state.lock() else {
                    return;
                };
                state.constraints.clone()
            };
            runner.prepare(&id, &constraints).await;
            match super::plan_recovery::host_plan(
                &running_cell,
                runner.as_ref(),
                task,
                FLOW_GUIDE.to_owned(),
            )
            .await
            {
                Ok(flow) => {
                    let Ok(facts) = running_cell.state.lock().map(|state| state.facts.clone())
                    else {
                        runner.release(&id);
                        return;
                    };
                    let known = super::names::known_names(&flow, &facts);
                    let fact_names = super::names::fact_names(&facts);
                    let missing = crate::agentic::missing_inputs(&flow, &known, &fact_names);
                    if let Ok(mut state) = running_cell.state.lock() {
                        state.flow = flow.clone();
                    }
                    if !missing.is_empty() {
                        runner.release(&id);
                        publish(
                            &running_cell,
                            super::publish::needs_input(&missing),
                            "The recovered plan needs caller values before it can start.",
                        );
                        return;
                    }
                    drive(
                        running_cell,
                        runner,
                        vec![Run {
                            flow,
                            allow_destructive: constraints.allow_destructive,
                            rescue: None,
                        }],
                    )
                    .await;
                }
                Err(reason) => {
                    runner.release(&id);
                    publish(
                        &running_cell,
                        TaskStatus::NeedsPlan {
                            guide: FLOW_GUIDE.to_owned(),
                        },
                        &format!("The host could not write a valid plan: {reason}"),
                    );
                }
            }
        });
        if let Ok(mut slot) = cell.worker.lock() {
            *slot = Some(WorkerSlot {
                abort: worker.abort_handle(),
                done: done_rx,
            });
        }
    }

    pub(super) fn start_planned(
        &self,
        request: &StartTaskRequest,
        facts: Facts,
        task: &str,
        planner: crate::planner::Planner,
    ) -> AgentResponse<TaskView> {
        let placeholder = Flow {
            app: String::new(),
            vars: BTreeMap::new(),
            steps: Vec::new(),
        };
        let Some(cell) = self.register(&placeholder, facts, request) else {
            return too_many();
        };
        publish(&cell, TaskStatus::Running, "Planning the task.");
        let (done, done_rx) = watch::channel(false);
        let running_cell = cell.clone();
        let runner = self.runner.clone();
        let task = task.to_owned();
        let surfaces = request.constraints.surfaces.clone();
        let completion = WorkerCompletion(done);
        let worker = tokio::spawn(async move {
            let _completion = completion;
            plan_then_drive(running_cell, runner, planner, task, surfaces).await;
        });
        if let Ok(mut slot) = cell.worker.lock() {
            *slot = Some(WorkerSlot {
                abort: worker.abort_handle(),
                done: done_rx,
            });
        }
        AgentResponse::ok(cell.current_view())
    }

    pub(super) fn spawn(&self, cell: &Arc<Cell>, runs: Vec<Run>) {
        publish(cell, TaskStatus::Running, "The task is running.");
        let (done, done_rx) = watch::channel(false);
        let running_cell = cell.clone();
        let runner = self.runner.clone();
        let completion = WorkerCompletion(done);
        let worker = tokio::spawn(async move {
            let _completion = completion;
            drive(running_cell, runner, runs).await;
        });
        if let Ok(mut slot) = cell.worker.lock() {
            *slot = Some(WorkerSlot {
                abort: worker.abort_handle(),
                done: done_rx,
            });
        }
    }

    pub(super) fn find(&self, id: &TaskId) -> Option<Arc<Cell>> {
        let number = id.0.strip_prefix("t-")?.parse().ok()?;
        self.cells.lock().ok()?.get(&number).cloned()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum PaymentCheckpointState {
    /// The task has not reached a payment checkpoint.
    #[default]
    Unavailable,
    /// A non-continuable payment checkpoint may be exchanged by the host.
    Available,
    /// The one host continuation was consumed.
    Transitioned,
    /// The checkpoint was cancelled before the host authorized it.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostWorkState {
    Open,
    Active,
    PaymentActive,
    Closed,
}

pub(super) struct WorkerCompletion(watch::Sender<bool>);

impl WorkerCompletion {
    pub(super) fn new(done: watch::Sender<bool>) -> Self {
        Self(done)
    }
}

impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
