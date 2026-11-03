//! Tests for task-report failures returned through the agent response envelope.

use super::report_sink_failure;

#[test]
fn a_task_report_sink_failure_is_a_recoverable_agent_response() {
    let response = report_sink_failure::<serde_json::Value>();

    assert!(!response.ok);
    assert!(response.data.is_none());
    assert!(response.error.is_some());
    let Some(error) = response.error else {
        return;
    };
    assert_eq!(error.code, "TRACE_SINK_FAILED");
    assert!(error.message.contains("could not be persisted"));
    assert!(error.hint.contains("retry TaskReport"));
    assert!(error.recoverable);
}
