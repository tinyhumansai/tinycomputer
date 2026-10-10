//! [`Tasks`], the controller, and the calls it answers: planning, starting,
//! awaiting, continuing, cancelling, and reporting a task.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinycomputer_bus::agent::{
    AgentError, AgentResponse, AwaitTaskRequest, ContinueTaskRequest, LanguageModelConfiguration,
    PaymentMode, PlanTaskRequest, StartTaskRequest, TaskId, TaskPlan, TaskReport,
    TaskReportRequest, TaskStatus, TaskView,
};
use tinycomputer_core::Facts;

use super::errors::{no_planner, no_such_task, poisoned, too_many};
use super::names::{fact_names, is_undefined, known_names};
use super::publish::{needs_input, publish, records, state_name};
use super::store::{Cell, Run};
use super::{FlowRunner, MAX_AWAIT_MS};
use crate::rescue::Rescuer;
use crate::shape::Shaper;

/// The task controller.
pub struct Tasks {
    pub(super) runner: Arc<dyn FlowRunner>,
    pub(super) planner: Option<crate::planner::Planner>,
    pub(super) rescuer: Option<Rescuer>,
    pub(super) shaper: Option<Shaper>,
    pub(super) cells: Mutex<BTreeMap<u64, Arc<Cell>>>,
    pub(super) counter: AtomicU64,
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Tasks").finish_non_exhaustive()
    }
}

