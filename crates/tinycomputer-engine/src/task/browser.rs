//! A browser-only [`super::FlowRunner`] for hosts embedding the engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tinycomputer_browser::{ActionContext, Browser, BrowserSurface, SessionOptions};
use tinycomputer_bus::agent::{PaymentMode, TaskConstraints, TaskId};
use tinycomputer_bus::browser::{ScreenshotRequest, SessionId};
use tinycomputer_bus::{DesktopError, DesktopResponse, JevOperation, RunFlowRequest};
use tinycomputer_core::surface::{Candidate, Depth, Screen, Surface};

use super::{CaptureFuture, FlowFuture, FlowRunner, PrepareFuture, TextFuture};
use crate::SecretHost;
use crate::{JevRuntime, UsageRecorder};

/// Runs task flows directly on one in-process browser surface per task.
///
/// The host owns the browser launcher and the configured Jev runtime. This
/// runner adds no bus or desktop adapter. Each task keeps its browser session
/// across pauses and continuations, and [`FlowRunner::release`] closes it when
/// the task ends. When a `SecretHost` is configured, tasks have no screenshot
/// artifacts, since host-held values rendered by a page cannot be scrubbed
/// from pixels.
pub struct BrowserRunner {
    browser: Arc<Browser>,
    runtime: JevRuntime,
    options: SessionOptions,
    surfaces: Mutex<HashMap<TaskId, BrowserSurface>>,
    secret_host: Option<Arc<dyn SecretHost>>,
}

impl std::fmt::Debug for BrowserRunner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserRunner")
            .field("runtime", &self.runtime)
            .field("options", &self.options)
            .field("surfaces", &"[task surfaces]")
            .finish_non_exhaustive()
    }
}

impl BrowserRunner {
    /// Creates a runner using `browser` and the host's configured `runtime`.
    #[must_use]
    pub fn new(browser: Arc<Browser>, runtime: JevRuntime) -> Self {
        Self {
            browser,
            runtime,
            options: SessionOptions::default(),
            surfaces: Mutex::new(HashMap::new()),
            secret_host: None,
        }
    }

    /// Uses `host` to resolve opaque secret handles and type them at the
    /// moment a task fills a field. Editable control values are also hidden
    /// from task observations while this host is present; the host can still
    /// read and verify a field through the private surface APIs.
    #[must_use]
    pub fn with_secret_host(mut self, host: Arc<dyn SecretHost>) -> Self {
        self.secret_host = Some(host);
        self
    }

    /// The browser session this task runner opened, once it is prepared.
    #[must_use]
    pub fn session_id(&self, task: &TaskId) -> Option<SessionId> {
        self.surfaces.lock().ok()?.get(task)?.session()
    }

    /// Uses `options` as the host defaults for sessions this runner opens.
    #[must_use]
    pub fn with_session_options(mut self, options: SessionOptions) -> Self {
        self.options = options;
        self
    }

    fn surface(&self, task: &TaskId, constraints: &TaskConstraints) -> BrowserSurface {
        let fresh = || {
            let mut options = self.options.clone();
            if constraints.browser_endpoint.is_some() {
                options.endpoint.clone_from(&constraints.browser_endpoint);
            }
            if constraints.browser_executable.is_some() {
                options
                    .executable
                    .clone_from(&constraints.browser_executable);
            }
            if constraints.browser_profile.is_some() {
                options
                    .user_data_dir
                    .clone_from(&constraints.browser_profile);
            }
            if !constraints.origins.is_empty() {
                options.allowed_origins.clone_from(&constraints.origins);
            }
            options.headless = !constraints.headed;
            let surface = BrowserSurface::new(
                self.browser.clone(),
                options,
                tokio::runtime::Handle::current(),
            )
            .with_action_context(ActionContext {
                task_id: Some(task.0.clone()),
                session_id: None,
                payment_in_play: constraints.payment == PaymentMode::FillThenApprove,
            });
            if self.secret_host.is_some() {
                surface.without_form_values()
            } else {
                surface
            }
        };

        self.surfaces.lock().map_or_else(
            |_| fresh(),
            |mut surfaces| surfaces.entry(task.clone()).or_insert_with(fresh).clone(),
        )
    }
}

impl FlowRunner for BrowserRunner {
    fn run(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
    ) -> FlowFuture {
        self.run_with_runtime(task, constraints, request, &self.runtime)
    }

