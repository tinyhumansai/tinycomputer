//! The `ConsequenceHost` interface: the host callback that lets a host's
//! policy and approval path rule on an irreversible or paying action before
//! the module carries it out.
//!
//! The module classifies controls itself (`tinycomputer-core::safety`): a
//! control that cannot be taken back, or that moves money, is never pressed
//! on the model's say-so. A task pauses before it as `NeedsApproval`, and
//! answering `ContinueTask { approve: true }` is what lets it go ahead. With
//! this interface served, the module asks the host first and only honours
//! the approval when the host allows it, so the host's security policy, not
//! just a person's click, has the last word.
//!
//! One member, synchronous request/response over plain data:
//!
//! - [`methods::REVIEW`] takes a [`ReviewActionRequest`], returns a
//!   [`ReviewActionResponse`].
//!
//! A host that does not serve the interface leaves the module's own approval
//! pause as the only gate. A host that serves it but answers an error, or
//! cannot be reached, denies: the action is declined, never assumed safe.

use serde::{Deserialize, Serialize};

use crate::agent::HostContext;

/// The interface the host serves consequence review on.
pub const INTERFACE: &str = "ai.tinyhumans.tinycomputer.ConsequenceHost";

/// The object path the host serves [`INTERFACE`] at.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinycomputer/ConsequenceHost";

/// One constant per member of [`INTERFACE`].
pub mod methods {
    /// Rules on one action. Takes a [`crate::consequence_host::ReviewActionRequest`].
    pub const REVIEW: &str = "Review";
}

/// Every member of [`INTERFACE`], in dispatch order.
pub const METHODS: [&str; 1] = [methods::REVIEW];

/// What an action commits to; the module's classification of its control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsequenceKind {
    /// Cannot be taken back: a message sent, data deleted, a booking
    /// confirmed.
    Irreversible,
    /// Money changes hands.
    Payment,
}

/// `Review`: one action the module is about to carry out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewActionRequest {
    /// The task asking, as `TaskView.id` names it.
    pub task: String,
    /// What it would do, in plain words.
    pub action: String,
    /// The control it would press.
    pub target: String,
    /// What the module classified the action as.
    pub kind: ConsequenceKind,
    /// Opaque host-issued context from the `ContinueTask` request. The module
    /// relays this value unchanged and grants it no authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_context: Option<HostContext>,
}

/// The reply to `Review`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewActionResponse {
    /// Whether the host lets the action go ahead.
    pub allow: bool,
    /// Why not, in plain words, when `allow` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod consequence_host_tests;
