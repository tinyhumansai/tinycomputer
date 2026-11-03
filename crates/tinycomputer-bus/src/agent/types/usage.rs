//! Provider-reported model usage accumulated by task and model role.

use serde::{Deserialize, Serialize};

/// Model calls and the usage their providers reported for one task role.
///
/// Token and cost values stay absent when no provider supplied them. Costs are
/// integer nanodollars, and are included only when a provider reports them;
/// this crate does not infer prices from token counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    /// Model requests made, including calls whose usage was not reported.
    #[serde(default)]
    pub calls: u64,
    /// Provider-reported input tokens, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Provider-reported cached input tokens, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    /// Provider-reported output tokens, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Provider-reported USD charge in integer nanodollars, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd_nanos: Option<i64>,
}

/// Per-role model usage accumulated over a task's lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskUsage {
    /// TypeSafe/Jev decision calls.
    #[serde(default, skip_serializing_if = "ModelUsage::is_empty")]
    pub jev: ModelUsage,
    /// Calls that turn the task description into a flow.
    #[serde(default, skip_serializing_if = "ModelUsage::is_empty")]
    pub planner: ModelUsage,
    /// Calls that recover a failed flow step.
    #[serde(default, skip_serializing_if = "ModelUsage::is_empty")]
    pub rescue: ModelUsage,
    /// Calls that shape extracted records into the requested answer.
    #[serde(default, skip_serializing_if = "ModelUsage::is_empty")]
    pub output: ModelUsage,
}

impl TaskUsage {
    /// Whether no role has recorded calls or provider-reported measurements.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.jev.is_empty()
            && self.planner.is_empty()
            && self.rescue.is_empty()
            && self.output.is_empty()
    }
}

impl ModelUsage {
    /// Whether no calls or provider-reported measurements have been recorded.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.calls == 0
            && self.input_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cost_usd_nanos.is_none()
    }

    /// Adds one request and any measurements its provider returned.
    pub fn record(
        &mut self,
        input_tokens: Option<u64>,
        cached_input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd_nanos: Option<i64>,
    ) {
        self.calls = self.calls.saturating_add(1);
        add_unsigned(&mut self.input_tokens, input_tokens);
        add_unsigned(&mut self.cached_input_tokens, cached_input_tokens);
        add_unsigned(&mut self.output_tokens, output_tokens);
        if let Some(cost) = cost_usd_nanos {
            self.cost_usd_nanos =
                Some(self.cost_usd_nanos.unwrap_or_default().saturating_add(cost));
        }
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
