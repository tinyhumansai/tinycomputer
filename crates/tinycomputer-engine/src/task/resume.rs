//! Answering a paused task: supplying the values it needs, approving or
//! declining its irreversible action, and retrying once a person got it past
//! a wall.

use std::sync::Arc;

use tinycomputer_bus::agent::{
    AgentError, AgentResponse, ContinueTaskRequest, TaskStatus, TaskView,
};
use tinycomputer_bus::{Flow, FlowAction, FlowStep};
use tinycomputer_core::{Facts, SecretFactHandle};

use super::Tasks;
use super::errors::poisoned;
use super::interpret::Resume;
use super::names::{fact_names, is_undefined, known_names};
use super::publish::{needs_input, publish, stopped_summary};
use super::store::{Cell, Run};

impl Tasks {
    pub(super) fn supply(
        &self,
        cell: &Arc<Cell>,
        request: ContinueTaskRequest,
    ) -> AgentResponse<TaskView> {
        let Ok(mut state) = cell.state.lock() else {
            return poisoned();
        };
        // A supplied value is secret when its name says so, like any other.
        // Handles stay with the host; only their opaque references enter task
        // state and the model-facing request.
        let supplied = match Facts::with_handles(
            request.inputs,
            request.fact_handles.into_iter().map(|(name, handle)| {
                (
                    name,
                    SecretFactHandle {
                        handle: handle.handle,
                        mask: handle.mask,
                        kind: handle.kind,
                    },
                )
            }),
            std::iter::empty::<String>(),
        ) {
            Ok(facts) => facts,
            Err(error) => {
                return AgentResponse::err(AgentError::new(
                    "INVALID_SECRET_FACT",
                    error.to_string(),
                    "provide each secret input as a non-empty host handle with a unique name",
                    true,
                ));
            }
        };
        for name in supplied.names() {
            state.omitted_secrets.remove(name);
        }
        state.facts = state.facts.merged(&supplied);
        let mut input_flow = state.flow.clone();
        if let Some(Resume::Retry { steps, .. }) = &state.resume {
            input_flow.steps.extend(steps.iter().cloned());
        }
        let known = known_names(&input_flow, &state.facts);
        let facts = fact_names(&state.facts);
        let missing = crate::agentic::missing_inputs(&input_flow, &known, &facts);
        if !missing.is_empty() {
            drop(state);
            publish(
                cell,
                needs_input(&missing),
                "The task still needs values before it can start.",
            );
            return AgentResponse::ok(cell.current_view());
        }
        // A newly supplied value can turn a reference that only looked
        // undefined at `StartTask` into a fact used somewhere Jev must never
        // see it, so the flow is checked again in full now that every name
        // it uses is finally known, rather than trusting the check `start`
        // already ran against an incomplete `facts` set.
        let problems = crate::agentic::check_flow(&state.flow, &known, &facts)
            .errors
            .into_iter()
            .filter(|error| !is_undefined(error))
            .collect::<Vec<_>>();
        if !problems.is_empty() {
            drop(state);
            return AgentResponse::err(AgentError::new(
                "INVALID_FLOW",
                problems.join("; "),
                "fix the flow; Describe returns the guide",
                true,
            ));
        }
        if let Some(status) = state.return_after_input.take() {
            let summary = stopped_summary(&status);
            drop(state);
            publish(cell, status, &summary);
            return AgentResponse::ok(cell.current_view());
        }
        let run = Run {
            flow: state.flow.clone(),
            allow_destructive: state.constraints.allow_destructive,
            rescue: None,
        };
        drop(state);
        self.spawn(cell, vec![run]);
        AgentResponse::ok(cell.current_view())
    }

    pub(super) fn decide(
        &self,
        cell: &Arc<Cell>,
        approve: Option<bool>,
    ) -> AgentResponse<TaskView> {
        let Some(approve) = approve else {
            return AgentResponse::err(AgentError::new(
                "APPROVAL_REQUIRED",
                "the task is waiting for an approval",
                "call ContinueTask with approve set to true or false",
                true,
            ));
        };
        let Ok(mut state) = cell.state.lock() else {
            return poisoned();
        };
        let resume = state.resume.take();
        let allow = state.constraints.allow_destructive;
        let vars = state.flow.vars.clone();
        drop(state);
        if let (true, Some(Resume::Approval { phrase, app, rest })) = (approve, resume) {
            let mut runs = vec![Run {
                flow: Flow {
                    app: app.clone(),
                    vars: vars.clone(),
                    steps: vec![FlowStep::Action(FlowAction::StopBefore(phrase))],
                },
                allow_destructive: true,
                rescue: None,
            }];
            if !rest.is_empty() {
                runs.push(Run {
                    flow: Flow {
                        app,
                        vars,
                        steps: rest,
                    },
                    allow_destructive: allow,
                    rescue: None,
                });
            }
            self.spawn(cell, runs);
        } else {
            publish(
                cell,
                TaskStatus::Cancelled,
                "The irreversible action was declined, so the task stopped before it.",
            );
            self.runner.release(&cell.view.borrow().id);
        }
        AgentResponse::ok(cell.current_view())
    }

    /// Runs the step a person has just got the task past, and the rest.
    pub(super) fn retry(&self, cell: &Arc<Cell>) -> AgentResponse<TaskView> {
        let Ok(mut state) = cell.state.lock() else {
            return poisoned();
        };
        let resume = state.resume.take();
        let allow = state.constraints.allow_destructive;
        let vars = state.flow.vars.clone();
        drop(state);
        if let Some(Resume::Retry { app, steps }) = resume {
            self.spawn(
                cell,
                vec![Run {
                    flow: Flow { app, vars, steps },
                    allow_destructive: allow,
                    rescue: None,
                }],
            );
        }
        AgentResponse::ok(cell.current_view())
    }
}
