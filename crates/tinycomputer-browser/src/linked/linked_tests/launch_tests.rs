//! Tests for browser launch command rewriting and policy refusal.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::super::{AMBIENT_BROWSER_ENV, enforce_launch_with_config};
use serde_json::json;

#[test]
fn enforced_launch_uses_only_the_host_proxy_and_network_containment_flags() {
    let launch = enforce_launch_with_config(
        json!({"id":"launch","action":"launch","headless":true,"args":[]}),
        "http://127.0.0.1:39123",
        "session-secret",
        false,
    )
    .expect("enforced launch");
    assert_eq!(launch["proxy"]["server"], "http://127.0.0.1:39123");
    assert_eq!(launch["proxy"]["bypass"], "<-loopback>");
    assert_eq!(launch["proxy"]["username"], "tinycomputer");
    assert_eq!(launch["proxy"]["password"], "session-secret");
    assert_eq!(
        launch["args"],
        json!([
            "--disable-quic",
            "--disable-http2",
            "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"
        ])
    );
}

#[test]
fn enforced_launch_refuses_external_cdp_and_caller_flags() {
    let cdp = enforce_launch_with_config(
        json!({"action":"launch","cdpUrl":"http://127.0.0.1:9222"}),
        "http://127.0.0.1:39123",
        "session-secret",
        false,
    );
    assert!(cdp.is_err());
    let args = enforce_launch_with_config(
        json!({"action":"launch","args":["--proxy-server=http://attacker.test"]}),
        "http://127.0.0.1:39123",
        "session-secret",
        false,
    );
    assert!(args.is_err());
    let executable = enforce_launch_with_config(
        json!({"action":"launch","executablePath":"/tmp/attacker-browser"}),
        "http://127.0.0.1:39123",
        "session-secret",
        false,
    );
    assert!(executable.is_err());
    let ambient = enforce_launch_with_config(
        json!({"action":"launch"}),
        "http://127.0.0.1:39123",
        "session-secret",
        true,
    );
    assert!(ambient.is_err());
}

/// agent-browser applies these after the enforced proxy arguments, so an
/// ambient value must refuse the launch rather than reach the browser.
#[test]
fn ambient_plugins_extensions_args_and_init_scripts_refuse_the_launch() {
    for name in [
        "AGENT_BROWSER_PLUGINS",
        "AGENT_BROWSER_EXTENSIONS",
        "AGENT_BROWSER_ARGS",
        "AGENT_BROWSER_INIT_SCRIPTS",
        "AGENT_BROWSER_PROXY",
        "AGENT_BROWSER_CDP",
    ] {
        assert!(AMBIENT_BROWSER_ENV.contains(&name), "{name}");
    }
}
