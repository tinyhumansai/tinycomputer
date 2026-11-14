//! Building the service from the module configuration, running its work off
//! the dispatch task, and reading the desktop's availability.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use tinybus::{Error as TinyBusError, Result as TinyBusResult};
use tinycomputer_browser::{AgentBrowser, Browser, EnforcedAgentBrowser};
use tinycomputer_bus::agent::{
    AgentResponse, ContinueTaskRequest, StartTaskRequest, SurfaceAvailability, SurfaceKind,
    TaskStatus, TaskView,
};
use tinycomputer_bus::config::{ComputerConfig, PlannerConfig};
use tinycomputer_bus::{DesktopResponse, JevConfig};
use tinycomputer_engine as agentic;

use super::DesktopService;
use crate::tinybus_module::browser_egress::BusAuthorizer;
use crate::tinybus_module::config::{BrowserDefaults, cursor_config, trace_sink_destination};
use crate::tinybus_module::consequence_gate::{ConsequenceLink, ConsequencePort};
use crate::tinybus_module::runner::WorkspaceRunner;
use crate::tinybus_module::site_memory::{SiteLink, SiteMemoryPort, refused_flow};
use crate::tinybus_module::trace_sink::TraceLink;
use crate::{Desktop, Result};

/// Decodes the host-owned fields from the public config while leaving the
/// module's existing desktop and cursor extensions in their original object.
fn computer_config(config: &serde_json::Value) -> Result<ComputerConfig> {
    if !config.is_null() && config.as_object().is_none() {
        return Err(crate::Error::ConfigNotAnObject);
    }
    let mut browser_config = config.clone();
    if let Some(browser) = browser_config
        .as_object_mut()
        .and_then(|object| object.get_mut("browser"))
        .and_then(serde_json::Value::as_object_mut)
    {
        browser.remove("egress");
    }
    Ok(ComputerConfig {
        jev: config_field(config, "jev", "a valid Jev configuration object")?,
        planner: config_field(
            config,
            "planner",
            "a planner configuration object with an api_key and an approved route",
        )?,
        browser: config_field(
            &browser_config,
            "browser",
            "an object with optional executable and browser defaults",
        )?,
        trace_path: config_field(config, "trace_path", "a string trace path")?,
    })
}

fn config_field<T: DeserializeOwned>(
    config: &serde_json::Value,
    field: &'static str,
    expected: &'static str,
) -> Result<Option<T>> {
    let Some(value) = config.as_object().and_then(|object| object.get(field)) else {
        return Ok(None);
    };
    if field == "trace_path" && value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| crate::Error::ConfigFieldType { field, expected })
}

/// Keeps module-owned desktop options outside the shared host contract.
/// `session_id`, `trace_strict`, and `headed` belong to `Desktop`; `cursor` is
/// decoded separately by the existing cursor adapter.
fn desktop_config(raw: &serde_json::Value, computer: &ComputerConfig) -> serde_json::Value {
    let mut desktop = serde_json::Map::new();
    if let Some(options) = raw.as_object() {
        for key in ["session_id", "trace_strict", "headed"] {
            if let Some(value) = options.get(key) {
                desktop.insert(key.to_owned(), value.clone());
            }
        }
    }
    if let Some(trace_path) = &computer.trace_path {
        desktop.insert(
            "trace_path".into(),
            serde_json::Value::String(trace_path.clone()),
        );
    }
    serde_json::Value::Object(desktop)
}

impl DesktopService {
    /// Builds the service from the module configuration blob.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Desktop::from_config`] rejects.
    #[cfg(test)]
    pub(crate) fn from_config(config: &serde_json::Value) -> Result<Self> {
        if super::super::config::BrowserDefaults::egress_from_raw(config)?.is_some() {
            return Err(crate::Error::ConfigFieldType {
                field: "browser",
                expected: "a module set up with its authenticated host callback connection",
            });
        }
        Self::with_browser(config, Arc::new(Browser::new(Arc::new(AgentBrowser))))
    }

