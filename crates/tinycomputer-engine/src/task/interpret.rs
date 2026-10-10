//! What a finished flow run means for the task: carry on, or pause or stop
//! with a status the caller can act on.

use tinycomputer_bus::agent::{PaymentMode, TaskStatus};
use tinycomputer_bus::{
    DesktopResponse, Flow, FlowAction, FlowRunResult, FlowStep, FlowStopReason, StepOutcome,
    StepReport,
};
use tinycomputer_core::{Consequence, consequence};

/// What the task does after a run.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Next {
    /// The run finished; go on to the next one, or finish the task.
    Continue,
    /// Stop with this status. For an approval, `resume` holds what to run
    /// once it is granted.
    Stop {
        status: Box<TaskStatus>,
        resume: Option<Resume>,
    },
}

/// What runs when a paused task is continued.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Resume {
    /// Perform the approved irreversible action, then the steps after it.
    Approval {
        /// The `stop_before` phrase, run again with the action allowed.
        phrase: String,
        /// The application the flow was on when it stopped.
        app: String,
        /// The top-level steps after the paused one.
        rest: Vec<FlowStep>,
    },
    /// Run the failed step again, and everything after it, once a person has
    /// got past what blocked it.
    Retry {
        /// The application the flow was on when it failed.
        app: String,
        /// The failed top-level step and the ones after it.
        steps: Vec<FlowStep>,
    },
}

/// Interprets a run of `flow`, stopped at payment as `payment` says.
pub(super) fn run_outcome(
    flow: &Flow,
    reply: &DesktopResponse,
    payment: PaymentMode,
) -> (Next, Option<FlowRunResult>) {
    if !reply.ok {
        let (reason, hint) = reply.error.as_ref().map_or_else(
            || ("the flow could not run".to_owned(), String::new()),
            |error| {
                (
                    error.message.clone(),
                    error.suggestion.clone().unwrap_or_default(),
                )
            },
        );
        let hint = if hint.is_empty() {
            "check that Jev is configured and the surfaces are available".to_owned()
        } else {
            hint
        };
        return (failed(None, reason, hint, false), None);
    }
    let Some(result) = reply
        .data
        .clone()
        .and_then(|data| serde_json::from_value::<FlowRunResult>(data).ok())
    else {
        return (
            failed(
                None,
                "the flow runtime returned an unreadable result".to_owned(),
                String::new(),
                false,
            ),
            None,
        );
    };
    let next = match result.stop {
        FlowStopReason::Completed => Next::Continue,
        FlowStopReason::StoppedBeforeDestructive => stopped_before(flow, &result, payment),
        FlowStopReason::StepFailed => {
            let failure = result
                .steps
                .iter()
                .rev()
                .find(|step| step.outcome == StepOutcome::Failed);
            let index = failure.and_then(|step| top_index(&step.path));
            let reason =
                failure.map_or_else(|| "a step failed".to_owned(), |step| step.note.clone());
            // No step a rescue writes starts a browser that will not start:
            // the task fails at once, saying what to set. Live, four rescues
            // of a bare `BROWSER_UNAVAILABLE` spent minutes before giving up.
            if failure.is_some_and(no_browser) {
                return (
                    failed(index, reason, NO_BROWSER.to_owned(), false),
                    Some(result),
                );
            }
            let mut next = failed(
                index,
                reason,
                "the screen may not offer what the step describes; reword it, split it, or take over"
                    .to_owned(),
                true,
            );
            if let (Some(index), Next::Stop { resume, .. }) = (index, &mut next) {
                *resume = Some(Resume::Retry {
                    app: app_at(flow, index),
                    steps: flow.steps[index..].to_vec(),
                });
            }
            next
        }
        FlowStopReason::ActionBudget => failed(
            None,
            "the action budget ran out".to_owned(),
            "raise budget.max_actions".to_owned(),
            true,
        ),
        FlowStopReason::ModelBudget => failed(
            None,
            "the decision budget ran out".to_owned(),
            "raise budget.max_model_calls".to_owned(),
            true,
        ),
        FlowStopReason::Invalid => failed(
            None,
            result.steps.first().map_or_else(
                || "the flow is invalid".to_owned(),
                |step| step.note.clone(),
            ),
            "fix the flow; Describe returns the guide".to_owned(),
            false,
        ),
    };
    (next, Some(result))
}

