//! Loads the built module through the real `TinyBus` loader and calls it.
//!
//! This is the same path a production host takes: the dylib is attested
//! against `modules.toml`, loaded into a broker, given its private
//! configuration by reinitialization, and called over a connection. Nothing
//! here reaches into the module's Rust API — every runner in this crate that
//! drives the module (the lab, `task_live`, `task_fixture`) goes through
//! [`Host`], so what they exercise is exactly what a host sees.
//!
//! Build and attest the module with `scripts/build-module`; it prints the path
//! to pass as `TINYCOMPUTER_MODULE`.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tinybus::{
    Connection,
    broker::Broker,
    module::{ModuleHost, ModuleState},
    transport::memory::MemoryBus,
};
use tinycomputer_bus::agent::{
    AgentResponse, AwaitTaskRequest, Capabilities, ContinueTaskRequest, PlanTaskRequest,
    StartTaskRequest, TaskId, TaskPlan, TaskRef, TaskReport, TaskReportRequest, TaskView,
};
use tinycomputer_bus::browser::{
    OutputChunk, OutputRef, OutputRequest, ReadOutputRequest, ScreenshotRequest, SessionId,
    SessionInfo, SessionRef, SessionRequest,
};
use tinycomputer_bus::{
    DesktopResponse, FlowRunResult, FlowValidation, JevConfig, JevProvider, JevRunResult,
    RunFlowRequest, RunGoalRequest, ValidateFlowRequest, names,
};

/// Boxed error for the lab's command-line plumbing.
pub type LabError = Box<dyn std::error::Error + Send + Sync>;

/// A loaded, configured tinycomputer module and a proxy to it.
pub struct Host {
    proxy: tinybus::Proxy,
    broker: tokio::task::JoinHandle<tinybus::Result<()>>,
    _client: Connection,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Host").finish_non_exhaustive()
    }
}

/// Why a held output could not be read.
enum Unread {
    /// The bus call itself failed; trying again may work.
    Transport(LabError),
    /// The module answered with an error or with bytes that do not add up;
    /// trying again would get the same.
    Invalid(LabError),
}

impl Unread {
    fn into_error(self) -> LabError {
        match self {
            Self::Transport(error) | Self::Invalid(error) => error,
        }
    }
}

/// Aborts the broker's task if dropped while still armed: a load that fails
/// part-way must not leave the broker, and the module it loaded, running.
struct AbortOnDrop {
    task: tokio::task::AbortHandle,
    armed: bool,
}

impl AbortOnDrop {
    fn new(task: tokio::task::AbortHandle) -> Self {
        Self { task, armed: true }
    }

    /// Stops guarding, once the host has loaded and owns the task.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.task.abort();
        }
    }
}

/// The private `jev` configuration for `key` on `OpenRouter`: the module's
/// default endpoint and model unless `model` names one, in which case the
/// decisions endpoint that serves the aliases is used too.
///
/// # Errors
///
/// Fails only when the configuration does not serialize.
pub fn jev_config(key: String, model: Option<String>) -> Result<Value, LabError> {
    let mut jev = JevConfig::new(key);
    jev.provider = JevProvider::OpenRouter;
    if let Some(model) = model {
        jev.endpoint_url = Some("https://openrouter.ai/api/alpha/decisions".to_owned());
        jev.model = Some(model);
    }
    Ok(serde_json::to_value(jev)?)
}

/// `OPENROUTER_API_KEY`, which Jev and the planner both need.
///
/// # Errors
///
/// Fails when it is not exported.
pub fn openrouter_key() -> Result<String, LabError> {
    std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| io::Error::other("OPENROUTER_API_KEY is not exported").into())
}

