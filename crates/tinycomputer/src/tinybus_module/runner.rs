//! The flow runner behind the Agent members: each task's flow runs on a
//! [`Workspace`] joining the desktop and a browser session of its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tinycomputer_browser::{
    Browser, BrowserSurface, ScreenCursor, ScreenshotRequest, SessionOptions,
};
use tinycomputer_bus::DesktopResponse;
use tinycomputer_bus::agent::{SurfaceKind, TaskConstraints, TaskId};
use tinycomputer_engine::{
    CaptureFuture, FlowFuture, FlowRunner, JevRuntime, PrepareFuture, TextFuture, Workspace,
};

use super::config::BrowserDefaults;
use crate::Desktop;

type TaskWorkspace = Workspace<Desktop, BrowserSurface>;

/// Runs task flows with the module's Jev runtime, on one workspace per task
/// so a continuation picks up where the last run left off.
#[derive(Debug)]
pub(super) struct WorkspaceRunner {
    pub(super) desktop: Desktop,
    pub(super) jev: Option<JevRuntime>,
    pub(super) browser: Arc<Browser>,
    /// How every task's browser launches, and how it reads a page.
    pub(super) defaults: BrowserDefaults,
    /// The screen's one agent cursor, which each task's browser shares with
    /// the desktop.
    pub(super) cursor: Arc<ScreenCursor>,
    pub(super) workspaces: Mutex<HashMap<TaskId, (TaskWorkspace, Option<BrowserSurface>)>>,
}

impl WorkspaceRunner {
    /// A runner with no task workspaces yet, opening each task's browser
    /// session on `browser` — the one the browser members serve, so a
    /// caller can inspect a task's session and read what it captured.
    pub(super) fn new(desktop: Desktop, jev: Option<JevRuntime>, browser: Arc<Browser>) -> Self {
        Self {
            desktop,
            jev,
            browser,
            defaults: BrowserDefaults::default(),
            cursor: Arc::new(ScreenCursor::off()),
            workspaces: Mutex::new(HashMap::new()),
        }
    }

    /// The task's workspace, created on first use: the desktop, and a
    /// browser session shaped by `constraints` unless they exclude the
    /// browser. Needs a Tokio runtime, which every caller runs on.
    fn workspace(&self, task: &TaskId, constraints: &TaskConstraints) -> TaskWorkspace {
        let fresh = || {
            let desktop = (constraints.surfaces.is_empty()
                || constraints.surfaces.contains(&SurfaceKind::Desktop))
            .then(|| self.desktop.clone());
            let browser = (constraints.surfaces.is_empty()
                || constraints.surfaces.contains(&SurfaceKind::Browser))
            .then(|| {
                BrowserSurface::new(
                    self.browser.clone(),
                    self.defaults.apply(SessionOptions {
                        endpoint: constraints.browser_endpoint.clone(),
                        executable: constraints.browser_executable.clone(),
                        headless: !constraints.headed,
                        user_data_dir: constraints.browser_profile.clone(),
                        allowed_origins: constraints.origins.clone(),
                        ..SessionOptions::default()
                    }),
                    tokio::runtime::Handle::current(),
                )
                .with_cursor(self.cursor.clone())
                .with_perception(self.defaults.perception)
                .with_settle(self.defaults.settle)
            });
            (Workspace::new(desktop, browser.clone()), browser)
        };
        self.workspaces.lock().map_or_else(
            |_| fresh().0,
            |mut workspaces| {
                workspaces
                    .entry(task.clone())
                    .or_insert_with(fresh)
                    .0
                    .clone()
            },
        )
    }
}

impl FlowRunner for WorkspaceRunner {
    fn run(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: tinycomputer_bus::RunFlowRequest,
    ) -> FlowFuture {
        let Some(runtime) = self.jev.as_ref() else {
            return Box::pin(async { jev_not_configured("run-flow") });
        };
        // Every run of one task — the first flow and each continuation —
        // journals to one file, so a task reads back as one story.
        let runtime = runtime.journaled_as(&format!("task-{task}"));
        Box::pin(tinycomputer_engine::run_flow(
            self.workspace(task, constraints),
            runtime,
            request,
        ))
    }

