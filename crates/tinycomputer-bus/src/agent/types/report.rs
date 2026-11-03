//! Everything a task did, including the rescues it took.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{TaskUsage, TaskView};
use crate::browser::OutputRef;
use crate::flow::{Flow, FlowStep, GroundingHint, JevExchange, StepReport};

/// `TaskReport`: everything a task did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskReport {
    /// The task as it stands.
    pub view: TaskView,
    /// Provider-reported model usage accumulated over the task's lifetime.
    #[serde(default, skip_serializing_if = "TaskUsage::is_empty")]
    pub usage: TaskUsage,
    /// The flow it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Flow>,
    /// One report per step run.
    pub steps: Vec<StepReport>,
    /// What `extract` and `pick` steps collected, by variable name.
    pub records: BTreeMap<String, Vec<BTreeMap<String, String>>>,
    /// Screenshots taken at checkpoints, approvals, and failures.
    pub artifacts: Vec<OutputRef>,
    /// Grounding hints learned, to pass as `StartTask.memory` next time.
    pub learned: Vec<GroundingHint>,
    /// Every Jev exchange, when `StartTask.trace` was set.
    pub trace: Vec<JevExchange>,
    /// Each time a failed step was handed to the reasoning model, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rescues: Vec<Rescue>,
}

/// One rescue: a failed step, what the reasoning model made of it, and
/// whether its guidance got the task past it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rescue {
    /// The zero-based top-level index of the failed step, in the flow that
    /// was running when it failed.
    pub step: usize,
    /// Why the step failed.
    pub failure: String,
    /// What the model said was wrong, or why it gave up.
    pub reason: String,
    /// The steps the model put in place of the failed one; empty when it
    /// gave up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<FlowStep>,
    /// How many of the steps right after the failed one `steps` also do,
    /// dropped from the flow; never one holding a `stop_before`.
    #[serde(default)]
    pub covers: usize,
    /// How the rescue went.
    pub outcome: RescueOutcome,
}

/// How a [`Rescue`] went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RescueOutcome {
    /// The guidance is running.
    Running,
    /// The guidance's steps all finished.
    Recovered,
    /// One of the guidance's own steps failed.
    FailedAgain,
    /// The model gave up, failed, or never gave valid guidance.
    GaveUp,
}