impl Host {
    /// Loads the module at `module`, which must be listed in the
    /// `modules.toml` beside it, and hands it `config` as its private module
    /// configuration — the same JSON object a production host records for it
    /// (`jev`, `planner`, `browser`, `cursor`, …; see `MODULE.md`).
    ///
    /// # Errors
    ///
    /// Fails when the module is not attested, cannot load, or refuses the
    /// configuration.
    pub async fn load(module: &Path, config: Value) -> Result<Self, LabError> {
        verify_allowlisted(module)?;
        let bus = MemoryBus::new();
        let broker = Broker::new();
        let broker_task = broker.spawn(bus.clone());
        let guard = AbortOnDrop::new(broker_task.abort_handle());
        let module_host = ModuleHost::new(broker);
        let info = module_host.load_file(module)?;
        if info.name != "tinycomputer" {
            return Err(
                io::Error::other(format!("loaded unexpected module `{}`", info.name)).into(),
            );
        }
        let client = Connection::connect(bus.connect().await?).await?;
        wait_for_module(&client).await?;
        wait_until_ready(&module_host).await?;
        client.reinitialize_module("tinycomputer", config).await?;
        // A flow drives a real application for minutes; the bus default is
        // sized for single commands.
        let proxy = client
            .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?
            .with_timeout(std::time::Duration::from_secs(1_800));
        if proxy.attestation().await?.is_none() {
            return Err(io::Error::other(
                "tinycomputer is not attested; build it with scripts/build-module",
            )
            .into());
        }
        guard.disarm();
        Ok(Self {
            proxy,
            broker: broker_task,
            _client: client,
        })
    }

    /// Calls `member` with one positional argument and returns the envelope.
    ///
    /// # Errors
    ///
    /// Fails only on a transport error; a failed command is an `ok: false`
    /// envelope.
    pub async fn call(&self, member: &str, argument: Value) -> Result<DesktopResponse, LabError> {
        let arguments = if argument.is_null() {
            json!([])
        } else {
            json!([argument])
        };
        Ok(self.proxy.call(member, arguments).await?)
    }

    /// Runs a flow.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error envelope.
    pub async fn run_flow(&self, request: &RunFlowRequest) -> Result<FlowRunResult, LabError> {
        let reply: DesktopResponse = self
            .proxy
            .call_confidential(names::methods::RUN_FLOW, (request,))
            .await?;
        data(reply)
    }

    /// Runs a goal with the single-goal Jev loop, for comparison.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error envelope.
    pub async fn run_goal(&self, request: &RunGoalRequest) -> Result<JevRunResult, LabError> {
        let reply: DesktopResponse = self
            .proxy
            .call_confidential(names::methods::RUN_GOAL, (request,))
            .await?;
        data(reply)
    }

    /// Validates a candidate flow.
    ///
    /// # Errors
    ///
    /// Fails on a transport error.
    pub async fn validate(&self, flow: &Value) -> Result<FlowValidation, LabError> {
        let reply = self
            .call(
                names::methods::VALIDATE_FLOW,
                serde_json::to_value(ValidateFlowRequest { flow: flow.clone() })?,
            )
            .await?;
        data(reply)
    }

