//! Tests for the browser members, over the in-memory bus.
//!
//! The service's [`Browser`] runs on a scripted engine rather than a linked
//! Chrome, so these check what the bus adds — the one-object requests, the
//! shared envelope, and the error codes and recovery hints — on a machine
//! with no browser installed. Chrome itself is exercised in the Docker lab.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Interface, Proxy};
use tinycomputer_browser::{Browser, Engine, Launcher, Reply};
use tinycomputer_bus::browser::names::methods;
use tinycomputer_bus::{DeliveryDisposition, DesktopResponse, names};

use super::super::DesktopService;

/// An engine that answers each command the way agent-browser would for a
/// page at `https://example.com/`, refuses the ref `e9` as stale, and records
/// every command it is sent.
struct Scripted(Arc<Mutex<Vec<Value>>>);

impl Engine for Scripted {
    fn execute(&mut self, command: Value) -> Reply<'_> {
        let reply = answer(&command);
        self.0
            .lock()
            .expect("the log is not poisoned")
            .push(command);
        Box::pin(async move { reply })
    }
}

fn answer(command: &Value) -> Value {
    let page = json!({"url": "https://example.com/", "title": "Example"});
    match command["action"].as_str() {
        Some("navigate") => json!({"success": true, "data": {
            "url": command["url"], "title": "Example"
        }}),
        Some("screenshot") => {
            let path = command["path"]
                .as_str()
                .expect("a screenshot names its path");
            std::fs::write(path, b"scripted image").expect("the scratch file is writable");
            json!({"success": true, "data": {"path": path}})
        }
        Some("click") if command["selector"] == "@e9" => {
            json!({"success": false, "error": "Unknown ref: e9"})
        }
        Some("url") => json!({"success": true, "data": {"url": page["url"]}}),
        Some("title") => json!({"success": true, "data": {"title": page["title"]}}),
        _ => json!({"success": true, "data": page}),
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedLauncher(pub(super) Arc<Mutex<Vec<Value>>>);

impl Launcher for ScriptedLauncher {
    fn open(&self, _session: &str) -> Box<dyn Engine> {
        Box::new(Scripted(self.0.clone()))
    }
}

/// A private scratch directory for one test, removed when it drops.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tinycomputer-bus-browser-{name}-{}",
            std::process::id()
        ));
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

/// Serves a scripted-browser service on a fresh bus and returns the serving
/// connection, to keep alive, and a proxy to it.
async fn serve(scratch: &Scratch) -> tinybus::Result<(Connection, Proxy)> {
    serve_with(scratch, &json!({}), Arc::new(ScriptedLauncher::default())).await
}

/// Serves a service configured by `config`, whose browser opens sessions on
/// `launcher`.
async fn serve_with(
    scratch: &Scratch,
    config: &Value,
    launcher: Arc<ScriptedLauncher>,
) -> tinybus::Result<(Connection, Proxy)> {
    serve_on(
        config,
        Arc::new(Browser::with_scratch(launcher, scratch.0.clone())),
    )
    .await
}

/// Serves a service configured by `config` on `browser`.
async fn serve_on(config: &Value, browser: Arc<Browser>) -> tinybus::Result<(Connection, Proxy)> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = DesktopService::with_browser(config, browser).expect("the config is valid");
    let server = Connection::connect(bus.connect().await?).await?;
    server
        .serve_at(names::OBJECT_PATH.try_into()?, service)
        .await?;
    server.request_name(names::INTERFACE).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    // The serving connection owns the name; the caller holds it for as long
    // as it calls.
    Ok((server, proxy))
}

async fn call(proxy: &Proxy, member: &str, request: Value) -> tinybus::Result<DesktopResponse> {
    proxy.call(member, (request,)).await
}

/// Calls `BrowserReadOutput` on `service` directly. It is confidential, and the
/// in-memory module is never attested, so the bus would refuse it.
async fn read_output(service: &DesktopService, request: Value) -> tinybus::Result<DesktopResponse> {
    let member = methods::READ_OUTPUT.try_into()?;
    assert!(service.requires_confidential(&member));
    Ok(
        serde_json::from_value(service.call(&member, json!([request])).await?)
            .expect("an envelope"),
    )
}