    /// Builds the service with authenticated host callbacks available on the
    /// module connection. A configured egress scope selects the enforced
    /// browser launcher, and a configured trace sink receives desktop traces.
    pub(crate) fn from_config_with_connection(
        config: &serde_json::Value,
        connection: tinybus::Connection,
    ) -> TinyBusResult<Self> {
        let egress = BrowserDefaults::egress_from_raw(config)
            .map_err(|error| TinyBusError::failed(error.to_string()))?;
        let browser = if let Some(egress) = &egress {
            let executable = computer_config(config)
                .map_err(|error| TinyBusError::failed(error.to_string()))?
                .browser
                .and_then(|browser| browser.executable);
            Arc::new(Browser::new(Arc::new(
                EnforcedAgentBrowser::new(Arc::new(BusAuthorizer::new(
                    connection.clone(),
                    egress.policy_id.clone(),
                    egress.generation,
                )))
                .with_executable(executable),
            )))
        } else {
            Arc::new(Browser::new(Arc::new(AgentBrowser)))
        };
        let trace_link = if let Some(destination) = trace_sink_destination(config)
            .map_err(|error| TinyBusError::failed(error.to_string()))?
        {
            Some(TraceLink::new(connection, destination)?)
        } else {
            None
        };
        Self::with_browser_and_defaults(config, browser, egress, trace_link)
            .map_err(|error| TinyBusError::failed(error.to_string()))
    }

    /// Builds the service from the module configuration blob, opening browser
    /// sessions on `browser`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Desktop::from_config`] rejects.
    #[cfg(test)]
    pub(crate) fn with_browser(config: &serde_json::Value, browser: Arc<Browser>) -> Result<Self> {
        let egress = BrowserDefaults::egress_from_raw(config)?;
        if egress.is_some() {
            return Err(crate::Error::ConfigFieldType {
                field: "browser",
                expected: "a module set up with its authenticated host callback connection",
            });
        }
        if trace_sink_destination(config)?.is_some() {
            return Err(crate::Error::ConfigFieldType {
                field: "trace_sink",
                expected: "a module set up with its host callback connection",
            });
        }
        if super::super::config::consequence_host_destination(config)?.is_some() {
            return Err(crate::Error::ConfigFieldType {
                field: "consequence_host",
                expected: "a module set up with its confidential host callback connection",
            });
        }
        Self::with_browser_and_defaults(config, browser, egress, None)
    }

    fn with_browser_and_defaults(
        config: &serde_json::Value,
        browser: Arc<Browser>,
        egress: Option<crate::tinybus_module::config::BrowserEgressConfig>,
        trace_link: Option<Arc<TraceLink>>,
    ) -> Result<Self> {
        let cursor = Arc::new(cursor_config(config)?);
        let computer = computer_config(config)?;
        let mut desktop =
            Desktop::from_config(&desktop_config(config, &computer))?.with_cursor(cursor.clone());
        if let Some(link) = &trace_link {
            desktop = desktop.with_trace_sink(link.clone());
        }
        let jev = computer
            .jev
            .as_ref()
            .map(|config: &JevConfig| {
                agentic::JevRuntime::configure(config).map_err(|_| crate::Error::ConfigFieldType {
                    field: "jev",
                    expected: "a valid Jev configuration object",
                })
            })
            .transpose()?;
        // The planner's configuration also brings the rescuer, on the same
        // route unless it names its own `rescue_route`: a failed step is
        // handed to it before a task fails.
        let planner = computer
            .planner
            .as_ref()
            .map(|config: &PlannerConfig| {
                let invalid = || crate::Error::ConfigFieldType {
                    field: "planner",
                    expected: "a planner configuration object with an api_key and an approved route",
                };
                let planner = agentic::open_router(config).map_err(|_| invalid())?;
                let rescuer = agentic::open_router_rescuer(config).map_err(|_| invalid())?;
                let shaper = agentic::open_router_shaper(config).map_err(|_| invalid())?;
                Ok::<_, crate::Error>((planner, rescuer, shaper))
            })
            .transpose()?;
        let captcha = computer
            .captcha
            .as_ref()
            .map(captcha_solver)
            .transpose()?;
        let browser_defaults = BrowserDefaults::from_config(computer.browser.as_ref(), egress);
        let mut runner = WorkspaceRunner::new(desktop.clone(), jev.clone(), browser.clone());
        runner.defaults.clone_from(&browser_defaults);
        runner.cursor = cursor;
        runner.captcha = captcha;
        let mut tasks = agentic::Tasks::new(Arc::new(runner));
        if let Some((planner, rescuer, shaper)) = planner {
            tasks = tasks
                .with_planner(planner)
                .with_rescuer(rescuer)
                .with_shaper(shaper);
        }
        let tasks = Arc::new(tasks);
        Ok(Self {
            desktop,
            jev,
            tasks,
            browser,
            browser_defaults,
            accessibility: Arc::new(super::accessibility::Access::default()),
            site_link: None,
            consequence_link: None,
            trace_link,
        })
    }

