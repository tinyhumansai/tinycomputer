//! Tests for trace-sink host callback delivery and rejection.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn callbacks_commit_events_artifacts_and_reports_over_the_bus()
-> Result<(), Box<dyn std::error::Error>> {
    let (link, host, _host_connection) = connected_link(CompleteRecordingHost::default()).await?;
    let deadline = TraceSinkDeadline::after(5_000)?;
    link.append(&serde_json::json!({"event": "started"}), deadline, 4096)
        .map_err(|error| error.to_string())?;
    let artifact_deadline = TraceSinkDeadline::after(5_000)?;
    let reference = link
        .store_artifact(
            TraceSinkArtifact {
                kind: EngineArtifactKind::Screenshot,
                name: "screen.png",
                bytes: &[1, 2, 3],
                deadline: artifact_deadline,
            },
            TraceSinkArtifactLimits {
                screenshot_bytes: 128,
                screenshot_count: 2,
                refmap_bytes: 64,
            },
        )
        .map_err(|error| error.to_string())?;
    link.write_report(TaskReportRecord {
        task_id: "task-1".into(),
        report: empty_task_report()?,
    })
    .await?;

    assert_eq!(reference, "trace-artifact-1");
    assert_eq!(
        host.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_slice(),
        &[serde_json::json!({"event": "started"})]
    );
    assert_eq!(
        host.artifacts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[0]
            .bytes,
        [1, 2, 3]
    );
    assert_eq!(
        host.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[0]
            .task_id,
        "task-1"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_service_routes_task_reports_to_the_confidential_host_callback()
-> Result<(), Box<dyn std::error::Error>> {
    use tinybus::Interface;
    use tinycomputer_bus::agent::{
        AgentResponse, StartTaskRequest, TaskId, TaskReport, TaskReportRequest, TaskView,
    };
    use tinycomputer_bus::names;

    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let host = CompleteRecordingHost::default();
    host_connection
        .serve_at(trace_sink::OBJECT_PATH.try_into()?, host.clone())
        .await?;
    host_connection.request_name(trace_sink::INTERFACE).await?;
    let module_connection = Connection::connect(bus.connect().await?).await?;
    let host_name = host_connection
        .unique_name()
        .ok_or("host connection has no unique name")?;
    let config = serde_json::json!({
        "trace_sink": {"destination": host_name.as_str()}
    });
    let service = crate::tinybus_module::DesktopService::from_config_with_connection(
        &config,
        module_connection.clone(),
    )?;

    let started: AgentResponse<TaskView> = serde_json::from_value(
        service
            .call(
                &names::methods::START_TASK.try_into()?,
                serde_json::json!([serde_json::from_value::<StartTaskRequest>(
                    serde_json::json!({
                        "flow": {"app": "Mail", "steps": ["start a new message"]}
                    }),
                )?]),
            )
            .await?,
    )?;
    let task_id = started.data.ok_or("start task returned no view")?.id;
    let report: AgentResponse<TaskReport> = serde_json::from_value(
        service
            .call(
                &names::methods::TASK_REPORT.try_into()?,
                serde_json::json!([TaskReportRequest::new(TaskId::new(task_id.to_string()))]),
            )
            .await?,
    )?;

    assert_eq!(
        report.error.as_ref().map(|error| error.code.as_str()),
        Some("TRACE_SINK_FAILED")
    );
    assert!(
        host.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "the unattested test module must not deliver a confidential report"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_host_callbacks_return_errors_to_the_caller()
-> Result<(), Box<dyn std::error::Error>> {
    let host = CompleteRecordingHost::default();
    host.reject.store(true, Ordering::Release);
    let (link, _, _host_connection) = connected_link(host).await?;

    assert!(
        link.append(
            &serde_json::json!({"event": "rejected"}),
            TraceSinkDeadline::after(5_000)?,
            4096,
        )
        .is_err()
    );
    assert!(
        link.store_artifact(
            TraceSinkArtifact {
                kind: EngineArtifactKind::RefMap,
                name: "refs.json",
                bytes: &[4, 5],
                deadline: TraceSinkDeadline::after(5_000)?,
            },
            TraceSinkArtifactLimits {
                screenshot_bytes: 128,
                screenshot_count: 2,
                refmap_bytes: 64,
            },
        )
        .is_err()
    );
    assert!(
        link.write_report(TaskReportRecord {
            task_id: "task-1".into(),
            report: empty_task_report()?,
        })
        .await
        .is_err()
    );
    Ok(())
}
