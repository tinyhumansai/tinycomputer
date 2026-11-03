//! Validate continuation plans and prevent reopening the shop start page.

use std::sync::Arc;

use crate::task::publish::publish;
use crate::task::store::Cell;
use tinycomputer_bus::agent::TaskStatus;

pub(super) fn ensure_handoff_budget(cell: &Cell) -> Result<(), String> {
    let state = cell
        .state
        .lock()
        .map_err(|_| "task state is unavailable".to_owned())?;
    if state.spent.model_calls
        > state
            .budget
            .max_model_calls
            .unwrap_or(crate::task::DEFAULT_MODEL_CALLS)
    {
        return Err("continuation planning exhausted the task model call budget".to_owned());
    }
    if state
        .budget
        .max_elapsed_ms
        .is_some_and(|max| state.spent.elapsed_ms > max)
    {
        return Err("continuation planning exhausted the task time budget".to_owned());
    }
    Ok(())
}

pub(super) fn validate_handoff_plan(
    cell: &Arc<Cell>,
    plan: tinycomputer_bus::agent::TaskPlan,
    facts: &tinycomputer_core::Facts,
    origin: &str,
) -> Result<Option<tinycomputer_bus::Flow>, String> {
    let known = crate::task::names::known_names(&plan.flow, facts);
    let fact_names = crate::task::names::fact_names(facts);
    let errors = crate::agentic::check_flow(&plan.flow, &known, &fact_names)
        .errors
        .into_iter()
        .filter(|error| !crate::task::names::is_undefined(error))
        .collect::<Vec<_>>();
    if plan.flow.steps.is_empty() || !errors.is_empty() {
        return Err(if plan.flow.steps.is_empty() {
            "the planner returned an empty continuation".to_owned()
        } else {
            format!(
                "the planner returned an invalid continuation: {}",
                errors.join("; ")
            )
        });
    }
    if reopens_origin(&plan.flow, origin) {
        return Err("the planner tried to reopen the current shop's start page".to_owned());
    }
    let missing = crate::agentic::missing_inputs(&plan.flow, &known, &fact_names);
    let waiting = if plan.questions.is_empty() {
        (!missing.is_empty()).then(|| crate::task::publish::needs_input(&missing))
    } else {
        Some(TaskStatus::NeedsInput {
            fields: plan.questions,
        })
    };
    if let Ok(mut state) = cell.state.lock() {
        state.flow = plan.flow.clone();
    }
    if let Some(status) = waiting {
        publish(
            cell,
            status,
            "The remaining task needs values before it can continue.",
        );
        return Ok(None);
    }
    Ok(Some(plan.flow))
}

fn reopens_origin(flow: &tinycomputer_bus::Flow, origin: &str) -> bool {
    let Ok(origin) = url::Url::parse(origin) else {
        return true;
    };
    if origin.scheme() != "https" || !origin.username().is_empty() || origin.password().is_some() {
        return true;
    }
    reopens_origin_in_steps(&flow.steps, &origin)
}

fn reopens_origin_in_steps(steps: &[tinycomputer_bus::FlowStep], origin: &url::Url) -> bool {
    use tinycomputer_bus::FlowAction;
    steps.iter().any(|step| match step.action() {
        FlowAction::Browse(address) => {
            let Ok(url) = url::Url::parse(&address) else {
                return false;
            };
            url.scheme() == "https"
                && url.host_str() == origin.host_str()
                && url.port_or_known_default() == origin.port_or_known_default()
                && url.username().is_empty()
                && url.password().is_none()
                && is_front_page(url.path())
        }
        FlowAction::If(branch) => {
            reopens_origin_in_steps(&branch.then, origin)
                || reopens_origin_in_steps(&branch.otherwise, origin)
        }
        FlowAction::RepeatUntil(repeat) => reopens_origin_in_steps(&repeat.steps, origin),
        _ => false,
    })
}

fn is_front_page(path: &str) -> bool {
    let segments = path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty());
    let segments = segments.collect::<Vec<_>>();
    segments.len() <= 2
        && segments.iter().all(|segment| {
            let bytes = segment.as_bytes();
            (bytes.len() == 2 && bytes.iter().all(u8::is_ascii_alphabetic))
                || (bytes.len() == 5
                    && bytes[..2].iter().all(u8::is_ascii_alphabetic)
                    && matches!(bytes[2], b'-' | b'_')
                    && bytes[3..].iter().all(u8::is_ascii_alphabetic))
        })
}