    fn visible_text(&self, task: &TaskId) -> TextFuture {
        let workspace = self.workspace(task, &TaskConstraints::default());
        Box::pin(async move {
            tokio::task::spawn_blocking(move || workspace.visible_text())
                .await
                .unwrap_or_default()
        })
    }

    fn capture(&self, task: &TaskId) -> CaptureFuture {
        let session = self
            .workspaces
            .lock()
            .ok()
            // Only when the browser is the side the task is on: after a
            // move to a desktop application its page is stale evidence.
            .and_then(|workspaces| {
                workspaces
                    .get(task)
                    .filter(|(workspace, _)| workspace.browser_active())
                    .and_then(|(_, browser)| browser.clone())
            })
            .and_then(|browser| browser.session());
        let browser = self.browser.clone();
        Box::pin(async move {
            browser
                .screenshot(&session?, ScreenshotRequest::default())
                .await
                .ok()
        })
    }

    fn prepare(&self, task: &TaskId, constraints: &TaskConstraints) -> PrepareFuture {
        if !self.defaults.prelaunch {
            return Box::pin(async {});
        }
        let _workspace = self.workspace(task, constraints);
        let browser = self.workspaces.lock().ok().and_then(|workspaces| {
            workspaces
                .get(task)
                .and_then(|(_, browser)| browser.clone())
        });
        Box::pin(async move {
            if let Some(browser) = browser {
                // Opening the session is what the first step would wait for;
                // one that fails here fails again, and is reported, there.
                let _opened = tokio::task::spawn_blocking(move || browser.open()).await;
            }
        })
    }

    fn open_page(&self, task: &TaskId, url: &str) -> PrepareFuture {
        // In the browser `prepare` opened, which it does unless prelaunch is
        // off.
        let browser = self
            .defaults
            .prelaunch
            .then(|| {
                self.workspaces.lock().ok().and_then(|workspaces| {
                    workspaces
                        .get(task)
                        .and_then(|(_, browser)| browser.clone())
                })
            })
            .flatten();
        // Journaled with the task's flows (see `run`): the plan's outcome
        // waits for this load.
        let journal = self
            .jev
            .as_ref()
            .filter(|runtime| runtime.journaling())
            .map(|runtime| runtime.journaled_as(&format!("task-{task}")));
        let url = url.to_owned();
        Box::pin(async move {
            if let Some(browser) = browser {
                let started = std::time::Instant::now();
                // A page that will not load fails again, and is reported,
                // at the step that browses there.
                let loaded = tokio::task::spawn_blocking(move || browser.open_at(&url))
                    .await
                    .unwrap_or(false);
                if let Some(journal) = journal {
                    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    journal.journal_event(
                        "open_page",
                        || serde_json::json!({"wall_ms": wall_ms, "loaded": loaded}),
                    );
                }
            }
        })
    }

    fn warm(&self, task: &TaskId, votes: u32) -> PrepareFuture {
        let Some(runtime) = self.jev.as_ref() else {
            return Box::pin(async {});
        };
        // Journaled with the task's flows (see `run`).
        let runtime = runtime.journaled_as(&format!("task-{task}"));
        Box::pin(async move { runtime.warm(votes).await })
    }

    fn journal(&self, task: Option<&TaskId>, event: &str, fields: &dyn Fn() -> serde_json::Value) {
        let Some(runtime) = self.jev.as_ref().filter(|runtime| runtime.journaling()) else {
            return;
        };
        match task {
            // Into the file the task's flows journal to (see `run`).
            Some(task) => runtime
                .journaled_as(&format!("task-{task}"))
                .journal_event(event, fields),
            None => runtime.journal_event(event, fields),
        }
    }

    fn release(&self, task: &TaskId) {
        let released = self
            .workspaces
            .lock()
            .ok()
            .and_then(|mut workspaces| workspaces.remove(task));
        if let Some((_, Some(browser))) = released {
            browser.close();
        }
    }
}

/// The reply an agentic member gives when no Jev runtime was configured.
pub(super) fn jev_not_configured(command: &str) -> DesktopResponse {
    DesktopResponse::err(
        command,
        tinycomputer_bus::DesktopError::new(
            "JEV_NOT_CONFIGURED",
            "Jev must be supplied through private module configuration",
        ),
    )
}
