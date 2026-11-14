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
    host: HostBrowser,
}

/// What the host, never a browser call, chooses for an enforced launch: the
/// Chrome binary and the profile folder kept between sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HostBrowser {
    pub(crate) executable: Option<String>,
    pub(crate) profile: Option<String>,
}

impl HostBrowser {
    #[cfg(test)]
    pub(crate) fn executable(executable: &str) -> Self {
        Self {
            executable: Some(executable.to_owned()),
            profile: None,
        }
    }
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
        Self {
            authorizer,
            host: HostBrowser::default(),
        }
    }

    /// Launch the host-configured Chrome or Chromium binary. It is host
    /// control, like the proxy: a launch may repeat it but never name another.
    #[must_use]
    pub fn with_executable(mut self, executable: Option<String>) -> Self {
        self.host.executable = executable;
        self
    }

    /// Let a launch use the host-configured profile folder, so cookies and
    /// sign-ins survive between sessions. It is host control, like the
    /// executable: a launch may name this folder but never another, and it
    /// runs with extensions off, since an extension stored in the profile
    /// could set a proxy that outranks the enforced one.
    #[must_use]
    pub fn with_profile(mut self, profile: Option<String>) -> Self {
        self.host.profile = profile;
        self
    }
}

impl Launcher for AgentBrowser {
    fn open(&self, session: &str) -> Box<dyn Engine> {
        Box::new(Linked {
            state: DaemonState::with_options(state_options(session)),
            egress_authorizer: None,
            proxy: None,
            proxy_session_id: String::new(),
            proxy_password: String::new(),
            host: HostBrowser::default(),
            enforced_launch: false,
        })
    }
}

impl Launcher for EnforcedAgentBrowser {
    fn open(&self, session: &str) -> Box<dyn Engine> {
        let mut engine = enforced(self.authorizer.clone(), session);
        engine.host.clone_from(&self.host);
        Box::new(engine)
    }
}

/// An enforced session. The proxy password is its own secret: the session id
/// reaches the host in every egress callback, so it must not also unlock the
/// proxy.
fn enforced(authorizer: Arc<dyn EgressAuthorizer>, session: &str) -> Linked {
    Linked {
        state: DaemonState::with_options(state_options(session)),
        egress_authorizer: Some(authorizer),
        proxy: None,
        proxy_session_id: private_session_id(),
        proxy_password: private_session_id(),
        host: HostBrowser::default(),
        enforced_launch: false,
    }
}

struct Linked {
    state: DaemonState,
    egress_authorizer: Option<Arc<dyn EgressAuthorizer>>,
    proxy: Option<EgressProxy>,
    proxy_session_id: String,
    proxy_password: String,
    host: HostBrowser,
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
                if self.proxy_session_id.len() != 48 || self.proxy_password.len() != 48 {
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
                        self.proxy_password.clone(),
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
                match enforce_launch(
                    command.clone(),
                    &proxy.url(),
                    &self.proxy_password,
                    &self.host,
                ) {
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
    host: &HostBrowser,
) -> Result<Value, &'static str> {
    let has_ambient_browser_config = command.get("action").and_then(Value::as_str)
        == Some("launch")
        && has_ambient_browser_config();
    enforce_launch_with_config(
        command,
        proxy,
        proxy_password,
        has_ambient_browser_config,
        host,
    )
}

fn enforce_launch_with_config(
    mut command: Value,
    proxy: &str,
    proxy_password: &str,
    has_ambient_browser_config: bool,
    host: &HostBrowser,
) -> Result<Value, &'static str> {
    let host_executable = host.executable.as_deref();
    if command.get("action").and_then(Value::as_str) != Some("launch") {
        return Ok(command);
    }
    if [
        "cdpUrl",
        "cdpPort",
        "autoConnect",
        "provider",
        "storageState",
        "restore",
    ]
    .iter()
    .any(|key| command.get(key).is_some())
    {
        return Err("network policy refuses attached or externally managed browsers");
    }
    // Only the host chooses the profile folder, as it does the binary: a
    // launch may name the host's own folder, or none for a fresh profile.
    if let Some(requested) = command.get("profile")
        && (host.profile.is_none() || requested.as_str() != host.profile.as_deref())
    {
        return Err("network policy refuses a browser profile the host did not configure");
    }
    // Only the host chooses the binary. A launch may repeat the host's own
    // executable (hosts send it with each session) but not name another.
    if let Some(requested) = command.get("executablePath")
        && (host_executable.is_none() || requested.as_str() != host_executable)
    {
        return Err("network policy refuses a browser executable the host did not configure");
    }
    if command
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| !args.is_empty())
    {
        return Err("network policy refuses caller-supplied browser launch arguments");
    }
    // agent-browser runs plugins and loads extensions and init scripts after
    // this module's proxy and resolver arguments, so any of them could undo
    // egress enforcement.
    if ["plugins", "extensions", "initScripts"]
        .iter()
        .any(|key| command.get(key).is_some())
    {
        return Err("network policy refuses caller-supplied plugins, extensions and init scripts");
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
    // The enforced flags above do not depend on which Chrome binary runs.
    if let Some(executable) = host_executable {
        command["executablePath"] = json!(executable);
    }
    let mut args = vec![
        "--disable-quic",
        "--disable-http2",
        "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
    ];
    // Chrome ranks an extension's proxy above the command line's, and a kept
    // profile can carry installed extensions.
    if command.get("profile").is_some() {
        args.push("--disable-extensions");
    }
    command["args"] = json!(args);
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