    fn page_origin(&self, task: &TaskId) -> super::PageOriginFuture {
        let Some(session) = self.session_id(task) else {
            return Box::pin(async { None });
        };
        let browser = self.browser.clone();
        Box::pin(async move {
            let url = browser
                .list_sessions()
                .await
                .ok()?
                .into_iter()
                .find(|info| info.id == session)?
                .url;
            secure_page_origin(&url)
        })
    }

    fn run_with_usage(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
        usage: Arc<UsageRecorder>,
    ) -> FlowFuture {
        let runtime = self.runtime.clone().with_usage_recorder(usage);
        self.run_with_runtime(task, constraints, request, &runtime)
    }

    fn visible_text(&self, task: &TaskId) -> TextFuture {
        let surface = self.surface(task, &TaskConstraints::default());
        let host = self.secret_host.clone();
        let task_id = task.0.clone();
        Box::pin(async move {
            let text: Vec<String> = tokio::task::spawn_blocking(move || {
                surface
                    .observe("browser", None, Depth::Full)
                    .map(|screen| {
                        screen
                            .context
                            .into_iter()
                            .chain(screen.candidates.iter().filter_map(|node| {
                                let held = node
                                    .value
                                    .as_ref()
                                    .and_then(serde_json::Value::as_str)
                                    .filter(|held| !held.trim().is_empty());
                                match (&node.name, held) {
                                    (Some(name), Some(held)) => Some(format!("{name} = {held:?}")),
                                    (name, _) => name.clone(),
                                }
                            }))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            match host {
                Some(host) => text
                    .into_iter()
                    .map(|line| host.redact(&task_id, &line))
                    .collect(),
                None => text,
            }
        })
    }

    fn capture(&self, task: &TaskId) -> CaptureFuture {
        if self.secret_host.is_some() {
            return Box::pin(async { None });
        }
        let surface = self
            .surfaces
            .lock()
            .ok()
            .and_then(|surfaces| surfaces.get(task).cloned());
        let Some(session) = surface.and_then(|surface| surface.session()) else {
            return Box::pin(async { None });
        };
        let browser = self.browser.clone();
        Box::pin(async move {
            browser
                .screenshot(&session, ScreenshotRequest::default())
                .await
                .ok()
        })
    }

    fn prepare(&self, task: &TaskId, constraints: &TaskConstraints) -> PrepareFuture {
        let surface = self.surface(task, constraints);
        Box::pin(async move {
            let _opened = tokio::task::spawn_blocking(move || surface.open()).await;
        })
    }

    fn open_page(&self, task: &TaskId, url: &str) -> PrepareFuture {
        let surface = self
            .surfaces
            .lock()
            .ok()
            .and_then(|surfaces| surfaces.get(task).cloned());
        let Some(surface) = surface else {
            return Box::pin(async {});
        };
        let url = url.to_owned();
        Box::pin(async move {
            let _loaded = tokio::task::spawn_blocking(move || surface.open_at(&url)).await;
        })
    }

    fn warm(&self, task: &TaskId, votes: u32) -> PrepareFuture {
        let runtime = self.runtime.journaled_as(&format!("task-{task}"));
        Box::pin(async move { runtime.warm(votes).await })
    }

    fn warm_with_usage(
        &self,
        task: &TaskId,
        votes: u32,
        usage: Arc<crate::UsageRecorder>,
    ) -> PrepareFuture {
        let runtime = self
            .runtime
            .journaled_as(&format!("task-{task}"))
            .with_usage_recorder(usage);
        Box::pin(async move { runtime.warm(votes).await })
    }

    fn journal(&self, task: Option<&TaskId>, event: &str, fields: &dyn Fn() -> serde_json::Value) {
        if !self.runtime.journaling() {
            return;
        }
        let runtime = task.map_or_else(
            || self.runtime.clone(),
            |task| self.runtime.journaled_as(&format!("task-{task}")),
        );
        runtime.journal_event(event, fields);
    }

    fn release(&self, task: &TaskId) {
        if let Some(surface) = self
            .surfaces
            .lock()
            .ok()
            .and_then(|mut surfaces| surfaces.remove(task))
        {
            surface.close();
        }
    }

    fn redact(&self, task: &TaskId, text: &str) -> String {
        self.secret_host
            .as_ref()
            .map_or_else(|| text.to_owned(), |host| host.redact(&task.0, text))
    }
}

/// Returns only the origin of an HTTPS page, including a non-default port.
fn secure_page_origin(page_url: &str) -> Option<String> {
    let url = url::Url::parse(page_url).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(format!("{}/", url.origin().ascii_serialization()))
}

impl BrowserRunner {
    fn run_with_runtime(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
        runtime: &JevRuntime,
    ) -> FlowFuture {
        let runtime = runtime.journaled_as(&format!("task-{task}"));
        let surface = self.surface(task, constraints);
        let context = ActionContext {
            task_id: Some(task.0.clone()),
            session_id: None,
            payment_in_play: constraints.payment == PaymentMode::FillThenApprove,
        };
        let mut request = request;
        request.task_id = Some(task.0.clone());
        let secret_host = self.secret_host.clone();
        let task_id = task.0.clone();
        Box::pin(async move {
            if surface.update_action_context(context).await.is_err() {
                return DesktopResponse::err(
                    "update-action-context",
                    DesktopError::new(
                        "ACTION_CONTEXT_FAILED",
                        "the retained browser session could not update its host payment context",
                    ),
                );
            }
            match secret_host {
                Some(host) => {
                    let surface = SecretRedactingSurface {
                        inner: surface,
                        host: host.clone(),
                        task_id,
                    };
                    tinycomputer_engine_flow(surface, runtime, request, Some(host)).await
                }
                None => tinycomputer_engine_flow(surface, runtime, request, None).await,
            }
        })
    }
}

/// Keeps known host-held values out of every model-facing screen and browser
/// reply while delegating live reads and actions to the underlying surface.
/// Editable field values are removed structurally by
/// `BrowserSurface::without_form_values`; this layer also scrubs page text,
/// labels, non-form values, and error output using the host's intent-scoped
/// redactor.
#[derive(Clone)]
struct SecretRedactingSurface {
    inner: BrowserSurface,
    host: Arc<dyn SecretHost>,
    task_id: String,
}

impl SecretRedactingSurface {
    fn redact_screen(&self, screen: &mut Screen) {
        redact_screen(&*self.host, &self.task_id, screen);
    }

    fn redact_response(&self, response: DesktopResponse) -> DesktopResponse {
        redact_response(&*self.host, &self.task_id, response)
    }
}

impl Surface for SecretRedactingSurface {
    fn observe(
        &self,
        app: &str,
        root: Option<&str>,
        depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        let mut screen = match self.inner.observe(app, root, depth) {
            Ok(screen) => screen,
            Err(error) => {
                return Err(redact_observation_error(&*self.host, &self.task_id, error));
            }
        };
        self.redact_screen(&mut screen);
        Ok(screen)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        self.redact_response(self.inner.execute(operation, target, text))
    }

    fn authorize_secret_type(&self, target: &Candidate) -> Result<String, String> {
        self.inner.authorize_secret_type(target)
    }

    fn read_value(&self, target: &Candidate) -> Option<String> {
        self.inner.read_value(target)
    }

    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse {
        self.redact_response(self.inner.paste(app, target, text))
    }

    fn press(&self, app: &str, combo: &str) -> DesktopResponse {
        self.redact_response(self.inner.press(app, combo))
    }

    fn launch(&self, app: &str) -> DesktopResponse {
        self.redact_response(self.inner.launch(app))
    }

    fn settle(&self) {
        self.inner.settle();
    }

    fn settle_briefly(&self) {
        self.inner.settle_briefly();
    }

    fn await_change(&self, ms: u64) -> bool {
        self.inner.await_change(ms)
    }

    fn navigate(&self, url: &str) -> DesktopResponse {
        self.redact_response(self.inner.navigate(url))
    }

    fn back(&self, app: &str) -> DesktopResponse {
        self.redact_response(self.inner.back(app))
    }

    fn dismiss_cover(&self, target: &Candidate) -> DesktopResponse {
        self.redact_response(self.inner.dismiss_cover(target))
    }
}

fn redact_observation_error(
    host: &dyn SecretHost,
    task_id: &str,
    response: Box<DesktopResponse>,
) -> Box<DesktopResponse> {
    Box::new(redact_response(host, task_id, *response))
}

fn redact_response(
    host: &dyn SecretHost,
    task_id: &str,
    response: DesktopResponse,
) -> DesktopResponse {
    let redact = |text: &str| host.redact(task_id, text);
    let Ok(mut value) = serde_json::to_value(response) else {
        return redaction_failed_response();
    };

    // These fields can be used to address or replay an operation. Rewriting
    // them into a placeholder would make an apparently valid response point
    // at a different target, so fail closed on a collision.
    if operational_collision(&value, &redact) {
        return redaction_failed_response();
    }
    super::redact_value(&mut value, &redact);
    serde_json::from_value(value).unwrap_or_else(|_| redaction_failed_response())
}

fn operational_collision(value: &serde_json::Value, redact: &impl Fn(&str) -> String) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            let operational = matches!(
                key.as_str(),
                "command" | "code" | "ref" | "ref_id" | "target" | "action" | "available_actions"
            );
            (operational && value.as_str().is_some_and(|text| redact(text) != text))
                || operational_collision(value, redact)
        }),
        serde_json::Value::Array(items) => {
            items.iter().any(|item| operational_collision(item, redact))
        }
        _ => false,
    }
}

