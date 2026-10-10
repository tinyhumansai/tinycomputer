//! Building the service from the module configuration, running its work off
//! the dispatch task, and reading the desktop's availability.

use std::sync::Arc;

use tinybus::{Error as TinyBusError, Result as TinyBusResult};
use tinycomputer_browser::{AgentBrowser, Browser};
use tinycomputer_bus::agent::{SurfaceAvailability, SurfaceKind};
use tinycomputer_bus::{DesktopResponse, JevConfig};
use tinycomputer_engine as agentic;

use super::DesktopService;
use crate::tinybus_module::config::{BrowserDefaults, cursor_config};
use crate::tinybus_module::runner::WorkspaceRunner;
use crate::{Desktop, Result};

impl DesktopService {
    /// Builds the service from the module configuration blob.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Desktop::from_config`] rejects.
    pub(crate) fn from_config(config: &serde_json::Value) -> Result<Self> {
        Self::with_browser(config, Arc::new(Browser::new(Arc::new(AgentBrowser))))
    }

    /// Builds the service from the module configuration blob, opening browser
    /// sessions on `browser`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Desktop::from_config`] rejects.
    pub(crate) fn with_browser(config: &serde_json::Value, browser: Arc<Browser>) -> Result<Self> {
        let cursor = Arc::new(cursor_config(config)?);
        let desktop = Desktop::from_config(config)?.with_cursor(cursor.clone());
        let jev = config
            .as_object()
            .and_then(|object| object.get("jev"))
            .map(|value| {
                let config: JevConfig = serde_json::from_value(value.clone()).map_err(|_| {
                    crate::Error::ConfigFieldType {
                        field: "jev",
                        expected: "a valid Jev configuration object",
                    }
                })?;
                agentic::JevRuntime::configure(&config).map_err(|_| crate::Error::ConfigFieldType {
                    field: "jev",
                    expected: "a valid Jev configuration object",
                })
            })
            .transpose()?;
        // The planner's configuration also brings the rescuer, on the same
        // route unless it names its own `rescue_route`: a failed step is
        // handed to it before a task fails.
        let planner = config
            .as_object()
            .and_then(|object| object.get("planner"))
            .map(|value| {
                let invalid = || crate::Error::ConfigFieldType {
                    field: "planner",
                    expected: "a planner configuration object with an api_key and an approved route",
                };
                let config: agentic::PlannerConfig =
                    serde_json::from_value(value.clone()).map_err(|_| invalid())?;
                let planner = agentic::open_router(&config).map_err(|_| invalid())?;
                let rescuer = agentic::open_router_rescuer(&config).map_err(|_| invalid())?;
                let shaper = agentic::open_router_shaper(&config).map_err(|_| invalid())?;
                Ok::<_, crate::Error>((planner, rescuer, shaper))
            })
            .transpose()?;
        let browser_defaults = BrowserDefaults::from_config(config)?;
        let mut runner = WorkspaceRunner::new(desktop.clone(), jev.clone(), browser.clone());
        runner.defaults.clone_from(&browser_defaults);
        runner.cursor = cursor;
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
