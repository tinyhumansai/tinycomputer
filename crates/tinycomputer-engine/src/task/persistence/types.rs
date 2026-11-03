//! The versioned snapshot envelope and saved task state.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tinycomputer_bus::agent::{Rescue, TaskConstraints, TaskId, TaskStatus, TaskUsage, TaskView};
use tinycomputer_bus::{Flow, GroundingHint, JevExchange, StepReport};

use super::super::interpret::Resume;
use super::super::store::Spent;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Version and task data stored in a paused-task snapshot.
pub(super) struct Envelope {
    pub(super) version: u32,
    pub(super) task: PausedTask,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Serializable task state required to continue a paused task.
pub(super) struct PausedTask {
    pub(super) id: TaskId,
    pub(super) view: TaskView,
    pub(super) flow: Flow,
    #[serde(default)]
    pub(super) handoff_to_planner: bool,
    pub(super) goal: String,
    pub(super) shared_facts: BTreeMap<String, String>,
    pub(super) secret_handles: BTreeMap<String, tinycomputer_bus::agent::SecretHandle>,
    pub(super) omitted_secrets: BTreeSet<String>,
    pub(super) constraints: TaskConstraints,
    pub(super) budget: tinycomputer_bus::agent::TaskBudget,
    pub(super) memory: Vec<GroundingHint>,
    pub(super) trace: bool,
    pub(super) steps: Vec<StepReport>,
    pub(super) exchanges: Vec<JevExchange>,
    pub(super) learned: Vec<GroundingHint>,
    pub(super) reads: BTreeMap<String, String>,
    pub(super) dialog_left_open: bool,
    pub(super) finished: usize,
    pub(super) resume: Option<Resume>,
    pub(super) return_after_input: Option<TaskStatus>,
    pub(super) spent: Spent,
    pub(super) rescues: Vec<Rescue>,
    pub(super) output: Option<tinycomputer_bus::agent::TaskOutput>,
    pub(super) artifacts: Vec<tinycomputer_bus::browser::OutputRef>,
    pub(super) usage: TaskUsage,
}