fn redaction_failed_response() -> DesktopResponse {
    DesktopResponse::err(
        "redaction",
        DesktopError::new(
            "REDACTION_FAILED",
            "the browser response could not be safely redacted",
        ),
    )
}

fn redact_screen(host: &dyn SecretHost, task_id: &str, screen: &mut Screen) {
    fn candidate(host: &dyn SecretHost, task_id: &str, node: &mut Candidate) -> bool {
        let redact = |text: &str| host.redact(task_id, text);
        let reference = redact(&node.ref_id);
        let role = redact(&node.role);
        // Refs and actions are executable metadata. If redaction finds a
        // collision, omit the candidate instead of manufacturing an invalid
        // target that could address a different control.
        if reference != node.ref_id || role != node.role {
            return false;
        }
        node.available_actions
            .retain(|action| redact(action) == *action);
        node.name = node.name.take().map(|text| redact(&text));
        node.description = node.description.take().map(|text| redact(&text));
        node.states.retain(|state| redact(state) == *state);
        for path in &mut node.path {
            *path = redact(path);
        }
        if let Some(value) = &mut node.value {
            super::redact_value(value, &redact);
        }
        if let Some(bounds) = &mut node.bounds {
            let original = bounds.clone();
            super::redact_value(bounds, &redact);
            if *bounds != original {
                node.bounds = None;
            }
        }
        node.children
            .retain_mut(|child| candidate(host, task_id, child));
        true
    }
    let redact = |text: &str| host.redact(task_id, text);
    screen.app = redact(&screen.app);
    screen.window = screen.window.take().map(|text| redact(&text));
    screen.surface = redact(&screen.surface);
    for text in &mut screen.context {
        *text = redact(text);
    }
    screen
        .unexplored
        .retain(|reference| redact(reference) == *reference);
    screen
        .candidates
        .retain_mut(|node| candidate(host, task_id, node));
    screen
        .text_nodes
        .retain_mut(|node| candidate(host, task_id, node));
}

