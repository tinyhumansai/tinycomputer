//! Public task-manager behavior on a scripted browser-only runner.

use std::sync::Arc;

use serde_json::{Value, json};
use tinycomputer_browser::{
    ActionContext, ActionGate, ActionRequest, ActionVerdict, Browser, Engine, Launcher, Reply,
    SessionOptions,
};
use tinycomputer_bus::agent::{
    AwaitTaskRequest, PaymentMode, SecretHandle, StartTaskRequest, SurfaceKind, TaskConstraints,
    TaskStatus,
};
use tinycomputer_bus::{
    DesktopResponse, FlowRunResult, FlowStopReason, JevMetrics, RunFlowRequest, StepOutcome,
    StepReport,
};
use tinycomputer_bus::{Flow, FlowAction, FlowStep};

use super::{FlowFuture, FlowRunner};
use crate::{
    BrowserRunner, Completion, JevRuntime, LanguageModel, Rescuer, SecretHost, SecretTypeRequest,
    Tasks, Turn,
};

struct ScriptedBrowserRunner {
    failure: Option<&'static str>,
}

impl FlowRunner for ScriptedBrowserRunner {
    fn run(
        &self,
        _task: &tinycomputer_bus::agent::TaskId,
        constraints: &TaskConstraints,
        _request: RunFlowRequest,
    ) -> FlowFuture {
        assert_eq!(constraints.surfaces, [SurfaceKind::Browser]);
        let failure = self.failure;
        let result = FlowRunResult {
            stop: if failure.is_some() {
                FlowStopReason::StepFailed
            } else {
                FlowStopReason::Completed
            },
            steps: vec![StepReport {
                path: "1".to_owned(),
                kind: "browse".to_owned(),
                text: "open the local page".to_owned(),
                outcome: if failure.is_some() {
                    StepOutcome::Failed
                } else {
                    StepOutcome::Done
                },
                turns: 0,
                jev_calls: 0,
                actions: Vec::new(),
                loops: Vec::new(),
                confidence: None,
                note: failure.unwrap_or_default().to_owned(),
            }],
            vars: std::collections::BTreeMap::new(),
            pending: None,
            learned: Vec::new(),
            actions: 0,
            metrics: JevMetrics::default(),
            trace: Vec::new(),
            dialog_left_open: false,
        };
        let reply = serde_json::to_value(result).map_or_else(
            |error| {
                DesktopResponse::err(
                    "run-flow",
                    tinycomputer_bus::DesktopError::new("INTERNAL", error.to_string()),
                )
            },
            |value| DesktopResponse::ok("run-flow", value),
        );
        Box::pin(async move { reply })
    }
}

#[tokio::test]
async fn public_task_manager_runs_a_scripted_browser_flow_to_done_without_models()
-> Result<(), Box<dyn std::error::Error>> {
    let tasks = Tasks::new(Arc::new(ScriptedBrowserRunner { failure: None }));
    let started = tasks.start(&request());
    assert!(started.ok, "{:?}", started.error);
    let Some(started) = started.data else {
        return Err("the task start reply had no view".into());
    };

    let finished = settle(&tasks, &started.id).await?;
    assert!(matches!(finished.status, TaskStatus::Done { .. }));
    Ok(())
}

struct RefusalRecovery;

impl LanguageModel for RefusalRecovery {
    fn complete(&self, _turns: &[Turn]) -> Completion {
        Box::pin(async { Err("scripted recovery is unavailable".to_owned()) })
    }
}

