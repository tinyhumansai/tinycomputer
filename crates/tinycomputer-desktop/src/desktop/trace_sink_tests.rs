//! Tests for desktop trace-sink configuration and callback behavior.

use super::*;
use crate::{
    TraceSink, TraceSinkArtifact, TraceSinkArtifactLimits, TraceSinkDeadline, TraceSinkError,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<Value>>,
}

impl TraceSink for RecordingSink {
    fn append(
        &self,
        event: &Value,
        _deadline: TraceSinkDeadline,
        _byte_budget: u64,
    ) -> std::result::Result<(), TraceSinkError> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.clone());
        Ok(())
    }

    fn store_artifact(
        &self,
        _artifact: TraceSinkArtifact<'_>,
        _limits: TraceSinkArtifactLimits,
    ) -> std::result::Result<String, TraceSinkError> {
        Ok("opaque:test".into())
    }
}

#[test]
fn opted_in_sink_receives_events_without_opening_the_configured_trace_path() {
    static NEXT_TEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = NEXT_TEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "tinycomputer-native-trace-{}-{nonce}.jsonl",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&path);
    let sink = Arc::new(RecordingSink::default());
    let desktop = Desktop::new()
        .with_trace(path.clone(), true)
        .with_trace_sink(sink.clone());

    let reply = desktop.version();

    assert!(reply.ok, "a committed trace event lets the command succeed");
    assert!(desktop.is_tracing());
    assert!(!path.exists(), "sink mode must not create the legacy file");
    assert!(
        sink.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|event| event["event"] == "command.end")
    );
}
