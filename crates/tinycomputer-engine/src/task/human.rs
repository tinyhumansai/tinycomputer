//! Telling a wall only a person can pass from an ordinary failure, and
//! trying a captcha before asking a person to.

use tinycomputer_bus::agent::{TaskId, TaskStatus};

use super::FlowRunner;
use super::interpret::Resume;
use super::store::{Cell, Run};
use crate::CaptchaOutcome;
use crate::tinycomputer_bus_flow::Flow;

/// A recoverable failure in front of something only a person can pass — a
/// captcha, a one-time code, a login wall — becomes `needs_human`, and the
/// failed step runs again once they have. Anything else stays a failure.
///
/// A captcha or "prove you are human" check is first handed to the runner's
/// solver when `may_solve`; once it is solved, the failed step and the rest
/// come back as the run to go on with (`Err`), and no person is asked.
pub(super) async fn human_wall(
    cell: &Cell,
    runner: &dyn FlowRunner,
    status: TaskStatus,
    may_solve: bool,
) -> Result<TaskStatus, Run> {
    let retryable = matches!(
        status,
        TaskStatus::Failed {
            recoverable: true,
            ..
        }
    ) && cell
        .state
        .lock()
        .is_ok_and(|state| matches!(state.resume, Some(Resume::Retry { .. })));
    let id = cell.view.borrow().id.clone();
    let wall = if retryable {
        tinycomputer_core::human_needed(&runner.visible_text(&id).await)
    } else {
        None
    };
    if let Some(action) = wall {
        let mut tried = String::new();
        if may_solve && tinycomputer_core::is_bot_check(&action) {
            match solve(runner, &id).await {
                CaptchaOutcome::Solved { .. } => {
                    if let Some(run) = retry_run(cell) {
                        return Err(run);
                    }
                }
                CaptchaOutcome::Unsolved { attempts, reason } => {
                    tried = format!(" (tried {attempts} time(s) automatically: {reason})");
                }
                CaptchaOutcome::Unsupported { reason } => {
                    tried = format!(" (not one the solver can answer: {reason})");
                }
                CaptchaOutcome::Unavailable => {}
            }
        }
        return Ok(TaskStatus::NeedsHuman {
            reason: format!("{action}, then continue the task{tried}"),
            screenshot: None,
        });
    }
    // No person can help, so there is nothing to retry.
    if let Ok(mut state) = cell.state.lock()
        && matches!(state.resume, Some(Resume::Retry { .. }))
    {
        state.resume = None;
    }
    Ok(status)
}

/// The runner's solver on `task`'s page, journaled.
async fn solve(runner: &dyn FlowRunner, task: &TaskId) -> CaptchaOutcome {
    let outcome = runner.solve_captcha(task).await;
    runner.journal(Some(task), "captcha", &|| match &outcome {
        CaptchaOutcome::Unavailable => serde_json::json!({"outcome": "unavailable"}),
        CaptchaOutcome::Solved { attempts } => {
            serde_json::json!({"outcome": "solved", "attempts": attempts})
        }
        CaptchaOutcome::Unsupported { reason } => {
            serde_json::json!({"outcome": "unsupported", "reason": reason})
        }
        CaptchaOutcome::Unsolved { attempts, reason } => {
            serde_json::json!({"outcome": "unsolved", "attempts": attempts, "reason": reason})
        }
    });
    outcome
}

/// The run that tries the failed step again, as a person's "done" would.
fn retry_run(cell: &Cell) -> Option<Run> {
    let mut state = cell.state.lock().ok()?;
    let Some(Resume::Retry { app, steps }) = state.resume.take() else {
        return None;
    };
    Some(Run {
        flow: Flow {
            app,
            vars: state.flow.vars.clone(),
            steps,
        },
        allow_destructive: state.constraints.allow_destructive,
        rescue: None,
    })
}
