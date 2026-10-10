//! High-level intent flows run by Jev decision loops.
//!
//! A [`Flow`](tinycomputer_bus::Flow) says *what* to accomplish, step by step, with no UI knowledge.
//! This module grounds each step on the live screen by composing many small
//! Jev questions in deterministic Rust:
//!
//! - `ask` builds the questions: completion and condition Nouls, a progress
//!   Score, an obstacle Noul, and element Choices of at most 20 options.
//! - `ground` picks one element for a purpose: grounding memory first, then
//!   region-by-region narrowing, then a relabelled re-ask and a yes/no
//!   corroboration when the first answer is not confident.
//! - `act` is the loop behind a `do` step: judge, choose an app-agnostic move,
//!   act, and judge again, with obstacle dismissal and undo.
//! - `enter` matches slots to fields in one request and delivers each text
//!   with read-back verification.
//! - `steps` implements the remaining step kinds.
//! - `memory` remembers which element grounded which step.
//! - `vote` asks each decision several ways at once and averages the answers.
//! - `wide` is the wide strategy: one request per `do` turn over a digest of
//!   the screen, carrying the judgement, the obstacle, and every move's
//!   target; `survey` ranks a crowded screen's regions first, and `ledger`
//!   is the working memory every wide question sees.
//! - Deliberation (`docs/technical/specs/jev-deliberation.md`) decides on evidence
//!   rather than one probability: `evidence` reads a question's ballot into
//!   accept, deliberate, or abstain; `escalate` asks a close call more ways,
//!   `duel` settles close candidates two at a time; `denoise` ranks what is
//!   in view first; `expect` checks an action did what it should; and
//!   `checkpoint` undoes a mistake and verifies the undo.
//!
//! Every request carries the run's brief — the goal, whom it is for, the
//! plan and where the run is in it, what it has chosen so far, and what kind
//! of page is showing — so each small decision is made knowing the whole
//! task. Secrets never leave as values: every request is masked so a secret
//! reads `${name}` wherever it would have appeared.
//!
//! See `docs/technical/specs/jev-intent-flows.md` for the design and its rationale.
//!
//! `FlowRun`'s methods are split by concern: `run` starts a run and runs
//! its steps, `decide` asks Jev through one door, `brief` builds the brief,
//! `look` reads the screen, and `action` runs one desktop action.

mod act;
mod action;
mod ask;
mod attention;
mod backend;
mod brief;
mod checkpoint;
mod decide;
mod denoise;
mod duel;
mod enter;
mod escalate;
mod evidence;
mod expect;
mod front;
mod ground;
mod hedge;
mod ledger;
mod look;
mod memory;
mod quorum;
mod reflect;
mod run;
mod steps;
mod survey;
mod validate;
mod view;
mod vote;
mod wide;

pub(crate) use validate::{check as check_flow, missing_inputs};
pub(in crate::agentic) use vote::MAX_VOTES;

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use serde_json::json;
use tinycomputer_bus::{
    Deliberation, DesktopError, DesktopResponse, FLOW_GUIDE, FlowActionRecord, FlowBrief, FlowLoop,
    FlowStopReason, FlowStrategy, GroundingHint, JevExchange, JevMetrics, JevTarget,
    RunFlowRequest, StepOutcome, StepReport, ValidateFlowRequest,
};
use tinycomputer_core::Facts;

use super::{JevRuntime, merge_metrics, response};
use backend::AgentBackend;
use front::Front;
use view::Candidate;

/// Upper bound on [`RunFlowRequest::max_actions`].
const MAX_ACTIONS: u32 = 120;
/// Upper bound on [`RunFlowRequest::max_model_calls`]. Jev is cheap, and
/// every framing of a voted decision is one evaluation.
const MAX_CALLS: u32 = 10_000;
/// Choices, picks, and entries remembered for the brief's `so_far`.
const MAX_SO_FAR: usize = 12;
/// Longest goal the brief carries, in characters.
const MAX_GOAL: usize = 600;
/// Longest plan line the brief carries, in characters.
const MAX_PLAN_LINE: usize = 120;
/// Longest `so_far` note the brief carries, in characters.
const MAX_SO_FAR_NOTE: usize = 200;
/// Largest request sent to Jev, in bytes of JSON. Past its token limit a
/// request is refused outright, which ends the run: directly, Jev answers
/// HTTP 400 (`max_tokens_exceeded`), and 120 KB passed while 160 KB did not;
/// through the Tiny Humans gateway the limit is lower and comes back as HTTP
/// 502, where 57 KB (23,600 tokens) passed and 68 KB did not. A request whose
/// questions outgrow it is asked in parts (`decide::split`), so only a state
/// too large on its own is ever cut (`decide::fit`).
const MAX_REQUEST_BYTES: usize = 48_000;
/// Consecutive unreadable observations that fail a step.
const MAX_BLIND_LOOKS: u32 = 3;
/// Truncated subtrees one exploration reads at most.
const MAX_EXPLORED: usize = 4;