    /// Runs one engine command on a blocking thread.
    ///
    /// A panic inside a command surfaces as a `TinyBus` error rather than
    /// taking the runtime down: one malformed request must not cost every other
    /// caller their connection.
    pub(super) async fn run<F>(&self, command: F) -> TinyBusResult<DesktopResponse>
    where
        F: FnOnce(Desktop) -> DesktopResponse + Send + 'static,
    {
        let desktop = self.desktop.clone();

        tokio::task::spawn_blocking(move || command(desktop))
            .await
            .map_err(|error| TinyBusError::failed(format!("desktop command failed: {error}")))
    }

    /// Runs a task-store call on a blocking thread, as every member runs its
    /// work off the dispatch task: the store takes locks, and starting a task
    /// spawns its worker from there.
    pub(super) async fn on_tasks<T, F>(&self, call: F) -> TinyBusResult<T>
    where
        F: FnOnce(&agentic::Tasks) -> T + Send + 'static,
        T: Send + 'static,
    {
        let tasks = self.tasks.clone();
        tokio::task::spawn_blocking(move || call(&tasks))
            .await
            .map_err(|error| TinyBusError::failed(format!("task call failed: {error}")))
    }

    pub(in crate::tinybus_module) fn jev_runtime(&self) -> Option<agentic::JevRuntime> {
        self.jev.clone()
    }
}

