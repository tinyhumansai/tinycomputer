use super::*;

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_loopback_navigation_uses_the_authenticated_session_proxy() {
    let fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = fixture.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = fixture.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 22\r\nConnection: close\r\n\r\n<h1>proxy fixture</h1>",
            )
            .await
            .unwrap();
    });
    let authorizations = Arc::new(AtomicUsize::new(0));
    let mut browser = EnforcedAgentBrowser::new(Arc::new(RejectAllButFixture {
        address,
        authorizations: authorizations.clone(),
        denied_targets: Arc::new(Mutex::new(Vec::new())),
    }))
    .open("live-egress");
    let launch = browser
        .execute(json!({"id":"launch","action":"launch","headless":true,"args":[]}))
        .await;
    assert_eq!(launch["success"], true, "{launch}");
    let navigation = browser
        .execute(json!({
            "id":"navigate",
            "action":"navigate",
            "url":format!("http://127.0.0.1:{}/", address.port())
        }))
        .await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(
        navigation["success"],
        true,
        "{navigation}; proxy authorizations: {}",
        authorizations.load(Ordering::SeqCst)
    );
    server.await.unwrap();
}

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_dials_only_the_exact_host_authorized_socket() {
    let allowed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let allowed_address = allowed.local_addr().unwrap();
    let denied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    denied.set_nonblocking(true).unwrap();
    let denied_address = denied.local_addr().unwrap();
    assert_ne!(allowed_address.port(), denied_address.port());

    let allowed_server = tokio::spawn(async move {
        let (mut socket, _) = allowed.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 30\r\nConnection: close\r\n\r\n<h1>pinned socket fixture</h1>",
            )
            .await
            .unwrap();
    });
    let mut browser = EnforcedAgentBrowser::new(Arc::new(AuthorizeExactSocket(allowed_address)))
        .open("live-egress-socket-pinning");
    let launch = browser
        .execute(json!({"id":"launch","action":"launch","headless":true,"args":[]}))
        .await;
    assert_eq!(launch["success"], true, "{launch}");
    let navigation = browser
        .execute(json!({
            "id":"navigate",
            "action":"navigate",
            "url":format!("http://{denied_address}/")
        }))
        .await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;

    assert_eq!(navigation["success"], true, "{navigation}");
    allowed_server.await.unwrap();
    assert_eq!(
        denied.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "Chromium must reach only the exact socket returned by the host grant"
    );
}

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_uses_a_host_permitted_cross_origin_subresource_socket() {
    let cdn_fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cdn_address = cdn_fixture.local_addr().unwrap();
    let page_fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let page_address = page_fixture.local_addr().unwrap();
    let html = format!(
        "<img src=\"http://cdn.fixture.test:{}/asset.png\"><p>page fixture</p>",
        cdn_address.port()
    );
    let page_server = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), page_fixture.accept())
            .await
            .expect("Chromium requests the permitted page")
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
            html.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let cdn_server = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), cdn_fixture.accept())
            .await
            .expect("Chromium requests the permitted cross-origin asset")
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let addresses = HashMap::from([
        ("127.0.0.1".to_owned(), page_address),
        ("cdn.fixture.test".to_owned(), cdn_address),
    ]);
    let mut browser = EnforcedAgentBrowser::new(Arc::new(AllowFixtureHosts {
        addresses,
        authorizations: Arc::new(AtomicUsize::new(0)),
    }))
    .open("live-egress-permitted-cross-origin");
    let launch = browser
        .execute(json!({"id":"launch","action":"launch","headless":true,"args":[]}))
        .await;
    assert_eq!(launch["success"], true, "{launch}");
    let navigation = browser
        .execute(json!({
            "id":"navigate",
            "action":"navigate",
            "url":format!("http://127.0.0.1:{}/", page_address.port())
        }))
        .await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(navigation["success"], true, "{navigation}");
    page_server.await.unwrap();
    cdn_server.await.unwrap();
}
