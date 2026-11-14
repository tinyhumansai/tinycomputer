//! The captcha solver's configuration.

use serde::{Deserialize, Serialize};

use crate::agentic::JevConfig;

/// The model a captcha configuration asks when it names none: Jev 1.14,
/// the first to read images and answer Box questions.
pub const DEFAULT_CAPTCHA_MODEL: &str = "jev-preview";

/// The captcha solver a task tries before it asks a person to get past a
/// captcha or a "prove you are human" check (contract 3.1).
///
/// Absent means no solver: such a check pauses the task as `needs_human`,
/// as before. `jev` is its own decision route, since the solver needs a
/// model that reads images (`jev.model` defaults to
/// [`DEFAULT_CAPTCHA_MODEL`]); its credential is delivered like the main
/// `jev` one, through confidential module configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptchaConfig {
    /// The image-reading decision route the solver asks.
    pub jev: JevConfig,
    /// Image challenges submitted before handing the page to a person.
    /// Absent means the module's default, 3; at most 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
}