#[cfg(test)]
mod redaction_tests {
    use std::sync::Arc;

    use serde_json::json;
    use tinycomputer_bus::{DesktopError, DesktopResponse};
    use tinycomputer_core::surface::{Candidate, Screen, describe};

    use super::{redact_observation_error, redact_response, redact_screen};
    use crate::{SecretHost, SecretTypeRequest};

    struct Host;

    impl SecretHost for Host {
        fn type_secret(&self, _request: &SecretTypeRequest) -> Result<(), String> {
            Ok(())
        }

        fn redact(&self, _task_id: &str, text: &str) -> String {
            text.replace("private-canary", "[hidden]")
                .replace("123456789", "[hidden]")
        }
    }

    #[test]
    fn jev_screen_context_and_candidate_labels_are_redacted_before_model_use() {
        let host: Arc<dyn SecretHost> = Arc::new(Host);
        let mut screen = Screen {
            app: "browser".into(),
            window: Some("private-canary page".into()),
            surface: "window".into(),
            candidates: vec![
                Candidate {
                    ref_id: "e1".into(),
                    role: "button".into(),
                    name: Some("private-canary action".into()),
                    value: Some(
                        json!({"private-canary": 123_456_789, "[hidden]": "private-canary"}),
                    ),
                    path: vec!["private-canary parent".into()],
                    states: vec!["private-canary selected".into()],
                    available_actions: vec!["Click".into(), "private-canary action".into()],
                    bounds: Some(json!({"x": 10, "y": 2})),
                    ..Candidate::default()
                },
                Candidate {
                    ref_id: "private-canary-ref".into(),
                    role: "button".into(),
                    name: Some("Unsafe ref".into()),
                    available_actions: vec!["Click".into()],
                    ..Candidate::default()
                },
            ],
            context: vec!["Restored private-canary value".into()],
            unexplored: vec!["private-canary-ref".into(), "e2".into()],
            text_nodes: Vec::new(),
        };

        redact_screen(host.as_ref(), "task-1", &mut screen);

        let candidate = &screen.candidates[0];
        assert_eq!(candidate.name.as_deref(), Some("[hidden] action"));
        assert_eq!(candidate.path, ["[hidden] parent"]);
        assert_eq!(candidate.states, Vec::<String>::new());
        assert!(candidate.value.as_ref().is_some_and(|value| {
            value["[hidden]"] == "[hidden]" && value["[hidden]#2"] == "[hidden]"
        }));
        assert!(
            candidate
                .bounds
                .as_ref()
                .is_some_and(|bounds| bounds["x"] == 10)
        );
        assert_eq!(candidate.available_actions, ["Click"]);
        assert!(
            !screen
                .candidates
                .iter()
                .any(|node| node.ref_id.contains("private-canary"))
        );
        assert_eq!(screen.unexplored, ["e2"]);
        assert_eq!(screen.context[0], "Restored [hidden] value");
        assert_eq!(screen.candidates[0].ref_id, "e1");
    }

