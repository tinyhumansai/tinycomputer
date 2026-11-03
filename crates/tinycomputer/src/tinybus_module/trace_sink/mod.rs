//! The optional host-owned persistence path for desktop traces and reports.
//!
//! The desktop engine emits trace events synchronously, sometimes while a
//! task is on a Tokio executor thread. This port moves every trace callback to
//! one dedicated worker before making a bounded synchronous wait, so the bus
//! acknowledgement never runs on the caller's executor thread. One worker
//! preserves accepted event order for this module instance. A shared in-flight
//! guard refuses overlapping callbacks as bounded backpressure; it does not
//! identify whether a call is causally re-entrant.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use tinybus::{Connection, Error as TinyBusError, Proxy, Result as TinyBusResult};
use tinycomputer_bus::trace_sink::{
    self, SinkAck, TaskReportRecord, TraceArtifact, TraceArtifactKind, TraceArtifactRef, TraceEvent,
};
mod engine_adapter;

use engine_adapter::{remaining_millis, remaining_timeout};
#[cfg(test)]
use engine_adapter::{send, validate_artifact_size};
#[cfg(test)]
use tinycomputer_desktop::{
    TraceSink as EngineTraceSink, TraceSinkArtifact, TraceSinkArtifactKind as EngineArtifactKind,
    TraceSinkArtifactLimits, TraceSinkDeadline,
};
pub(super) const CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) enum Command {
    Append {
        record: serde_json::Value,
        deadline: Instant,
        byte_budget: u64,
        reply: SyncSender<Result<(), ()>>,
    },
    StoreArtifact {
        kind: TraceArtifactKind,
        name: String,
        bytes: Vec<u8>,
        deadline: Instant,
        limits: tinycomputer_bus::trace_sink::TraceArtifactLimits,
        reply: SyncSender<Result<String, ()>>,
    },
}

/// The synchronous sink adapter, backed by a worker that owns the bus calls.
pub(crate) struct TraceLink {
    connection: Connection,
    destination: tinybus::BusName,
    pub(super) worker: std::sync::Mutex<Option<SyncSender<Command>>>,
    worker_thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    pub(super) callback_active: Arc<AtomicBool>,
    confidential: bool,
}

impl std::fmt::Debug for TraceLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TraceLink").finish_non_exhaustive()
    }
}

impl TraceLink {
    pub(crate) fn new(
        connection: Connection,
        destination: tinybus::BusName,
    ) -> TinyBusResult<Arc<Self>> {
        Self::new_with_confidentiality(connection, destination, true)
    }

    #[cfg(test)]
    fn new_for_bus_tests(
        connection: Connection,
        destination: tinybus::BusName,
    ) -> TinyBusResult<Arc<Self>> {
        Self::new_with_confidentiality(connection, destination, false)
    }

