//! How a flow run is asked for: loops, deliberation, strategy, hints, the brief,
//! and validation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Flow;
#[cfg(doc)]
use super::{FlowRunResult, StepReport};
use crate::agent::SecretHandle;

/// One Jev decision loop.
///
/// Reported per step in [`StepReport::loops`], and named in
/// [`RunFlowRequest::disabled_loops`] to measure what a loop contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowLoop {
    /// The completion judge.
    Completion,
    /// The progress judge.
    Progress,
    /// The app-agnostic move chooser.
    Moves,
    /// Region-by-region narrowing.
    Narrowing,
    /// Yes/no corroboration of a target.
    Corroboration,
    /// Relabelled re-asking.
    Consistency,
    /// Slot-to-field matching for `enter`.
    Slots,
    /// Obstacle detection and dismissal.
    Obstacles,
    /// Undo and try the next candidate.
    Undo,
    /// Grounding memory.
    Memory,
    /// Asking each decision several ways and averaging the answers.
    Vote,
    /// Naming the kind of page on screen for the brief.
    PageKind,
    /// Checking an entered form for validation errors.
    Validation,
    /// The attention pass of the wide strategy: which regions of a crowded
    /// screen matter to the step, and which are distraction.
    Survey,
    /// The wide strategy's structured screen digest: regions, what is in
    /// front, noise collapsed. Off, the wide strategy shows the flat element
    /// list the narrow strategy does.
    Digest,
    /// Checking, after a `choose` pressed something, that the screen shows
    /// the choice it asked for, and repairing it once when it does not.
    Reflection,
    /// Deciding from the evidence behind an answer — its margin over the
    /// runner-up and how many framings agreed — rather than from one
    /// probability: accept, deliberate further, or abstain.
    Evidence,
    /// Asking an undecided question again, more ways, before acting on it.
    Escalation,
    /// Settling close candidates by asking about them two at a time, in
    /// both orders.
    Duel,
    /// Narrowing a crowded screen level by level while keeping the two best
    /// branches wherever a level is close.
    TreeGrounding,
    /// Ranking what is in view first and leaving out elements that cannot
    /// serve any step: disabled, zero-size, or pressed without effect again.
    Denoise,
    /// Predicting what an action should change and checking the screen for
    /// it, so a wrong click is noticed.
    Expectation,
    /// Recording where the step started and restoring it after a mistake,
    /// verified against the record.
    Checkpoint,
    /// Returning to a checkpoint and trying the next-best candidate.
    Backtrack,
    /// Asking, before the step, what on screen needs attention first — the
    /// step, or a distraction such as a consent card or promo toast — and
    /// clearing a distraction with its least-committal control.
    Attention,
}

/// How much the flow runtime deliberates before it acts on a decision.
///
/// Jev's probabilities measure how concentrated an answer is, not how likely
/// it is to be right, so a single number near a threshold decides badly.
/// Deliberation reads the evidence behind each answer and, when it is thin,
/// asks more — more framings, pairwise duels, contrasting questions — before
/// acting; it checks each action's effect, and undoes and retries a mistake.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deliberation {
    /// The single-threshold gates alone, as before deliberation existed.
    Off,
    /// The evidence gate, more framings and pairwise duels on a close call,
    /// effect checks, checkpoints, and one backtrack per step.
    Standard,
    /// Everything `Standard` does, plus contrasting questions, judging over
    /// several views of the screen, a flat-versus-tree cross-check when
    /// grounding, a stricter bar for irreversible presses, and up to three
    /// backtracks per step.
    #[default]
    Deep,
}

/// How the flow runtime spends its Jev decisions.
///
/// Both strategies apply the same thresholds and the same safety rules; they
/// differ in how many requests a turn takes and how much each one shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStrategy {
    /// Many small requests in sequence: judge the screen, then ground an
    /// element, narrowing a crowded screen region by region.
    #[default]
    Narrow,
    /// One wide request per turn over a structured digest of the screen,
    /// carrying the judgement, the obstacle to dismiss, and the candidate
    /// targets for every move at once, with the run's working memory; a
    /// crowded screen is surveyed first for which regions matter.
    Wide,
}

/// Where an element was found for one step, so a later run can try it first.
///
/// It carries no ref: refs die with their snapshot. Role, label, and ancestor
/// path are what survive between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(default)]
pub struct GroundingHint {
    /// Application the element lives in.
    pub app: String,
    /// The normalized step text or slot the element grounded.
    pub key: String,
    /// Accessibility role.
    pub role: String,
    /// Accessible name, when it has one.
    pub name: Option<String>,
    /// Labels of the element's ancestors, outermost first.
    pub path: Vec<String>,
}

