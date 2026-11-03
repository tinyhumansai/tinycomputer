//! Which language models plan, rescue, and shape a task, and through which
//! route: the non-secret summary `Describe` serves.

use serde::{Deserialize, Serialize};

/// The OpenAI-compatible route the planner, the rescuer, and the shaper call
/// their models through (contract 2.8).
///
/// Each route has exactly one approved base URL, which the module's private
/// `planner` configuration may only repeat as its `endpoint_url`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageModelProvider {
    /// `OpenRouter` (`https://openrouter.ai/api/v1`), with an `OpenRouter`
    /// key. The default, and the only route before contract 2.8.
    #[default]
    OpenRouter,
    /// Tiny Humans' OpenAI-compatible gateway
    /// (`https://api.tinyhumans.ai/openai/v1`), with the host's `TinyHumans`
    /// bearer. It takes the same `vendor/model` ids as `OpenRouter`.
    TinyHumans,
    /// A host-selected OpenAI-compatible endpoint, only for Rust embedding
    /// hosts; `TinyBus` configuration does not allow custom endpoints.
    OpenAiCompatible,
}

/// Non-secret summary of one configured language model: its route and model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageModelConfiguration {
    /// The route its requests take.
    pub provider: LanguageModelProvider,
    /// The model id it asks for.
    pub model: String,
    /// The exact base URL override, when one was configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
}