    /// The flow authoring guide.
    ///
    /// # Errors
    ///
    /// Fails on a transport error.
    pub async fn guide(&self) -> Result<String, LabError> {
        let reply = self.call(names::methods::FLOW_GUIDE, Value::Null).await?;
        let value: Value = data(reply)?;
        Ok(value
            .get("guide")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// `Describe`: what the module offers, and how to call it.
    ///
    /// # Errors
    ///
    /// Fails on a transport error.
    pub async fn describe(&self) -> Result<Capabilities, LabError> {
        Ok(self.proxy.call(names::methods::DESCRIBE, ()).await?)
    }

    /// `PlanTask`: a flow for a plain-language task, from the planner.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error reply.
    pub async fn plan_task(&self, request: &PlanTaskRequest) -> Result<TaskPlan, LabError> {
        agent(
            self.proxy
                .call(names::methods::PLAN_TASK, (request,))
                .await?,
        )
    }

    /// `StartTask`, delivered confidentially: it carries the facts.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error reply.
    pub async fn start_task(&self, request: &StartTaskRequest) -> Result<TaskView, LabError> {
        agent(
            self.proxy
                .call_confidential(names::methods::START_TASK, (request,))
                .await?,
        )
    }

    /// `AwaitTask`: the task's view once it changes, or after `timeout_ms`.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error reply.
    pub async fn await_task(&self, id: &TaskId, timeout_ms: u64) -> Result<TaskView, LabError> {
        let request = AwaitTaskRequest {
            id: id.clone(),
            timeout_ms,
        };
        agent(
            self.proxy
                .call(names::methods::AWAIT_TASK, (request,))
                .await?,
        )
    }

    /// `ContinueTask`, delivered confidentially: inputs are facts.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error reply.
    pub async fn continue_task(&self, request: &ContinueTaskRequest) -> Result<TaskView, LabError> {
        agent(
            self.proxy
                .call_confidential(names::methods::CONTINUE_TASK, (request,))
                .await?,
        )
    }

    /// `CancelTask`.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error reply.
    pub async fn cancel_task(&self, id: &TaskId) -> Result<TaskView, LabError> {
        let request = TaskRef { id: id.clone() };
        agent(
            self.proxy
                .call(names::methods::CANCEL_TASK, (request,))
                .await?,
        )
    }

    /// `TaskReport`, delivered confidentially: it carries page data.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error reply.
    pub async fn task_report(&self, id: &TaskId) -> Result<TaskReport, LabError> {
        let request = TaskReportRequest::new(id.clone());
        agent(
            self.proxy
                .call_confidential(names::methods::TASK_REPORT, (request,))
                .await?,
        )
    }

    /// `BrowserListSessions`: every browser session the module holds, a
    /// running task's included.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error envelope.
    pub async fn browser_sessions(&self) -> Result<Vec<SessionInfo>, LabError> {
        data(
            self.proxy
                .call(tinycomputer_bus::browser::names::methods::LIST_SESSIONS, ())
                .await?,
        )
    }

    /// `BrowserCloseSession`.
    ///
    /// # Errors
    ///
    /// Fails on a transport error or an error envelope.
    pub async fn close_browser_session(&self, session: &SessionId) -> Result<(), LabError> {
        let request = SessionRef {
            session: session.clone(),
        };
        let _closed: Value = data(
            self.proxy
                .call(
                    tinycomputer_bus::browser::names::methods::CLOSE_SESSION,
                    (request,),
                )
                .await?,
        )?;
        Ok(())
    }

    /// A screenshot of `session`'s page as image bytes: `BrowserScreenshot`,
    /// then [`Host::read_output`].
    ///
    /// # Errors
    ///
    /// Fails on a transport error, an error envelope, or a bad chunk.
    pub async fn browser_screenshot(&self, session: &SessionId) -> Result<Vec<u8>, LabError> {
        use tinycomputer_bus::browser::names::methods;
        let request = SessionRequest::new(session.clone(), ScreenshotRequest::default());
        let output: OutputRef = data(self.proxy.call(methods::SCREENSHOT, (request,)).await?)?;
        self.read_output(&output).await
    }

    /// A held output's bytes — a screenshot this host took, or one a task
    /// report names: `BrowserReadOutput` (a confidential call) chunk by chunk
    /// until the end, then `BrowserReleaseOutput`, the handle protocol a host
    /// follows. After a transport failure the output is kept, so a retry can
    /// still read it before it expires; after any other failure it is
    /// released.
    ///
    /// # Errors
    ///
    /// Fails on a transport error, an error envelope (an expired output is
    /// `OUTPUT_NOT_FOUND`), a chunk that is not base64, or an image whose
    /// length disagrees with its handle.
    pub async fn read_output(&self, output: &OutputRef) -> Result<Vec<u8>, LabError> {
        let read = self.read_chunks(output).await;
        // A transport failure may pass, and a task's last screenshot cannot
        // be taken again once its session is gone, so the output is kept for
        // a retry until it expires. Anything else — read in full, or data
        // that is wrong and will stay wrong — is released now.
        if matches!(read, Err(Unread::Transport(_))) {
            return read.map_err(Unread::into_error);
        }
        let request = OutputRequest {
            output: output.id.clone(),
        };
        let released = self
            .proxy
            .call::<DesktopResponse>(
                tinycomputer_bus::browser::names::methods::RELEASE_OUTPUT,
                (request,),
            )
            .await;
        let bytes = read.map_err(Unread::into_error)?;
        let _released: Value = data(released?)?;
        Ok(bytes)
    }

    async fn read_chunks(&self, output: &OutputRef) -> Result<Vec<u8>, Unread> {
        use tinycomputer_bus::browser::names::methods;
        let mut bytes = Vec::new();
        loop {
            let request = ReadOutputRequest {
                offset: bytes.len() as u64,
                ..ReadOutputRequest::new(output.id.clone())
            };
            let reply = self
                .proxy
                .call_confidential(methods::READ_OUTPUT, (request,))
                .await
                .map_err(|error| Unread::Transport(error.into()))?;
            let chunk: OutputChunk = data(reply).map_err(Unread::Invalid)?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&chunk.data)
                .map_err(|error| Unread::Invalid(error.into()))?;
            bytes.extend(decoded);
            if chunk.eof {
                break;
            }
        }
        if bytes.len() as u64 != output.total_bytes {
            return Err(Unread::Invalid(
                io::Error::other("the screenshot came back short").into(),
            ));
        }
        Ok(bytes)
    }

    /// Stops the in-memory broker.
    pub fn shutdown(self) {
        self.broker.abort();
    }
}

/// The payload of a successful envelope, or its error as a lab error.
///
/// # Errors
///
/// Fails when the envelope is an error or its data does not decode as `T`.
pub fn data<T: DeserializeOwned>(reply: DesktopResponse) -> Result<T, LabError> {
    if let Some(error) = reply.error {
        return Err(io::Error::other(format!(
            "{} failed: {}: {}",
            reply.command, error.code, error.message
        ))
        .into());
    }
    Ok(serde_json::from_value(reply.data.unwrap_or(Value::Null))?)
}

/// The value of a successful task reply, or its error as a lab error.
///
/// # Errors
///
/// Fails when the reply is an error.
pub fn agent<T>(reply: AgentResponse<T>) -> Result<T, LabError> {
    match (reply.data, reply.error) {
        (Some(data), _) => Ok(data),
        (None, Some(error)) => Err(io::Error::other(format!(
            "{}: {} ({})",
            error.code, error.message, error.hint
        ))
        .into()),
        (None, None) => Err(io::Error::other("the reply carried neither data nor an error").into()),
    }
}

async fn wait_for_module(client: &Connection) -> Result<(), LabError> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if client
                .list_names()
                .await?
                .iter()
                .any(|name| name.as_str() == names::INTERFACE)
            {
                return Ok::<(), tinybus::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| io::Error::other("timed out waiting for tinycomputer"))??;
    Ok(())
}

