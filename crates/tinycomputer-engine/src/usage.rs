//! Per-task model usage collection shared by model adapters.

use std::sync::Mutex;

use tinycomputer_bus::agent::{ModelUsage, TaskUsage};

use crate::task::TaskCallBudget;

/// The model role whose provider-reported usage is being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    /// TypeSafe/Jev decisions.
    Jev,
    /// Plain-language task planning.
    Planner,
    /// Recovery guidance after a failed step.
    Rescue,
    /// Shaping extracted data into a requested answer.
    Output,
}

/// Thread-safe, per-task accumulator for provider-reported model usage.
///
/// A task owns one recorder and passes it to every model adapter it uses.
/// Missing provider measurements remain absent; this type never estimates
/// tokens or cost.
#[derive(Debug, Default)]
pub struct UsageRecorder {
    usage: Mutex<TaskUsage>,
    task_budget: Mutex<Option<TaskCallBudget>>,
}

impl UsageRecorder {
    /// Starts a recorder with usage accumulated before a task was persisted.
    #[must_use]
    pub fn from_snapshot(snapshot: TaskUsage) -> Self {
        Self {
            usage: Mutex::new(snapshot),
            task_budget: Mutex::new(None),
        }
    }

    /// The task's opaque host-call budget, when this recorder belongs to a
    /// managed task. Hosts use it to reserve work that happens outside a
    /// flow run, such as payment-field mapping.
    #[must_use]
    pub fn task_call_budget(&self) -> Option<TaskCallBudget> {
        self.task_budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn set_task_call_budget(&self, budget: TaskCallBudget) {
        *self
            .task_budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(budget);
    }

    /// Records one model call for `role`, retaining only reported values.
    pub fn record_call(
        &self,
        role: ModelRole,
        input_tokens: Option<u64>,
        cached_input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd_nanos: Option<i64>,
    ) {
        self.record_started(role);
        self.record_measurements(
            role,
            input_tokens,
            cached_input_tokens,
            output_tokens,
            cost_usd_nanos,
        );
    }

    /// Records that a model request started, even if it is later cancelled.
    pub fn record_started(&self, role: ModelRole) {
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let role = role_usage(&mut usage, role);
        role.calls = role.calls.saturating_add(1);
    }

    /// Adds provider attempts beyond the first attempt already counted.
    pub fn record_additional_calls(&self, role: ModelRole, calls: u64) {
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let role = role_usage(&mut usage, role);
        role.calls = role.calls.saturating_add(calls);
    }

    /// Adds provider measurements for a request already counted with
    /// [`UsageRecorder::record_started`].
    pub fn record_measurements(
        &self,
        role: ModelRole,
        input_tokens: Option<u64>,
        cached_input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd_nanos: Option<i64>,
    ) {
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let role = role_usage(&mut usage, role);
        add_unsigned(&mut role.input_tokens, input_tokens);
        add_unsigned(&mut role.cached_input_tokens, cached_input_tokens);
        add_unsigned(&mut role.output_tokens, output_tokens);
        if let Some(cost) = cost_usd_nanos {
            role.cost_usd_nanos =
                Some(role.cost_usd_nanos.unwrap_or_default().saturating_add(cost));
        }
    }

    /// Returns the usage accumulated so far.
    #[must_use]
    pub fn snapshot(&self) -> TaskUsage {
        *self
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn role_usage(usage: &mut TaskUsage, role: ModelRole) -> &mut ModelUsage {
    match role {
        ModelRole::Jev => &mut usage.jev,
        ModelRole::Planner => &mut usage.planner,
        ModelRole::Rescue => &mut usage.rescue,
        ModelRole::Output => &mut usage.output,
    }
}

fn add_unsigned(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or_default().saturating_add(value));
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
