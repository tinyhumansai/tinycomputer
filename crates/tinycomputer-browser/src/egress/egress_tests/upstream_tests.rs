//! Destinations routed through an upstream proxy, end to end through the
//! egress proxy: policy still runs for the destination, the upstream proxy's
//! own dial is granted by the host, and other hosts are unaffected.

use super::simulator::*;
use crate::{BrowserDialTarget, DialGrant, EgressAuthorizer, EgressProxy, UpstreamProxy};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const PROXY_HOST: &str = "gate.proxy.test";

/// Grants each known hostname its one address and records every consume and
/// validation, so a test can see which dials the host was asked for.
#[derive(Default)]
struct RoutingAuthorizer {
    hosts: HashMap<String, Vec<SocketAddr>>,
    authorized: Mutex<Vec<String>>,
    consumed: Mutex<Vec<(String, SocketAddr)>>,
    validated: Mutex<Vec<String>>,
}

impl EgressAuthorizer for RoutingAuthorizer {
    fn authorize(
        &self,
        target: BrowserDialTarget,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<DialGrant>> + Send>> {
        self.authorized.lock().unwrap().push(format!(
            "{}://{}:{}",
            target.scheme, target.hostname, target.port
        ));
        let grant = self.hosts.get(&target.hostname).map(|addresses| DialGrant {
            lease: format!("lease-{}", target.hostname),
            addresses: addresses.clone(),
        });
        Box::pin(async move {
            grant.ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
        })
    }

    fn consume(
        &self,
        target: BrowserDialTarget,
        grant: DialGrant,
        address: SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
        let allowed = grant.addresses.contains(&address);
        if allowed {
            self.consumed
                .lock()
                .unwrap()
                .push((target.hostname, address));
        }
        Box::pin(async move {
            if allowed {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "wrong address",
                ))
            }
        })
    }

    fn validate(
        &self,
        lease: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<bool>> + Send>> {
        self.validated.lock().unwrap().push(lease);
        Box::pin(async { Ok(true) })
    }
}

fn upstream_proxy(port: u16) -> UpstreamProxy {
    UpstreamProxy::new(
        &format!("http://resi-user:resi%20pass@{PROXY_HOST}:{port}"),
        &["reddit.com".to_owned()],
    )
    .unwrap()
}

fn expected_proxy_authorization() -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("resi-user:resi pass")
    )
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

/// A destination listener that must never be dialed.
fn untouched_destination() -> (std::net::TcpListener, SocketAddr) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    (listener, address)
}

async fn bind(authorizer: Arc<RoutingAuthorizer>, upstream: UpstreamProxy) -> EgressProxy {
    EgressProxy::bind_with_upstreams(
        authorizer,
        "routed-session".into(),
        FIXTURE_PASSWORD.into(),
        Arc::from([upstream]),
    )
    .await
    .unwrap()
}

async fn send(proxy: &EgressProxy, request: &str) -> TcpStream {
    let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
        .await
        .unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    client
}

fn connect_request(authority: &str) -> String {
    format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: {}\r\n\r\n",
        proxy_authorization()
    )
}

#[tokio::test]
async fn a_routed_connect_tunnels_through_the_upstream_proxy_with_its_credentials() {
    let (destination, destination_address) = untouched_destination();
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let authorizer = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([
            ("www.reddit.com".to_owned(), vec![destination_address]),
            (PROXY_HOST.to_owned(), vec![upstream_address]),
        ]),
        ..RoutingAuthorizer::default()
    });
    let proxy = bind(authorizer.clone(), upstream_proxy(upstream_address.port())).await;

    let mut client = send(&proxy, &connect_request("www.reddit.com:443")).await;
    let (mut tunnel, _) = upstream.accept().await.unwrap();
    let head = read_head(&mut tunnel).await;
    assert_eq!(
        head,
        format!(
            "CONNECT {destination_address} HTTP/1.1\r\nHost: {destination_address}\r\nProxy-Authorization: {}\r\n\r\n",
            expected_proxy_authorization()
        )
    );
    tunnel
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await
        .unwrap();
    let mut response = [0u8; 64];
    let n = client.read(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response[..n]).contains("200 Connection Established"));
    client.write_all(b"ping").await.unwrap();
    let mut payload = [0u8; 4];
    tunnel.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"ping");
    tunnel.write_all(b"pong").await.unwrap();
    client.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"pong");

    assert_eq!(
        *authorizer.authorized.lock().unwrap(),
        vec![
            "https://www.reddit.com:443".to_owned(),
            format!("http://{PROXY_HOST}:{}", upstream_address.port()),
        ]
    );
    assert_eq!(
        *authorizer.consumed.lock().unwrap(),
        vec![
            ("www.reddit.com".to_owned(), destination_address),
            (PROXY_HOST.to_owned(), upstream_address),
        ]
    );
    let validated = authorizer.validated.lock().unwrap().clone();
    assert!(validated.contains(&"lease-www.reddit.com".to_owned()));
    assert!(validated.contains(&format!("lease-{PROXY_HOST}")));
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn a_plain_http_request_to_a_routed_host_also_goes_through_a_tunnel() {
    let (_destination, destination_address) = untouched_destination();
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let authorizer = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([
            ("reddit.com".to_owned(), vec![destination_address]),
            (PROXY_HOST.to_owned(), vec![upstream_address]),
        ]),
        ..RoutingAuthorizer::default()
    });
    let proxy = bind(authorizer, upstream_proxy(upstream_address.port())).await;

    let mut client = send(
        &proxy,
        &format!(
            "GET http://reddit.com/r/x?y=1 HTTP/1.1\r\nHost: reddit.com\r\nProxy-Authorization: {}\r\n\r\n",
            proxy_authorization()
        ),
    )
    .await;
    let (mut tunnel, _) = upstream.accept().await.unwrap();
    let head = read_head(&mut tunnel).await;
    assert!(
        head.starts_with("CONNECT reddit.com:80 HTTP/1.1\r\n"),
        "{head}"
    );
    tunnel
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await
        .unwrap();
    let forwarded = read_head(&mut tunnel).await;
    assert!(
        forwarded.starts_with("GET /r/x?y=1 HTTP/1.1\r\n"),
        "{forwarded}"
    );
    assert!(
        !forwarded
            .to_ascii_lowercase()
            .contains("proxy-authorization")
    );
    tunnel
        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    drop(tunnel);
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 204"));
}

