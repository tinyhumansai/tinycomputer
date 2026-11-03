//! Where a task is, what step it is on, and what it needs from the caller.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{TaskId, TaskUsage};
use crate::browser::OutputRef;

/// A task's current state, as every call reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskView {
    /// The task.
    pub id: TaskId,
    /// Where it is, and what it needs.
    pub status: TaskStatus,
    /// One sentence on what is happening, for a model or a person.
    pub summary: String,
    /// The step in progress or paused on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<StepView>,
    /// Steps finished over steps planned, from 0 to 1.
    pub progress: f32,
    /// The member calls that make sense now, such as `AwaitTask` or
    /// `ContinueTask`.
    pub next: Vec<String>,
    /// Provider-reported model usage accumulated so far, including on a
    /// cancelled or timed-out task.
    #[serde(default, skip_serializing_if = "TaskUsage::is_empty")]
    pub usage: TaskUsage,
}

/// The step a task is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepView {
    /// Its position, from zero.
    pub index: usize,
    /// How many top-level steps the flow has.
    pub total: usize,
    /// Its kind, such as `browse` or `enter`.
    pub kind: String,
    /// What it is trying to do.
    pub intent: String,
    /// The surface it runs on, such as `browser` or `Mail`.
    pub surface: String,
}

/// Where a task is, and what it needs from the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Working. Call `AwaitTask`.
    Running,
    /// Paused for values it does not have. Answer with `ContinueTask.inputs`.
    NeedsInput {
        /// What to supply.
        fields: Vec<InputField>,
    },
    /// Paused before an irreversible action. Answer with
    /// `ContinueTask.approve`.
    NeedsApproval {
        /// What it would do, in plain words.
        action: String,
        /// The control it would press.
        target: String,
        /// The screen as it stands, for a person to check.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screenshot: Option<OutputRef>,
    },
    /// Stopped at a checkpoint — always on reaching a payment page. The task
    /// hands over here; for a payment it cannot be continued.
    Checkpoint {
        /// Why it stopped.
        reason: String,
        /// The page or window it stopped on.
        location: String,
        /// The screen as it stands.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screenshot: Option<OutputRef>,
        /// What has been done so far, in plain words.
        summary: String,
        /// Whether `ContinueTask.approve` may take it further.
        continuable: bool,
    },
    /// Blocked on something only a person can do: a captcha, a login, a
    /// one-time code. Answer `ContinueTask.answer = "done"` once it is done.
    NeedsHuman {
        /// What the person must do.
        reason: String,
        /// The screen as it stands.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screenshot: Option<OutputRef>,
    },
    /// A plain-language task arrived and no planner is configured. Write a
    /// flow from `guide` and start again with `StartTask.flow`.
    NeedsPlan {
        /// The flow authoring guide.
        guide: String,
    },
    /// Finished.
    Done {
        /// The outcome, in plain words.
        answer: String,
        /// What `extract` and `pick` steps collected, by variable name.
        records: BTreeMap<String, Vec<BTreeMap<String, String>>>,
        /// The answer in the shape `StartTask.output` asked for, checked
        /// against its schema; absent when no output was asked for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
    },
    /// Could not finish.
    Failed {
        /// The step that failed, from zero.
        step: Option<usize>,
        /// Why.
        reason: String,
        /// What a caller could change to succeed.
        hint: String,
        /// Whether starting again, changed per the hint, can succeed.
        recoverable: bool,
    },
    /// Stopped by `CancelTask`.
    Cancelled,
}

impl TaskStatus {
    /// Whether the task is over and will not change again.
    #[must_use]
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            Self::Done { .. }
                | Self::Failed { .. }
                | Self::Cancelled
                | Self::Checkpoint {
                    continuable: false,
                    ..
                }
        )
    }
}

/// A value a paused task needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputField {
    /// The name to answer under, in `ContinueTask.inputs`.
    pub name: String,
    /// Why it is needed, in plain words.
    pub why: String,
    /// What kind of value it is.
    pub kind: InputKind,
    /// The allowed values, for a choice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

/// The kind of value an input field takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// Free text.
    Text,
    /// A calendar date, as `YYYY-MM-DD`.
    Date,
    /// An email address.
    Email,
    /// A phone number, with country code.
    Phone,
    /// A number.
    Number,
    /// One of `options`.
    Choice,
}
