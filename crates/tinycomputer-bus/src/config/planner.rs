//! Typed planner and model-route configuration carried in a module load.

use serde::{Deserialize, Serialize};

use crate::agent::LanguageModelProvider;

/// Which approved provider and credential the planner roles use.
///
/// The module checks the provider's endpoint against its own allow-list before
/// constructing a client. `api_key` is never included in the custom `Debug`
/// representation.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRoute {
    /// The `OpenRouter` key or `TinyHumans` bearer.
    pub api_key: String,
    /// The OpenAI-compatible provider; `OpenRouter` when absent.
    #[serde(default)]
    pub provider: LanguageModelProvider,
    /// An exact repetition of the provider's approved base URL, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    /// Product attribution for the `TinyHumans` route, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_name: Option<String>,
}

impl std::fmt::Debug for ModelRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelRoute")
            .field("api_key", &"[REDACTED]")
            .field("provider", &self.provider)
            .field("endpoint_url", &self.endpoint_url)
            .field("sdk_name", &self.sdk_name)
            .finish()
    }
}

/// The planner, rescue, and output roles configured for the module.
///
/// The route fields (`api_key`, `provider`, `endpoint_url`, and `sdk_name`) sit
/// at the top level of the object beside the three model names. The planner
/// and output roles use this route; the rescuer uses `rescue_route` when set.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerConfig {
    /// The planner, output, and fallback rescue route.
    #[serde(flatten)]
    pub route: ModelRoute,
    /// The planning model, or the module's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The rescue model, or the module's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescue_model: Option<String>,
    /// The output-shaping model, or the module's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_model: Option<String>,
    /// A separate route with its own credential for rescue calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescue_route: Option<ModelRoute>,
}

impl PlannerConfig {
    /// Returns the rescue route, falling back to the planner route.
    #[must_use]
    pub fn rescuer_route(&self) -> &ModelRoute {
        self.rescue_route.as_ref().unwrap_or(&self.route)
    }
}

impl std::fmt::Debug for PlannerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlannerConfig")
            .field("route", &self.route)
            .field("model", &self.model)
            .field("rescue_model", &self.rescue_model)
            .field("output_model", &self.output_model)
            .field("rescue_route", &self.rescue_route)
            .finish()
    }
}