/// Waits until the loader reports the module ready for calls. The module
/// claims its bus name while its setup is still running, and the loader
/// refuses a reconfiguration until that setup has returned.
async fn wait_until_ready(module_host: &ModuleHost) -> Result<(), LabError> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let state = module_host
                .list()
                .into_iter()
                .find(|info| info.name == "tinycomputer")
                .map(|info| info.state);
            match state {
                Some(ModuleState::Ready | ModuleState::Serving) => return Ok(()),
                None
                | Some(
                    ModuleState::Discovered | ModuleState::Resolved | ModuleState::Initializing,
                ) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
                Some(
                    state @ (ModuleState::Rejected { .. }
                    | ModuleState::Unresolved { .. }
                    | ModuleState::Faulted { .. }
                    | ModuleState::Failed { .. }
                    | ModuleState::Stopped
                    | ModuleState::Disabled),
                ) => {
                    return Err(io::Error::other(format!(
                        "tinycomputer did not start: {state:?}"
                    )));
                }
            }
        }
    })
    .await
    .map_err(|_| io::Error::other("timed out waiting for tinycomputer to finish starting"))??;
    Ok(())
}

fn verify_allowlisted(module: &Path) -> Result<(), LabError> {
    let file_name = module
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("module path has no UTF-8 file name"))?;
    let manifest_path = module
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("modules.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|_| io::Error::other("modules.toml is required beside the module"))?;
    let prefix = format!("\"{file_name}\" = \"");
    let expected = manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| io::Error::other("module is absent from modules.toml"))?;
    let observed = tinybus::module::sha256_file(module)?;
    if observed != expected {
        return Err(io::Error::other(
            "module checksum does not match modules.toml; rebuild with scripts/lab",
        )
        .into());
    }
    Ok(())
}

/// The module path from `TINYCOMPUTER_MODULE`, or `scripts/build-module`'s
/// default output.
#[must_use]
pub fn module_path() -> PathBuf {
    std::env::var_os("TINYCOMPUTER_MODULE").map_or_else(
        || {
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map_or_else(|| PathBuf::from("target"), PathBuf::from);
            target.join("lab").join(library_name())
        },
        PathBuf::from,
    )
}

fn library_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libtinycomputer.dylib"
    } else if cfg!(target_os = "windows") {
        "tinycomputer.dll"
    } else {
        "libtinycomputer.so"
    }
}
