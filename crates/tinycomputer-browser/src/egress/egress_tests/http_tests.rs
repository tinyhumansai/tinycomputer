//! HTTP request, body, authentication, and refusal tests.

use super::simulator::*;
use crate::EgressProxy;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[tokio::test]
async fn denied_destination_never_connects_to_the_fixture() {
    let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let address = destination.local_addr().unwrap();
    let proxy = EgressProxy::bind(
        Arc::new(FixtureAuthorizer {
            address,
            allow: false,
            consumed: Arc::new(AtomicBool::new(false)),
        }),
        "denied-session".into(),
        FIXTURE_PASSWORD.into(),
    )
    .await
    .unwrap();
    let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT metadata.test:{} HTTP/1.1\r\nHost: metadata.test:{}\r\nProxy-Authorization: {}\r\n\r\n",
                address.port(),
                address.port(),
                proxy_authorization()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = [0; 128];
    let n = client.read(&mut response).await.unwrap();
    assert!(
        std::str::from_utf8(&response[..n])
            .unwrap()
            .contains("403 Forbidden")
    );
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}
#[tokio::test]
async fn unauthenticated_local_clients_cannot_use_the_session_policy_broker() {
    let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let address = destination.local_addr().unwrap();
    let proxy = EgressProxy::bind(
        Arc::new(FixtureAuthorizer {
            address,
            allow: true,
            consumed: Arc::new(AtomicBool::new(false)),
        }),
        "protected-session".into(),
        FIXTURE_PASSWORD.into(),
    )
    .await
    .unwrap();
    let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT public.example:{} HTTP/1.1\r\nHost: public.example:{}\r\n\r\n",
                address.port(),
                address.port()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = [0; 256];
    let n = client.read(&mut response).await.unwrap();
    let response = std::str::from_utf8(&response[..n]).unwrap();
    assert!(response.contains("407 Proxy Authentication Required"));
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn approved_http_request_is_rewritten_and_forwarded_to_approved_fixture() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = destination.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = destination.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 512];
        loop {
            let n = socket.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..n]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = std::str::from_utf8(&request).unwrap();
        assert!(request.starts_with("GET /control?ready=1 HTTP/1.1\r\n"));
        assert!(request.contains(&format!("Host: control.test:{}\r\n", address.port())));
        assert!(request.contains("Connection: close\r\n"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let authorizer = Arc::new(FixtureAuthorizer {
        address,
        allow: true,
        consumed: Arc::new(AtomicBool::new(false)),
    });
    let proxy = EgressProxy::bind(
        authorizer.clone(),
        "http-session".into(),
        FIXTURE_PASSWORD.into(),
    )
    .await
    .unwrap();
    let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "GET http://control.test:{}/control?ready=1 HTTP/1.1\r\nHost: control.test:{}\r\nProxy-Authorization: {}\r\nConnection: keep-alive\r\n\r\n",
                address.port(),
                address.port(),
                proxy_authorization()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(
        std::str::from_utf8(&response)
            .unwrap()
            .ends_with("\r\n\r\nok")
    );
    assert!(authorizer.consumed.load(Ordering::SeqCst));
    server.await.unwrap();
}

#[tokio::test]
async fn hop_by_hop_headers_are_not_forwarded() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = destination.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = destination.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 512];
        loop {
            let n = socket.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..n]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = std::str::from_utf8(&request).unwrap().to_ascii_lowercase();
        for dropped in ["x-internal:", "keep-alive:", "te:", "upgrade:"] {
            assert!(
                !request.contains(&format!("\r\n{dropped}")),
                "{dropped} was forwarded"
            );
        }
        assert!(request.contains("x-kept: yes\r\n"));
        assert!(request.contains("connection: close\r\n"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let authorizer = Arc::new(FixtureAuthorizer {
        address,
        allow: true,
        consumed: Arc::new(AtomicBool::new(false)),
    });
    let proxy = EgressProxy::bind(authorizer, "hop-session".into(), FIXTURE_PASSWORD.into())
        .await
        .unwrap();
    let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "GET http://control.test:{}/ HTTP/1.1\r\nHost: control.test:{}\r\nProxy-Authorization: {}\r\nConnection: keep-alive, X-Internal\r\nX-Internal: 1\r\nKeep-Alive: timeout=5\r\nTE: trailers\r\nUpgrade: h2c\r\nX-Kept: yes\r\n\r\n",
                address.port(),
                address.port(),
                proxy_authorization()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(
        std::str::from_utf8(&response)
            .unwrap()
            .ends_with("\r\n\r\nok")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn duplicate_lengths_and_pipelined_authorities_are_refused_before_dial() {
    let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let address = destination.local_addr().unwrap();
    for suffix in [
        "Content-Length: 0\r\nContent-Length: 0\r\n\r\n",
        "Content-Length: 0\r\n\r\nGET http://other.test/ HTTP/1.1\r\nHost: other.test\r\n\r\n",
    ] {
        let proxy = EgressProxy::bind(
            Arc::new(FixtureAuthorizer {
                address,
                allow: true,
                consumed: Arc::new(AtomicBool::new(false)),
            }),
            "malformed-session".into(),
            FIXTURE_PASSWORD.into(),
        )
        .await
        .unwrap();
        let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
            .await
            .unwrap();
        client
            .write_all(
                format!(
                    "GET http://one.test:{}/ HTTP/1.1\r\nHost: one.test:{}\r\nProxy-Authorization: {}\r\n{}",
                    address.port(),
                    address.port(),
                    proxy_authorization(),
                    suffix
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = [0; 128];
        let n = client.read(&mut response).await.unwrap();
        assert!(
            std::str::from_utf8(&response[..n])
                .unwrap()
                .contains("403 Forbidden")
        );
        assert_eq!(
            destination.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
}
