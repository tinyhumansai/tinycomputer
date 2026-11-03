//! The `SiteMemoryHost` interface: the host callback that lets the stateless
//! module keep what finished browser tasks learned about a site.
//!
//! The module returns, with every finished task, the plan it ran and the
//! elements its steps found, and takes them back on the next task on that
//! site. It keeps nothing between tasks itself, so the host serves this
//! interface and owns where the bytes live (a file, a storage port). The
//! reuse rules, the fact-value filtering and each plan's or hint's 30-day
//! freshness are the module's (`tinycomputer-core::sites`), not the host's.
//!
//! A host may also periodically remove unchanged backing files as physical
//! storage maintenance. Schedule that cleanup outside `Load` requests; it is
//! independent of the module's per-plan and per-hint freshness rules.
//!
//! Three members, all synchronous request/response over plain data:
//!
//! - [`methods::LOAD`] takes a [`LoadSiteRequest`], returns a
//!   [`LoadSiteResponse`]; a site with nothing kept answers an empty memory.
//! - [`methods::SAVE`] takes a [`SaveSiteRequest`], returns a
//!   [`SaveSiteResponse`]; saving an empty memory removes the site.
//! - [`methods::FORGET`] takes a [`ForgetSiteRequest`], returns a
//!   [`ForgetSiteResponse`]; no site forgets every site.
//!
//! Site memory never reaches an agent's memory or prompts: it goes to the
//! module alone.

use serde::{Deserialize, Serialize};

use crate::flow::{Flow, GroundingHint};

/// The interface the host serves site memory on.
pub const INTERFACE: &str = "ai.tinyhumans.tinycomputer.SiteMemoryHost";

/// The object path the host serves [`INTERFACE`] at.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinycomputer/SiteMemoryHost";

/// One constant per member of [`INTERFACE`].
pub mod methods {
    /// Reads one site's memory. Takes a [`crate::site_memory::LoadSiteRequest`].
    pub const LOAD: &str = "Load";
    /// Replaces one site's memory. Takes a [`crate::site_memory::SaveSiteRequest`].
    pub const SAVE: &str = "Save";
    /// Forgets one site, or every site. Takes a
    /// [`crate::site_memory::ForgetSiteRequest`].
    pub const FORGET: &str = "Forget";
}

/// Every member of [`INTERFACE`], in dispatch order.
pub const METHODS: [&str; 3] = [methods::LOAD, methods::SAVE, methods::FORGET];

/// What finished tasks left about one site.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteMemory {
    /// Plans that reached a goal.
    #[serde(default)]
    pub plans: Vec<SavedPlan>,
    /// Elements finished tasks found.
    #[serde(default)]
    pub hints: Vec<SavedHint>,
}

impl SiteMemory {
    /// Whether nothing is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty() && self.hints.is_empty()
    }
}

/// A plan that reached a goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPlan {
    /// The goal it reached, with its whitespace collapsed and lower-cased.
    pub goal: String,
    /// The facts it ran with, as one digest, so no value is stored.
    #[serde(default)]
    pub facts_id: String,
    /// The plan.
    pub flow: Flow,
    /// When a run last finished on it, in seconds since the epoch.
    pub ok_at: u64,
    /// How many finished runs it served.
    pub runs: u32,
}

/// An element a finished task's step found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedHint {
    /// The element.
    pub hint: GroundingHint,
    /// When a finished run last found it, in seconds since the epoch.
    pub ok_at: u64,
}

/// `Load`: one site's memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadSiteRequest {
    /// The site, a plain lower-cased host name.
    pub site: String,
}

/// The reply to `Load`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSiteResponse {
    /// What is kept; empty for a site with nothing.
    #[serde(default)]
    pub memory: SiteMemory,
    /// Counts the times the site was forgotten (`Forget`). The module sends
    /// back the value it loaded with its `Save`, and the host refuses a save
    /// whose epoch is no longer current, so a task that was running when its
    /// site was forgotten cannot write it back. A host that never forgets
    /// under a running task may always answer `0`.
    #[serde(default)]
    pub epoch: u64,
}

/// `Save`: replaces one site's memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveSiteRequest {
    /// The site, a plain lower-cased host name.
    pub site: String,
    /// What to keep; empty removes the site.
    pub memory: SiteMemory,
    /// The `epoch` the site was loaded with; see [`LoadSiteResponse::epoch`].
    #[serde(default)]
    pub epoch: u64,
}

/// The reply to `Save`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveSiteResponse {
    /// Whether the host holds the memory as sent. `false` when the host
    /// refused it: too large, learning switched off, or the site forgotten
    /// since it was loaded (a stale `epoch`).
    pub saved: bool,
}

/// `Forget`: one site, or every site.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetSiteRequest {
    /// The site; `None` forgets every site.
    #[serde(default)]
    pub site: Option<String>,
}

/// The reply to `Forget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetSiteResponse {
    /// How many sites were forgotten.
    pub forgotten: u64,
}

#[cfg(test)]
mod site_memory_tests;
