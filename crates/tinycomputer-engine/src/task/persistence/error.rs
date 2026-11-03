//! Errors returned while saving and restoring paused tasks.

use tinycomputer_bus::agent::TaskId;

/// Why a paused task could not be saved or restored.
#[derive(Debug)]
pub enum PausedTaskError {
    /// The task is absent from this controller.
    NoSuchTask(TaskId),
    /// Only pauses that `ContinueTask` can resume may be saved.
    NotResumable,
    /// A secret value appears in task state outside its facts map.
    SecretValueInState,
    /// The snapshot uses a different persistence version.
    UnsupportedVersion(u32),
    /// The snapshot is malformed or its state is inconsistent.
    Invalid(String),
    /// The runner is attached through an endpoint URL which may carry
    /// credentials and cannot safely be moved to a new process.
    HostBoundEndpoint,
    /// The snapshot could not be encoded or decoded as JSON.
    Json(serde_json::Error),
}

impl std::fmt::Display for PausedTaskError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchTask(id) => write!(formatter, "no task {id}"),
            Self::NotResumable => formatter.write_str("the task is not in a resumable pause"),
            Self::SecretValueInState => formatter.write_str(
                "a secret value appears in task state; clear it from the state before exporting",
            ),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "paused-task snapshot version {version} is unsupported"
                )
            }
            Self::Invalid(reason) => write!(formatter, "invalid paused-task snapshot: {reason}"),
            Self::HostBoundEndpoint => formatter.write_str(
                "tasks attached through browser_endpoint cannot be persisted; reattach through the new host",
            ),
            Self::Json(error) => write!(formatter, "paused-task JSON: {error}"),
        }
    }
}

impl std::error::Error for PausedTaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for PausedTaskError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