/// The captcha solver's own Jev route and settings, from `config`: the
/// route's model defaults to one that reads images.
///
/// # Errors
///
/// [`crate::Error::ConfigFieldType`] when the route cannot form a client.
fn captcha_solver(
    config: &tinycomputer_bus::config::CaptchaConfig,
) -> Result<(agentic::JevRuntime, tinycomputer_engine::CaptchaSettings)> {
    let mut route = config.jev.clone();
    if route.model.as_deref().is_none_or(|model| model.trim().is_empty()) {
        route.model = Some(tinycomputer_bus::config::DEFAULT_CAPTCHA_MODEL.to_owned());
    }
    let runtime =
        agentic::JevRuntime::configure(&route).map_err(|_| crate::Error::ConfigFieldType {
            field: "captcha",
            expected: "a captcha configuration with a valid Jev route",
        })?;
    let settings = config.max_attempts.map_or_else(
        tinycomputer_engine::CaptchaSettings::default,
        tinycomputer_engine::CaptchaSettings::with_attempts,
    );
    Ok((runtime, settings))
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

/// Whether the desktop surface is usable, from a `Permissions` reply: the
/// accessibility permission must be granted (or not needed on this platform).
impl DesktopService {
    /// Starts dropping expired held outputs every [`SWEEP_INTERVAL`], so a
    /// screenshot a caller never reads or releases is freed after its time
    /// to live even when no further output call arrives to expire it. The
    /// sweep ends once the service's browser is gone. Needs a Tokio runtime,
    /// which `setup` runs on.
    ///
    /// [`SWEEP_INTERVAL`]: tinycomputer_browser::SWEEP_INTERVAL
    pub(crate) fn sweep_outputs(&self) {
        tokio::spawn(sweep_every(
            Arc::downgrade(&self.browser),
            tinycomputer_browser::SWEEP_INTERVAL,
        ));
    }
}

/// Sweeps `browser`'s held outputs every `every`, until it is dropped.
pub(in crate::tinybus_module) async fn sweep_every(
    browser: std::sync::Weak<Browser>,
    every: std::time::Duration,
) {
    let mut ticks = tokio::time::interval(every);
    loop {
        ticks.tick().await;
        let Some(browser) = browser.upgrade() else {
            return;
        };
        // A poisoned output store is reported on the next output call;
        // the sweep has no caller to report it to.
        let _swept = browser.sweep_outputs();
    }
}

pub(in crate::tinybus_module) fn desktop_availability(
    permissions: &DesktopResponse,
) -> SurfaceAvailability {
    let accessibility = permissions
        .data
        .as_ref()
        .filter(|_| permissions.ok)
        .and_then(|data| data.get("accessibility"));
    let state = accessibility
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str);
    let reason = match state {
        Some("granted" | "not_required") => None,
        Some("denied") => Some(
            accessibility
                .and_then(|value| value.get("suggestion"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("grant the accessibility permission")
                .to_owned(),
        ),
        _ => Some(permissions.error.as_ref().map_or_else(
            || "the accessibility permission could not be read".to_owned(),
            |error| error.message.clone(),
        )),
    };
    SurfaceAvailability {
        kind: SurfaceKind::Desktop,
        available: reason.is_none(),
        reason,
    }
}

#[cfg(test)]
impl DesktopService {
    /// Inject platform fixtures for unit tests, keeping real devices untouched.
    pub(crate) fn with_native_fixture(mut self) -> Self {
        self.accessibility = super::accessibility::fixture();
        self
    }
}

impl DesktopService {
    /// Keeps what browser tasks learn about a site through `port`, the host's
    /// `SiteMemoryHost`.
    pub(crate) fn with_site_memory(mut self, port: Box<dyn SiteMemoryPort>) -> Self {
        self.site_link = Some(Arc::new(SiteLink::new(port)));
        self
    }

    /// Has the host rule on actions its tasks are approved to take, through
    /// `port`, the host's `ConsequenceHost`.
    pub(crate) fn with_consequence_review(mut self, port: Box<dyn ConsequencePort>) -> Self {
        self.consequence_link = Some(Arc::new(ConsequenceLink::new(port)));
        self
    }

    /// Before an approval reaches the task, asks the host whether the action
    /// it is paused on may go ahead. A denial turns the approval into a
    /// decline, so the task never takes the action; an approval of nothing
    /// pending, a decline, or a host that does not review actions passes
    /// through unchanged.
    pub(super) async fn review_approval(
        &self,
        request: &mut ContinueTaskRequest,
    ) -> TinyBusResult<()> {
        let Some(link) = self.consequence_link.clone() else {
            return Ok(());
        };
        if request.approve != Some(true) {
            return Ok(());
        }
        let id = request.id.clone();
        let listed = self.on_tasks(agentic::Tasks::list).await?;
        let paused = listed
            .data
            .unwrap_or_default()
            .into_iter()
            .find(|view| view.id == id)
            .and_then(|view| match view.status {
                TaskStatus::NeedsApproval { action, target, .. } => Some((action, target)),
                _ => None,
            });
        link.screen(request, paused).await;
        Ok(())
    }

    /// Starts a task with what earlier tasks on its `site` left, and follows
    /// it for what it may teach. A saved plan the engine refuses is
    /// forgotten and the task started again to be planned afresh.
    pub(super) async fn start_remembering(
        &self,
        mut request: StartTaskRequest,
    ) -> TinyBusResult<AgentResponse<TaskView>> {
        let Some(link) = self.site_link.clone() else {
            return self.on_tasks(move |tasks| tasks.start(&request)).await;
        };
        let mut prepared = link.prepare(&mut request).await;
        let mut reply = self.start_once(request.clone()).await?;
        if refused_flow(&reply)
            && let Some(reusing) = prepared.as_mut().filter(|reusing| reusing.reuses_plan())
        {
            link.refused(reusing).await;
            request.flow = None;
            reply = self.start_once(request).await?;
        }
        if let (Some(prepared), Some(view)) = (prepared, reply.data.as_ref()) {
            link.follow(&view.id, prepared);
        }
        self.learn_from(&reply).await;
        Ok(reply)
    }

    async fn start_once(
        &self,
        request: StartTaskRequest,
    ) -> TinyBusResult<AgentResponse<TaskView>> {
        self.on_tasks(move |tasks| tasks.start(&request)).await
    }

    /// Learns from `reply`'s task once it has ended.
    pub(super) async fn learn_from(&self, reply: &AgentResponse<TaskView>) {
        let (Some(link), Some(view)) = (&self.site_link, reply.data.as_ref()) else {
            return;
        };
        let tasks = self.tasks.clone();
        link.learn(view, |id| tasks.report(id)).await;
    }
}
