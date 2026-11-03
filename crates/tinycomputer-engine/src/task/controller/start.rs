//! Validating and starting tasks.

use tinycomputer_bus::agent::{AgentError, AgentResponse, PaymentMode, StartTaskRequest, TaskView};
use tinycomputer_core::{Facts, SecretFactHandle};

use super::Tasks;
use crate::task::errors::{no_planner, too_many};
use crate::task::names::{fact_names, is_undefined, known_names};
use crate::task::publish::{needs_input, publish};
use crate::task::store::Run;

impl Tasks {
    /// Starts a task and returns at once with its first view.
    ///
    /// # Panics
    ///
    /// When called outside a Tokio runtime: the task runs on a spawned worker.
    #[must_use]
    pub fn start(&self, request: &StartTaskRequest) -> AgentResponse<TaskView> {
        let facts = match Facts::with_handles(
            request.facts.clone(),
            request.fact_handles.iter().map(|(name, handle)| {
                (
                    name.clone(),
                    SecretFactHandle {
                        handle: handle.handle.clone(),
                        mask: handle.mask.clone(),
                        kind: handle.kind.clone(),
                    },
                )
            }),
            request.secret_facts.clone(),
        ) {
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
        if let Some(error) = self.start_refusal(request) {
            return AgentResponse::err(error);
        }
        let Some(flow) = request.flow.clone() else {
            if let (Some(task), Some(planner)) = (&request.task, &self.planner) {
                return self.start_planned(request, facts, task, planner.clone());
            }
            if request.task.is_some() {
                return self.register_planless(request, facts);
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
        AgentResponse::ok(cell.current_view())
    }

    fn start_refusal(&self, request: &StartTaskRequest) -> Option<AgentError> {
        constraints_refusal(&request.constraints).or_else(|| {
            handoff_refusal(
                request,
                self.planner.is_some(),
                self.runner.plan_recovery_enabled(),
            )
        })
    }
}

fn handoff_refusal(
    request: &StartTaskRequest,
    planner_configured: bool,
    host_recovery_enabled: bool,
) -> Option<AgentError> {
    if !request.handoff_to_planner {
        return None;
    }
    if request.flow.is_none()
        || request
            .task
            .as_deref()
            .is_none_or(|task| task.trim().is_empty())
    {
        return Some(AgentError::new(
            "INVALID_REQUEST",
            "planner handoff needs both a supplied flow and a non-empty task goal",
            "pass `task`, `flow`, and `handoff_to_planner: true` together",
            true,
        ));
    }
    if !planner_configured && !host_recovery_enabled {
        return Some(no_planner());
    }
    None
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
