use super::*;

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_subresource_never_reaches_a_denied_destination() {
    let denied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    denied.set_nonblocking(true).unwrap();
    let denied_address = denied.local_addr().unwrap();
    let denied_targets = Arc::new(Mutex::new(Vec::new()));
    let fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = fixture.local_addr().unwrap();
    let body = format!(
        "<img src=\"http://127.0.0.1:{}/private.png\">",
        denied_address.port()
    );
    let fixture_server = tokio::spawn(async move {
        let (mut socket, _) = fixture.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let mut browser = EnforcedAgentBrowser::new(Arc::new(RejectAllButFixture {
        address,
        authorizations: Arc::new(AtomicUsize::new(0)),
        denied_targets: denied_targets.clone(),
    }))
    .open("live-egress-subresource");
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
    wait_for_denied_target(&denied_targets, denied_address.port()).await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(navigation["success"], true, "{navigation}");
    fixture_server.await.unwrap();
    assert_eq!(
        denied.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the denied subresource destination must never receive a connection"
    );
}

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_worker_fetch_never_reaches_a_denied_destination() {
    let denied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    denied.set_nonblocking(true).unwrap();
    let denied_address = denied.local_addr().unwrap();
    let denied_targets = Arc::new(Mutex::new(Vec::new()));
    let fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = fixture.local_addr().unwrap();
    let html = "<script>new Worker('/worker.js')</script><p>worker fixture</p>".to_string();
    let worker = format!(
        "fetch('http://127.0.0.1:{}/private').catch(() => {{}});",
        denied_address.port()
    );
    let fixture_server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), fixture.accept())
                .await
                .expect("Chromium requests the page and worker script")
                .unwrap();
            let mut request = [0; 4096];
            let size = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            let (status, content_type, body, worker_script) = if request.contains("GET /worker.js")
            {
                ("200 OK", "application/javascript", worker.as_str(), true)
            } else if request.contains("GET / ") {
                ("200 OK", "text/html", html.as_str(), false)
            } else {
                ("404 Not Found", "text/plain", "not found", false)
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            if worker_script {
                break;
            }
        }
    });
    let mut browser = EnforcedAgentBrowser::new(Arc::new(RejectAllButFixture {
        address,
        authorizations: Arc::new(AtomicUsize::new(0)),
        denied_targets: denied_targets.clone(),
    }))
    .open("live-egress-worker");
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
    wait_for_denied_target(&denied_targets, denied_address.port()).await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(navigation["success"], true, "{navigation}");
    fixture_server.await.unwrap();
    assert_eq!(
        denied.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the denied worker fetch destination must never receive a connection"
    );
}

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_redirect_never_reaches_a_denied_destination() {
    let denied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    denied.set_nonblocking(true).unwrap();
    let denied_address = denied.local_addr().unwrap();
    let denied_targets = Arc::new(Mutex::new(Vec::new()));
    let fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = fixture.local_addr().unwrap();
    let fixture_server = tokio::spawn(async move {
        let (mut socket, _) = fixture.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    denied_address.port()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let mut browser = EnforcedAgentBrowser::new(Arc::new(RejectAllButFixture {
        address,
        authorizations: Arc::new(AtomicUsize::new(0)),
        denied_targets: denied_targets.clone(),
    }))
    .open("live-egress-redirect");
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
    wait_for_denied_target(&denied_targets, denied_address.port()).await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(navigation["success"], false, "{navigation}");
    fixture_server.await.unwrap();
    assert_eq!(
        denied.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the denied redirect destination must never receive a connection"
    );
}

#[tokio::test]
#[cfg(live_browser_tests)]
async fn live_chromium_websocket_never_reaches_a_denied_destination() {
    let denied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    denied.set_nonblocking(true).unwrap();
    let denied_address = denied.local_addr().unwrap();
    let denied_targets = Arc::new(Mutex::new(Vec::new()));
    let fixture = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = fixture.local_addr().unwrap();
    let body = format!(
        "<script>new WebSocket('ws://127.0.0.1:{}/private')</script>",
        denied_address.port()
    );
    let fixture_server = tokio::spawn(async move {
        let (mut socket, _) = fixture.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let mut browser = EnforcedAgentBrowser::new(Arc::new(RejectAllButFixture {
        address,
        authorizations: Arc::new(AtomicUsize::new(0)),
        denied_targets: denied_targets.clone(),
    }))
    .open("live-egress-websocket");
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
    wait_for_denied_target(&denied_targets, denied_address.port()).await;
    browser
        .execute(json!({"id":"close","action":"close"}))
        .await;
    assert_eq!(navigation["success"], true, "{navigation}");
    fixture_server.await.unwrap();
    assert_eq!(
        denied.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the denied WebSocket destination must never receive a connection"
    );
}
