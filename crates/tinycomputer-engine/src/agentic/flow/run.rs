//! Running a flow: starting it, running each step in order, and finishing
//! with the run's result.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    time::Instant,
};

use serde_json::json;
use tinycomputer_bus::{
    Deliberation, DesktopResponse, Flow, FlowAction, FlowLoop, FlowRunResult, FlowStep,
    FlowStopReason, JevMetrics, RunFlowRequest, StepOutcome, StepReport,
};
use tinycomputer_core::Facts;

use super::{
    Ended, FlowRun, Halt, MAX_ACTIONS, MAX_CALLS, StepLog,
    backend::AgentBackend,
    front::Front,
    ledger, steps,
    validate::{self, step_path, substitute_safe},
    vote,
};
use crate::agentic::{JevRuntime, journal::millis, response};

/// Every `stop_before` phrase in `steps`, gathered from every branch of
/// `if` and every round of `repeat_until`: a control the flow names there is
/// irreversible regardless of which branch a run actually takes.
fn stop_before_phrases(steps: &[FlowStep]) -> Vec<String> {
    let mut phrases = Vec::new();
    collect_stop_before(steps, &mut phrases);
    phrases
}

fn collect_stop_before(steps: &[FlowStep], phrases: &mut Vec<String>) {
    for step in steps {
        match step.action() {
            FlowAction::StopBefore(phrase) => phrases.push(phrase),
            FlowAction::RepeatUntil(repeat) => collect_stop_before(&repeat.steps, phrases),
            FlowAction::If(branch) => {
                collect_stop_before(&branch.then, phrases);
                collect_stop_before(&branch.otherwise, phrases);
            }
            _ => {}
        }
    }
}

impl<'r, B: AgentBackend + Sync> FlowRun<'r, B> {
    pub(super) fn new(backend: B, runtime: &'r JevRuntime, request: &RunFlowRequest) -> Self {
        // A flow's own definitions may name the caller's values
        // (`"first_name": "${first name}"`), so they are expanded once
        // against them. The caller's values are never rescanned: one that
        // happens to contain `${…}` stays as written.
        let mut vars = request
            .flow
            .vars
            .iter()
            .map(|(name, value)| (name.clone(), validate::substitute(value, &request.vars)))
            .collect::<BTreeMap<_, _>>();
        // What the task has read so far outranks the flow's own declaration
        // of it: a planner declares each read's variable up front, empty, and
        // a flow resumed after a rescue declares it again, so `${total}`
        // read before the rescue expanded to nothing after it. The caller's
        // own values outrank both.
        vars.extend(request.collected.clone());
        vars.extend(request.vars.clone());
        let facts = validate::carrying_facts(&request.flow.vars, &request.facts);
        let secrets = Facts::with_secrets(
            facts
                .iter()
                .filter_map(|name| Some((name.clone(), vars.get(name)?.clone()))),
            facts
                .iter()
                .filter(|name| vars.contains_key(*name))
                .cloned(),
        )
        .unwrap_or_default();
        let outline = request
            .flow
            .steps
            .iter()
            .map(|step| {
                let (kind, text) = describe_step(&step.action(), &vars, &facts);
                format!("{kind}: {text}")
            })
            .collect();
        Self {
            backend,
            runtime,
            app: request.flow.app.clone(),
            stop_before: stop_before_phrases(&request.flow.steps),
            vars,
            // A flow variable defined from a secret now holds that secret's
            // value, so it is kept out of model-facing text the same way.
            facts,
            brief: request.brief.clone(),
            outline,
            so_far: Vec::new(),
            page: None,
            votes: request.votes.clamp(1, vote::MAX_VOTES),
            secrets,
            allow_destructive: request.allow_destructive,
            include_values: request.include_values,
            max_actions: request.max_actions.min(MAX_ACTIONS),
            max_calls: request.max_model_calls.min(MAX_CALLS),
            disabled: request
                .disabled_loops
                .iter()
                .copied()
                .filter(|flow_loop| *flow_loop != FlowLoop::Slots)
                .collect(),
            memory: request.memory.clone(),
            learned: Vec::new(),
            history: Vec::new(),
            metrics: JevMetrics::default(),
            actions: 0,
            reports: Vec::new(),
            pending: None,
            blind_looks: 0,
            tracing: request.trace,
            trace: Vec::new(),
            started: Instant::now(),
            step: String::new(),
            strategy: request.strategy,
            ledger: ledger::Ledger::default(),
            attention: None,
            decisions: 0,
            rounds: 0,
            read: request.collected.keys().cloned().collect(),
            refused: BTreeSet::new(),
            typed: BTreeSet::new(),
            typed_last: None,
            deliberation: request.deliberation,
            ballots: BTreeMap::new(),
            asked: BTreeMap::new(),
            location: None,
            frontier: Vec::new(),
            expecting: None,
            step_location: None,
            step_cleared: BTreeSet::new(),
            step_covered: None,
            front: Front::new(request.dialog_left_open),
        }
    }