/// A run stopped in front of an irreversible control. Paying is a final
/// checkpoint unless the task was allowed to fill the payment form, when it
/// is an approval like any other irreversible action — never automatic.
fn stopped_before(flow: &Flow, result: &FlowRunResult, payment: PaymentMode) -> Next {
    let gated = result
        .steps
        .iter()
        .rev()
        .find(|step| step.outcome == StepOutcome::Gated);
    let phrase = gated.map_or_else(String::new, |step| step.text.clone());
    let target = result
        .pending
        .as_ref()
        .and_then(|target| target.name.clone())
        .unwrap_or_default();
    let pays = consequence(&target) == Consequence::Payment
        || consequence(&phrase) == Consequence::Payment;
    if pays && payment == PaymentMode::StopAtPayment {
        return Next::Stop {
            status: Box::new(TaskStatus::Checkpoint {
                reason: format!("reached the payment step ({target}); paying is left to you"),
                location: target,
                screenshot: None,
                summary: summary(result),
                continuable: false,
            }),
            resume: None,
        };
    }
    let path = gated.map(|step| step.path.as_str());
    let index = path.and_then(containing_top_index);
    // A top-level `stop_before` has fully finished once it is approved, so
    // the rest resumes right after it. One nested in an `if` or
    // `repeat_until` (path `4.2` or `4.r1.2`) has not: that whole top-level
    // step is still in progress, and which branch or round it was in is not
    // recoverable from the path alone, so the containing step is resumed
    // from its own start rather than silently dropped along with everything
    // that follows it.
    let rest = match (index, path) {
        (Some(index), Some(path)) if path.contains('.') => flow.steps[index..].to_vec(),
        (Some(index), _) => flow.steps[index + 1..].to_vec(),
        (None, _) => Vec::new(),
    };
    Next::Stop {
        status: Box::new(TaskStatus::NeedsApproval {
            action: phrase.clone(),
            target,
            screenshot: None,
        }),
        resume: Some(Resume::Approval {
            phrase,
            app: app_at(flow, index.unwrap_or(flow.steps.len())),
            rest,
        }),
    }
}

/// The application in front at top-level step `index`: the last `open` or
/// `browse` before it, else the flow's own.
pub(super) fn app_at(flow: &Flow, index: usize) -> String {
    flow.steps[..index.min(flow.steps.len())]
        .iter()
        .rev()
        .find_map(|step| match step.action() {
            FlowAction::Open(app) => Some(app),
            FlowAction::Browse(_) => Some(crate::workspace::BROWSER.to_owned()),
            _ => None,
        })
        .unwrap_or_else(|| flow.app.clone())
}

/// The top-level index of a step path such as `3` (index 2); `None` for a
/// nested step such as `4.2`.
pub(super) fn top_index(path: &str) -> Option<usize> {
    path.parse::<usize>().ok()?.checked_sub(1)
}

/// The top-level index a step path belongs to, whether the path names a
/// top-level step directly (`3`) or one nested inside it, such as `4.2` (an
/// `if` branch) or `4.r1.2` (a `repeat_until` round).
pub(super) fn containing_top_index(path: &str) -> Option<usize> {
    path.split('.')
        .next()?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

/// What the finished steps did, in one line.
pub(super) fn summary(result: &FlowRunResult) -> String {
    let done = result
        .steps
        .iter()
        .filter(|step| matches!(step.outcome, StepOutcome::Done | StepOutcome::AlreadyDone))
        .map(|step| step.text.as_str())
        .collect::<Vec<_>>();
    if done.is_empty() {
        "no steps finished".to_owned()
    } else {
        format!("done: {}", done.join("; "))
    }
}

/// Top-level steps a run finished, for progress.
pub(super) fn finished(steps: &[StepReport]) -> usize {
    steps
        .iter()
        .filter(|step| {
            top_index(&step.path).is_some()
                && matches!(step.outcome, StepOutcome::Done | StepOutcome::AlreadyDone)
        })
        .count()
}

/// The hint for a task whose browser could not be started.
const NO_BROWSER: &str = "no browser could be started: give the task the path of Chrome or Chromium (constraints.browser_executable, or the module's browser.executable), then start it again";

/// Whether `step` failed because no browser could be started: its last
/// action, opening the browser or an address in it, was refused with
/// `BROWSER_UNAVAILABLE`.
fn no_browser(step: &StepReport) -> bool {
    step.actions
        .last()
        .is_some_and(|action| !action.ok && action.note == "BROWSER_UNAVAILABLE")
}

fn failed(step: Option<usize>, reason: String, hint: String, recoverable: bool) -> Next {
    Next::Stop {
        status: Box::new(TaskStatus::Failed {
            step,
            reason,
            hint,
            recoverable,
        }),
        resume: None,
    }
}