/// Runs `request` on `surface`: a `Desktop`, or a
/// [`Workspace`](crate::Workspace) joining the desktop and the browser.
pub async fn run_flow<S: AgentBackend + Sync>(
    surface: S,
    runtime: JevRuntime,
    request: RunFlowRequest,
) -> DesktopResponse {
    let label = if request.brief.goal.is_empty() {
        request.flow.app.clone()
    } else {
        format!("{}: {}", request.flow.app, request.brief.goal)
    };
    let runtime = runtime.begin_run("flow", &label);
    run_flow_with(surface, &runtime, request).await
}

/// Checks a flow without touching the desktop or Jev.
#[must_use]
pub fn validate_flow(request: &ValidateFlowRequest) -> DesktopResponse {
    let (_, validation) = validate::validate(&request.flow, &BTreeSet::new(), &BTreeSet::new());
    response("validate-flow", &validation)
}

/// The flow authoring guide, as prompt text.
#[must_use]
pub fn flow_guide() -> DesktopResponse {
    DesktopResponse::ok(
        "flow-guide",
        json!({"guide": FLOW_GUIDE, "step_kinds": tinycomputer_bus::STEP_KINDS}),
    )
}

pub(super) async fn run_flow_with<B: AgentBackend + Sync>(
    backend: B,
    runtime: &JevRuntime,
    request: RunFlowRequest,
) -> DesktopResponse {
    let known = request
        .vars
        .keys()
        .chain(request.collected.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let validation = validate::check(&request.flow, &known, &request.facts);
    if !validation.valid {
        return DesktopResponse::err(
            "run-flow",
            DesktopError::new("FLOW_INVALID", validation.errors.join("; ")),
        );
    }
    let flow = request.flow.clone();
    let mut run = FlowRun::new(backend, runtime, &request);
    let stop = match run.start(&flow).await {
        Ok(()) => FlowStopReason::Completed,
        Err(Halt::Stop(stop)) => stop,
        Err(Halt::Error(error)) => return *error,
        Err(Halt::Failed(_)) => FlowStopReason::StepFailed,
    };
    run.finish(stop)
}

/// Why a run stopped early.
#[derive(Debug)]
pub(super) enum Halt {
    /// The run stops with a structured reason.
    Stop(FlowStopReason),
    /// A provider failure the caller must see as an error envelope.
    Error(Box<DesktopResponse>),
    /// The current step failed; the note says why.
    Failed(String),
}

/// What one step spent and did, accumulated while it runs.
#[derive(Debug, Default)]
pub(super) struct StepLog {
    pub(super) calls: u32,
    pub(super) turns: u32,
    pub(super) actions: Vec<FlowActionRecord>,
    pub(super) loops: BTreeSet<FlowLoop>,
    pub(super) confidence: Option<f64>,
    /// Whether the step typed to filter a list: the option it then pressed
    /// should leave the list, or show selected (`steps::left_unchosen`).
    pub(super) filtered: bool,
}

impl StepLog {
    pub(super) fn used(&mut self, flow_loop: FlowLoop) {
        self.loops.insert(flow_loop);
    }
}

/// How a step ended, and why.
#[derive(Debug)]
pub(super) struct Ended {
    pub(super) outcome: StepOutcome,
    pub(super) note: String,
}

impl Ended {
    pub(super) fn new(outcome: StepOutcome, note: impl Into<String>) -> Self {
        Self {
            outcome,
            note: note.into(),
        }
    }
}

/// State of one flow run.
pub(super) struct FlowRun<'r, B> {
    pub(super) backend: B,
    runtime: &'r JevRuntime,
    pub(super) app: String,
    pub(super) vars: BTreeMap<String, String>,
    /// Names among `vars` that are the task's facts: never expanded into any
    /// text a Jev evaluation sees, as a runtime backstop behind validation.
    pub(super) facts: BTreeSet<String>,
    pub(super) allow_destructive: bool,
    pub(super) include_values: bool,
    max_actions: u32,
    max_calls: u32,
    disabled: BTreeSet<FlowLoop>,
    pub(super) memory: Vec<GroundingHint>,
    pub(super) learned: Vec<GroundingHint>,
    pub(super) history: Vec<String>,
    metrics: JevMetrics,
    actions: u32,
    reports: Vec<StepReport>,
    pub(super) pending: Option<JevTarget>,
    blind_looks: u32,
    tracing: bool,
    trace: Vec<JevExchange>,
    /// When the run began, for the journal's end-of-run wall time.
    started: Instant,
    step: String,
    /// What the run is for, shown with every question.
    brief: FlowBrief,
    /// The flow's top-level steps as the brief lists them.
    outline: Vec<String>,
    /// What the run has chosen, picked, and entered so far, newest last.
    pub(super) so_far: Vec<String>,
    /// The kind of page last seen, by Jev's reading of it.
    pub(super) page: Option<String>,
    /// How many ways each decision is asked.
    votes: u32,
    /// The secret values, to mask every request with.
    secrets: Facts,
    /// Every `stop_before` phrase the flow declares, gathered once up front
    /// so an ordinary step's destructive gate can recognize a control the
    /// flow has already named as irreversible, in its own words.
    pub(super) stop_before: Vec<String>,
    /// How decisions are asked.
    strategy: FlowStrategy,
    /// The run's working memory, for the wide strategy's questions.
    ledger: ledger::Ledger,
    /// The current step's survey of a crowded screen, if one was asked.
    attention: Option<survey::Attention>,
    /// Decisions made so far: each one request, whatever its framings.
    decisions: u32,
    /// Round trips to Jev: a batch of decisions asked at once is one.
    rounds: u32,
    /// Variables read from the screen so far, by name.
    read: Vec<String>,
    /// Kinds of element (`view::element_kind`) that refused text in this
    /// step: options in a list, never pressed by a `do` move while the step
    /// looks for somewhere to type.
    pub(super) refused: BTreeSet<String>,
    /// Kinds of element (`view::element_kind`) this run typed into: a
    /// field holding text the flow typed shows no choice the page made
    /// (`steps::already_holds`).
    pub(super) typed: BTreeSet<String>,
    /// The field the run's last action typed into, while nothing else has
    /// acted since but waits: the focus is still in it.
    pub(super) typed_last: Option<Candidate>,
    /// How much the run deliberates before acting on a decision.
    deliberation: Deliberation,
    /// Every framing's own answer to each question, under the original
    /// keys, from the latest decision that asked it: the evidence a
    /// deliberating decision reads (`evidence/`) and widens (`escalate`).
    ballots: BTreeMap<String, Vec<tinyinference_decisions::Answer>>,
    /// How many framings each question was asked in, by the latest decision
    /// that asked it and any widening since. A decision ended on a quorum
    /// holds fewer answers than that, and `escalate` must widen past every
    /// framing asked, not only those in the ballot.
    asked: BTreeMap<String, usize>,
    /// The address the surface last reported, on a surface that has them:
    /// a checkpoint's location, and how a navigation is noticed.
    pub(super) location: Option<String>,
    /// The runners-up of the step's latest grounding, best first: the
    /// branches a backtrack tries next (`checkpoint/`).
    pub(super) frontier: Vec<Candidate>,
    /// The last press and what it was meant to do, while the turn after it
    /// is judged: the judge then asks whether it did (`expect/`).
    pub(super) expecting: Option<(String, String)>,
    /// The address the current step began at, to return to when the step
    /// is found to have gone wrong.
    pub(super) step_location: Option<String>,
    /// What the current step cleared out of the way (`attention/`), by
    /// control signature, across every loop that attends within it: an
    /// Escape or a close that did not clear it once will not the next time.
    pub(super) step_cleared: BTreeSet<String>,
    /// The current step's last press that stayed covered, and what covered
    /// it, while no press has landed since (`act/uncover.rs`): its failure
    /// says so, so a rescue deals with the cover rather than redo the steps
    /// before it.
    pub(super) step_covered: Option<String>,
    /// What is in front, and whether the run's own press put it there
    /// (`front.rs`); built knowing whether the task's run before this one
    /// left its own dialog in front ([`RunFlowRequest::dialog_left_open`]).
    pub(in crate::agentic::flow) front: Front,
}

#[cfg(test)]
mod flow_tests;
