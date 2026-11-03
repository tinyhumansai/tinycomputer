use super::*;

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_dns_rebinding_never_reaches_the_rebound_private_socket() {
    let allowed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = allowed_listener.local_addr().unwrap().port();
    let private = std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).unwrap();
    private.set_nonblocking(true).unwrap();
    let private_address = private.local_addr().unwrap();
    let allowed_address = allowed_listener.local_addr().unwrap();
    let extra_allowed_connections = Arc::new(AtomicUsize::new(0));
    let observed_addresses = Arc::new(Mutex::new(Vec::new()));
    let extra_connections = extra_allowed_connections.clone();
    let body = format!("<img src=\"http://fixture.test:{port}/after-rebind.png\"><p>fixture</p>");
    let allowed_server = tokio::spawn(async move {
        let (mut socket, _) =
            tokio::time::timeout(Duration::from_secs(10), allowed_listener.accept())
                .await
                .expect("authorized navigation reaches the allowed fixture")
                .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        drop(socket);

        while let Ok((socket, _)) = allowed_listener.accept().await {
            extra_connections.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    let denied_targets = Arc::new(Mutex::new(Vec::new()));
    let mut browser = EnforcedAgentBrowser::new(Arc::new(RebindingAuthorizer {
        allowed_address,
        private_address,
        authorizations: AtomicUsize::new(0),
        denied_targets: denied_targets.clone(),
        observed_addresses: observed_addresses.clone(),
    }))
    .open("live-egress-dns-rebinding");
    let launch = browser
        .execute(json!({"id":"launch","action":"launch","headless":true,"args":[]}))
        .await;
    assert_eq!(launch["success"], true, "{launch}");
    let navigation = browser
        .execute(json!({
            "id":"navigate",
            "action":"navigate",
            "url":format!("http://fixture.test:{port}/")
        }))
        .await;
    wait_for_denied_target(&denied_targets, port).await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(navigation["success"], true, "{navigation}");
    let observed_addresses = observed_addresses.lock().unwrap().clone();
    assert_eq!(
        observed_addresses.first(),
        Some(&allowed_address),
        "the controlled resolver first returns the permitted address"
    );
    assert!(
        observed_addresses.len() >= 2
            && observed_addresses[1..]
                .iter()
                .all(|address| *address == private_address),
        "the controlled resolver must rebind the name to the private address: {observed_addresses:?}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), async {
            while extra_allowed_connections.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "the denied rebound request reached the previously allowed socket"
    );
    assert_eq!(
        private.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the rebound private destination must never receive a connection"
    );
    allowed_server.abort();
    let _ = allowed_server.await;
}
