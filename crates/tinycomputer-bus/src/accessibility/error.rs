//! Errors returned by fallible accessibility operations.

/// Failure from a public accessibility operation.
#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum Error {
    /// The requested operation is unavailable on this operating system.
    #[error("operation is unsupported on this platform")]
    UnsupportedPlatform,
    /// Text insertion was refused by the native backend.
    #[error("text insertion failed: {0}")]
    InsertionFailed(String),
    /// A listener lease was released or does not belong to this module instance.
    #[error("unknown listener handle")]
    UnknownListener,
    /// A focus query could not obtain focused text.
    #[error("focused text query failed: {0}")]
    FocusQuery(String),
    /// A helper process or OS command did not answer before its deadline.
    #[error("accessibility helper timed out: {0}")]
    HelperTimeout(String),
    /// Focus moved to a different application before the operation completed.
    #[error("focus shifted from '{expected}' to '{actual}', aborting insertion")]
    FocusChanged {
        /// Application captured by the caller.
        expected: String,
        /// Application that currently owns focus.
        actual: String,
    },
    /// The focused element changed to an incompatible role.
    #[error("focus role changed from '{expected}' to '{actual}', aborting insertion")]
    FocusRoleChanged {
        /// Element role captured by the caller.
        expected: String,
        /// Role of the currently focused element.
        actual: String,
    },
    /// The focused element moved from its captured bounds.
    #[error("focused element bounds changed, aborting insertion")]
    FocusTargetChanged,
    /// The Globe listener could not be started, inspected, or stopped.
    #[error("Globe listener operation failed: {0}")]
    GlobeListener(String),
}

/// Result type for public accessibility operations.
pub type Result<T> = std::result::Result<T, Error>;
