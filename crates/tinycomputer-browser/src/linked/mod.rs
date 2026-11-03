//! agent-browser, linked in-process, as this crate's engine.
//!
//! Each session gets its own `DaemonState`, built from explicit options so
//! nothing leaks in from the host process's `AGENT_BROWSER_*` environment,
//! and a session id unique to this process so its on-disk bindings never
//! collide with another module's.
//!
//! agent-browser keeps one process-wide piece of state — the active frame a
//! command resolves selectors in — so commands from different sessions are
//! serialized through [`ENGINE`]. Calls on one session are already serialized
//! by the session lock.

use crate::{
    egress::{EgressAuthorizer, EgressProxy},
    engine::{Engine, Launcher, Reply},
};
use agent_browser::{DaemonState, StateOptions, execute_command};
use serde_json::{Value, json};
use std::sync::Arc;

/// Serializes commands across sessions; see the module docs.
static ENGINE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The [`Launcher`] for agent-browser linked in this process.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentBrowser;

/// A launcher that pins every owned Chromium process to a host-created proxy.
/// The proxy address is host control and cannot be selected by browser calls.
#[derive(Clone)]
pub struct EnforcedAgentBrowser {
    authorizer: Arc<dyn EgressAuthorizer>,
}

impl std::fmt::Debug for EnforcedAgentBrowser {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EnforcedAgentBrowser([host authorizer])")
    }
}

impl EnforcedAgentBrowser {
    /// Use the authenticated host callback for every launched session.
    #[must_use]
    pub fn new(authorizer: Arc<dyn EgressAuthorizer>) -> Self {
        Self { authorizer }
    }
}

impl Launcher for AgentBrowser {
    fn open(&self, session: &str) -> Box<dyn Engine> {
        Box::new(Linked {
            state: DaemonState::with_options(state_options(session)),
            egress_authorizer: None,
            proxy: None,
            proxy_session_id: String::new(),
            enforced_launch: false,
        })
    }
}

impl Launcher for EnforcedAgentBrowser {
    fn open(&self, session: &str) -> Box<dyn Engine> {
        Box::new(Linked {
            state: DaemonState::with_options(state_options(session)),
            egress_authorizer: Some(self.authorizer.clone()),
            proxy: None,
            proxy_session_id: private_session_id(),
            enforced_launch: false,
        })
    }
}

struct Linked {
    state: DaemonState,
    egress_authorizer: Option<Arc<dyn EgressAuthorizer>>,
    proxy: Option<EgressProxy>,
    proxy_session_id: String,
    enforced_launch: bool,
}

impl Engine for Linked {
    fn execute(&mut self, command: Value) -> Reply<'_> {
        let action = command
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let launch = self.egress_authorizer.is_some() && action.as_deref() == Some("launch");
        Box::pin(async move {
            let command = if launch {
                if self.proxy_session_id.len() != 48 {
                    return failure_reply(
                        &command,
                        "network policy could not create a private session identity",
                    );
                }
                let Some(authorizer) = self.egress_authorizer.clone() else {
                    return failure_reply(&command, "network policy has no host authorizer");
                };
                if self.proxy.is_none() {
                    match EgressProxy::bind(
                        authorizer,
                        self.proxy_session_id.clone(),
                        self.proxy_session_id.clone(),
                    )
                    .await
                    {
                        Ok(proxy) => self.proxy = Some(proxy),
                        Err(_) => {
                            return failure_reply(&command, "network policy proxy could not start");
                        }
                    }
                }
                let Some(proxy) = self.proxy.as_ref() else {
                    return failure_reply(&command, "network policy proxy could not start");
                };
                match enforce_launch(command.clone(), &proxy.url(), &self.proxy_session_id) {
                    Ok(command) => command,
                    Err(reason) => return failure_reply(&command, reason),
                }
            } else {
                command
            };
            let _serialized = ENGINE.lock().await;
            if self.egress_authorizer.is_some()
                && action.as_deref() == Some("read")
                && command.get("url").and_then(Value::as_str).is_some()
            {
                return failure_reply(
                    &command,
                    "network policy refuses process-side reads with a URL",
                );
            }
            #[allow(clippy::redundant_closure_for_method_calls)]
            let browser_unavailable = self
                .state
                .browser
                .as_mut()
                // BrowserManager is private in agent-browser, so its method cannot
                // be named as a function item here.
                .is_none_or(|browser| browser.has_process_exited());
            let closing_exited_browser =
                action.as_deref() == Some("close") && self.enforced_launch && browser_unavailable;
            if self.egress_authorizer.is_some()
                && !launch
                && !closing_exited_browser
                && (!self.enforced_launch || browser_unavailable)
            {
                self.enforced_launch = false;
                self.proxy.take();
                return failure_reply(
                    &command,
                    "network policy requires an enforced launch with a live browser session",
                );
            }
            let reply = Box::pin(execute_command(&command, &mut self.state)).await;
            if launch && reply.get("success").and_then(Value::as_bool) == Some(true) {
                self.enforced_launch = true;
            }
            if action.as_deref() == Some("close")
                && reply.get("success").and_then(Value::as_bool) == Some(true)
            {
                self.enforced_launch = false;
                self.proxy.take();
            }
            reply
        })
    }
}

