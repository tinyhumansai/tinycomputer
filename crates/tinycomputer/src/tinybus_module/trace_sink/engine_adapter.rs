//! Implements the desktop engine's synchronous trace callbacks over the worker queue.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use tinycomputer_desktop::{
    TraceSink as EngineTraceSink, TraceSinkArtifact, TraceSinkArtifactKind as EngineArtifactKind,
    TraceSinkArtifactLimits, TraceSinkDeadline, TraceSinkError,
};

use super::{CALL_TIMEOUT, Command, TraceLink};

impl EngineTraceSink for TraceLink {
    fn append(
        &self,
        record: &serde_json::Value,
        deadline: TraceSinkDeadline,
        byte_budget: u64,
    ) -> std::result::Result<(), TraceSinkError> {
        if self.callback_active.load(Ordering::Acquire) {
            return Err(TraceSinkError::Internal(
                "trace sink callback is busy".into(),
            ));
        }
        let deadline = operation_deadline(deadline)
            .ok_or_else(|| TraceSinkError::Internal("trace sink deadline expired".into()))?;
        let timeout = remaining_timeout(deadline)
            .ok_or_else(|| TraceSinkError::Internal("trace sink deadline expired".into()))?;
        let (reply, result) = mpsc::sync_channel(1);
        send(
            &self.worker,
            Command::Append {
                record: record.clone(),
                deadline,
                byte_budget,
                reply,
            },
        )?;
        receive(&result, timeout)
            .map_err(|_| TraceSinkError::Internal("trace sink acknowledgment timed out".into()))?
            .map_err(|()| TraceSinkError::Internal("trace sink rejected event".into()))
    }

    fn store_artifact(
        &self,
        artifact: TraceSinkArtifact<'_>,
        limits: TraceSinkArtifactLimits,
    ) -> std::result::Result<String, TraceSinkError> {
        validate_artifact_size(artifact.bytes.len())?;
        if self.callback_active.load(Ordering::Acquire) {
            return Err(TraceSinkError::Internal(
                "trace sink callback is busy".into(),
            ));
        }
        let deadline = operation_deadline(artifact.deadline)
            .ok_or_else(|| TraceSinkError::Internal("trace artifact deadline expired".into()))?;
        let timeout = remaining_timeout(deadline)
            .ok_or_else(|| TraceSinkError::Internal("trace artifact deadline expired".into()))?;
        let (reply, result) = mpsc::sync_channel(1);
        let kind = match artifact.kind {
            EngineArtifactKind::Screenshot => {
                tinycomputer_bus::trace_sink::TraceArtifactKind::Screenshot
            }
            EngineArtifactKind::RefMap => tinycomputer_bus::trace_sink::TraceArtifactKind::RefMap,
        };
        send(
            &self.worker,
            Command::StoreArtifact {
                kind,
                name: artifact.name.to_owned(),
                bytes: artifact.bytes.to_vec(),
                deadline,
                limits: tinycomputer_bus::trace_sink::TraceArtifactLimits {
                    screenshot_bytes: limits.screenshot_bytes,
                    screenshot_count: limits.screenshot_count,
                    refmap_bytes: limits.refmap_bytes,
                },
                reply,
            },
        )?;
        receive(&result, timeout)
            .map_err(|_| {
                TraceSinkError::Internal("trace artifact acknowledgment timed out".into())
            })?
            .map_err(|()| TraceSinkError::Internal("trace sink rejected artifact".into()))
    }
}

pub(super) fn validate_artifact_size(size: usize) -> std::result::Result<(), TraceSinkError> {
    if size > tinycomputer_bus::trace_sink::MAX_TRACE_ARTIFACT_BYTES {
        return Err(TraceSinkError::Internal(
            "trace artifact exceeds the 8 MiB bus limit".into(),
        ));
    }
    Ok(())
}

pub(super) fn remaining_timeout(deadline: Instant) -> Option<Duration> {
    let remaining = deadline.checked_duration_since(Instant::now())?;
    let timeout = remaining.min(CALL_TIMEOUT);
    (timeout.as_millis() > 0).then_some(timeout)
}

pub(super) fn remaining_millis(deadline: Instant) -> Option<u64> {
    let remaining = deadline.checked_duration_since(Instant::now())?;
    let millis = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
    (millis > 0).then_some(millis)
}

pub(super) fn send(
    worker: &std::sync::Mutex<Option<SyncSender<Command>>>,
    command: Command,
) -> std::result::Result<(), TraceSinkError> {
    let worker = worker
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(worker) = worker.as_ref() else {
        return Err(TraceSinkError::Internal("trace sink worker stopped".into()));
    };
    worker.try_send(command).map_err(|error| match error {
        TrySendError::Full(_) => TraceSinkError::Internal("trace sink worker is busy".into()),
        TrySendError::Disconnected(_) => {
            TraceSinkError::Internal("trace sink worker stopped".into())
        }
    })
}

pub(super) fn operation_deadline(deadline: TraceSinkDeadline) -> Option<Instant> {
    let remaining = deadline.remaining();
    (!remaining.is_zero())
        .then(|| Instant::now().checked_add(remaining))
        .flatten()
}

pub(super) fn receive<T>(receiver: &Receiver<T>, timeout: Duration) -> Result<T, RecvTimeoutError> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| receiver.recv_timeout(timeout))
        }
        Ok(_) => Err(RecvTimeoutError::Timeout),
        Err(_) => receiver.recv_timeout(timeout),
    }
}
