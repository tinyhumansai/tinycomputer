//! Tests pinning the trace sink contract's wire representations.

use super::*;

#[test]
fn sink_records_keep_their_wire_shapes() -> Result<(), serde_json::Error> {
    let event = TraceEvent {
        record: serde_json::json!({
            "event": "command.end", "seq": 4, "session_id": "s1"
        }),
        deadline_ms: 900,
        byte_budget: 4096,
    };
    let wire = serde_json::to_value(event)?;
    assert_eq!(wire["record"]["event"], "command.end");
    assert_eq!(wire["deadline_ms"], 900);
    assert_eq!(wire["byte_budget"], 4096);

    let artifact = TraceArtifact {
        kind: TraceArtifactKind::RefMap,
        name: "snapshot.json".into(),
        bytes: vec![1, 2],
        deadline_ms: 700,
        limits: TraceArtifactLimits {
            screenshot_bytes: 10,
            screenshot_count: 2,
            refmap_bytes: 20,
        },
    };
    let wire = serde_json::to_value(artifact)?;
    assert_eq!(
        wire,
        serde_json::json!({
            "kind": "ref_map",
            "name": "snapshot.json",
            "bytes": "AQI=",
            "deadline_ms": 700,
            "limits": {
                "screenshot_bytes": 10,
                "screenshot_count": 2,
                "refmap_bytes": 20
            }
        })
    );
    let decoded: TraceArtifact = serde_json::from_value(wire)?;
    assert_eq!(decoded.bytes, [1, 2]);
    Ok(())
}

#[test]
fn report_record_reuses_the_agent_report_contract() -> Result<(), serde_json::Error> {
    let report = TaskReportRecord {
        task_id: "task-1".into(),
        report: serde_json::from_value(serde_json::json!({
            "view": {
                "id": "task-1",
                "status": {"state": "running"},
                "summary": "running",
                "progress": 0.0,
                "next": []
            },
            "steps": [], "records": {}, "artifacts": [], "learned": [], "trace": []
        }))?,
    };
    let wire = serde_json::to_value(report)?;
    assert_eq!(wire["task_id"], "task-1");
    assert_eq!(wire["report"]["trace"], serde_json::json!([]));
    Ok(())
}

#[test]
fn the_host_interface_is_separate_and_has_stable_member_names() -> Result<(), serde_json::Error> {
    assert_ne!(INTERFACE, crate::names::INTERFACE);
    assert_eq!(METHODS, ["Append", "StoreArtifact", "WriteReport"]);
    assert_eq!(SinkAck {}, serde_json::from_str("{}")?);
    Ok(())
}
