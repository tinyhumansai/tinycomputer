//! The public, typed part of the module's load-time configuration.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::agentic::JevConfig;

use super::{BrowserConfig, PlannerConfig};

/// The configuration shared by hosts and the loaded `TinyComputer` module.
///
/// Credentials in `jev` and `planner` must be delivered through `TinyBus`'s
/// confidential module-configuration path. Omit an unset `jev`, `planner`, or
/// `browser` section; an explicit `null` is rejected. `trace_path` retains the
/// desktop adapter's historical behavior where `null` means unset. The nested
/// configuration types redact their credentials from `Debug` output.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComputerConfig {
    /// The decision model and its approved provider route, when configured.
    #[serde(
        default = "missing_option",
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub jev: Option<JevConfig>,
    /// The planner, rescue, and output model routes, when configured.
    #[serde(
        default = "missing_option",
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub planner: Option<PlannerConfig>,
    /// Defaults applied to browser sessions opened by the module.
    #[serde(
        default = "missing_option",
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub browser: Option<BrowserConfig>,
    /// The desktop trace file path, when tracing is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_path: Option<String>,
}

fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    T::deserialize(deserializer).map(Some)
}

fn missing_option<T>() -> Option<T> {
    None
}

impl std::fmt::Debug for ComputerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComputerConfig")
            .field("jev", &self.jev)
            .field("planner", &self.planner)
            .field("browser", &self.browser)
            .field(
                "trace_path",
                &self.trace_path.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}
