//! Opt-in real-Chromium coverage through the loaded `TinyComputer` module.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod authority_denial_tests;
mod dns_rebinding_tests;
mod transport_classes_tests;
mod worker;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use tinybus::{Connection, broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
use tinycomputer_bus::{
    DesktopResponse,
    browser::names as browser_names,
    browser_egress::{
        self, BrowserEgressConsumeRequest, BrowserEgressDecision, BrowserEgressGrant,
        BrowserEgressTarget, BrowserEgressValidateRequest,
    },
    names,
};

#[derive(Clone)]
struct EgressHost {
    allowed: Arc<AtomicBool>,
    allowed_addresses: Vec<String>,
    resolved_address: Option<Arc<Mutex<String>>>,
    grant_generation: Option<u64>,
    observations: Arc<Mutex<Vec<String>>>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.BrowserEgressHost")]
impl EgressHost {
    async fn authorize(&self, request: BrowserEgressTarget) -> tinybus::Result<BrowserEgressGrant> {
        tokio::task::yield_now().await;
        let resolved_address = self
            .resolved_address
            .as_ref()
            .map(|resolved| resolved.lock().unwrap().clone());
        let allowed_address = self.allowed_addresses.iter().find(|address| {
            resolved_address
                .as_ref()
                .is_none_or(|resolved| *address == resolved)
                && address
                    .rsplit(':')
                    .next()
                    .and_then(|port| port.parse::<u16>().ok())
                    == Some(request.port)
        });
        let allow = self.allowed.load(Ordering::SeqCst)
            && request.hostname == "fixture.test"
            && allowed_address.is_some();
        self.observations.lock().unwrap().push(format!(
            "authorize:{}:{}:{}:{}",
            request.policy_id, request.expected_generation, request.hostname, request.port
        ));
        if let Some(resolved_address) = resolved_address {
            self.observations.lock().unwrap().push(format!(
                "resolve:{}:{}:{}",
                request.hostname, request.port, resolved_address
            ));
        }
        Ok(BrowserEgressGrant {
            generation: self.grant_generation.unwrap_or(request.expected_generation),
            authorization: if allow {
                "fixture-lease"
            } else {
                "denied-after-policy-change"
            }
            .into(),
            addresses: if allow {
                allowed_address.cloned().into_iter().collect()
            } else {
                Vec::new()
            },
        })
    }

    async fn consume(
        &self,
        request: BrowserEgressConsumeRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        tokio::task::yield_now().await;
        self.observations
            .lock()
            .unwrap()
            .push(format!("consume:{}", request.address));
        Ok(BrowserEgressDecision {
            allowed: self.allowed.load(Ordering::SeqCst)
                && self.allowed_addresses.contains(&request.address),
        })
    }

    async fn validate(
        &self,
        request: BrowserEgressValidateRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        tokio::task::yield_now().await;
        self.observations
            .lock()
            .unwrap()
            .push(format!("validate:{}", request.authorization));
        Ok(BrowserEgressDecision {
            allowed: self.allowed.load(Ordering::SeqCst)
                && request.authorization == "fixture-lease",
        })
    }
}

async fn serve_fixture(
    listener: tokio::net::TcpListener,
    expected: Vec<u8>,
    extra_connections: Arc<AtomicUsize>,
    served: tokio::sync::oneshot::Sender<()>,
    worker_done: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let (mut stream, _) = listener
        .accept()
        .await
        .expect("authorized Chromium socket arrives");
    let mut request = [0_u8; 4096];
    let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut request).await;
    assert!(String::from_utf8_lossy(&request).contains("GET /"));
    let response = [
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: ".as_slice(),
        expected.len().to_string().as_bytes(),
        b"\r\nConnection: close\r\n\r\n".as_slice(),
        expected.as_slice(),
    ]
    .concat();
    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, &response).await;
    let _ = served.send(());
    drop(stream);
    let mut worker_done = worker_done;
    while let Ok((mut stream, _)) = listener.accept().await {
        extra_connections.fetch_add(1, Ordering::SeqCst);
        let mut request = [0_u8; 4096];
        let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut request).await;
        if String::from_utf8_lossy(&request).contains("GET /worker-done ") {
            let _ = tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
            if let Some(done) = worker_done.take() {
                let _ = done.send(());
            }
        }
    }
}

fn successful_data(reply: &DesktopResponse) -> &serde_json::Value {
    assert!(reply.ok, "{} failed: {:?}", reply.command, reply.error);
    reply.data.as_ref().expect("successful response has data")
}

struct LiveBrowser {
    proxy: tinybus::Proxy,
    session: serde_json::Value,
    allowed: Arc<AtomicBool>,
    observations: Arc<Mutex<Vec<String>>>,
    broker_task: tokio::task::JoinHandle<tinybus::Result<()>>,
    _host_connection: Connection,
    _module_host: ModuleHost,
}

async fn start_browser_with_fixture_host(
    allowed_address: String,
    page_port: u16,
    page_path: &str,
    served: tokio::sync::oneshot::Receiver<()>,
) -> tinybus::Result<LiveBrowser> {
    start_browser_with_fixture_hosts(vec![allowed_address], page_port, page_path, served).await
}

async fn start_browser_with_fixture_hosts(
    allowed_addresses: Vec<String>,
    page_port: u16,
    page_path: &str,
    served: tokio::sync::oneshot::Receiver<()>,
) -> tinybus::Result<LiveBrowser> {
    start_browser_with_fixture_hosts_and_resolution(
        allowed_addresses,
        page_port,
        page_path,
        served,
        None,
    )
    .await
}