#[tokio::test]
async fn a_browser_action_refusal_keeps_its_reason_in_rescue_and_task_report()
-> Result<(), Box<dyn std::error::Error>> {
    const REASON: &str = "the host refused this browser action";
    let tasks = Tasks::new(Arc::new(ScriptedBrowserRunner {
        failure: Some(REASON),
    }))
    .with_rescuer(Rescuer::new(Arc::new(RefusalRecovery)));
    let started = tasks.start(&request());
    assert!(started.ok, "{:?}", started.error);
    let Some(started) = started.data else {
        return Err("the task start reply had no view".into());
    };
    let finished = settle(&tasks, &started.id).await?;
    let TaskStatus::Failed { reason, .. } = &finished.status else {
        return Err(format!("unexpected refusal status: {:?}", finished.status).into());
    };
    assert!(reason.contains(REASON), "{reason}");
    let reported_reason = reason.clone();

    let report = tasks.report(&started.id);
    assert!(report.ok, "{:?}", report.error);
    let Some(report) = report.data else {
        return Err("the task report reply had no report".into());
    };
    assert_eq!(report.view.status, finished.status);
    assert_eq!(report.steps[0].note, reported_reason);
    assert_eq!(report.rescues[0].failure, reported_reason);
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub(super) struct BrowserScript {
    commands: Arc<std::sync::Mutex<Vec<Value>>>,
    current_url: Arc<std::sync::Mutex<String>>,
    echo_secret: bool,
}

impl BrowserScript {
    fn commands(&self) -> Vec<Value> {
        self.commands
            .lock()
            .map(|commands| commands.clone())
            .unwrap_or_default()
    }
}

pub(super) struct BrowserScriptEngine(pub(super) BrowserScript);

impl Engine for BrowserScriptEngine {
    fn execute(&mut self, command: Value) -> Reply<'_> {
        if let Ok(mut commands) = self.0.commands.lock() {
            commands.push(command.clone());
        }
        let action = command["action"].as_str().unwrap_or_default();
        let data = match action {
            "navigate" => {
                let url = command["url"].as_str().unwrap_or_default().to_owned();
                if let Ok(mut current_url) = self.0.current_url.lock() {
                    *current_url = url.clone();
                }
                json!({"url": url, "title": "Local fixture"})
            }
            "url" => json!({
                "url": self
                    .0
                    .current_url
                    .lock()
                    .map(|url| url.clone())
                    .unwrap_or_default()
            }),
            "title" => json!({"title": "Local fixture"}),
            "snapshot" => {
                let snapshot = if self.0.echo_secret {
                    "- heading \"Browser runner fixture\" [ref=h1]\n- textbox \"Card number\" [ref=card]: 4111 1111 1111 1111\n- textbox \"Email\" [ref=email]: unknown-autofill-canary\n- text: The saved card was 4111 1111 1111 1111"
                } else {
                    "- heading \"Browser runner fixture\" [ref=h1]"
                };
                json!({
                    "snapshot": snapshot,
                    "refs": {"h1": {"role": "heading", "name": "Browser runner fixture"}}
                })
            }
            "evaluate" => json!({"result": 42}),
            "screenshot" => {
                let Some(path) = command["path"].as_str() else {
                    return Box::pin(async {
                        json!({"id":"", "success":false, "error":"screenshot path missing"})
                    });
                };
                if let Err(error) = std::fs::write(path, png_header(1280, 800)) {
                    return Box::pin(async move {
                        json!({"id":"", "success":false, "error":error.to_string()})
                    });
                }
                json!({"path": path})
            }
            _ => json!({}),
        };
        Box::pin(async move { json!({"id":"", "success":true, "data":data}) })
    }
}

impl Launcher for BrowserScript {
    fn open(&self, _session: &str) -> Box<dyn Engine> {
        Box::new(BrowserScriptEngine(self.clone()))
    }
}

#[derive(Default)]
struct RecordingGate {
    contexts: std::sync::Mutex<Vec<ActionContext>>,
    refusal: Option<String>,
}

impl ActionGate for RecordingGate {
    fn check(&self, request: &ActionRequest) -> ActionVerdict {
        if let Ok(mut contexts) = self.contexts.lock() {
            contexts.push(request.context.clone());
        }
        if let Some(reason) = &self.refusal
            && request.destination_url.is_some()
        {
            return ActionVerdict::Refuse {
                reason: reason.clone(),
            };
        }
        ActionVerdict::Allow
    }
}

