//! Live Chromium checks that browser transport classes use host authorization.

use super::*;

async fn serve_once(
    listener: tokio::net::TcpListener,
    expected_path: &'static str,
    response: String,
    served: Option<tokio::sync::oneshot::Sender<()>>,
    completed: tokio::sync::mpsc::Sender<&'static str>,
) {
    let (mut stream, _) = listener
        .accept()
        .await
        .expect("authorized Chromium socket arrives");
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let Ok(Ok(read)) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::io::AsyncReadExt::read(&mut stream, &mut chunk),
        )
        .await
        else {
            panic!("browser request timed out");
        };
        assert!(
            read != 0,
            "browser closed before sending a complete request"
        );
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&request);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .expect("browser request has a target");
    let path =
        url::Url::parse(target).map_or_else(|_| target.to_owned(), |url| url.path().to_owned());
    assert_eq!(path, expected_path);
    if expected_path == "/socket" {
        let header = |name: &str| {
            request.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case(name).then(|| value.trim())
            })
        };
        assert_eq!(
            header("Upgrade").map(str::to_ascii_lowercase).as_deref(),
            Some("websocket"),
            "socket fixture only accepts a WebSocket upgrade request: {request}"
        );
        assert!(
            header("Connection").is_some_and(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            }),
            "socket fixture only accepts Connection: Upgrade: {request}"
        );
        assert!(
            header("Sec-WebSocket-Key").is_some_and(|value| !value.is_empty()),
            "socket fixture only accepts a request with Sec-WebSocket-Key: {request}"
        );
    }
    tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
        .await
        .expect("fixture response is written");
    if let Some(served) = served {
        let _ = served.send(());
    }
    let _ = completed.send(expected_path).await;
}

fn fixture_response(status: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\n{headers}Connection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn live_chromium_redirect_subresource_and_websocket_requests_use_host_authorization()
-> tinybus::Result<()> {
    let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let page_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let pixel_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let socket_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect_address = redirect_listener.local_addr()?.to_string();
    let page_address = page_listener.local_addr()?.to_string();
    let pixel_address = pixel_listener.local_addr()?.to_string();
    let socket_address = socket_listener.local_addr()?.to_string();
    let redirect_port = redirect_listener.local_addr()?.port();
    let page_port = page_listener.local_addr()?.port();
    let pixel_port = pixel_listener.local_addr()?.port();
    let socket_port = socket_listener.local_addr()?.port();

    let (served_tx, served_rx) = tokio::sync::oneshot::channel();
    let (completed_tx, mut completed_rx) = tokio::sync::mpsc::channel(4);
    let redirect = tokio::spawn(serve_once(
        redirect_listener,
        "/redirect",
        fixture_response(
            "302 Found",
            &format!("Location: http://fixture.test:{page_port}/page\r\n"),
            "",
        ),
        None,
        completed_tx.clone(),
    ));
    let html = format!(
        "<!doctype html><link rel=\"icon\" href=\"data:,\"><img src=\"http://fixture.test:{pixel_port}/pixel\"><script>new WebSocket(\"ws://fixture.test:{socket_port}/socket\")</script>"
    );
    let page = tokio::spawn(serve_once(
        page_listener,
        "/page",
        fixture_response("200 OK", "Content-Type: text/html\r\n", &html),
        Some(served_tx),
        completed_tx.clone(),
    ));
    let pixel = tokio::spawn(serve_once(
        pixel_listener,
        "/pixel",
        fixture_response("204 No Content", "", ""),
        None,
        completed_tx.clone(),
    ));
    let socket = tokio::spawn(serve_once(
        socket_listener,
        "/socket",
        fixture_response("403 Forbidden", "", ""),
        None,
        completed_tx.clone(),
    ));
    drop(completed_tx);

    let browser = start_browser_with_fixture_hosts(
        vec![
            redirect_address.clone(),
            page_address.clone(),
            pixel_address.clone(),
            socket_address.clone(),
        ],
        redirect_port,
        "/redirect",
        served_rx,
    )
    .await?;
    let mut paths = Vec::new();
    for _ in 0..4 {
        paths.push(
            tokio::time::timeout(Duration::from_secs(10), completed_rx.recv())
                .await
                .expect("each transport request reaches its fixture listener")
                .expect("fixture reports every observed request"),
        );
    }
    paths.sort_unstable();
    assert_eq!(paths, ["/page", "/pixel", "/redirect", "/socket"]);

    {
        let observations = browser.observations.lock().unwrap();
        for port in [redirect_port, page_port, pixel_port, socket_port] {
            let authorization = format!("authorize:fixture-policy:41:fixture.test:{port}");
            assert!(
                observations.iter().any(|call| call == &authorization),
                "transport destination on port {port} did not reach host authorization: {observations:?}"
            );
        }
        for address in [
            &redirect_address,
            &page_address,
            &pixel_address,
            &socket_address,
        ] {
            let consumption = format!("consume:{address}");
            assert!(
                observations.iter().any(|call| call == &consumption),
                "transport destination {address} did not consume its one-use grant: {observations:?}"
            );
        }
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
    redirect.abort();
    page.abort();
    pixel.abort();
    socket.abort();
    Ok(())
}