impl Tasks {
    /// A controller that runs flows with `runner`.
    #[must_use]
    pub fn new(runner: Arc<dyn FlowRunner>) -> Self {
        Self {
            runner,
            planner: None,
            rescuer: None,
            shaper: None,
            cells: Mutex::new(BTreeMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    /// This controller, turning plain-language tasks into flows with
    /// `planner`.
    #[must_use]
    pub fn with_planner(mut self, planner: crate::planner::Planner) -> Self {
        self.planner = Some(planner);
        self
    }

    /// This controller, handing a failed step to `rescuer` for guidance
    /// before the task fails.
    #[must_use]
    pub fn with_rescuer(mut self, rescuer: Rescuer) -> Self {
        self.rescuer = Some(rescuer);
        self
    }

    /// This controller, turning what a finished task read into the shape
    /// its `output` asks for with `shaper`.
    #[must_use]
    pub fn with_shaper(mut self, shaper: Shaper) -> Self {
        self.shaper = Some(shaper);
        self
    }

    /// Whether a task may ask for its answer in a shape.
    #[must_use]
    pub fn output_configured(&self) -> bool {
        self.shaper.is_some()
    }

    /// Whether a rescuer is configured.
    #[must_use]
    pub fn rescue_configured(&self) -> bool {
        self.rescuer.is_some()
    }

    /// Whether a planner is configured.
    #[must_use]
    pub fn planner_configured(&self) -> bool {
        self.planner.is_some()
    }

    /// The planner's route and model, when a planner is configured and says.
    #[must_use]
    pub fn planner_model(&self) -> Option<&LanguageModelConfiguration> {
        self.planner.as_ref()?.configuration()
    }

    /// The rescuer's route and model, when a rescuer is configured and says.
    #[must_use]
    pub fn rescue_model(&self) -> Option<&LanguageModelConfiguration> {
        self.rescuer.as_ref()?.configuration()
    }

    /// The shaper's route and model, when a shaper is configured and says.
    #[must_use]
    pub fn output_model(&self) -> Option<&LanguageModelConfiguration> {
        self.shaper.as_ref()?.configuration()
    }

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
            super::timing::planned(&outcome, used, drafting, planner.configuration())
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

    /// Starts a task and returns at once with its first view.
    ///
    /// # Panics
    ///
    /// When called outside a Tokio runtime: the task runs on a spawned worker.
    #[must_use]
    pub fn start(&self, request: &StartTaskRequest) -> AgentResponse<TaskView> {
        let facts = match Facts::with_secrets(request.facts.clone(), request.secret_facts.clone()) {
            Ok(facts) => facts,
            Err(error) => {
                return AgentResponse::err(AgentError::new(
                    "UNKNOWN_SECRET",
                    error.to_string(),
                    "name only facts you pass in secret_facts",
                    true,
                ));
            }
        };
        if let Some(output) = &request.output {
            if self.shaper.is_none() {
                return AgentResponse::err(AgentError::new(
                    "OUTPUT_UNAVAILABLE",
                    "an output shape needs the planner configured, which shapes the answer",
                    "configure the planner, or leave out `output` and read `done.records`",
                    true,
                ));
            }
            if let Some(Err(problem)) = output.schema.as_ref().map(crate::shape::schema::supported)
            {
                return AgentResponse::err(AgentError::new(
                    "INVALID_OUTPUT",
                    problem,
                    "use only the supported schema keywords; TaskOutput lists them",
                    true,
                ));
            }
        }
        if let Some(refusal) = constraints_refusal(&request.constraints) {
            return AgentResponse::err(refusal);
        }
        let Some(flow) = request.flow.clone() else {
            if let (Some(task), Some(planner)) = (&request.task, &self.planner) {
                return self.start_planned(request, facts, task, planner.clone());
            }
            if request.task.is_some() {
                return self.register_planless(request);
            }
            return AgentResponse::err(AgentError::new(
                "INVALID_REQUEST",
                "a task needs `flow`, or `task` with a planner configured",
                "write a flow with Describe's guide and pass it as `flow`",
                true,
            ));
        };
        let known = known_names(&flow, &facts);
        let fact_names = fact_names(&facts);
        let validation = crate::agentic::check_flow(&flow, &known, &fact_names);
        let problems = validation
            .errors
            .iter()
            .filter(|error| !is_undefined(error))
            .cloned()
            .collect::<Vec<_>>();
        if !problems.is_empty() {
            return AgentResponse::err(AgentError::new(
                "INVALID_FLOW",
                problems.join("; "),
                "fix the flow; Describe returns the guide",
                true,
            ));
        }
        let Some(cell) = self.register(&flow, facts, request) else {
            return too_many();
        };
        let missing = crate::agentic::missing_inputs(&flow, &known, &fact_names);
        if missing.is_empty() {
            self.spawn(
                &cell,
                vec![Run {
                    allow_destructive: request.constraints.allow_destructive,
                    flow,
                    rescue: None,
                }],
            );
        } else {
            publish(
                &cell,
                needs_input(&missing),
                "The task needs values before it can start.",
            );
        }
        AgentResponse::ok(cell.view.borrow().clone())
    }

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
        AgentResponse::ok(changes.borrow().clone())
    }

    /// Answers what a paused task asked for, and resumes it.
    #[must_use]
    pub fn continue_task(&self, request: ContinueTaskRequest) -> AgentResponse<TaskView> {
        let Some(cell) = self.find(&request.id) else {
            return no_such_task(&request.id);
        };
        let status = cell.view.borrow().status.clone();
        let waited = cell
            .state
            .lock()
            .ok()
            .and_then(|state| state.waiting_since)
            .map(|since| since.elapsed());
        let id = request.id.clone();
        let paused = state_name(&status);
        let reply = match status {
            TaskStatus::NeedsInput { .. } => self.supply(&cell, request),
            TaskStatus::NeedsApproval { .. } => self.decide(&cell, request.approve),
            TaskStatus::NeedsHuman { .. } => self.retry(&cell),
            other => AgentResponse::err(AgentError::new(
                "NOT_WAITING",
                format!(
                    "the task is not waiting for an answer: {}",
                    state_name(&other)
                ),
                "call AwaitTask until the task asks for something",
                true,
            )),
        };
        // The wait is over only once the task has left it: an answer that is
        // refused, or that still leaves values missing, keeps it waiting.
        let left = cell
            .state
            .lock()
            .is_ok_and(|state| state.waiting_since.is_none());
        if let (Some(waited), true) = (waited, left) {
            self.runner.journal(Some(&id), "resume", &|| {
                super::timing::resumed(paused, waited)
            });
        }
        reply
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
        if !cell.view.borrow().status.is_final() {
            if let Some(worker) = cell.worker.lock().ok().and_then(|mut worker| worker.take()) {
                worker.abort();
            }
            publish(&cell, TaskStatus::Cancelled, "The task was cancelled.");
        }
        self.runner.release(id);
        AgentResponse::ok(cell.view.borrow().clone())
    }

    /// Everything the task did.
    #[must_use]
    pub fn report(&self, id: &TaskId) -> AgentResponse<TaskReport> {
        let Some(cell) = self.find(id) else {
            return no_such_task(id);
        };
        let view = cell.view.borrow().clone();
        let Ok(state) = cell.state.lock() else {
            return poisoned();
        };
        AgentResponse::ok(TaskReport {
            view,
            flow: Some(state.flow.clone()),
            steps: state.steps.clone(),
            records: records(&state.reads),
            artifacts: state.artifacts.clone(),
            learned: state.learned.clone(),
            trace: state.exchanges.clone(),
            rescues: state.rescues.clone(),
        })
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
                .map(|cell| cell.view.borrow().clone())
                .collect(),
        )
    }
}

/// Why a task's constraints cannot start it, if they cannot: a payment form
/// filled with no named site to type card details on (`*` names none), a
/// relative profile folder, which would land wherever the module's host
/// happens to run, a browser binary that is no absolute path to an
/// executable file on this machine (a bare name would be looked up on the
/// `PATH`), or either
/// beside a browser to attach to, which launches nothing. Paths are taken
/// as given: one with a space around it names another folder or file.
fn constraints_refusal(
    constraints: &tinycomputer_bus::agent::TaskConstraints,
) -> Option<AgentError> {
    if constraints.payment == PaymentMode::FillThenApprove
        && (constraints.origins.is_empty()
            || constraints
                .origins
                .iter()
                .any(|origin| origin.trim() == "*"))
    {
        return Some(AgentError::new(
            "ORIGINS_REQUIRED",
            "filling a payment form needs the sites card details may be typed on, and `*` names none",
            "list them in constraints.origins, or leave payment at stop_at_payment",
            true,
        ));
    }
    let launched =
        constraints.browser_profile.is_some() || constraints.browser_executable.is_some();
    let browser = if launched && constraints.browser_endpoint.is_some() {
        "browser_profile and browser_executable choose a browser to launch, and browser_endpoint attaches to one already running"
    } else if constraints
        .browser_profile
        .as_deref()
        .is_some_and(|folder| !std::path::Path::new(folder).is_absolute())
    {
        "browser_profile must be an absolute folder"
    } else if constraints
        .browser_executable
        .as_deref()
        .is_some_and(|binary| {
            let binary = std::path::Path::new(binary);
            !binary.is_absolute() || !launchable(binary)
        })
    {
        "browser_executable must be the absolute path of a browser binary on this machine"
    } else {
        return None;
    };
    Some(AgentError::new(
        "INVALID_REQUEST",
        browser,
        "give constraints.browser_profile as an absolute folder and browser_executable as the absolute path of a browser binary, or leave them out; neither goes with browser_endpoint",
        true,
    ))
}

/// Whether `binary` is a file this machine would run: one marked executable
/// where files carry that mark.
fn launchable(binary: &std::path::Path) -> bool {
    let Ok(metadata) = binary.metadata() else {
        return false;
    };
    #[cfg(unix)]
    let runs = std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o111 != 0;
    #[cfg(not(unix))]
    let runs = true;
    metadata.is_file() && runs
}