#[tokio::test]
async fn public_task_manager_runs_the_real_browser_runner_without_a_model()
-> Result<(), Box<dyn std::error::Error>> {
    let script = BrowserScript::default();
    let gate = Arc::new(RecordingGate::default());
    let browser = Arc::new(Browser::new(Arc::new(script.clone())).with_action_gate(gate.clone()));
    let runner = Arc::new(
        BrowserRunner::new(browser, runtime()?).with_session_options(SessionOptions {
            headless: true,
            ..SessionOptions::default()
        }),
    );
    let tasks = Tasks::new(runner);
    let started = tasks.start(&request());
    assert!(started.ok, "{:?}", started.error);
    let Some(started) = started.data else {
        return Err("the task start reply had no view".into());
    };

    let finished = settle(&tasks, &started.id).await?;
    assert!(matches!(finished.status, TaskStatus::Done { .. }));
    assert!(script.commands().iter().any(|command| {
        command["action"] == "navigate" && command["url"] == "http://127.0.0.1:8765/index.html"
    }));
    let contexts = gate
        .contexts
        .lock()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    assert!(contexts.iter().any(|context| {
        context.task_id.as_deref() == Some(started.id.0.as_str()) && !context.payment_in_play
    }));
    Ok(())
}

#[tokio::test]
async fn retained_browser_updates_payment_gate_context_without_reopening()
-> Result<(), Box<dyn std::error::Error>> {
    let script = BrowserScript::default();
    let gate = Arc::new(RecordingGate::default());
    let browser = Arc::new(Browser::new(Arc::new(script)).with_action_gate(gate.clone()));
    let runner = BrowserRunner::new(browser, runtime()?).with_session_options(SessionOptions {
        headless: true,
        ..SessionOptions::default()
    });
    let task = tinycomputer_bus::agent::TaskId::new("t-payment-context");
    let mut constraints = TaskConstraints {
        surfaces: vec![SurfaceKind::Browser],
        origins: vec!["http://127.0.0.1:8765".to_owned()],
        payment: PaymentMode::StopAtPayment,
        ..TaskConstraints::default()
    };
    runner.prepare(&task, &constraints).await;
    let session = runner
        .session_id(&task)
        .ok_or("prepared task session was not retained")?;
    let make_request = || RunFlowRequest {
        flow: Flow {
            app: "browser".to_owned(),
            vars: std::collections::BTreeMap::new(),
            steps: vec![FlowStep::Action(FlowAction::Browse(
                "http://127.0.0.1:8765/index.html".to_owned(),
            ))],
        },
        ..RunFlowRequest::default()
    };
    let first = runner.run(&task, &constraints, make_request()).await;
    assert!(first.ok, "{:?}", first.error);

    constraints.payment = PaymentMode::FillThenApprove;
    let second = runner.run(&task, &constraints, make_request()).await;
    assert!(second.ok, "{:?}", second.error);
    assert_eq!(runner.session_id(&task).as_ref(), Some(&session));
    let contexts = gate
        .contexts
        .lock()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    assert!(contexts.iter().any(|context| {
        context.task_id.as_deref() == Some(task.0.as_str())
            && context.session_id.as_deref() == Some(session.as_str())
            && !context.payment_in_play
    }));
    assert!(contexts.iter().any(|context| {
        context.task_id.as_deref() == Some(task.0.as_str())
            && context.session_id.as_deref() == Some(session.as_str())
            && context.payment_in_play
    }));
    runner.release(&task);
    Ok(())
}