fn data(reply: &DesktopResponse) -> &Value {
    assert!(reply.ok, "{} failed: {:?}", reply.command, reply.error);
    reply
        .data
        .as_ref()
        .expect("a successful reply carries data")
}

#[tokio::test]
async fn a_session_runs_open_navigate_click_and_close_over_the_bus() -> tinybus::Result<()> {
    let scratch = Scratch::new("loop");
    let (_server, proxy) = serve(&scratch).await?;

    let opened = call(&proxy, methods::OPEN_SESSION, json!({})).await?;
    assert_eq!(opened.command, "browser-open-session");
    let session = data(&opened)["id"].clone();

    let navigated = call(
        &proxy,
        methods::NAVIGATE,
        json!({"session": session, "url": "https://example.com/next"}),
    )
    .await?;
    assert_eq!(data(&navigated)["url"], "https://example.com/next");

    let clicked = call(
        &proxy,
        methods::PERFORM,
        json!({"session": session, "action": "click", "target": {"kind": "ref", "value": "e3"}}),
    )
    .await?;
    assert_eq!(clicked.command, "browser-perform");
    assert!(clicked.ok);

    let listed: DesktopResponse = proxy.call(methods::LIST_SESSIONS, ()).await?;
    assert_eq!(data(&listed).as_array().map(Vec::len), Some(1));

    let closed = call(&proxy, methods::CLOSE_SESSION, json!({"session": session})).await?;
    assert_eq!(data(&closed)["closed"], true);
    let again = call(&proxy, methods::CLOSE_SESSION, json!({"session": session})).await?;
    assert!(again.ok, "closing a closed session succeeds");
    Ok(())
}

#[tokio::test]
async fn a_screenshot_is_read_back_in_chunks_and_released() -> tinybus::Result<()> {
    let scratch = Scratch::new("shot");
    let browser = Arc::new(Browser::with_scratch(
        Arc::new(ScriptedLauncher::default()),
        scratch.0.clone(),
    ));
    let (_server, proxy) = serve_on(&json!({}), browser.clone()).await?;
    // A second service on the same browser sees the same held outputs.
    let direct = DesktopService::with_browser(&json!({}), browser).expect("the config is valid");
    let opened = call(&proxy, methods::OPEN_SESSION, json!({})).await?;
    let session = data(&opened)["id"].clone();

    let shot = call(&proxy, methods::SCREENSHOT, json!({"session": session})).await?;
    let output = data(&shot)["id"].clone();
    assert_eq!(data(&shot)["total_bytes"], 14);

    call(&proxy, methods::READ_OUTPUT, json!({"output": output}))
        .await
        .expect_err("the bytes never cross the bus in a plain frame");
    let first = read_output(&direct, json!({"output": output, "max_len": 8})).await?;
    assert_eq!(data(&first)["eof"], false);
    let rest = read_output(&direct, json!({"output": output, "offset": 8})).await?;
    assert_eq!(data(&rest)["eof"], true);

    let released = call(&proxy, methods::RELEASE_OUTPUT, json!({"output": output})).await?;
    assert_eq!(data(&released)["released"], true);
    let gone = read_output(&direct, json!({"output": output})).await?;
    let error = gone.error.expect("a released output is gone");
    assert_eq!(error.code, "OUTPUT_NOT_FOUND");
    assert!(error.suggestion.is_some());
    Ok(())
}

