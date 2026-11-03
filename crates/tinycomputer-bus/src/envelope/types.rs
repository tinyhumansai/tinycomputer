//! The response envelope, its error payload, and the delivery vocabulary.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// The envelope version this contract encodes.
///
/// It tracks the `agent-desktop` output envelope rather than
/// [`crate::CONTRACT_VERSION`]: the former describes the reply shape, the
/// latter describes the member set and the payloads.
pub const ENVELOPE_VERSION: &str = "2.5";

/// The reply from every member of [`crate::INTERFACE`].
///
/// Exactly one of [`DesktopResponse::data`] and [`DesktopResponse::error`] is
/// present, selected by [`DesktopResponse::ok`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopResponse {
    /// The envelope version. [`ENVELOPE_VERSION`] for replies this module
    /// produces.
    #[serde(default)]
    pub version: String,
    /// Whether the command succeeded.
    pub ok: bool,
    /// The command name, in the engine's own spelling — `list-apps`, not
    /// `ListApps`. Present on both outcomes so a batched caller can correlate.
    pub command: String,
    /// The command's result. Present when [`DesktopResponse::ok`].
    #[serde(
        default,
        deserialize_with = "deserialize_present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub data: Option<Value>,
    /// Why the command failed. Present when not [`DesktopResponse::ok`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<DesktopError>,
}

fn deserialize_present_value<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

impl DesktopResponse {
    /// Builds a successful reply to `command` carrying `data`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_bus::DesktopResponse;
    /// let reply = DesktopResponse::ok("version", serde_json::json!({"version": "0.8.3"}));
    /// assert!(reply.ok && reply.error.is_none());
    /// ```
    #[must_use]
    pub fn ok(command: impl Into<String>, data: Value) -> Self {
        Self {
            version: ENVELOPE_VERSION.to_owned(),
            ok: true,
            command: command.into(),
            data: Some(data),
            error: None,
        }
    }

    /// Builds a failed reply to `command` carrying `error`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_bus::{DesktopError, DesktopResponse};
    /// let reply = DesktopResponse::err("click", DesktopError::new("STALE_REF", "ref expired"));
    /// assert!(!reply.ok && reply.data.is_none());
    /// ```
    #[must_use]
    pub fn err(command: impl Into<String>, error: DesktopError) -> Self {
        Self {
            version: ENVELOPE_VERSION.to_owned(),
            ok: false,
            command: command.into(),
            data: None,
            error: Some(error),
        }
    }

    /// Checks the envelope version and outcome fields, then decodes the
    /// successful result as `T`.
    ///
    /// A failed command is returned as [`DesktopResponseError::Remote`]. A
    /// malformed envelope is rejected before its data can be mistaken for a
    /// successful result.
    ///
    /// # Errors
    ///
    /// Returns an error when the version is unsupported, the outcome fields
    /// disagree, the remote command failed, or its data has the wrong shape.
    pub fn into_data<T: DeserializeOwned>(self) -> Result<T, DesktopResponseError> {
        if self.version != ENVELOPE_VERSION {
            return Err(DesktopResponseError::Version {
                expected: ENVELOPE_VERSION,
                actual: self.version,
            });
        }
        match (self.ok, self.data, self.error) {
            (true, Some(data), None) => {
                serde_json::from_value(data).map_err(DesktopResponseError::Decode)
            }
            (false, None, Some(error)) => Err(DesktopResponseError::Remote(Box::new(error))),
            (ok, data, error) => Err(DesktopResponseError::InconsistentOutcome {
                ok,
                has_data: data.is_some(),
                has_error: error.is_some(),
            }),
        }
    }
}

/// A failed result from [`DesktopResponse::into_data`].
#[derive(Debug, thiserror::Error)]
pub enum DesktopResponseError {
    /// The module returned an envelope version this crate does not understand.
    #[error("unsupported desktop response version {actual}; expected {expected}")]
    Version {
        /// The envelope version this crate accepts.
        expected: &'static str,
        /// The version sent by the module.
        actual: String,
    },
    /// The success flag disagrees with whether data or an error is present.
    #[error(
        "inconsistent desktop response outcome (ok={ok}, has_data={has_data}, has_error={has_error})"
    )]
    InconsistentOutcome {
        /// Whether the module marked the response successful.
        ok: bool,
        /// Whether a data field was present.
        has_data: bool,
        /// Whether an error field was present.
        has_error: bool,
    },
    /// The command failed with a structured module error.
    #[error("desktop command failed")]
    Remote(Box<DesktopError>),
    /// The success data did not match the expected payload type.
    #[error("desktop response data had the wrong shape: {0}")]
    Decode(#[source] serde_json::Error),
}

