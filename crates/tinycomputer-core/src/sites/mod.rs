//! What browser tasks learn about a site, kept for its next task.
//!
//! The module returns, with every finished task, the plan it ran
//! (`TaskReport.flow`) and the elements its steps found
//! (`TaskReport.learned`), and takes both back on `StartTask`: a ready `flow`
//! skips planning, and `memory` lets a step confirm a remembered element with
//! one yes/no question instead of searching the page for it. This module is
//! the *rules*; where the bytes live is the host's, behind the
//! `SiteMemoryHost` interface (`tinycomputer_bus::site_memory`).
//!
//! - Only a task that finished (`done`, or stopped at its payment checkpoint)
//!   teaches anything.
//! - A plan is reused only for the same goal, with the same facts, on the
//!   same site. It is kept only when its run needed no rescue and its steps
//!   hold no fact value the goal does not. A task that fails, or needs a
//!   rescue, on a reused plan forgets it, and so does a module that refuses
//!   it, the task then being planned afresh.
//! - An element whose name, key or place holds a fact value, however the
//!   module spells it, or whose name reads like page text rather than a
//!   control's label, is not kept.
//! - What no finished run has used for 30 days is dropped.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use tinycomputer_bus::agent::TaskReport;
use tinycomputer_bus::site_memory::{SavedHint, SavedPlan, SiteMemory};
use tinycomputer_bus::{Flow, GroundingHint};

pub use tinycomputer_bus::sites::site_named;

/// The normalized site name in an HTTP(S) URL, or `None` when the URL has no
/// safe HTTP(S) host.
#[must_use]
pub fn site_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    site_named(parsed.host_str()?)
}

/// Most elements kept per site; the oldest go first.
pub const MAX_HINTS: usize = 50;
/// Most plans kept per site; the one finished on longest ago goes first.
pub const MAX_PLANS: usize = 20;
/// How long a plan or an element is kept without a finished run using it,
/// and a site's file without a change.
pub const KEEP_SECS: u64 = 30 * 24 * 60 * 60;
/// Longest element name kept: a longer one is page text, not a label.
pub const MAX_NAME_CHARS: usize = 80;
/// Shortest fact value looked for: a shorter one (a count, an initial) is in
/// too many labels to tell anything.
pub const MIN_FACT_CHARS: usize = 3;
/// Largest serialized site memory a host should keep: anything bigger was
/// not written by [`SiteMemoryRules::fit`].
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// A started task, followed for what it may teach once it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Following {
    site: String,
    goal: String,
    facts_id: String,
    reused_plan: bool,
    facts: Vec<String>,
}

impl Following {
    /// Follows a task for `goal` with `facts` on `site` (a plain host name,
    /// see [`site_named`]). `reused_plan` says it runs a plan saved here.
    #[must_use]
    pub fn new(
        site: String,
        goal: &str,
        facts: &BTreeMap<String, String>,
        reused_plan: bool,
    ) -> Self {
        Self {
            site,
            goal: goal_key(goal),
            facts_id: facts_id(facts),
            reused_plan,
            facts: fact_values(facts.values()),
        }
    }

    /// The site it teaches.
    #[must_use]
    pub fn site(&self) -> &str {
        &self.site
    }

    /// The goal as plans are matched by ([`goal_key`]).
    #[must_use]
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// The facts digest plans are matched by ([`facts_id`]).
    #[must_use]
    pub fn facts_id(&self) -> &str {
        &self.facts_id
    }

    /// Whether the task runs a plan saved for its goal and facts.
    #[must_use]
    pub fn reuses_plan(&self) -> bool {
        self.reused_plan
    }

    /// Marks the saved plan as forgotten, the module having refused it, so
    /// the task is planned afresh.
    pub fn plan_refused(&mut self) {
        self.reused_plan = false;
    }

    /// Adds `inputs`, the values a paused task is given, to the values it
    /// may not keep.
    pub fn note_inputs<'a>(&mut self, inputs: impl IntoIterator<Item = &'a String>) {
        for value in fact_values(inputs) {
            if !self.facts.contains(&value) {
                self.facts.push(value);
            }
        }
    }
}