    #[test]
    fn observation_errors_are_redacted_before_they_are_returned() {
        let host = Host;
        let error = Box::new(DesktopResponse::err(
            "observe",
            DesktopError::new("private-canary-code", "page contained private-canary"),
        ));

        let safe = redact_observation_error(&host, "task-1", error);
        assert!(safe.error.as_ref().is_some_and(|error| {
            error.code == "REDACTION_FAILED"
                && error.message == "the browser response could not be safely redacted"
        }));
    }

    #[test]
    fn response_redaction_covers_numeric_values_and_object_keys() {
        let response = DesktopResponse::ok(
            "snapshot",
            json!({
                "metadata": {"private-canary": "123456789", "[hidden]": 123_456_789},
                "message": "private-canary",
            }),
        );

        let safe = redact_response(&Host, "task-1", response);
        assert!(safe.data.as_ref().is_some_and(|data| {
            data["metadata"]["[hidden]"] == "[hidden]"
                && data["metadata"]["[hidden]#2"] == "[hidden]"
                && data["message"] == "[hidden]"
        }));
    }

    #[test]
    fn response_with_secret_in_an_operational_ref_fails_closed() {
        let response = DesktopResponse::ok(
            "snapshot",
            json!({"target": {"ref_id": "private-canary-ref"}}),
        );

        let safe = redact_response(&Host, "task-1", response);

        assert!(!safe.ok);
        assert!(
            safe.error
                .as_ref()
                .is_some_and(|error| error.code == "REDACTION_FAILED")
        );
    }

    #[test]
    fn secret_shaped_bounds_are_dropped_before_model_description() {
        let host: Arc<dyn SecretHost> = Arc::new(Host);
        let mut screen = Screen {
            app: "browser".into(),
            window: None,
            surface: "window".into(),
            candidates: vec![
                Candidate {
                    ref_id: "e1".into(),
                    role: "button".into(),
                    bounds: Some(json!({"x": 20, "y": 30, "width": 100, "height": 40})),
                    ..Candidate::default()
                },
                Candidate {
                    ref_id: "e2".into(),
                    role: "group".into(),
                    bounds: Some(json!({"label": "private-canary", "offset": 123_456_789})),
                    ..Candidate::default()
                },
            ],
            context: Vec::new(),
            unexplored: Vec::new(),
            text_nodes: Vec::new(),
        };

        redact_screen(host.as_ref(), "task-1", &mut screen);

        assert_eq!(
            screen.candidates[0].bounds,
            Some(json!({"x": 20, "y": 30, "width": 100, "height": 40}))
        );
        assert!(screen.candidates[1].bounds.is_none());
        let model_description =
            serde_json::to_string(&describe(&screen.candidates[1], true)).unwrap_or_default();
        assert!(!model_description.contains("private-canary"));
        assert!(!model_description.contains("123456789"));
    }
}

async fn tinycomputer_engine_flow(
    surface: impl Surface + Sync,
    runtime: JevRuntime,
    request: RunFlowRequest,
    secret_host: Option<Arc<dyn SecretHost>>,
) -> DesktopResponse {
    crate::run_flow_with_host(surface, &runtime, request, secret_host).await
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod browser_tests;
