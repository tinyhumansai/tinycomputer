//! Loads a built module through the real `TinyBus` dynamic loader.
//!
//! The unit tests serve the interface in-process; this proves the compiled
//! `cdylib` exports the ABI, announces its manifest, claims its name, and
//! answers a call. A release archive is not accepted until this passes against
//! the artifact that would ship.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{
    AccessFuture, FailureReport, FailureReporter, FailureSink, HostCallbackEndpoint, ModuleAccess,
    ModuleHost, ProxyFailure, ReconfigurableModule, ReconfigureFailure, ReleaseRecord,
};
use tinybus::transport::memory::MemoryBus;
use tinycomputer_bus::config::ComputerConfig;
use tinycomputer_bus::{DesktopResponse, METHODS, names, trace_sink};
use tinycomputer_client::ComputerClient;

static TINYCOMPUTER_RELEASE: ReleaseRecord = ReleaseRecord {
    id: "tinycomputer",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.16.0",
    release_url: "https://example.invalid/tinycomputer",
    assets: &[],
};

#[derive(Default)]
struct SilentFailureSink;

impl FailureSink for SilentFailureSink {
    fn capture(&self, _report: &FailureReport<'_>) {}
}

struct LoadedModuleAccess {
    bus: MemoryBus,
    host: Arc<ModuleHost>,
    reporter: FailureReporter,
}

impl ModuleAccess for LoadedModuleAccess {
    fn find(&self, id: &str) -> Option<&'static ReleaseRecord> {
        (id == TINYCOMPUTER_RELEASE.id).then_some(&TINYCOMPUTER_RELEASE)
    }

    fn enabled(&self) -> bool {
        true
    }

    fn reporter(&self) -> &FailureReporter {
        &self.reporter
    }

    fn ensure_loaded<'a>(&'a self, id: &'a str) -> AccessFuture<'a, Result<(), String>> {
        let Some(record) = self.find(id) else {
            return Box::pin(
                async move { Err(format!("module `{id}` is not in the release table")) },
            );
        };
        let bus_name = record.bus_name;
        Box::pin(async move {
            self.host
                .ensure_ready(bus_name)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn proxy<'a>(
        &'a self,
        bus_name: &'a str,
        object_path: &'a str,
    ) -> AccessFuture<'a, Result<tinybus::Proxy, ProxyFailure>> {
        let bus = self.bus.clone();
        let bus_name = bus_name.to_owned();
        let object_path = object_path.to_owned();
        Box::pin(async move {
            let transport = bus.connect().await.map_err(|_| ProxyFailure::Transport)?;
            let connection = Connection::connect(transport)
                .await
                .map_err(|_| ProxyFailure::Transport)?;
            connection
                .proxy(&bus_name, &object_path, &bus_name)
                .map_err(|_| ProxyFailure::Contract)
        })
    }

    fn reinitialize<'a>(
        &'a self,
        id: &'a str,
        config: serde_json::Value,
    ) -> AccessFuture<'a, Result<(), ReconfigureFailure>> {
        Box::pin(async move {
            self.host
                .reinitialize(id, config)
                .await
                .map(|_| ())
                .map_err(|_| ReconfigureFailure::Failed)
        })
    }
}

#[derive(Clone, Default)]
struct TraceSinkCapture {
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    reports: Arc<Mutex<Vec<trace_sink::TaskReportRecord>>>,
    fail_reports: Arc<AtomicBool>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.TraceSinkHost")]
impl TraceSinkCapture {
    async fn append(&self, event: trace_sink::TraceEvent) -> tinybus::Result<trace_sink::SinkAck> {
        tokio::task::yield_now().await;
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.record);
        Ok(trace_sink::SinkAck {})
    }

    async fn store_artifact(
        &self,
        _artifact: trace_sink::TraceArtifact,
    ) -> tinybus::Result<trace_sink::TraceArtifactRef> {
        tokio::task::yield_now().await;
        Err(tinybus::Error::failed(
            "verification fixture does not persist trace artifacts",
        ))
    }

