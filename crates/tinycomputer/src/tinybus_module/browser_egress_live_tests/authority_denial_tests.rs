//! Live Chromium checks that missing and stale host authority denies dialing.

use super::*;

struct DenialBrowser {
    proxy: tinybus::Proxy,
    session: serde_json::Value,
    broker_task: tokio::task::JoinHandle<tinybus::Result<()>>,
    _host_connection: Option<Connection>,
    _module_host: ModuleHost,
}

async fn start_denial_browser(
    host: Option<EgressHost>,
    policy_id: &str,
    generation: u64,
) -> tinybus::Result<DenialBrowser> {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host_connection = if let Some(host) = host {
        let connection = Connection::connect(bus.connect().await?).await?;
        connection
            .serve_at(browser_egress::OBJECT_PATH.try_into()?, host)
            .await?;
        connection.request_name(browser_egress::INTERFACE).await?;
        Some(connection)
    } else {
        None
    };

    let module_host = ModuleHost::new(broker);
    let module = crate::linked_module().expect("linked module manifest is valid");
    module_host
        .attach_linked_with_config(
            module,
            serde_json::json!({
                "browser": {"egress": {"policy_id": policy_id, "generation": generation}}
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

    Ok(DenialBrowser {
        proxy,
        session,
        broker_task,
        _host_connection: host_connection,
        _module_host: module_host,
    })
}

async fn close_denial_browser(browser: DenialBrowser) -> tinybus::Result<()> {
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
    let _ = browser.broker_task.await;
    Ok(())
}

#[tokio::test]
async fn live_chromium_denies_loopback_when_egress_authority_is_missing() -> tinybus::Result<()> {
    let denied_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = denied_listener.local_addr()?.port();
    let browser = start_denial_browser(None, "missing-authority", 7).await?;
    let url = format!("http://127.0.0.1:{port}/must-not-connect");
    let denied: DesktopResponse = browser
        .proxy
        .call(
            browser_names::methods::NAVIGATE,
            (serde_json::json!({"session": browser.session, "url": url}),),
        )
        .await?;

    assert!(
        !denied.ok,
        "navigation without an egress authority unexpectedly succeeded"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), denied_listener.accept())
            .await
            .is_err(),
        "loopback destination received a connection without an egress authority"
    );
    close_denial_browser(browser).await?;
    Ok(())
}

#[tokio::test]
async fn live_chromium_denies_stale_egress_generation_before_dialing() -> tinybus::Result<()> {
    let denied_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let allowed_address = denied_listener.local_addr()?.to_string();
    let port = denied_listener.local_addr()?.port();
    let observations = Arc::new(Mutex::new(Vec::new()));
    let browser = start_denial_browser(
        Some(EgressHost {
            allowed: Arc::new(AtomicBool::new(true)),
            allowed_addresses: vec![allowed_address],
            resolved_address: None,
            grant_generation: Some(40),
            observations: observations.clone(),
        }),
        "stale-generation",
        41,
    )
    .await?;
    let url = format!("http://fixture.test:{port}/stale-generation");
    let denied: DesktopResponse = browser
        .proxy
        .call(
            browser_names::methods::NAVIGATE,
            (serde_json::json!({"session": browser.session, "url": url}),),
        )
        .await?;

    assert!(
        !denied.ok,
        "navigation with a stale egress generation unexpectedly succeeded"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), denied_listener.accept())
            .await
            .is_err(),
        "stale-generation socket received a connection"
    );
    close_denial_browser(browser).await?;

    let calls = observations.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("authorize:stale-generation:41:fixture.test:")),
        "the live module did not ask the host for authorization: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("consume:")),
        "stale grant reached socket consumption: {calls:?}"
    );
    Ok(())
}