#[tokio::test]
async fn browser_action_refusal_from_the_real_runner_survives_rescue_and_report()
-> Result<(), Box<dyn std::error::Error>> {
    const REASON: &str = "the host refused this browser action";
    let script = BrowserScript::default();
    let gate = Arc::new(RecordingGate {
        contexts: std::sync::Mutex::default(),
        refusal: Some(REASON.to_owned()),
    });
    let browser = Arc::new(Browser::new(Arc::new(script.clone())).with_action_gate(gate.clone()));
    let mut request = request();
    request.constraints.payment = PaymentMode::FillThenApprove;
    request.constraints.origins = vec!["http://127.0.0.1:8765".to_owned()];
    let tasks = Tasks::new(Arc::new(BrowserRunner::new(browser, runtime()?)))
        .with_rescuer(Rescuer::new(Arc::new(RefusalRecovery)));
    let started = tasks.start(&request);
    assert!(started.ok, "{:?}", started.error);
    let Some(started) = started.data else {
        return Err("the task start reply had no view".into());
    };

    let finished = settle(&tasks, &started.id).await?;
    let TaskStatus::Failed { reason, .. } = &finished.status else {
        return Err(format!("unexpected refusal status: {:?}", finished.status).into());
    };
    assert!(reason.contains(REASON), "{reason}");
    let reported_reason = reason.clone();
    assert!(!script.commands().iter().any(|command| {
        command["action"] == "navigate" && command["url"] == "http://127.0.0.1:8765/index.html"
    }));
    let contexts = gate
        .contexts
        .lock()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    assert!(contexts.iter().any(|context| {
        context.task_id.as_deref() == Some(started.id.0.as_str()) && context.payment_in_play
    }));
    let report = tasks.report(&started.id);
    assert!(report.ok, "{:?}", report.error);
    let Some(report) = report.data else {
        return Err("the task report reply had no report".into());
    };
    assert_eq!(report.view.status, finished.status);
    assert_eq!(report.steps[0].note, reported_reason);
    assert_eq!(report.rescues[0].failure, reported_reason);
    Ok(())
}

#[tokio::test]
async fn runner_hooks_share_the_task_surface_and_release_its_session()
-> Result<(), Box<dyn std::error::Error>> {
    let script = BrowserScript::default();
    let runner = BrowserRunner::new(Arc::new(Browser::new(Arc::new(script.clone()))), runtime()?);
    let task = tinycomputer_bus::agent::TaskId::new("t-hooks");
    let constraints = TaskConstraints::default();
    runner.prepare(&task, &constraints).await;
    assert!(runner.session_id(&task).is_some());
    runner
        .open_page(&task, "http://127.0.0.1:8765/index.html")
        .await;
    let visible = runner.visible_text(&task).await;
    assert!(
        visible
            .iter()
            .any(|line| line.contains("Browser runner fixture"))
    );
    assert!(runner.capture(&task).await.is_some());
    runner.warm(&task, 1).await;
    runner.journal(None, "test", &|| json!({"phase":"host"}));
    runner.journal(Some(&task), "test", &|| json!({"phase":"task"}));
    runner.release(&task);
    assert!(runner.session_id(&task).is_none());
    assert!(runner.capture(&task).await.is_none());
    runner
        .open_page(
            &tinycomputer_bus::agent::TaskId::new("t-unknown"),
            "https://unused.test",
        )
        .await;
    runner.release(&tinycomputer_bus::agent::TaskId::new("t-unknown"));
    Ok(())
}

#[tokio::test]
async fn handoff_origin_uses_the_live_page_url_and_keeps_its_port()
-> Result<(), Box<dyn std::error::Error>> {
    let script = BrowserScript::default();
    let runner = BrowserRunner::new(Arc::new(Browser::new(Arc::new(script.clone()))), runtime()?);
    let task = tinycomputer_bus::agent::TaskId::new("t-live-origin");
    runner.prepare(&task, &TaskConstraints::default()).await;
    runner
        .open_page(&task, "https://shop.example:8443/checkout")
        .await;
    // Model an asynchronous redirect after the preceding action's cached
    // SessionInfo was written.
    *script
        .current_url
        .lock()
        .map_err(|error| std::io::Error::other(error.to_string()))? =
        "https://checkout.example:9443/confirm".to_owned();

    let origin = runner.page_origin(&task).await;
    assert_eq!(origin.as_deref(), Some("https://checkout.example:9443/"));
    assert!(
        script
            .commands()
            .iter()
            .any(|command| command["action"] == "snapshot")
    );
    runner.release(&task);
    Ok(())
}