async fn start_browser_with_fixture_hosts_and_resolution(
    allowed_addresses: Vec<String>,
    page_port: u16,
    page_path: &str,
    served: tokio::sync::oneshot::Receiver<()>,
    resolved_address: Option<Arc<Mutex<String>>>,
) -> tinybus::Result<LiveBrowser> {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let allowed = Arc::new(AtomicBool::new(true));
    let observations = Arc::new(Mutex::new(Vec::new()));
    host_connection
        .serve_at(
            browser_egress::OBJECT_PATH.try_into()?,
            EgressHost {
                allowed: allowed.clone(),
                allowed_addresses,
                resolved_address,
                grant_generation: None,
                observations: observations.clone(),
            },
        )
        .await?;
    host_connection
        .request_name(browser_egress::INTERFACE)
        .await?;

    let module_host = ModuleHost::new(broker);
    let module = crate::linked_module().expect("linked module manifest is valid");
    module_host
        .attach_linked_with_config(
            module,
            serde_json::json!({
                "browser": {"egress": {"policy_id": "fixture-policy", "generation": 41}}
            }),
        )
        .expect("TinyComputer module loads through ModuleHost");
    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    let opened: DesktopResponse = proxy
        .call(
            browser_names::methods::OPEN_SESSION,
            (serde_json::json!({}),),
        )
        .await?;
    let session = successful_data(&opened)["id"].clone();
    assert_eq!(successful_data(&opened)["launched"], true);
    let url = format!("http://fixture.test:{page_port}{page_path}");
    let navigated: DesktopResponse = proxy
        .call(
            browser_names::methods::NAVIGATE,
            (serde_json::json!({"session": session, "url": url}),),
        )
        .await?;
    assert!(
        navigated.ok,
        "authorized fixture navigation failed: {:?}",
        navigated.error
    );
    tokio::time::timeout(Duration::from_secs(10), served)
        .await
        .expect("fixture page is served")
        .expect("fixture server reports the response");

    Ok(LiveBrowser {
        proxy,
        session,
        allowed,
        observations,
        broker_task,
        _host_connection: host_connection,
        _module_host: module_host,
    })
}

#[tokio::test]
async fn live_chromium_module_dials_only_the_host_returned_socket_and_rejects_changed_policy()
-> tinybus::Result<()> {
    let allowed_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = allowed_listener.local_addr()?.port();
    let denied_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let denied_address = denied_listener.local_addr()?;
    let denied_port = denied_address.port();
    assert_ne!(port, denied_port);
    let allowed_address = allowed_listener.local_addr()?.to_string();
    let extra_allowed_connections = Arc::new(AtomicUsize::new(0));
    let (served_tx, served_rx) = tokio::sync::oneshot::channel();
    let allowed_server = tokio::spawn(serve_fixture(
        allowed_listener,
        b"<!doctype html><link rel=\"icon\" href=\"data:,\"><title>allowed fixture</title>"
            .to_vec(),
        extra_allowed_connections.clone(),
        served_tx,
        None,
    ));

    let browser = start_browser_with_fixture_hosts(
        vec![allowed_address.clone(), denied_address.to_string()],
        port,
        "/allowed",
        served_rx,
    )
    .await?;

    let authorizations_before_policy_change = browser
        .observations
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with("authorize:fixture-policy:41:fixture.test:"))
        .count();
    let extra_connections_before_policy_change = extra_allowed_connections.load(Ordering::SeqCst);
    // Simulate a policy refresh: the host changes from granting the fixture
    // socket to denying requests for the same hostname.
    browser.allowed.store(false, Ordering::SeqCst);
    let denied_url = format!("http://fixture.test:{denied_port}/after-policy-change");
    let denied: DesktopResponse = browser
        .proxy
        .call(
            browser_names::methods::NAVIGATE,
            (serde_json::json!({"session": browser.session, "url": denied_url}),),
        )
        .await?;
    assert!(
        !denied.ok,
        "navigation after host policy denial unexpectedly succeeded"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), denied_listener.accept())
            .await
            .is_err(),
        "denied policy-change socket received a connection"
    );
    assert_eq!(
        extra_allowed_connections.load(Ordering::SeqCst),
        extra_connections_before_policy_change,
        "policy-denied navigation connected to the previously allowed socket"
    );

    {
        let calls = browser.observations.lock().unwrap();
        assert!(
            calls
                .iter()
                .any(|call| call == &format!("consume:{allowed_address}")),
            "host did not consume the exact returned socket: {calls:?}"
        );
        let authorizations_after_policy_change = calls
            .iter()
            .filter(|call| call.starts_with("authorize:fixture-policy:41:fixture.test:"))
            .count();
        assert!(
            authorizations_after_policy_change > authorizations_before_policy_change,
            "the post-change request did not reach the host authorizer: {calls:?}"
        );
        let denied_address = denied_listener.local_addr()?.to_string();
        assert!(
            !calls
                .iter()
                .any(|call| call == &format!("consume:{denied_address}")),
            "denied socket was consumed: {calls:?}"
        );
    }
    let closed: DesktopResponse = browser
        .proxy
        .call(
            browser_names::methods::CLOSE_SESSION,
            (serde_json::json!({"session": browser.session}),),
        )
        .await?;
    assert!(
        closed.ok,
        "browser session did not close: {:?}",
        closed.error
    );
    browser.broker_task.abort();
    allowed_server.abort();
    Ok(())
}