fn private_session_id() -> String {
    let mut bytes = [0u8; 24];
    if getrandom::fill(&mut bytes).is_err() {
        return String::new();
    }
    bytes
        .iter()
        .fold(String::with_capacity(48), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        })
}

fn state_options(session: &str) -> StateOptions {
    StateOptions {
        session_id: format!("tinycomputer-{}-{session}", std::process::id()),
        ..StateOptions::default()
    }
}

fn failure_reply(command: &Value, reason: &str) -> Value {
    json!({
        "id": command.get("id").cloned().unwrap_or(Value::Null),
        "success": false,
        "error": reason
    })
}

fn enforce_launch(
    command: Value,
    proxy: &str,
    proxy_password: &str,
) -> Result<Value, &'static str> {
    let has_ambient_browser_config = command.get("action").and_then(Value::as_str)
        == Some("launch")
        && has_ambient_browser_config();
    enforce_launch_with_config(command, proxy, proxy_password, has_ambient_browser_config)
}

fn enforce_launch_with_config(
    mut command: Value,
    proxy: &str,
    proxy_password: &str,
    has_ambient_browser_config: bool,
) -> Result<Value, &'static str> {
    if command.get("action").and_then(Value::as_str) != Some("launch") {
        return Ok(command);
    }
    if [
        "cdpUrl",
        "cdpPort",
        "autoConnect",
        "provider",
        "profile",
        "storageState",
        "restore",
        "executablePath",
    ]
    .iter()
    .any(|key| command.get(key).is_some())
    {
        return Err("network policy refuses attached or externally managed browsers");
    }
    if command
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| !args.is_empty())
    {
        return Err("network policy refuses caller-supplied browser launch arguments");
    }
    if has_ambient_browser_config {
        return Err("network policy refuses ambient external browser configuration");
    }
    // Chromium's special <-loopback> token removes its implicit direct
    // localhost bypass. Without this, loopback requests evade authorization.
    command["proxy"] = json!({
        "server": proxy,
        "bypass": "<-loopback>",
        "username": "tinycomputer",
        "password": proxy_password
    });
    command["engine"] = json!("chrome");
    command["args"] = json!([
        "--disable-quic",
        "--disable-http2",
        "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"
    ]);
    Ok(command)
}

/// agent-browser environment variables that can attach another browser or
/// change how the egress-enforced one launches. Plugins, extensions, extra
/// arguments and init scripts are applied after this module's proxy and
/// resolver arguments, so any of them could undo egress enforcement.
const AMBIENT_BROWSER_ENV: &[&str] = &[
    "AGENT_BROWSER_CDP",
    "AGENT_BROWSER_AUTO_CONNECT",
    "AGENT_BROWSER_PROVIDER",
    "AGENT_BROWSER_ENGINE",
    "AGENT_BROWSER_EXECUTABLE_PATH",
    "AGENT_BROWSER_PROXY",
    "AGENT_BROWSER_PROXY_BYPASS",
    "AGENT_BROWSER_PROXY_USERNAME",
    "AGENT_BROWSER_PROXY_PASSWORD",
    "AGENT_BROWSER_PROFILE",
    "AGENT_BROWSER_RESTORE",
    "AGENT_BROWSER_PLUGINS",
    "AGENT_BROWSER_EXTENSIONS",
    "AGENT_BROWSER_ARGS",
    "AGENT_BROWSER_INIT_SCRIPTS",
];

fn has_ambient_browser_config() -> bool {
    AMBIENT_BROWSER_ENV
        .iter()
        .any(|name| std::env::var_os(name).is_some())
}

#[cfg(test)]
mod linked_tests;