    async fn write_report(
        &self,
        report: trace_sink::TaskReportRecord,
    ) -> tinybus::Result<trace_sink::SinkAck> {
        tokio::task::yield_now().await;
        if self.fail_reports.load(Ordering::SeqCst) {
            return Err(tinybus::Error::failed("verification report sink failure"));
        }
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(report);
        Ok(trace_sink::SinkAck {})
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = module_argument()?;
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let module_host = Arc::new(ModuleHost::new(broker));
    let callback = module_host
        .prepare_host_callbacks(vec![HostCallbackEndpoint {
            path: trace_sink::OBJECT_PATH.parse()?,
            interface: trace_sink::INTERFACE.parse()?,
            members: trace_sink::METHODS
                .iter()
                .map(|member| (*member).parse())
                .collect::<Result<_, _>>()?,
        }])
        .await?;
    let capture = TraceSinkCapture::default();
    let trace_path = std::env::temp_dir().join(format!(
        "tinycomputer-module-trace-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let _trace_path = RemoveTraceFileOnDrop(trace_path.clone());
    callback
        .serve_at(trace_sink::OBJECT_PATH.parse()?, capture.clone())
        .await?;
    let config = serde_json::json!({
        "trace_sink": {"destination": callback.unique_name()},
        "trace_path": trace_path.to_string_lossy(),
        "trace_strict": true,
    });
    let info = module_host
        .load_file_with_host_callbacks(&module, config, &callback)
        .await?;
    let client_config = ComputerConfig::default();

    if info.name != "tinycomputer" {
        return Err(io::Error::other(format!(
            "loaded module `{}` instead of `{}`",
            info.name, "tinycomputer"
        ))
        .into());
    }

    let client = Connection::connect(bus.connect().await?).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let claimed = client.list_names().await?;
            if claimed.iter().any(|name| name.as_str() == names::INTERFACE) {
                return tinybus::Result::Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;

    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    // `Version` needs no permission and touches no other application, so it
    // proves the module answers without making the check depend on how the
    // build machine is configured.
    let reply: DesktopResponse = proxy.call(names::methods::VERSION, ()).await?;
    if !reply.ok {
        return Err(io::Error::other(format!(
            "module reported a failure for `{}`: {:?}",
            names::methods::VERSION,
            reply.error
        ))
        .into());
    }
    verify_trace_sink(&capture, &trace_path)?;

    verify_report_sink(&proxy, &capture).await?;

    verify_typed_client(&bus, &module_host, &client_config).await?;

    let declared: Vec<&str> = info
        .manifest
        .provides
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .map(tinybus::MemberName::as_str)
        .collect();
    if declared != METHODS {
        return Err(io::Error::other("manifest members differ from contract").into());
    }
    verify_globe_lifecycle(&proxy).await?;

    // Wrong-arity inputs stay safe even if a confidentiality guard regresses.
    for member in [
        names::accessibility::ACCESSIBILITY_FOCUS,
        names::accessibility::ACCESSIBILITY_VALIDATE_TARGET,
        names::accessibility::ACCESSIBILITY_PASTE,
    ] {
        let reply = proxy
            .call::<DesktopResponse>(member, serde_json::json!([]))
            .await;
        if !matches!(reply, Err(tinybus::Error::MethodFailed { name, .. }) if name == tinybus::Error::CONFIDENTIALITY_REQUIRED)
        {
            return Err(
                io::Error::other("native member failed to require confidential delivery").into(),
            );
        }
    }

    println!(
        "verified {} as TinyBus module `{}`, serving {} members",
        module.display(),
        info.name,
        METHODS.len()
    );
    broker_task.abort();
    Ok(())
}

async fn verify_typed_client(
    bus: &MemoryBus,
    host: &Arc<ModuleHost>,
    config: &ComputerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = ComputerClient::new(ReconfigurableModule::new(
        LoadedModuleAccess {
            bus: bus.clone(),
            host: Arc::clone(host),
            reporter: FailureReporter::new(Arc::new(SilentFailureSink)),
        },
        TINYCOMPUTER_RELEASE.id,
    ));
    client.record_loaded_with(config)?;
    let capabilities = client.describe(config).await?;
    if capabilities.contract_version != tinycomputer_bus::CONTRACT_VERSION {
        return Err(io::Error::other("typed client Describe contract version mismatch").into());
    }
    Ok(())
}

/// Removes a trace path if a regression sends sink-mode events to disk.
struct RemoveTraceFileOnDrop(PathBuf);

impl Drop for RemoveTraceFileOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Proves the loaded module emits a native desktop trace through its callback.
fn verify_trace_sink(
    capture: &TraceSinkCapture,
    legacy_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if legacy_path.exists() {
        return Err(io::Error::other("trace sink mode wrote the legacy trace file").into());
    }
    let events = capture
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let names = events
        .iter()
        .filter_map(|event| event.get("event").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    for required in ["trace.meta", "command.start", "command.end"] {
        if !names.contains(&required) {
            return Err(io::Error::other(format!(
                "loaded module trace callback did not receive `{required}`"
            ))
            .into());
        }
    }
    Ok(())
}

/// Exercises report delivery through the exact confidential callback binding.
async fn verify_report_sink(
    proxy: &tinybus::Proxy,
    capture: &TraceSinkCapture,
) -> Result<(), Box<dyn std::error::Error>> {
    use tinycomputer_bus::agent::{AgentResponse, StartTaskRequest, TaskReportRequest, TaskView};

    let started: AgentResponse<TaskView> = proxy
        .call_confidential(
            names::methods::START_TASK,
            (StartTaskRequest {
                flow: Some(serde_json::from_value(serde_json::json!({
                    "app": "verification",
                    "steps": ["verify report delivery"]
                }))?),
                ..StartTaskRequest::default()
            },),
        )
        .await?;
    if !started.ok {
        return Err(io::Error::other("module rejected the verification task flow").into());
    }
    let task_id = started
        .data
        .ok_or_else(|| io::Error::other("module did not start the no-op verification task"))?
        .id;
    let report: AgentResponse<tinycomputer_bus::agent::TaskReport> = proxy
        .call_confidential(
            names::methods::TASK_REPORT,
            (TaskReportRequest {
                id: task_id.clone(),
                trace: false,
            },),
        )
        .await?;
    if report.data.is_none() {
        return Err(io::Error::other("module did not return the verification task report").into());
    }
    let report_received = {
        let captured = capture
            .reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        captured.len() == 1 && captured[0].task_id == task_id.to_string()
    };
    if !report_received {
        return Err(
            io::Error::other("confidential host callback did not receive the task report").into(),
        );
    }
    capture.fail_reports.store(true, Ordering::SeqCst);
    let failed = proxy
        .call_confidential::<AgentResponse<tinycomputer_bus::agent::TaskReport>>(
            names::methods::TASK_REPORT,
            (TaskReportRequest {
                id: task_id,
                trace: false,
            },),
        )
        .await;
    let expected_failure = match failed {
        Ok(response) => {
            !response.ok
                && response.data.is_none()
                && response
                    .error
                    .is_some_and(|error| error.code == "TRACE_SINK_FAILED" && error.recoverable)
        }
        Err(_) => false,
    };
    if !expected_failure {
        return Err(io::Error::other("task report sink failure was not returned").into());
    }
    Ok(())
}

/// Checks native lease admission and terminal shutdown without device access.
async fn verify_globe_lifecycle(proxy: &tinybus::Proxy) -> Result<(), Box<dyn std::error::Error>> {
    // Invalid handles exercise the new artifact paths without touching devices.
    for member in [
        names::accessibility::GLOBE_POLL,
        names::accessibility::GLOBE_STOP,
    ] {
        let reply: DesktopResponse = proxy
            .call(
                member,
                (tinycomputer_bus::accessibility::GlobeHandle(
                    "fixture-invalid".into(),
                ),),
            )
            .await?;
        if reply.ok
            || reply
                .error
                .as_ref()
                .is_none_or(|error| error.code != "UNKNOWN_LISTENER")
        {
            return Err(io::Error::other("module accepted an unknown listener lease").into());
        }
    }

    let read: DesktopResponse = proxy
        .call(
            names::accessibility::GLOBE_READ,
            (tinycomputer_bus::accessibility::GlobeRead {
                handle: tinycomputer_bus::accessibility::GlobeHandle("fixture-invalid".into()),
                acknowledged_batch: None,
            },),
        )
        .await?;
    if read.ok
        || read
            .error
            .as_ref()
            .is_none_or(|error| error.code != "UNKNOWN_LISTENER")
    {
        return Err(io::Error::other("reliable read accepted an unknown lease").into());
    }
    let shutdown: DesktopResponse = proxy.call(names::accessibility::GLOBE_SHUTDOWN, ()).await?;
    if !shutdown.ok {
        return Err(io::Error::other("native terminal cleanup failed").into());
    }
    let late: DesktopResponse = proxy.call(names::accessibility::GLOBE_START, ()).await?;
    if late.ok {
        return Err(io::Error::other("listener started after terminal shutdown").into());
    }

    Ok(())
}

/// The path to the module under test, from the first argument.
fn module_argument() -> Result<PathBuf, io::Error> {
    std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("usage: verify_module <path-to-module>"))
}
