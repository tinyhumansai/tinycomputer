//! Finishing a task and publishing its final answer.

use std::sync::Arc;
use std::time::Duration;

use tinycomputer_bus::agent::{TaskId, TaskStatus};

use super::TaskRecords;
use crate::shape::Harvest;
use crate::task::FlowRunner;
use crate::task::SHAPE_TIMEOUT_MS;
use crate::task::artifact::capture;
use crate::task::publish::{publish, records};
use crate::task::redact_task_value;
use crate::task::store::Cell;

/// Every run finished: the task is done, with what it read, shaped as its
/// `output` asks when it asks.
pub(in crate::task) async fn finish(cell: &Cell, runner: &dyn FlowRunner) {
    // The last look at the surface, before shaping and before release.
    let _ = capture(cell, runner).await;
    let id = cell.view.borrow().id.clone();
    let Some((answer, records, harvest)) = finish_content(cell, runner, &id) else {
        return;
    };
    let result = match (harvest, cell.shaper.clone()) {
        (Some(harvest), Some(shaper)) => {
            let usage = task_usage(cell);
            let shaper = shaper.with_usage_recorder(usage);
            publish(
                cell,
                TaskStatus::Running,
                "Every step finished; shaping the answer.",
            );
            let answered = tokio::time::timeout(
                Duration::from_millis(SHAPE_TIMEOUT_MS),
                shaper.shape(&harvest),
            )
            .await
            .unwrap_or_else(|_| Err("shaping the answer took too long".to_owned()));
            match answered {
                Ok(result) => Some(result),
                Err(why) => {
                    let steps = cell.state.lock().map_or(0, |state| state.flow.steps.len());
                    let reason =
                        runner.redact(&id, &format!("the answer could not be shaped: {why}"));
                    publish(
                        cell,
                        TaskStatus::Failed {
                            step: Some(steps),
                            reason: reason.clone(),
                            hint: "every step finished; TaskReport holds the records they read"
                                .to_owned(),
                            recoverable: false,
                        },
                        &format!("The task failed: {reason}"),
                    );
                    runner.release(&cell.view.borrow().id);
                    return;
                }
            }
        }
        _ => None,
    };
    publish(
        cell,
        TaskStatus::Done {
            answer: answer.clone(),
            records,
            result: result.map(|mut value| {
                redact_task_value(runner, &id, &mut value);
                value
            }),
        },
        &answer,
    );
    runner.release(&cell.view.borrow().id);
}

fn finish_content(
    cell: &Cell,
    runner: &dyn FlowRunner,
    id: &TaskId,
) -> Option<(String, TaskRecords, Option<Harvest>)> {
    let state = cell.state.lock().ok()?;
    let reads = state
        .reads
        .iter()
        .map(|(name, value)| {
            let name = runner.redact(id, name);
            let value = runner.redact(id, value);
            format!("{name}: {value}")
        })
        .collect::<Vec<_>>();
    let answer = if reads.is_empty() {
        format!("Finished all {} steps.", state.flow.steps.len())
    } else {
        format!(
            "Finished all {} steps. {}",
            state.flow.steps.len(),
            reads.join("; ")
        )
    };
    let harvest = state.output.clone().map(|mut output| {
        output.instructions = runner.redact(id, &output.instructions);
        if let Some(schema) = &mut output.schema {
            redact_task_value(runner, id, schema);
        }
        Harvest {
            goal: runner.redact(id, &state.facts.redact(&state.goal)),
            output,
            reads: state
                .reads
                .iter()
                .map(|(name, value)| {
                    (
                        runner.redact(id, name),
                        runner.redact(id, &state.facts.redact(value)),
                    )
                })
                .collect(),
        }
    });
    let reads = state
        .reads
        .iter()
        .map(|(name, value)| (runner.redact(id, name), runner.redact(id, value)))
        .collect::<std::collections::BTreeMap<_, _>>();
    Some((
        runner.redact(id, &state.facts.redact(&answer)),
        records(&reads),
        harvest,
    ))
}

pub(in crate::task) fn task_usage(cell: &Cell) -> Arc<crate::UsageRecorder> {
    cell.state
        .lock()
        .map(|state| Arc::clone(&state.usage))
        .unwrap_or_default()
}

/// Every variable a step of `steps` writes: a `read`'s, `extract`'s, or
/// `pick`'s `into`, at any depth.
pub(in crate::task) fn written_names(
    steps: &[tinycomputer_bus::FlowStep],
) -> std::collections::BTreeSet<String> {
    use tinycomputer_bus::FlowAction;
    let mut names = std::collections::BTreeSet::new();
    for step in steps {
        match step.action() {
            FlowAction::Read(read) | FlowAction::Extract(read) => {
                names.insert(read.into);
            }
            FlowAction::Pick(pick) => names.extend(pick.into),
            FlowAction::If(branch) => {
                names.extend(written_names(&branch.then));
                names.extend(written_names(&branch.otherwise));
            }
            FlowAction::RepeatUntil(repeat) => names.extend(written_names(&repeat.steps)),
            _ => {}
        }
    }
    names
}
