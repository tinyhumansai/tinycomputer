//! Shared host fixtures for trace-sink callback and lifecycle tests.
//! The cases cover the `TinyBus` transport, payloads, deadlines, and shutdown.

#[path = "trace_sink_tests/callbacks_tests.rs"]
mod callbacks_tests;
#[path = "trace_sink_tests/deadline_tests.rs"]
mod deadline_tests;
#[path = "trace_sink_tests/lifecycle_tests.rs"]
mod lifecycle_tests;

use super::*;
use std::sync::Mutex;
use tinycomputer_bus::agent::TaskReport;
use tinycomputer_bus::trace_sink::{TaskReportRecord, TraceEvent};

#[derive(Clone, Default)]
struct RecordingHost(Arc<Mutex<Vec<serde_json::Value>>>);

#[derive(Clone, Default)]
struct CompleteRecordingHost {
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    artifacts: Arc<Mutex<Vec<TraceArtifact>>>,
    reports: Arc<Mutex<Vec<TaskReportRecord>>>,
    reject: Arc<AtomicBool>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.TraceSinkHost")]
impl CompleteRecordingHost {
    async fn append(&self, event: TraceEvent) -> TinyBusResult<SinkAck> {
        tokio::task::yield_now().await;
        if self.reject.load(Ordering::Acquire) {
            return Err(TinyBusError::failed("host rejected event"));
        }
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.record);
        Ok(SinkAck {})
    }

    async fn store_artifact(&self, artifact: TraceArtifact) -> TinyBusResult<TraceArtifactRef> {
        tokio::task::yield_now().await;
        if self.reject.load(Ordering::Acquire) {
            return Err(TinyBusError::failed("host rejected artifact"));
        }
        self.artifacts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(artifact);
        Ok(TraceArtifactRef {
            reference: "trace-artifact-1".into(),
        })
    }

    async fn write_report(&self, report: TaskReportRecord) -> TinyBusResult<SinkAck> {
        tokio::task::yield_now().await;
        if self.reject.load(Ordering::Acquire) {
            return Err(TinyBusError::failed("host rejected report"));
        }
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(report);
        Ok(SinkAck {})
    }
}

async fn connected_link(
    host: CompleteRecordingHost,
) -> Result<(Arc<TraceLink>, CompleteRecordingHost, Connection), Box<dyn std::error::Error>> {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    host_connection
        .serve_at(trace_sink::OBJECT_PATH.try_into()?, host.clone())
        .await?;
    host_connection.request_name(trace_sink::INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;
    let link = TraceLink::new_for_bus_tests(module, tinybus::BusName::new(trace_sink::INTERFACE)?)?;
    Ok((link, host, host_connection))
}

fn empty_task_report() -> Result<TaskReport, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "view": {
            "id": "task-1",
            "status": {"state": "running"},
            "summary": "working",
            "progress": 0.0,
            "next": []
        },
        "steps": [],
        "records": {},
        "artifacts": [],
        "learned": [],
        "trace": []
    }))
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.TraceSinkHost")]
impl RecordingHost {
    async fn append(&self, event: TraceEvent) -> TinyBusResult<SinkAck> {
        tokio::task::yield_now().await;
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.record);
        Ok(SinkAck {})
    }

    async fn store_artifact(&self, _: TraceArtifact) -> TinyBusResult<TraceArtifactRef> {
        tokio::task::yield_now().await;
        Err(TinyBusError::failed("unused by deadline test"))
    }

    async fn write_report(&self, _: TaskReportRecord) -> TinyBusResult<SinkAck> {
        tokio::task::yield_now().await;
        Err(TinyBusError::failed("unused by deadline test"))
    }
}

#[derive(Clone)]
struct BlockingRecordingHost {
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    started: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    release: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.TraceSinkHost")]
impl BlockingRecordingHost {
    async fn append(&self, event: TraceEvent) -> TinyBusResult<SinkAck> {
        if event.record["event"] == "blocker" {
            if let Some(started) = self
                .started
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _ = started.send(());
            }
            let release = self
                .release
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(release) = release {
                release
                    .await
                    .map_err(|_| TinyBusError::failed("test callback was not released"))?;
            }
        }
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.record);
        Ok(SinkAck {})
    }

    async fn store_artifact(&self, _: TraceArtifact) -> TinyBusResult<TraceArtifactRef> {
        tokio::task::yield_now().await;
        Err(TinyBusError::failed("unused by queued deadline test"))
    }

    async fn write_report(&self, _: TaskReportRecord) -> TinyBusResult<SinkAck> {
        tokio::task::yield_now().await;
        Err(TinyBusError::failed("unused by queued deadline test"))
    }
}