    fn new_with_confidentiality(
        connection: Connection,
        destination: tinybus::BusName,
        confidential: bool,
    ) -> TinyBusResult<Arc<Self>> {
        let worker_connection = connection.clone();
        let worker_destination = destination.clone();
        let callback_active = Arc::new(AtomicBool::new(false));
        let worker_callback_active = callback_active.clone();
        let runtime = tokio::runtime::Handle::current();
        if runtime.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return Err(TinyBusError::failed(
                "trace sink requires a multi-thread Tokio runtime",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_thread = std::thread::Builder::new()
            .name("tinycomputer-trace-sink".into())
            .spawn(move || {
                worker(
                    &runtime,
                    &worker_connection,
                    &worker_destination,
                    &worker_callback_active,
                    &receiver,
                    confidential,
                );
            })
            .map_err(|error| TinyBusError::failed(format!("trace sink worker failed: {error}")))?;
        Ok(Arc::new(Self {
            connection,
            destination,
            worker: std::sync::Mutex::new(Some(sender)),
            worker_thread: std::sync::Mutex::new(Some(worker_thread)),
            callback_active,
            confidential,
        }))
    }

    pub(crate) fn shutdown(&self) -> TinyBusResult<()> {
        let sender = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(sender);

        let mut worker_thread = self
            .worker_thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(worker_thread) = worker_thread.take() {
            worker_thread
                .join()
                .map_err(|_| TinyBusError::failed("trace sink worker panicked during shutdown"))?;
        }

        let deadline = Instant::now() + CALL_TIMEOUT + Duration::from_secs(1);
        while self.callback_active.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.callback_active.load(Ordering::Acquire) {
            return Err(TinyBusError::failed(
                "trace sink callback did not finish during shutdown",
            ));
        }
        Ok(())
    }

    pub(crate) async fn write_report(&self, record: TaskReportRecord) -> TinyBusResult<()> {
        let _guard = {
            let worker = self
                .worker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if worker.is_none() {
                return Err(TinyBusError::failed("trace sink worker stopped"));
            }
            CallbackGuard::try_acquire(&self.callback_active)
                .ok_or_else(|| TinyBusError::failed("trace sink callback is busy"))?
        };
        call_to_host::<SinkAck, _>(
            &self.proxy(CALL_TIMEOUT)?,
            trace_sink::methods::WRITE_REPORT,
            (record,),
            self.confidential,
        )
        .await
        .map(|_| ())
    }

    fn proxy(&self, timeout: Duration) -> TinyBusResult<Proxy> {
        self.connection
            .proxy(
                self.destination.as_str(),
                trace_sink::OBJECT_PATH,
                trace_sink::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(timeout))
    }
}

fn worker(
    runtime: &tokio::runtime::Handle,
    connection: &Connection,
    destination: &tinybus::BusName,
    callback_active: &Arc<AtomicBool>,
    receiver: &Receiver<Command>,
    confidential: bool,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Append {
                record,
                deadline,
                byte_budget,
                reply,
            } => {
                let Some(timeout) = remaining_timeout(deadline) else {
                    let _ = reply.send(Err(()));
                    continue;
                };
                let Some(deadline_ms) = remaining_millis(deadline) else {
                    let _ = reply.send(Err(()));
                    continue;
                };
                let Some(guard) = CallbackGuard::try_acquire(callback_active) else {
                    let _ = reply.send(Err(()));
                    continue;
                };
                let result = runtime.block_on(async {
                    let proxy = connection
                        .proxy(
                            destination.as_str(),
                            trace_sink::OBJECT_PATH,
                            trace_sink::INTERFACE,
                        )?
                        .with_timeout(timeout);
                    call_to_host::<SinkAck, _>(
                        &proxy,
                        trace_sink::methods::APPEND,
                        (TraceEvent {
                            record,
                            deadline_ms,
                            byte_budget,
                        },),
                        confidential,
                    )
                    .await
                    .map(|_| ())
                });
                drop(guard);
                let _ = reply.send(result.map_err(|_| ()));
            }
            Command::StoreArtifact {
                kind,
                name,
                bytes,
                deadline,
                limits,
                reply,
            } => {
                let Some(timeout) = remaining_timeout(deadline) else {
                    let _ = reply.send(Err(()));
                    continue;
                };
                let Some(deadline_ms) = remaining_millis(deadline) else {
                    let _ = reply.send(Err(()));
                    continue;
                };
                let Some(guard) = CallbackGuard::try_acquire(callback_active) else {
                    let _ = reply.send(Err(()));
                    continue;
                };
                let result = runtime.block_on(async {
                    let proxy = connection
                        .proxy(
                            destination.as_str(),
                            trace_sink::OBJECT_PATH,
                            trace_sink::INTERFACE,
                        )?
                        .with_timeout(timeout);
                    call_to_host::<TraceArtifactRef, _>(
                        &proxy,
                        trace_sink::methods::STORE_ARTIFACT,
                        (TraceArtifact {
                            kind,
                            name,
                            bytes,
                            deadline_ms,
                            limits,
                        },),
                        confidential,
                    )
                    .await
                    .map(|reply| reply.reference)
                });
                drop(guard);
                let _ = reply.send(result.map_err(|_| ()));
            }
        }
    }
}

async fn call<T, A>(proxy: &Proxy, member: &str, args: A) -> TinyBusResult<T>
where
    T: DeserializeOwned,
    A: Serialize,
{
    proxy.call_confidential(member, args).await
}

async fn call_to_host<T, A>(
    proxy: &Proxy,
    member: &str,
    args: A,
    confidential: bool,
) -> TinyBusResult<T>
where
    T: DeserializeOwned,
    A: Serialize,
{
    if confidential {
        call(proxy, member, args).await
    } else {
        proxy.call(member, args).await
    }
}

pub(super) struct CallbackGuard<'a>(&'a AtomicBool);

impl<'a> CallbackGuard<'a> {
    fn try_acquire(active: &'a AtomicBool) -> Option<Self> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(active))
    }
}

impl Drop for CallbackGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Drop for TraceLink {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
#[path = "trace_sink_tests.rs"]
mod tests;
