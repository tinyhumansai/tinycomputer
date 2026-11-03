//! Runs a browser task directly through `tinycomputer-engine`, without loading
//! the module or using `TinyBus`.
//!
//! Start the local travel fixture, then run this binary from the workspace:
//!
//! ```sh
//! python3 -m http.server 8765 --directory crates/tinycomputer-examples/fixtures/travel
//! OPENROUTER_API_KEY=... cargo run -p tinycomputer-examples --bin task_in_process
//! ```
//!
//! `TINYCOMPUTER_FIXTURE_URL` changes the fixture address, and
//! `TINYCOMPUTER_BROWSER_EXECUTABLE` selects a Chrome or Chromium binary.

use std::sync::Arc;
use tinycomputer_browser::{AgentBrowser, Browser};
use tinycomputer_bus::agent::{
    AwaitTaskRequest, StartTaskRequest, SurfaceKind, TaskConstraints, TaskStatus,
};
use tinycomputer_bus::agentic::{JevConfig, JevProvider};
use tinycomputer_engine::{BrowserRunner, JevRuntime, Tasks};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::var("TINYCOMPUTER_FIXTURE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8765".to_owned());
    let mut jev = JevConfig::new(std::env::var("OPENROUTER_API_KEY")?);
    jev.provider = JevProvider::OpenRouter;
    let runtime = JevRuntime::configure(&jev)
        .map_err(|error| std::io::Error::other(error.message.clone()))?;
    let options = tinycomputer_browser::SessionOptions {
        executable: std::env::var("TINYCOMPUTER_BROWSER_EXECUTABLE").ok(),
        ..tinycomputer_browser::SessionOptions::default()
    };
    let runner = Arc::new(
        BrowserRunner::new(Arc::new(Browser::new(Arc::new(AgentBrowser))), runtime)
            .with_session_options(options),
    );
    let tasks = Tasks::new(runner);
    let started = tasks.start(&StartTaskRequest {
        task: Some("Read the heading on the local fixture page.".to_owned()),
        flow: Some(serde_json::from_value(serde_json::json!({
            "app": "browser",
            "steps": [
                {"browse": format!("{base}/index.html")},
                "read the page heading"
            ]
        }))?),
        constraints: TaskConstraints {
            surfaces: vec![SurfaceKind::Browser],
            origins: vec![base],
            ..TaskConstraints::default()
        },
        ..StartTaskRequest::default()
    });
    let mut view = started.data.ok_or_else(|| {
        started
            .error
            .map_or_else(|| "task did not start".to_owned(), |error| error.message)
    })?;

    while !view.status.is_final() {
        let reply = tasks
            .await_task(AwaitTaskRequest {
                id: view.id.clone(),
                timeout_ms: 1_000,
            })
            .await;
        view = reply.data.ok_or_else(|| {
            reply
                .error
                .map_or_else(|| "task wait failed".to_owned(), |error| error.message)
        })?;
    }
    if matches!(view.status, TaskStatus::Done { .. }) {
        let report = tasks.report(&view.id);
        let report = report.data.ok_or_else(|| {
            report
                .error
                .map_or_else(|| "task report failed".to_owned(), |error| error.message)
        })?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    } else {
        Err(format!("task ended as {:?}", view.status).into())
    }
}