/// Runs a [`Flow`] with Jev decision loops.
///
/// Requires confidential delivery, like `RunGoal`: the texts a flow enters
/// travel with it.
// Each flag is an independent switch a caller sets by name; folding them
// into one enum would rename `allow_destructive`, `include_values` and
// `trace` on the wire, a major bump, for a tidiness no caller gains from.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunFlowRequest {
    /// The flow to run.
    pub flow: Flow,
    /// Values for `${name}` references, overriding the flow's own `vars`.
    pub vars: BTreeMap<String, String>,
    /// Names among `vars` whose value is a secret: a card number, a passport
    /// number, a password.
    ///
    /// A secret may be typed only as an `enter` step's value. Anywhere else
    /// `${name}` may appear in a flow — an `open` application name, a
    /// `browse` address, a `do`, `verify`, `wait_for`, or `stop_before` text,
    /// a `choose`'s `what`/`option`, a `read`/`extract`'s `what`, a `pick`'s
    /// `from`/`by`, a `repeat_until`/`if` condition, or an `enter` slot's own
    /// name — never sees a secret's value, because that text is what Jev is
    /// asked to reason about, or state it is shown on a later step.
    /// [`crate::FlowValidation`] rejects a flow that references a secret
    /// there, the flow runtime never expands one even if that check were
    /// bypassed, and every Jev request has each secret's value masked back
    /// to `${name}` — including where the page itself shows it.
    ///
    /// Every other variable is shared: its value may appear in step text and
    /// in [`RunFlowRequest::brief`].
    pub facts: BTreeSet<String>,
    /// Host-only handles for named secret facts, resolved only when a field
    /// is typed.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fact_handles: BTreeMap<String, SecretHandle>,
    /// The task making this run, for host-side secret resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Whether `stop_before` steps may perform their irreversible action.
    pub allow_destructive: bool,
    /// Whether ordinary field values may leave the machine for Jev.
    pub include_values: bool,
    /// Most desktop actions for the whole run; capped by the module at 120.
    pub max_actions: u32,
    /// Most Jev evaluations for the whole run; capped by the module at
    /// 10000. Every framing of a voted decision counts as one.
    pub max_model_calls: u32,
    /// How many ways each decision is asked, concurrently, before its
    /// answers are averaged: 1 asks once. Jev calls are cheap, so accuracy
    /// is bought with more of them. Capped by the module at 9.
    pub votes: u32,
    /// Turns a `do` step may spend before it fails as not accomplished; the
    /// module's default (8) when unset, capped by the module at 32. A shop's
    /// checkout can need more moves in one step than a desktop app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_do_turns: Option<u32>,
    /// Who the run is for and what it is after, shown to Jev with every
    /// question so each small decision is made knowing the whole task.
    pub brief: FlowBrief,
    /// Decision loops to turn off. Empty in production; set to measure what
    /// one loop contributes. [`FlowLoop::Slots`] cannot be turned off.
    pub disabled_loops: Vec<FlowLoop>,
    /// Grounding hints from earlier runs.
    pub memory: Vec<GroundingHint>,
    /// Whether to return every Jev exchange in [`FlowRunResult::trace`]. For
    /// development: the trace carries the screen state each question saw.
    pub trace: bool,
    /// How decisions are asked: [`FlowStrategy::Narrow`] unless set.
    pub strategy: FlowStrategy,
    /// How much the runtime deliberates before acting on a decision:
    /// [`Deliberation::Deep`] unless set.
    pub deliberation: Deliberation,
    /// What earlier runs of the same task read and saved, by variable name.
    /// The run starts with these as variables (a caller's `vars` win a
    /// clash) and recalls them to Jev as already collected, so a task that
    /// resumes — after a rescue, an approval, or a person's help — still
    /// knows what it has done.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub collected: BTreeMap<String, String>,
    /// Whether the run before this one, of the same task, left the task's
    /// own dialog in front ([`FlowRunResult::dialog_left_open`]): a dialog
    /// in front at this run's first look is then the task's current stage,
    /// to work within. Otherwise such a dialog is the page's (a promotion or
    /// a sign-up the page opened itself, a menu), never the task's.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dialog_left_open: bool,
}

impl Default for RunFlowRequest {
    fn default() -> Self {
        Self {
            flow: Flow::default(),
            vars: BTreeMap::new(),
            facts: BTreeSet::new(),
            fact_handles: BTreeMap::new(),
            task_id: None,
            allow_destructive: false,
            include_values: false,
            max_actions: 60,
            max_model_calls: 3000,
            votes: 7,
            max_do_turns: None,
            brief: FlowBrief::default(),
            disabled_loops: Vec::new(),
            memory: Vec::new(),
            trace: false,
            strategy: FlowStrategy::Narrow,
            deliberation: Deliberation::Deep,
            collected: BTreeMap::new(),
            dialog_left_open: false,
        }
    }
}

/// The task a flow run serves, as Jev is briefed on it.
///
/// Every Jev question carries it, so a decision about one control is made
/// knowing who the task is for, what it is after, and what must not happen.
/// It holds shared values only: a secret appears in `secrets` by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowBrief {
    /// The whole goal in plain language, such as "book the cheapest flight
    /// from Delhi to Srinagar on 18 October for one adult".
    pub goal: String,
    /// The shared details the task is carried out with, by name: whom it is
    /// for, their date of birth, email, and so on.
    pub details: BTreeMap<String, String>,
    /// The names of the secrets the task holds, which Jev only ever sees as
    /// `${name}`.
    pub secrets: Vec<String>,
    /// Standing rules, such as "stop before paying" or "decline paid
    /// extras".
    pub rules: Vec<String>,
}

impl FlowBrief {
    /// Whether the brief says nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.goal.is_empty()
            && self.details.is_empty()
            && self.secrets.is_empty()
            && self.rules.is_empty()
    }
}

/// Checks a flow without touching the desktop or Jev.
///
/// The flow is taken as raw JSON so a malformed one comes back as a list of
/// readable errors rather than as a bus decode failure — which is what a
/// model writing flows needs in order to repair one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidateFlowRequest {
    /// The candidate flow.
    pub flow: Value,
}

/// Result of [`ValidateFlowRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowValidation {
    /// Whether the flow can be run.
    pub valid: bool,
    /// Every problem found, each naming the step it is in.
    pub errors: Vec<String>,
    /// Steps counted, including nested ones.
    pub steps: usize,
}
