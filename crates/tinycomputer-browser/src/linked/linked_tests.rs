//! Tests for the linked engine that need no browser.
//!
//! agent-browser launches a browser before running any command not on its
//! skip list, so these send only commands that are on it (such as an empty
//! action): the engine answers in its envelope and nothing is launched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use std::{io, net::SocketAddr, sync::Arc};

use super::{AgentBrowser, ENGINE, EnforcedAgentBrowser, Linked, state_options};
use crate::{
    BrowserDialTarget, DialGrant, EgressAuthorizer,
    engine::{Engine, Launcher},
};
use agent_browser::DaemonState;

#[derive(Debug)]
struct RejectAll;

impl EgressAuthorizer for RejectAll {
    fn authorize(
        &self,
        _: BrowserDialTarget,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<DialGrant>> + Send>> {
        Box::pin(async { Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")) })
    }
    fn consume(
        &self,
        _: BrowserDialTarget,
        _: DialGrant,
        _: SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
        Box::pin(async { Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")) })
    }
    fn validate(
        &self,
        _: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<bool>> + Send>> {
        Box::pin(async { Ok(false) })
    }
}

#[tokio::test]
async fn an_empty_command_is_answered_without_launching_a_browser() {
    let mut engine = AgentBrowser.open("unit");
    let reply = engine.execute(json!({"id": "1", "action": ""})).await;
    assert_eq!(reply["success"], false);
    assert!(
        reply["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
}

#[tokio::test]
async fn enforced_launcher_creates_a_private_session_without_implicit_launch() {
    let launcher = super::EnforcedAgentBrowser::new(Arc::new(RejectAll));
    assert_eq!(
        format!("{launcher:?}"),
        "EnforcedAgentBrowser([host authorizer])"
    );

    let mut engine = launcher.open("private-launcher");
    let reply = engine
        .execute(json!({"id": "read", "action": "read"}))
        .await;

    assert_eq!(reply["success"], false);
    assert!(
        reply["error"]
            .as_str()
            .is_some_and(|error| error.contains("enforced launch with a live browser session"))
    );
}

#[tokio::test]
async fn enforced_browser_refuses_implicit_launch_before_any_request() {
    let mut engine = EnforcedAgentBrowser::new(Arc::new(RejectAll)).open("unit");
    let reply = engine
        .execute(json!({"id":"navigate","action":"navigate","url":"http://127.0.0.1/"}))
        .await;
    assert_eq!(reply["success"], false);
    assert!(reply["error"].as_str().unwrap().contains("enforced launch"));
}

#[tokio::test]
async fn a_closed_or_lost_browser_cannot_implicitly_launch_without_the_proxy() {
    let mut engine = Linked {
        state: DaemonState::with_options(state_options("closed")),
        egress_authorizer: Some(Arc::new(RejectAll)),
        proxy: None,
        proxy_session_id: "private-session-id".into(),
        enforced_launch: true,
    };
    let reply = engine
        .execute(json!({"id":"navigate","action":"navigate","url":"http://public.example/"}))
        .await;
    assert_eq!(reply["success"], false);
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("live browser session")
    );
}

#[tokio::test]
async fn an_exited_enforced_browser_can_still_be_closed() {
    let mut engine = Linked {
        state: DaemonState::with_options(state_options("close-exited")),
        egress_authorizer: Some(Arc::new(RejectAll)),
        proxy: None,
        proxy_session_id: "private-session-id".into(),
        enforced_launch: true,
    };

    let reply = engine.execute(json!({"id":"close","action":"close"})).await;

    assert_eq!(reply["success"], true);
    assert!(!engine.enforced_launch);
    assert!(engine.proxy.is_none());
}

#[tokio::test]
async fn enforced_browser_refuses_process_side_reads_with_a_url() {
    let mut engine = Linked {
        state: DaemonState::with_options(state_options("process-read")),
        egress_authorizer: Some(Arc::new(RejectAll)),
        proxy: None,
        proxy_session_id: "private-session-id".into(),
        enforced_launch: true,
    };
    let reply = engine
        .execute(json!({
            "id":"read",
            "action":"read",
            "url":"http://public.example/"
        }))
        .await;
    assert_eq!(reply["success"], false);
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("process-side reads with a URL")
    );
}

#[tokio::test]
async fn enforced_browser_rechecks_liveness_after_waiting_for_engine_lock() {
    let engine = Linked {
        state: DaemonState::with_options(state_options("closed-after-lock")),
        egress_authorizer: Some(Arc::new(RejectAll)),
        proxy: None,
        proxy_session_id: "private-session-id".into(),
        enforced_launch: true,
    };
    let serialized = ENGINE.lock().await;
    let pending = tokio::spawn(async move {
        let mut engine = engine;
        engine
            .execute(json!({
                "id":"navigate",
                "action":"navigate",
                "url":"http://public.example/"
            }))
            .await
    });
    tokio::task::yield_now().await;
    assert!(!pending.is_finished(), "the call should wait for ENGINE");
    drop(serialized);

    let reply = pending.await.unwrap();
    assert_eq!(reply["success"], false);
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("live browser session")
    );
}

mod launch_tests;
#[cfg(live_browser_tests)]
mod live_tests;