#[tokio::test]
async fn a_runner_with_a_secret_host_does_not_capture_unredacted_pixels()
-> Result<(), Box<dyn std::error::Error>> {
    struct Host;
    impl SecretHost for Host {
        fn type_secret(&self, _request: &SecretTypeRequest) -> Result<(), String> {
            Err("not used by this test".to_owned())
        }

        fn redact(&self, _task_id: &str, text: &str) -> String {
            text.to_owned()
        }
    }

    let script = BrowserScript::default();
    let runner = BrowserRunner::new(
        Arc::new(isolated_browser(script.clone(), "secret-capture")),
        runtime()?,
    )
    .with_secret_host(Arc::new(Host));
    let task = tinycomputer_bus::agent::TaskId::new("t-secret-capture");
    runner.prepare(&task, &TaskConstraints::default()).await;
    runner.open_page(&task, "https://shop.test/checkout").await;
    assert!(runner.capture(&task).await.is_none());

    let request = RunFlowRequest {
        fact_handles: std::collections::BTreeMap::from([(
            "card number".to_owned(),
            SecretHandle {
                handle: "opaque-card".to_owned(),
                mask: "•••• 1111".to_owned(),
                kind: "payment_card".to_owned(),
            },
        )]),
        ..RunFlowRequest::default()
    };
    let response = runner
        .run(&task, &TaskConstraints::default(), request)
        .await;
    assert!(!response.ok, "an empty flow is rejected before running");
    assert!(runner.capture(&task).await.is_none());
    assert!(
        !script
            .commands()
            .iter()
            .any(|command| command["action"] == "screenshot")
    );
    runner.release(&task);
    Ok(())
}

#[tokio::test]
async fn rescue_visible_text_is_redacted_by_the_host() -> Result<(), Box<dyn std::error::Error>> {
    struct Host;
    impl SecretHost for Host {
        fn type_secret(&self, _request: &SecretTypeRequest) -> Result<(), String> {
            Err("not used by this test".to_owned())
        }

        fn redact(&self, _task_id: &str, text: &str) -> String {
            text.replace("4111 1111 1111 1111", "•••• 1111")
        }
    }

    let script = BrowserScript {
        echo_secret: true,
        ..BrowserScript::default()
    };
    let runner = BrowserRunner::new(
        Arc::new(isolated_browser(script, "secret-rescue")),
        runtime()?,
    )
    .with_secret_host(Arc::new(Host));
    let task = tinycomputer_bus::agent::TaskId::new("t-secret-rescue");
    runner.prepare(&task, &TaskConstraints::default()).await;
    let visible = runner.visible_text(&task).await.join("\n");

    assert!(!visible.contains("4111 1111 1111 1111"));
    assert!(!visible.contains("unknown-autofill-canary"));
    assert!(visible.contains("Card number"));
    assert!(visible.contains("Email"));
    assert!(visible.contains("•••• 1111"));
    runner.release(&task);
    Ok(())
}

fn request() -> StartTaskRequest {
    StartTaskRequest {
        flow: Some(Flow {
            app: "browser".to_owned(),
            vars: std::collections::BTreeMap::new(),
            steps: vec![FlowStep::Action(FlowAction::Browse(
                "http://127.0.0.1:8765/index.html".to_owned(),
            ))],
        }),
        constraints: TaskConstraints {
            surfaces: vec![SurfaceKind::Browser],
            ..TaskConstraints::default()
        },
        ..StartTaskRequest::default()
    }
}

async fn settle(
    tasks: &Tasks,
    id: &tinycomputer_bus::agent::TaskId,
) -> Result<tinycomputer_bus::agent::TaskView, Box<dyn std::error::Error>> {
    loop {
        let reply = tasks
            .await_task(AwaitTaskRequest {
                id: id.clone(),
                timeout_ms: 1_000,
            })
            .await;
        assert!(reply.ok, "{:?}", reply.error);
        let Some(view) = reply.data else {
            return Err("the await reply had no task view".into());
        };
        if view.status.is_final() {
            return Ok(view);
        }
    }
}

fn runtime() -> Result<JevRuntime, std::io::Error> {
    JevRuntime::sage("local-test-key", false)
        .map_err(|error| std::io::Error::other(error.message.clone()))
}

fn isolated_browser(script: BrowserScript, name: &str) -> Browser {
    Browser::with_scratch(
        Arc::new(script),
        std::env::temp_dir().join(format!(
            "tinycomputer-task-browser-{name}-{}",
            std::process::id()
        )),
    )
}

fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend(width.to_be_bytes());
    png.extend(height.to_be_bytes());
    png
}