#[tokio::test]
async fn a_stale_ref_fails_with_the_desktop_code_and_recovery() -> tinybus::Result<()> {
    let scratch = Scratch::new("stale");
    let (_server, proxy) = serve(&scratch).await?;
    let opened = call(&proxy, methods::OPEN_SESSION, json!({})).await?;
    let session = data(&opened)["id"].clone();

    let reply = call(
        &proxy,
        methods::PERFORM,
        json!({"session": session, "action": "click", "target": {"kind": "ref", "value": "e9"}}),
    )
    .await?;
    assert!(!reply.ok);
    assert_eq!(reply.command, "browser-perform");
    let error = reply.error.expect("a failed reply carries its error");
    assert_eq!(error.code, "STALE_REF");
    let hint = error.recovery.expect("a stale ref has a way out");
    assert_eq!(hint.strategy, "refresh_snapshot_then_retry_original");
    assert!(hint.requires_fresh_snapshot);
    assert_eq!(
        error.details.expect("details carry the wire name")["name"],
        tinycomputer_bus::browser::errors::STALE_REF
    );
    Ok(())
}

#[tokio::test]
async fn a_key_for_an_element_the_focus_misses_is_refused_unsent() -> tinybus::Result<()> {
    // The scripted page never shows the focus on anything, as a heading that
    // takes none would leave it: the press is refused, not sent.
    let scratch = Scratch::new("focus");
    let launcher = Arc::new(ScriptedLauncher::default());
    let (_server, proxy) = serve_with(&scratch, &json!({}), launcher.clone()).await?;
    let opened = call(&proxy, methods::OPEN_SESSION, json!({})).await?;
    let session = data(&opened)["id"].clone();

    let reply = call(
        &proxy,
        methods::PERFORM,
        json!({
            "session": session,
            "action": "press",
            "key": "Enter",
            "target": {"kind": "ref", "value": "e5"},
        }),
    )
    .await?;
    let error = reply.error.expect("a missed focus fails");
    assert_eq!(error.code, "NOT_ACTIONABLE");
    assert_eq!(
        error.details.expect("details carry the wire name")["name"],
        tinycomputer_bus::browser::errors::NOT_ACTIONABLE
    );
    assert!(
        error
            .suggestion
            .is_some_and(|hint| hint.contains("act on the field itself"))
    );
    let sent = launcher.0.lock().expect("the log is not poisoned");
    assert!(sent.iter().any(|command| command["action"] == "focus"));
    assert!(!sent.iter().any(|command| command["action"] == "press"));
    Ok(())
}