    pub(super) async fn start(&mut self, flow: &Flow) -> Result<(), Halt> {
        let app = self.app.clone();
        let mut log = StepLog::default();
        self.act(&mut log, "launch", None, move |backend| {
            backend.launch(&app)
        })
        .await?;
        self.run_steps(&flow.steps, String::new()).await
    }

    /// Runs `steps` in order, recursing into `if` and `repeat_until`.
    pub(in crate::agentic::flow) fn run_steps<'s>(
        &'s mut self,
        steps: &'s [FlowStep],
        prefix: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), Halt>> + Send + 's>> {
        Box::pin(async move {
            for (index, step) in steps.iter().enumerate() {
                self.run_step(step, step_path(&prefix, index)).await?;
            }
            Ok(())
        })
    }

    async fn run_step(&mut self, step: &FlowStep, path: String) -> Result<(), Halt> {
        let action = step.action();
        let (kind, text) = describe_step(&action, &self.vars, &self.facts);
        let mut log = StepLog::default();
        self.begin_step(&path);
        let started = Instant::now();
        let result = self.run_action(&mut log, &action, &text, &path).await;
        let result = self.reflected(&mut log, &action, &text, result).await;
        let wall_ms = millis(started.elapsed());
        let (ended, halt) = match result {
            Ok(ended) => (ended, None),
            Err(Halt::Failed(note)) => {
                let note = self.with_cover(note);
                let ended = Ended::new(StepOutcome::Failed, note.clone());
                (ended, Some(Halt::Failed(note)))
            }
            Err(Halt::Stop(reason)) => {
                let outcome = if reason == FlowStopReason::StoppedBeforeDestructive {
                    StepOutcome::Gated
                } else {
                    StepOutcome::Failed
                };
                let note = match reason {
                    FlowStopReason::StoppedBeforeDestructive => {
                        "stopped in front of the irreversible action".to_owned()
                    }
                    FlowStopReason::ActionBudget => "the action budget ran out".to_owned(),
                    FlowStopReason::ModelBudget => "the Jev call budget ran out".to_owned(),
                    _ => format!("stopped: {reason:?}"),
                };
                (Ended::new(outcome, note), Some(Halt::Stop(reason)))
            }
            Err(error @ Halt::Error(_)) => return Err(error),
        };
        let text = self.secrets.mask(&text);
        let ended = Ended::new(ended.outcome, self.secrets.mask(&ended.note));
        self.step.clone_from(&path);
        self.runtime.journal.record("step", || {
            json!({
                "step": path,
                "kind": kind,
                "text": text,
                "outcome": ended.outcome,
                "note": ended.note,
                "turns": log.turns,
                "jev_calls": log.calls,
                "actions": log.actions.len(),
                "loops": log.loops,
                "confidence": log.confidence,
                "wall_ms": wall_ms,
            })
        });
        let finished = format!(
            "step {path} ({kind} {text:?}): {:?}, {}",
            ended.outcome, ended.note
        );
        self.ledger.finish(&finished);
        self.history.push(finished);
        if !matches!(&action, FlowAction::If(_) | FlowAction::RepeatUntil(_)) || halt.is_some() {
            self.reports.push(StepReport {
                path,
                kind: kind.to_owned(),
                text,
                outcome: ended.outcome,
                turns: log.turns,
                jev_calls: log.calls,
                actions: log.actions,
                loops: log.loops.into_iter().collect(),
                confidence: log.confidence,
                note: ended.note,
            });
        } else {
            let position = self
                .reports
                .iter()
                .position(|report| report.path.starts_with(&format!("{path}.")))
                .unwrap_or(self.reports.len());
            self.reports.insert(
                position,
                StepReport {
                    path,
                    kind: kind.to_owned(),
                    text,
                    outcome: ended.outcome,
                    turns: log.turns,
                    jev_calls: log.calls,
                    actions: log.actions,
                    loops: log.loops.into_iter().collect(),
                    confidence: log.confidence,
                    note: ended.note,
                },
            );
        }
        match halt {
            Some(Halt::Failed(_)) => Err(Halt::Stop(FlowStopReason::StepFailed)),
            Some(halt) => Err(halt),
            None => Ok(()),
        }
    }

    /// Runs one step's action. A step that grounds an element first clears
    /// what is in the way (`attention/`); a `do` step attends every turn.
    async fn run_action(
        &mut self,
        log: &mut StepLog,
        action: &FlowAction,
        text: &str,
        path: &str,
    ) -> Result<Ended, Halt> {
        if matches!(
            action,
            FlowAction::Choose(_)
                | FlowAction::Enter(_)
                | FlowAction::Pick(_)
                | FlowAction::Read(_)
                | FlowAction::Extract(_)
                | FlowAction::StopBefore(_)
        ) {
            self.clear_the_way(log, text).await?;
        }
        steps::run(self, log, action, text, path).await
    }

    /// A failed step's `note`, ending with the press a cover still refused
    /// in it and what covered it, when one did and no press landed since.
    fn with_cover(&self, note: String) -> String {
        match &self.step_covered {
            Some(covered) => format!("{note}; {covered}"),
            None => note,
        }
    }

    /// Resets what one step keeps, before step `path` runs.
    fn begin_step(&mut self, path: &str) {
        path.clone_into(&mut self.step);
        self.ledger.begin();
        self.refused.clear();
        self.frontier.clear();
        self.step_location.clone_from(&self.location);
        self.step_cleared.clear();
        self.step_covered = None;
        self.front.next_step();
    }

    pub(in crate::agentic::flow) fn enabled(&self, flow_loop: FlowLoop) -> bool {
        !self.disabled.contains(&flow_loop)
    }

    /// Whether the run deliberates on evidence at all, and `flow_loop` —
    /// one of deliberation's loops — is on.
    pub(in crate::agentic::flow) fn deliberates(&self, flow_loop: FlowLoop) -> bool {
        self.deliberation != Deliberation::Off && self.enabled(flow_loop)
    }

    /// Whether the run deliberates at the deep level.
    pub(in crate::agentic::flow) fn deep(&self) -> bool {
        self.deliberation == Deliberation::Deep
    }

    /// Jev evaluations the run may still make.
    pub(in crate::agentic::flow) fn room(&self) -> u32 {
        self.max_calls.saturating_sub(self.metrics.calls)
    }

    pub(in crate::agentic::flow) fn model(&self) -> &str {
        &self.runtime.configuration.model
    }

    pub(super) fn finish(self, stop: FlowStopReason) -> DesktopResponse {
        self.runtime.journal.record("end", || {
            json!({
                "stop": stop,
                "wall_ms": millis(self.started.elapsed()),
                "actions": self.actions,
                "metrics": self.metrics,
                "learned": self.learned.len(),
            })
        });
        response(
            "run-flow",
            &FlowRunResult {
                stop,
                steps: self.reports,
                vars: self.vars,
                pending: self.pending,
                learned: self.learned,
                actions: self.actions,
                metrics: self.metrics,
                trace: self.trace,
                dialog_left_open: self.front.left_open(),
            },
        )
    }
}