/// A goal as plans are matched by: lower-cased, its whitespace collapsed.
#[must_use]
pub fn goal_key(goal: &str) -> String {
    goal.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `facts`, names and values, as one digest, so a site file holds no value.
/// A plan is reused only with the facts it ran with: one chosen by a value
/// (a size, a city) never meets another, and none it reads goes unsupplied.
#[must_use]
pub fn facts_id(facts: &BTreeMap<String, String>) -> String {
    let bytes = serde_json::to_vec(facts).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// `text` as the module keys a step by: its words, lower-cased, with
/// anything between them turned to single spaces.
fn words(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The fact values worth looking for, lower-cased, each also as its words:
/// the module keys a step by its text with the values filled in, so
/// `asha.r@example.com` is kept as `asha r example com`.
fn fact_values<'a>(values: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut found = Vec::new();
    for value in values {
        let value = value.trim().to_lowercase();
        let spelled = words(&value);
        for form in [value, spelled] {
            if form.chars().count() >= MIN_FACT_CHARS && !found.contains(&form) {
                found.push(form);
            }
        }
    }
    found
}

/// The rules that keep, reuse and expire a [`SiteMemory`].
pub trait SiteMemoryRules {
    /// Drops what no finished run has used since `now - KEEP_SECS`.
    fn expire(&mut self, now: u64);

    /// The plan saved for `following`'s goal and facts.
    fn plan_for(&self, goal: &str, facts_id: &str) -> Option<&Flow>;

    /// Keeps what `following`'s finished run, reported in `report`, found,
    /// and the plan it ran when it needed no rescue; forgets a reused plan
    /// that did.
    fn learn(&mut self, following: &Following, report: &TaskReport, now: u64);

    /// Forgets the plan saved for `following`'s goal and facts.
    fn forget_plan(&mut self, following: &Following);

    /// Serializes within [`MAX_FILE_BYTES`], dropping the oldest plans and
    /// then the oldest elements until it fits.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error.
    fn fit(&mut self) -> serde_json::Result<Vec<u8>>;
}

impl SiteMemoryRules for SiteMemory {
    fn expire(&mut self, now: u64) {
        let fresh = |ok_at: u64| ok_at.saturating_add(KEEP_SECS) > now;
        self.plans.retain(|plan| fresh(plan.ok_at));
        self.hints.retain(|saved| fresh(saved.ok_at));
    }

    fn plan_for(&self, goal: &str, facts_id: &str) -> Option<&Flow> {
        self.plans
            .iter()
            .find(|plan| serves(plan, goal, facts_id))
            .map(|plan| &plan.flow)
    }

    fn learn(&mut self, following: &Following, report: &TaskReport, now: u64) {
        for hint in &report.learned {
            if !keepable_hint(hint, &following.facts) {
                continue;
            }
            self.hints
                .retain(|saved| !(saved.hint.app == hint.app && saved.hint.key == hint.key));
            self.hints.push(SavedHint {
                hint: hint.clone(),
                ok_at: now,
            });
        }
        let excess = self.hints.len().saturating_sub(MAX_HINTS);
        self.hints.drain(..excess);
        if !report.rescues.is_empty() {
            if following.reused_plan {
                self.forget_plan(following);
            }
            return;
        }
        let Some(flow) = report
            .flow
            .as_ref()
            .filter(|flow| keepable_plan(flow, following))
        else {
            return;
        };
        let runs = self
            .plans
            .iter()
            .find(|plan| serves(plan, &following.goal, &following.facts_id))
            .map_or(0, |plan| plan.runs);
        self.forget_plan(following);
        self.plans.push(SavedPlan {
            goal: following.goal.clone(),
            facts_id: following.facts_id.clone(),
            flow: flow.clone(),
            ok_at: now,
            runs: runs.saturating_add(1),
        });
        self.plans.sort_by_key(|plan| plan.ok_at);
        let excess = self.plans.len().saturating_sub(MAX_PLANS);
        self.plans.drain(..excess);
    }

    fn forget_plan(&mut self, following: &Following) {
        self.plans
            .retain(|plan| !serves(plan, &following.goal, &following.facts_id));
    }

    fn fit(&mut self) -> serde_json::Result<Vec<u8>> {
        loop {
            let bytes = serde_json::to_vec_pretty(&*self)?;
            if u64::try_from(bytes.len()).is_ok_and(|length| length <= MAX_FILE_BYTES) {
                return Ok(bytes);
            }
            if self.plans.is_empty() {
                if self.hints.is_empty() {
                    return Ok(bytes);
                }
                self.hints.remove(0);
            } else {
                self.plans.remove(0);
            }
        }
    }
}

/// Whether `plan` is the plan saved for `goal` with the facts `facts_id`.
fn serves(plan: &SavedPlan, goal: &str, facts_id: &str) -> bool {
    plan.goal == goal && plan.facts_id == facts_id
}

/// Whether `hint` may be kept: its name reads like a control's label, and
/// neither it, its key, nor its place holds one of `facts`.
fn keepable_hint(hint: &GroundingHint, facts: &[String]) -> bool {
    let name = hint.name.as_deref().unwrap_or_default();
    let mut texts = [name, hint.key.as_str()]
        .into_iter()
        .chain(hint.path.iter().map(String::as_str));
    name.chars().count() <= MAX_NAME_CHARS && !texts.any(|text| holds(text, facts))
}

/// Whether `flow` may be reused for its goal: it holds no fact value the
/// goal does not, which another task with the same goal could change.
fn keepable_plan(flow: &Flow, following: &Following) -> bool {
    let Ok(text) = serde_json::to_string(flow) else {
        return false;
    };
    let foreign: Vec<String> = following
        .facts
        .iter()
        .filter(|fact| !holds(&following.goal, std::slice::from_ref(*fact)))
        .cloned()
        .collect();
    !holds(&text, &foreign)
}

/// Whether `text`, as written or as its words, holds any of `facts`.
fn holds(text: &str, facts: &[String]) -> bool {
    let written = text.to_lowercase();
    let spelled = words(text);
    facts
        .iter()
        .any(|fact| written.contains(fact.as_str()) || spelled.contains(fact.as_str()))
}

#[cfg(test)]
mod sites_tests;