#[tokio::test]
async fn an_unknown_session_is_refused_before_anything_is_sent() -> tinybus::Result<()> {
    let scratch = Scratch::new("unknown");
    let (_server, proxy) = serve(&scratch).await?;

    for (member, request) in [
        (
            methods::NAVIGATE,
            json!({"session": "s-404", "url": "https://example.com"}),
        ),
        (methods::SNAPSHOT, json!({"session": "s-404"})),
        (methods::READ_PAGE, json!({"session": "s-404"})),
        (
            methods::EVALUATE,
            json!({"session": "s-404", "expression": "1"}),
        ),
        (methods::LIST_DOWNLOADS, json!({"session": "s-404"})),
        (methods::WAIT_DOWNLOAD, json!({"session": "s-404"})),
    ] {
        let reply = call(&proxy, member, request).await?;
        let error = reply.error.expect("an unknown session fails");
        assert_eq!(error.code, "SESSION_NOT_FOUND", "{member}");
        assert_eq!(
            error.disposition.delivery,
            DeliveryDisposition::NotDelivered,
            "{member}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_open_session_takes_the_configured_executable() -> tinybus::Result<()> {
    let scratch = Scratch::new("executable");
    let launcher = Arc::new(ScriptedLauncher::default());
    let (_server, proxy) = serve_with(
        &scratch,
        &json!({"browser": {"executable": "/opt/chromium"}}),
        launcher.clone(),
    )
    .await?;

    assert!(call(&proxy, methods::OPEN_SESSION, json!({})).await?.ok);
    let attached = call(
        &proxy,
        methods::OPEN_SESSION,
        json!({"endpoint": "http://127.0.0.1:9222"}),
    )
    .await?;
    assert_eq!(data(&attached)["launched"], false);

    let launch_commands = launcher
        .0
        .lock()
        .expect("the log is not poisoned")
        .iter()
        .filter(|command| command["action"] == "launch")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(launch_commands.len(), 2);
    assert_eq!(launch_commands[0]["executablePath"], "/opt/chromium");
    assert!(
        launch_commands[1].get("executablePath").is_none(),
        "an attached session launches nothing, so it takes no executable"
    );
    Ok(())
}

#[tokio::test]
async fn a_malformed_request_is_a_bus_error_not_a_panic() -> tinybus::Result<()> {
    let scratch = Scratch::new("malformed");
    let (_server, proxy) = serve(&scratch).await?;
    let reply = call(
        &proxy,
        methods::NAVIGATE,
        json!({"url": "https://example.com"}),
    )
    .await;
    assert!(reply.is_err(), "a request with no session does not decode");
    Ok(())
}

#[tokio::test]
async fn the_output_sweep_runs_until_the_browser_is_gone() {
    use crate::tinybus_module::dispatch::sweep_every;

    let scratch = Scratch::new("sweep");
    let browser = Arc::new(Browser::with_scratch(
        Arc::new(ScriptedLauncher::default()),
        scratch.0.clone(),
    ));
    let sweep = tokio::spawn(sweep_every(
        Arc::downgrade(&browser),
        std::time::Duration::from_millis(1),
    ));
    // It sweeps while the browser lives…
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert!(!sweep.is_finished());
    // …and ends on its own once the browser is dropped.
    drop(browser);
    tokio::time::timeout(std::time::Duration::from_secs(1), sweep)
        .await
        .expect("the sweep ends once the browser is gone")
        .expect("the sweep does not panic");
}

#[test]
fn a_timed_out_observation_retries_without_a_snapshot() {
    use crate::tinybus_module::dispatch::observing_reply;
    // Opening a session or reading the page changed nothing, and a snapshot
    // first is no help, so the hint is a plain retry.
    let reply = observing_reply::<()>(
        "browser-open-session",
        Err(tinycomputer_browser::Error::timeout("launch", 30_000)),
    );
    let error = reply.error.expect("a timeout fails the call");
    assert_eq!(error.code, "TIMEOUT");
    let hint = error.recovery.expect("a timeout has a way out");
    assert_eq!(hint.strategy, "retry_original");
    assert!(hint.retryable && !hint.requires_fresh_snapshot);
    // Any other failure keeps its own hint.
    let other = observing_reply::<()>(
        "browser-snapshot",
        Err(tinycomputer_browser::Error::browser_unavailable(
            "no chrome",
        )),
    );
    assert!(other.error.expect("fails").recovery.is_none());
}

#[tokio::test]
async fn a_profile_is_claimed_once_and_free_again_after_release() -> tinybus::Result<()> {
    let scratch = Scratch::new("profile-claim");
    let (_server, proxy) = serve(&scratch).await?;
    let folder = scratch.0.join("profile");
    std::fs::create_dir_all(&folder).expect("the profile folder is creatable");
    let path = folder.to_string_lossy().into_owned();

    let first = call(&proxy, methods::CLAIM_PROFILE, json!({"path": path})).await?;
    assert_eq!(data(&first)["outcome"], "claimed");
    assert_eq!(data(&first)["path"], json!(path));

    let busy = call(
        &proxy,
        methods::CLAIM_PROFILE,
        json!({"path": path, "fall_back_to_fresh": false}),
    )
    .await?;
    assert_eq!(data(&busy)["outcome"], "busy");

    let fresh = call(&proxy, methods::CLAIM_PROFILE, json!({"path": path})).await?;
    assert_eq!(data(&fresh)["outcome"], "fell_back_to_fresh");
    let fresh_path = data(&fresh)["path"]
        .as_str()
        .expect("a fresh path")
        .to_owned();
    assert_ne!(fresh_path, path);
    let _ = std::fs::remove_dir_all(&fresh_path);

    let released = call(&proxy, methods::RELEASE_PROFILE, json!({"path": path})).await?;
    assert_eq!(data(&released)["released"], true);
    let again = call(&proxy, methods::CLAIM_PROFILE, json!({"path": path})).await?;
    assert_eq!(data(&again)["outcome"], "claimed");
    call(&proxy, methods::RELEASE_PROFILE, json!({"path": path})).await?;
    Ok(())
}