/// A step's wire kind and its text with variables substituted.
///
/// This text is what a step report shows and what `recent_actions` carries
/// into every later Jev question, so it is built with [`substitute_safe`]:
/// even a step kind that may substitute a fact operationally (`open`,
/// `browse`) never repeats that value here.
fn describe_step(
    action: &FlowAction,
    vars: &BTreeMap<String, String>,
    facts: &BTreeSet<String>,
) -> (&'static str, String) {
    let (kind, text) = match action {
        FlowAction::Open(app) => ("open", app.clone()),
        FlowAction::Browse(url) => ("browse", url.clone()),
        FlowAction::Do(intent) => ("do", intent.clone()),
        FlowAction::Enter(slots) => (
            "enter",
            slots
                .0
                .iter()
                .map(|slot| slot.slot.clone())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        FlowAction::Choose(choose) => ("choose", format!("{} in {}", choose.option, choose.what)),
        FlowAction::Read(read) => ("read", format!("{} into {}", read.what, read.into)),
        FlowAction::Pick(pick) => ("pick", format!("{} by {}", pick.from, pick.by)),
        FlowAction::Extract(read) => ("extract", format!("{} into {}", read.what, read.into)),
        FlowAction::Verify(condition) => ("verify", condition.clone()),
        FlowAction::WaitFor(condition) => ("wait_for", condition.clone()),
        FlowAction::StopBefore(action) => ("stop_before", action.clone()),
        FlowAction::RepeatUntil(repeat) => ("repeat_until", repeat.condition.clone()),
        FlowAction::If(branch) => ("if", branch.condition.clone()),
    };
    (kind, substitute_safe(&text, vars, facts))
}
