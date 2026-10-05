//! Configuring the retained Jev client, and its non-secret summary.

use serde::{Deserialize, Serialize};

/// The decision service, and so the decision model, selected by the host.
///
/// The first three are routes to Jev itself; `open_jev` is `OpenJEV`'s public
/// Jev-compatible API, answering as the `openjev` model; `sage` puts Levanto
/// Sage behind the same loops in place of Jev. Each of those has exactly one
/// approved endpoint, which [`JevConfig::endpoint_url`] may only repeat
/// (contract 2.8 added `open_jev` and `sage`). `self_hosted` instead names an
/// operator-declared Jev-compatible decisions endpoint: it has no approved
/// route and no default model, so both [`JevConfig::endpoint_url`] and
/// [`JevConfig::model`] are required, and the endpoint is trusted because the
/// operator declared it (contract 2.9 added `self_hosted`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JevProvider {
    /// `TypeSafe`'s first-party System One API.
    #[default]
    TypeSafe,
    /// `OpenRouter`'s Jev-compatible decisions API.
    OpenRouter,
    /// Tiny Humans' authenticated `OpenRouter` proxy.
    TinyHumansOpenRouter,
    /// `OpenJEV`'s public System One API (`https://api.openjev.sh`), model
    /// `openjev` unless [`JevConfig::model`] names another. `openjev` is
    /// accepted as an alias on input.
    #[serde(alias = "openjev")]
    OpenJev,
    /// Levanto Sage (`https://sage.levanto.ai/`), answering the loops' Jev
    /// questions as its own calibrated decisions; see [`JevConfig::fast`].
    Sage,
    /// An operator-declared Jev-compatible decisions endpoint, such as a
    /// self-hosted open decision model. Requires [`JevConfig::endpoint_url`]
    /// and [`JevConfig::model`]; the API key is sent only to that endpoint.
    /// `selfhosted` is accepted as an alias on input.
    #[serde(alias = "selfhosted")]
    SelfHosted,
}

impl JevProvider {
    /// The model this provider answers as when [`JevConfig::model`] is
    /// absent: `jev-latest` for the Jev routes, `openjev` for `OpenJEV`, and
    /// `levanto-sage` for Sage, which takes no model selection. A
    /// self-hosted model has no default: the empty string, and a
    /// configuration without an explicit model is rejected.
    #[must_use]
    pub const fn default_model(self) -> &'static str {
        match self {
            Self::TypeSafe | Self::OpenRouter | Self::TinyHumansOpenRouter => "jev-latest",
            Self::OpenJev => "openjev",
            Self::Sage => "levanto-sage",
            Self::SelfHosted => "",
        }
    }
}

/// Configures the Jev client retained by the loaded module.
///
/// This payload must be sent with `TinyBus` confidential delivery. Its custom
/// [`Debug`](std::fmt::Debug) implementation never prints the API key.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JevConfig {
    api_key: String,
    /// Provider whose response contract should be validated.
    pub provider: JevProvider,
    /// Exact compatible endpoint, when the provider's conventional route is
    /// not desired — and always, for the `self_hosted` provider, whose
    /// operator-declared endpoint is the only route.
    pub endpoint_url: Option<String>,
    /// Jev model or alias. Absent means the provider's
    /// [`JevProvider::default_model`]. Sage takes no model selection and
    /// ignores it.
    pub model: Option<String>,
    /// Per-attempt HTTP timeout. Absent means the client default. Ignored by
    /// Sage.
    pub timeout_ms: Option<u64>,
    /// Additional transient retries. Absent means the client default. Ignored
    /// by Sage.
    pub max_retries: Option<u32>,
    /// Host product attribution for the `TinyHumans` proxy only.
    pub sdk_name: Option<String>,
    /// Sage only: score each choice in one pass rather than one pass per
    /// option, trading some calibration for latency. Absent means `false`;
    /// ignored by every other provider (contract 2.8).
    pub fast: Option<bool>,
}

impl JevConfig {
    /// Builds a configuration carrying `api_key` and provider defaults.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            ..Self::default()
        }
    }

    /// Deliberately exposes the API key to the module constructing the client.
    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

impl std::fmt::Debug for JevConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JevConfig")
            .field("api_key", &"[REDACTED]")
            .field("provider", &self.provider)
            .field("endpoint_url", &self.endpoint_url)
            .field("model", &self.model)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_retries", &self.max_retries)
            .field("sdk_name", &self.sdk_name)
            .field("fast", &self.fast)
            .finish()
    }
}

/// Non-secret summary of the retained decision client; `Describe` serves it
/// as `Capabilities.decision_model`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevConfiguration {
    /// Configured provider.
    pub provider: JevProvider,
    /// Requested model or alias.
    pub model: String,
    /// Exact endpoint override, when set.
    pub endpoint_url: Option<String>,
    /// Whether Sage scores each choice in one pass. Always `false` for the
    /// other providers, and left out of the wire form when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fast: bool,
}

// `skip_serializing_if` passes the field by reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
