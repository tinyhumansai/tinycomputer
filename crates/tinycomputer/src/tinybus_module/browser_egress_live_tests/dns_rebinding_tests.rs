//! Live Chromium coverage for a destination rebound between authorizations.

use super::*;

#[tokio::test]
async fn live_chromium_module_never_dials_a_rebound_private_socket() -> tinybus::Result<()> {
    let allowed_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = allowed_listener.local_addr()?.port();
    let rebound_listener =
        tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await?;
    let allowed_address = allowed_listener.local_addr()?.to_string();
    let rebound_address = rebound_listener.local_addr()?.to_string();
    assert_eq!(port, rebound_listener.local_addr()?.port());
    assert_ne!(allowed_address, rebound_address);
    let extra_connections = Arc::new(AtomicUsize::new(0));
    let (served_tx, served_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_fixture(
        allowed_listener,
        b"<!doctype html><title>allowed fixture</title>".to_vec(),
        extra_connections.clone(),
        served_tx,
        None,
    ));
    let resolved_address = Arc::new(Mutex::new(allowed_address.clone()));
    let browser = start_browser_with_fixture_hosts_and_resolution(
        vec![allowed_address.clone()],
        port,
        "/before-rebind",
        served_rx,
        Some(resolved_address.clone()),
    )
    .await?;

    let before_rebind_authorizations = browser
        .observations
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with("authorize:fixture-policy:41:fixture.test:"))
        .count();
    let connections_before_rebind = extra_connections.load(Ordering::SeqCst);
    *resolved_address.lock().unwrap() = rebound_address.clone();
    let denied: DesktopResponse = browser
        .proxy
        .call(
            browser_names::methods::NAVIGATE,
            (serde_json::json!({
                "session": browser.session,
                "url": format!("http://fixture.test:{port}/after-rebind")
            }),),
        )
        .await?;

    assert!(!denied.ok, "navigation to the rebound address was allowed");
    assert!(
        tokio::time::timeout(Duration::from_millis(500), rebound_listener.accept())
            .await
            .is_err(),
        "the rebound private socket received a connection"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), async {
            while extra_connections.load(Ordering::SeqCst) == connections_before_rebind {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "the request reached the previously allowed socket after rebinding"
    );

    let calls = browser.observations.lock().unwrap().clone();
    assert!(
        calls
            .iter()
            .filter(|call| call.starts_with("authorize:fixture-policy:41:fixture.test:"))
            .count()
            > before_rebind_authorizations,
        "the rebound request did not reach host authorization: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|call| call == &format!("resolve:fixture.test:{port}:{rebound_address}")),
        "the controlled resolver did not return the rebound address: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call == &format!("consume:{rebound_address}")),
        "the rebound socket was consumed: {calls:?}"
    );
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
    server.abort();
    let _ = server.await;
    Ok(())
}
