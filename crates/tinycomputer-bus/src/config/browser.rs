//! Portable browser-session defaults in [`super::ComputerConfig`].

use serde::{Deserialize, Serialize};

/// How a task gathers page evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserPerception {
    /// Use the browser's visual and accessibility evidence.
    #[default]
    Sight,
    /// Use only the accessibility tree.
    Tree,
}

/// How long a task lets a page settle after an action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSettle {
    /// Use the prompt's page-settle instruction.
    #[default]
    Prompt,
    /// Wait for a steady page before continuing.
    Steady,
}

/// Defaults applied when the module opens a browser session.
///
/// An attached session does not launch a browser, so launch-only settings do
/// not replace its endpoint or arguments.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserConfig {
    /// The Chrome or Chromium executable to launch, if the host selected one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// The User-Agent sent by every launched browser, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Extra command-line arguments for every launched browser.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// How a task reads a page.
    #[serde(skip_serializing_if = "is_default_perception")]
    pub perception: BrowserPerception,
    /// How a task lets a page settle after an action.
    #[serde(skip_serializing_if = "is_default_settle")]
    pub settle: BrowserSettle,
    /// Whether a browser-only task prelaunches its browser while planning.
    #[serde(skip_serializing_if = "is_true")]
    pub prelaunch: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            executable: None,
            user_agent: None,
            args: Vec::new(),
            perception: BrowserPerception::default(),
            settle: BrowserSettle::default(),
            prelaunch: true,
        }
    }
}

impl std::fmt::Debug for BrowserConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserConfig")
            .field(
                "executable",
                &self.executable.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "user_agent",
                &self.user_agent.as_ref().map(|_| "[REDACTED]"),
            )
            .field("args_count", &self.args.len())
            .field("perception", &self.perception)
            .field("settle", &self.settle)
            .field("prelaunch", &self.prelaunch)
            .finish()
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if requires &T.
fn is_default_perception(value: &BrowserPerception) -> bool {
    *value == BrowserPerception::default()
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if requires &T.
fn is_default_settle(value: &BrowserSettle) -> bool {
    *value == BrowserSettle::default()
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if requires &T.
fn is_true(value: &bool) -> bool {
    *value
}