#[tokio::test]
async fn a_host_outside_the_routed_domains_is_dialed_directly() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_address = destination.local_addr().unwrap();
    let (upstream, upstream_address) = untouched_destination();
    let authorizer = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([
            ("example.test".to_owned(), vec![destination_address]),
            (PROXY_HOST.to_owned(), vec![upstream_address]),
        ]),
        ..RoutingAuthorizer::default()
    });
    let proxy = bind(authorizer.clone(), upstream_proxy(upstream_address.port())).await;

    let mut client = send(
        &proxy,
        &connect_request(&format!("example.test:{}", destination_address.port())),
    )
    .await;
    let (mut direct, _) = destination.accept().await.unwrap();
    let mut response = [0u8; 64];
    let n = client.read(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response[..n]).contains("200 Connection Established"));
    client.write_all(b"ping").await.unwrap();
    let mut payload = [0u8; 4];
    direct.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"ping");
    assert_eq!(authorizer.authorized.lock().unwrap().len(), 1);
    assert_eq!(
        upstream.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn rejected_proxy_credentials_answer_bad_gateway() {
    let (_destination, destination_address) = untouched_destination();
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let authorizer = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([
            ("reddit.com".to_owned(), vec![destination_address]),
            (PROXY_HOST.to_owned(), vec![upstream_address]),
        ]),
        ..RoutingAuthorizer::default()
    });
    let proxy = bind(authorizer, upstream_proxy(upstream_address.port())).await;

    let mut client = send(&proxy, &connect_request("reddit.com:443")).await;
    let (mut tunnel, _) = upstream.accept().await.unwrap();
    read_head(&mut tunnel).await;
    tunnel
        .write_all(
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\n\r\n",
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 502 Bad Gateway"),
        "{response}"
    );
    assert!(!response.contains("Proxy-Authenticate"));
}

#[tokio::test]
async fn a_routed_destination_is_refused_when_the_host_denies_it_or_the_proxy() {
    let (upstream, upstream_address) = untouched_destination();
    let (_destination, destination_address) = untouched_destination();
    // The destination is denied: the upstream proxy is never asked for.
    let denies_destination = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([(PROXY_HOST.to_owned(), vec![upstream_address])]),
        ..RoutingAuthorizer::default()
    });
    // The proxy's own dial is denied: nothing is consumed or dialed.
    let denies_proxy = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([("reddit.com".to_owned(), vec![destination_address])]),
        ..RoutingAuthorizer::default()
    });
    for (authorizer, authorizations) in [(denies_destination, 1), (denies_proxy, 2)] {
        let proxy = bind(authorizer.clone(), upstream_proxy(upstream_address.port())).await;
        let mut client = send(&proxy, &connect_request("reddit.com:443")).await;
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).contains("403 Forbidden"));
        assert_eq!(authorizer.authorized.lock().unwrap().len(), authorizations);
        assert!(authorizer.consumed.lock().unwrap().is_empty());
    }
    assert_eq!(
        upstream.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn the_tunnel_is_pinned_to_an_approved_ipv4_address_before_ipv6() {
    let (_destination, destination_address) = untouched_destination();
    let ipv6 = SocketAddr::from(([0x2001, 0xdb8, 0, 0, 0, 0, 0, 7], destination_address.port()));
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let authorizer = Arc::new(RoutingAuthorizer {
        hosts: HashMap::from([
            ("reddit.com".to_owned(), vec![ipv6, destination_address]),
            (PROXY_HOST.to_owned(), vec![upstream_address]),
        ]),
        ..RoutingAuthorizer::default()
    });
    let proxy = bind(authorizer.clone(), upstream_proxy(upstream_address.port())).await;

    let _client = send(&proxy, &connect_request("reddit.com:443")).await;
    let (mut tunnel, _) = upstream.accept().await.unwrap();
    let head = read_head(&mut tunnel).await;
    assert!(
        head.starts_with(&format!("CONNECT {destination_address} HTTP/1.1\r\n")),
        "{head}"
    );
    assert_eq!(
        authorizer.consumed.lock().unwrap()[0],
        ("reddit.com".to_owned(), destination_address)
    );
}