/// Why a command failed, in the form a caller can act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopError {
    /// A stable machine-readable code — `STALE_REF`, `PERM_DENIED`,
    /// `ELEMENT_NOT_FOUND`, `PLATFORM_NOT_SUPPORTED`, and so on. Match on this
    /// rather than on [`DesktopError::message`].
    pub code: String,
    /// A human-readable explanation.
    pub message: String,
    /// What a caller might do instead, when the engine can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// A machine-readable recovery strategy, when one applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryHint>,
    /// Platform-specific detail, when the accessibility backend supplied any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_detail: Option<String>,
    /// Structured, code-specific detail — the supported surfaces for a
    /// `PLATFORM_NOT_SUPPORTED`, the competing matches for an
    /// `AMBIGUOUS_TARGET`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// How far the command got before it failed, and whether retrying it is
    /// safe.
    #[serde(default)]
    pub disposition: Delivery,
}

impl DesktopError {
    /// Builds an error carrying `code` and `message` and nothing else.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_bus::DesktopError;
    /// let error = DesktopError::new("INVALID_ARGS", "amount must be positive");
    /// assert_eq!(error.code, "INVALID_ARGS");
    /// assert!(error.suggestion.is_none());
    /// ```
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            suggestion: None,
            recovery: None,
            platform_detail: None,
            details: None,
            disposition: Delivery::default(),
        }
    }

    /// Returns this error with `suggestion` attached.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_bus::DesktopError;
    /// let error = DesktopError::new("STALE_REF", "gone").with_suggestion("re-snapshot");
    /// assert_eq!(error.suggestion.as_deref(), Some("re-snapshot"));
    /// ```
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// A machine-readable way out of a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryHint {
    /// The named strategy, for example
    /// `refresh_snapshot_then_retry_original`.
    pub strategy: String,
    /// Whether repeating the original command can succeed.
    pub retryable: bool,
    /// Whether the retry needs a fresh snapshot first, because the refs the
    /// original call used are no longer valid.
    pub requires_fresh_snapshot: bool,
    /// How long to wait before retrying, when the engine can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// How far a failed command got, and whether repeating it is safe.
///
/// The two fields are not independent: [`Delivery::retry`] follows from
/// [`Delivery::delivery`], and the engine rejects a pair that disagrees. They
/// are carried separately because the question a caller asks is "may I retry",
/// and answering it should not require the caller to reimplement the mapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    /// How far the command got.
    pub delivery: DeliveryDisposition,
    /// Whether repeating it is safe.
    pub retry: RetryDisposition,
}

impl Delivery {
    /// Builds the delivery pair implied by `delivery`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_bus::{Delivery, DeliveryDisposition, RetryDisposition};
    /// let delivery = Delivery::of(DeliveryDisposition::NotDelivered);
    /// assert_eq!(delivery.retry, RetryDisposition::Safe);
    /// ```
    #[must_use]
    pub fn of(delivery: DeliveryDisposition) -> Self {
        let retry = match delivery {
            DeliveryDisposition::NotDelivered => RetryDisposition::Safe,
            DeliveryDisposition::DeliveryUncertain
            | DeliveryDisposition::DeliveredUnverified
            | DeliveryDisposition::DeliveredVerified => RetryDisposition::Unsafe,
            DeliveryDisposition::Unknown => RetryDisposition::Unknown,
        };
        Self { delivery, retry }
    }
}

/// How far a command got before it failed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeliveryDisposition {
    /// The engine cannot say. The default.
    #[default]
    Unknown,
    /// The command never reached the application.
    NotDelivered,
    /// The command may or may not have reached the application.
    DeliveryUncertain,
    /// The command reached the application, but its effect was not confirmed.
    DeliveredUnverified,
    /// The command reached the application and its effect was confirmed.
    DeliveredVerified,
}

/// Whether repeating a failed command is safe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RetryDisposition {
    /// The engine cannot say. The default.
    #[default]
    Unknown,
    /// Nothing happened, so a retry cannot duplicate an effect.
    Safe,
    /// Something may have happened, so a retry could duplicate an effect.
    Unsafe,
}
