//! Upstream proxy routing through the module's real host boundary: the
//! routes come from the module config (`browser.egress.upstream_proxies`),
//! and every authorization, consume and lease check crosses TinyBus to the
//! host's `BrowserEgressHost` callback through `BusAuthorizer`.

use super::*;
use crate::tinybus_module::config::BrowserDefaults;
use std::collections::HashMap;
use tinycomputer_browser::EgressProxy;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const PROXY_HOST: &str = "gate.proxy.test";
const LOCAL_PASSWORD: &str = "local-proxy-password";
/// The approved destination address. It is never dialed: it only names the
/// tunnel's target at the upstream proxy.
const REDDIT: &str = "151.101.1.140:443";

/// A host that grants each known hostname its addresses and records every
/// callback, as `BrowserEgressHost` sees them.
#[derive(Clone)]
struct RoutingHost {
    hosts: HashMap<String, Vec<String>>,
    seen: Arc<Mutex<Vec<String>>>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.BrowserEgressHost")]
impl RoutingHost {
    async fn authorize(&self, request: BrowserEgressTarget) -> tinybus::Result<BrowserEgressGrant> {
        self.seen.lock().unwrap().push(format!(
            "authorize:{}://{}:{}",
            request.scheme, request.hostname, request.port
        ));
        let addresses = self
            .hosts
            .get(&request.hostname)
            .cloned()
            .ok_or_else(|| tinybus::Error::failed("refused"))?;
        Ok(BrowserEgressGrant {
            generation: 7,
            authorization: format!("lease:{}", request.hostname),
            addresses,
        })
    }

    async fn consume(
        &self,
        request: BrowserEgressConsumeRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        self.seen.lock().unwrap().push(format!(
            "consume:{}:{}",
            request.target.hostname, request.address
        ));
        Ok(BrowserEgressDecision { allowed: true })
    }

    async fn validate(
        &self,
        request: BrowserEgressValidateRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        self.seen
            .lock()
            .unwrap()
            .push(format!("validate:{}", request.authorization));
        Ok(BrowserEgressDecision { allowed: true })
    }
}

async fn routed_proxy(
    hosts: HashMap<String, Vec<String>>,
    upstream_port: u16,
) -> (EgressProxy, Arc<Mutex<Vec<String>>>, Connection) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let host = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    host.serve_at(
        contract::OBJECT_PATH.try_into().unwrap(),
        RoutingHost {
            hosts,
            seen: seen.clone(),
        },
    )
    .await
    .unwrap();
    host.request_name(contract::INTERFACE).await.unwrap();
    let module = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let egress = BrowserDefaults::egress_from_raw(&serde_json::json!({"browser": {"egress": {
        "policy_id": "policy-private",
        "generation": 7,
        "upstream_proxies": [{
            "url": format!("http://resi:pass@{PROXY_HOST}:{upstream_port}"),
            "domains": ["reddit.com"],
        }],
    }}}))
    .unwrap()
    .unwrap();
    let proxy = EgressProxy::bind_with_upstreams(
        Arc::new(BusAuthorizer::new(
            module,
            egress.policy_id.clone(),
            egress.generation,
        )),
        "session-private".into(),
        LOCAL_PASSWORD.into(),
        egress.upstream_proxies.clone().into(),
    )
    .await
    .unwrap();
    (proxy, seen, host)
}

async fn connect_through(proxy: &EgressProxy, authority: &str) -> TcpStream {
    // Basic base64 of `tinycomputer:` + LOCAL_PASSWORD.
    let local = "dGlueWNvbXB1dGVyOmxvY2FsLXByb3h5LXBhc3N3b3Jk";
    let mut client = TcpStream::connect(proxy.url().replace("http://", ""))
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic {local}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    client
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

#[tokio::test]
async fn a_configured_route_is_authorized_by_the_host_for_destination_and_proxy() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let (proxy, seen, _host) = routed_proxy(
        HashMap::from([
            ("www.reddit.com".to_owned(), vec![REDDIT.to_owned()]),
            (PROXY_HOST.to_owned(), vec![upstream_address.to_string()]),
        ]),
        upstream_address.port(),
    )
    .await;

    let mut client = connect_through(&proxy, "www.reddit.com:443").await;
    let (mut tunnel, _) = upstream.accept().await.unwrap();
    assert_eq!(
        read_head(&mut tunnel).await,
        format!(
            "CONNECT {REDDIT} HTTP/1.1\r\nHost: {REDDIT}\r\nProxy-Authorization: Basic cmVzaTpwYXNz\r\n\r\n"
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

    let calls = seen.lock().unwrap().clone();
    assert_eq!(
        calls[..4],
        [
            "authorize:https://www.reddit.com:443".to_owned(),
            format!("authorize:http://{PROXY_HOST}:{}", upstream_address.port()),
            format!("consume:www.reddit.com:{REDDIT}"),
            format!("consume:{PROXY_HOST}:{upstream_address}"),
        ]
    );
    assert!(calls.contains(&"validate:lease:www.reddit.com".to_owned()));
    assert!(calls.contains(&format!("validate:lease:{PROXY_HOST}")));
}

#[tokio::test]
async fn a_host_that_refuses_the_proxy_dial_leaves_the_route_closed() {
    let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let upstream_address = upstream.local_addr().unwrap();
    let (proxy, seen, _host) = routed_proxy(
        HashMap::from([("reddit.com".to_owned(), vec![REDDIT.to_owned()])]),
        upstream_address.port(),
    )
    .await;

    let mut client = connect_through(&proxy, "reddit.com:443").await;
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response).contains("403 Forbidden"));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            "authorize:https://reddit.com:443".to_owned(),
            format!("authorize:http://{PROXY_HOST}:{}", upstream_address.port()),
        ]
    );
    assert_eq!(
        upstream.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
