//! The reply envelope every Agent member returns, and its error.

use serde::{Deserialize, Serialize};

/// The version of the Agent response envelope this contract produces.
pub const AGENT_ENVELOPE_VERSION: &str = "1";

/// Every reply on the Agent interface: the value, or an error a caller can
/// act on. Never a transport failure for something the caller did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct AgentResponse<T> {
    /// The envelope version. [`AGENT_ENVELOPE_VERSION`] for module replies.
    #[serde(default)]
    pub version: String,
    /// Whether the call did what was asked.
    pub ok: bool,
    /// The result, when `ok`.
    #[serde(
        default = "missing_data",
        deserialize_with = "deserialize_present_data",
        skip_serializing_if = "Option::is_none"
    )]
    pub data: Option<T>,
    /// Why not, when not `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentError>,
}

fn deserialize_present_data<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn missing_data<T>() -> Option<T> {
    None
}

impl<T> AgentResponse<T> {
    /// A successful reply carrying `data`.
    #[must_use]
    pub fn ok(data: T) -> Self {
        Self {
            version: AGENT_ENVELOPE_VERSION.to_owned(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// A failed reply carrying `error`.
    #[must_use]
    pub fn err(error: AgentError) -> Self {
        Self {
            version: AGENT_ENVELOPE_VERSION.to_owned(),
            ok: false,
            data: None,
            error: Some(error),
        }
    }

    /// Checks the envelope version and outcome fields, then returns the
    /// successful value.
    ///
    /// A failed call is returned as [`AgentResponseError::Remote`].
    ///
    /// # Errors
    ///
    /// Returns an error when the version is unsupported, the outcome fields
    /// disagree, or the remote call failed.
    pub fn into_data(self) -> Result<T, AgentResponseError> {
        if self.version != AGENT_ENVELOPE_VERSION {
            return Err(AgentResponseError::Version {
                expected: AGENT_ENVELOPE_VERSION,
                actual: self.version,
            });
        }
        match (self.ok, self.data, self.error) {
            (true, Some(data), None) => Ok(data),
            (false, None, Some(error)) => Err(AgentResponseError::Remote(error)),
            (ok, data, error) => Err(AgentResponseError::InconsistentOutcome {
                ok,
                has_data: data.is_some(),
                has_error: error.is_some(),
            }),
        }
    }
}

/// A failed result from [`AgentResponse::into_data`].
#[derive(Debug, thiserror::Error)]
pub enum AgentResponseError {
    /// The module returned an envelope version this crate does not understand.
    #[error("unsupported Agent response version {actual}; expected {expected}")]
    Version {
        /// The envelope version this crate accepts.
        expected: &'static str,
        /// The version sent by the module.
        actual: String,
    },
    /// The success flag disagrees with whether data or an error is present.
    #[error(
        "inconsistent Agent response outcome (ok={ok}, has_data={has_data}, has_error={has_error})"
    )]
    InconsistentOutcome {
        /// Whether the module marked the response successful.
        ok: bool,
        /// Whether a data field was present.
        has_data: bool,
        /// Whether an error field was present.
        has_error: bool,
    },
    /// The Agent command failed with a structured module error.
    #[error("Agent call failed")]
    Remote(AgentError),
}

/// A failed call, phrased for a model to act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentError {
    /// A stable, `SCREAMING_SNAKE_CASE` code, such as `NO_SUCH_TASK`.
    pub code: String,
    /// What went wrong, in one sentence.
    pub message: String,
    /// What to do about it, in one sentence.
    pub hint: String,
    /// Whether retrying — possibly with the hint applied — can succeed.
    pub recoverable: bool,
}

impl AgentError {
    /// An error with a code, message, and hint.
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        hint: impl Into<String>,
        recoverable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: hint.into(),
            recoverable,
        }
    }
}

#[cfg(test)]
#[path = "reply_tests.rs"]
mod tests;
