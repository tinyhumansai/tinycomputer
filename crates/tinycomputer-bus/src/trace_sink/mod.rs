//! The optional confidential host callback for desktop trace events, trace
//! artifacts, and task reports. The module never chooses a persistence path.
//!
//! A host opts in explicitly through module configuration. Once selected, the
//! module sends events in order and does not fall back to a local file. An
//! acknowledged event is committed by the host; a failed acknowledgment is
//! reported using the desktop trace's strict/best-effort setting. Task report
//! writes are acknowledged before `TaskReport` returns to its caller. The
//! confidential caller identity is the only authority for the host's user and
//! scope; hosts must ignore payload fields when choosing that scope. A sink
//! callback must not call back into this module, which may be waiting for its
//! acknowledgment; both sides enforce bounded calls so a broken callback
//! fails instead of waiting forever.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

use crate::agent::TaskReport;

/// The interface the host serves for trace and report persistence.
pub const INTERFACE: &str = "ai.tinyhumans.tinycomputer.TraceSinkHost";

/// The object path the host serves [`INTERFACE`] at.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinycomputer/TraceSinkHost";

/// One constant per member of [`INTERFACE`].
pub mod methods {
    /// Commits one already-sanitized desktop trace event.
    pub const APPEND: &str = "Append";
    /// Commits one desktop trace artifact and returns its opaque reference.
    pub const STORE_ARTIFACT: &str = "StoreArtifact";
    /// Persists one task report.
    pub const WRITE_REPORT: &str = "WriteReport";
}

/// The members exposed by [`INTERFACE`], in dispatch order.
pub const METHODS: [&str; 3] = [
    methods::APPEND,
    methods::STORE_ARTIFACT,
    methods::WRITE_REPORT,
];

/// An acknowledgment with no additional result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinkAck {}

/// One sanitized, complete event emitted by agent-desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// The exact event object written by the desktop trace format.
    pub record: Value,
    /// Remaining milliseconds in the engine's synchronous sink deadline.
    pub deadline_ms: u64,
    /// Maximum cumulative JSONL bytes allowed for this trace scope.
    pub byte_budget: u64,
}

/// Which aggregate artifact budget applies to one trace artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceArtifactKind {
    /// A screenshot image.
    Screenshot,
    /// A saved accessibility reference map.
    RefMap,
}

/// Aggregate artifact limits for the active trace scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceArtifactLimits {
    /// Maximum total screenshot bytes.
    pub screenshot_bytes: u64,
    /// Maximum screenshot count.
    pub screenshot_count: u32,
    /// Maximum total reference-map bytes.
    pub refmap_bytes: u64,
}

/// One trace artifact sent to the host. The bytes may contain screen content
/// and therefore this member is confidential. `bytes` is standard padded
/// base64 on the wire and contains at most [`MAX_TRACE_ARTIFACT_BYTES`] decoded
/// bytes, keeping the callback under `TinyBus`'s 16 MiB frame cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceArtifact {
    /// Whether the bytes are a screenshot or a reference map.
    pub kind: TraceArtifactKind,
    /// Producer-generated name, scoped to the host's trace sink.
    pub name: String,
    /// Artifact content, encoded as standard padded base64 on the wire.
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
    /// Remaining milliseconds in the engine's admission deadline.
    pub deadline_ms: u64,
    /// Limits the host must enforce atomically for this trace scope.
    pub limits: TraceArtifactLimits,
}

/// Maximum decoded artifact size accepted by the trace sink callback.
pub const MAX_TRACE_ARTIFACT_BYTES: usize = MAX_ARTIFACT_BYTES;

/// The host's committed opaque reference for an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceArtifactRef {
    /// A bounded, opaque reference used in trace events.
    pub reference: String,
}

/// A report emitted after the caller explicitly requests `TaskReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskReportRecord {
    /// Task whose report is being persisted.
    pub task_id: String,
    /// Complete report returned to the caller.
    pub report: TaskReport,
}

#[cfg(test)]
#[path = "trace_sink_tests.rs"]
mod tests;
