//! Per-task usage recorder tests.

use super::{ModelRole, UsageRecorder};
use tinycomputer_bus::agent::TaskUsage;

#[test]
fn snapshots_only_reported_measurements_and_counts_unknown_calls() {
    let recorder = UsageRecorder::default();
    recorder.record_started(ModelRole::Jev);
    recorder.record_measurements(ModelRole::Jev, Some(8), None, Some(3), None);
    recorder.record_call(ModelRole::Planner, None, None, None, None);

    let usage = recorder.snapshot();
    assert_eq!(usage.jev.calls, 1);
    assert_eq!(usage.jev.input_tokens, Some(8));
    assert_eq!(usage.jev.cached_input_tokens, None);
    assert_eq!(usage.jev.output_tokens, Some(3));
    assert_eq!(usage.jev.cost_usd_nanos, None);
    assert_eq!(usage.planner.calls, 1);
    assert_eq!(usage.planner.input_tokens, None);
}

#[test]
fn a_restored_recorder_keeps_accumulating_from_its_snapshot() {
    let mut snapshot = TaskUsage::default();
    snapshot.jev.calls = 4;
    snapshot.jev.input_tokens = Some(12);
    let recorder = UsageRecorder::from_snapshot(snapshot);
    recorder.record_call(ModelRole::Jev, Some(3), None, None, None);

    let usage = recorder.snapshot();
    assert_eq!(usage.jev.calls, 5);
    assert_eq!(usage.jev.input_tokens, Some(15));
}
