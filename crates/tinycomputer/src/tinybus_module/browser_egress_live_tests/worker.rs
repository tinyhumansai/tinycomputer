use super::*;

#[tokio::test]
async fn live_chromium_module_routes_worker_egress_through_host_authorization()
-> tinybus::Result<()> {
    let allowed_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let page_port = allowed_listener.local_addr()?.port();
    let denied_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let denied_address = denied_listener.local_addr()?;
    let denied_port = denied_address.port();
    assert_ne!(page_port, denied_port);
    let html = format!(
        "<!doctype html><link rel=\"icon\" href=\"data:,\"><script>\
         const worker = new Worker(URL.createObjectURL(new Blob([\
         \"fetch('http://{denied_address}/worker').catch(() => {{}}).finally(() => fetch('http://fixture.test:{page_port}/worker-done').catch(() => {{}}))\"\
         ], {{ type: 'text/javascript' }})));</script>"
    )
    .into_bytes();
    let (served_tx, served_rx) = tokio::sync::oneshot::channel();
    let (worker_done_tx, worker_done_rx) = tokio::sync::oneshot::channel();
    let extra_allowed_connections = Arc::new(AtomicUsize::new(0));
    let allowed_server = tokio::spawn(serve_fixture(
        allowed_listener,
        html,
        extra_allowed_connections.clone(),
        served_tx,
        Some(worker_done_tx),
    ));

    let allowed_address = format!("127.0.0.1:{page_port}");
    let browser =
        start_browser_with_fixture_host(allowed_address, page_port, "/worker-fixture", served_rx)
            .await?;

    let worker_authorization = format!("authorize:fixture-policy:41:{denied_address}");
    let worker_authorized = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if browser
                .observations
                .lock()
                .unwrap()
                .iter()
                .any(|call| call == &worker_authorization)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if worker_authorized.is_err() {
        assert!(
            tokio::time::timeout(Duration::from_millis(500), denied_listener.accept())
                .await
                .is_err(),
            "worker request bypassed host authorization and connected directly"
        );
        panic!(
            "worker request did not reach host authorization: {:?}",
            browser.observations.lock().unwrap()
        );
    }
    tokio::select! {
        done = tokio::time::timeout(Duration::from_secs(10), worker_done_rx) => {
            done.expect("worker request completes within ten seconds")
                .expect("worker completion fixture receives the done request");
        }
        accepted = denied_listener.accept() => {
            let (stream, _) = accepted.expect("denied listener remains open");
            drop(stream);
            panic!("worker request connected directly to the denied private listener");
        }
    }
    assert_eq!(
        extra_allowed_connections.load(Ordering::SeqCst),
        1,
        "only the worker completion request reached the allowed fixture after navigation"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), denied_listener.accept())
            .await
            .is_err(),
        "worker request connected directly to the denied private listener after completion"
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
    allowed_server.abort();
    Ok(())
}
