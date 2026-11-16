//! Upstream proxy settings, matching, redaction and the `CONNECT` handshake.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SECRET_URL: &str = "http://res%40user:p%3Ass-word@gate.proxy.test:7000";

fn domains(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn target(address: &str) -> SocketAddr {
    address.parse().unwrap()
}

#[test]
fn a_domain_routes_itself_and_its_subdomains_only() {
    let proxy = UpstreamProxy::new(SECRET_URL, &domains(&["Reddit.com", ".redd.it."])).unwrap();
    for routed in [
        "reddit.com",
        "www.reddit.com",
        "old.reddit.com.",
        "i.redd.it",
        "WWW.REDDIT.COM",
    ] {
        assert!(proxy.routes(routed), "{routed}");
    }
    for direct in [
        "notreddit.com",
        "reddit.com.evil.test",
        "example.com",
        "it",
        "redd.it.test",
    ] {
        assert!(!proxy.routes(direct), "{direct}");
    }
}

#[test]
fn credentials_are_decoded_into_basic_authorization_and_never_debugged() {
    let proxy = UpstreamProxy::new(SECRET_URL, &domains(&["reddit.com"])).unwrap();
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("res@user:p:ss-word")
    );
    assert_eq!(proxy.authorization.as_deref(), Some(expected.as_str()));
    let dial = proxy.dial_target("session");
    assert_eq!(dial.scheme, "http");
    assert_eq!(dial.hostname, "gate.proxy.test");
    assert_eq!(dial.port, 7000);
    let debug = format!("{proxy:?}");
    assert_eq!(debug, "UpstreamProxy([REDACTED])");
    for secret in ["res", "ss-word", "gate.proxy.test", &expected] {
        assert!(!debug.contains(secret));
    }
}

#[test]
fn a_proxy_without_credentials_sends_no_authorization() {
    let proxy = UpstreamProxy::new("http://10.0.0.5", &domains(&["example.com"])).unwrap();
    assert!(proxy.authorization.is_none());
    assert_eq!(proxy.dial_target("s").port, 80);
    let v6 = UpstreamProxy::new("http://[2001:db8::1]:3128", &domains(&["example.com"])).unwrap();
    assert_eq!(v6.dial_target("s").hostname, "2001:db8::1");
}

#[test]
fn invalid_settings_are_refused_without_quoting_the_url() {
    let routed = domains(&["reddit.com"]);
    for url in [
        "https://user:hunter2@gate.proxy.test:7000",
        "socks5://user:hunter2@gate.proxy.test:7000",
        "http://user:hunter2@gate.proxy.test:7000/path",
        "http://user:hunter2@gate.proxy.test:7000/?q=1",
        "http://user:hunter2%zz@gate.proxy.test:7000",
        "http://us%3Aer:hunter2@gate.proxy.test:7000",
        "not a url hunter2",
    ] {
        let error = UpstreamProxy::new(url, &routed).unwrap_err();
        assert!(!error.to_string().contains("hunter2"), "{url}: {error}");
        assert!(!format!("{error:?}").contains("hunter2"));
    }
    for bad in [
        vec![],
        domains(&["*.reddit.com"]),
        domains(&[""]),
        domains(&["a..b"]),
        domains(&["reddit.com/x"]),
    ] {
        assert!(UpstreamProxy::new(SECRET_URL, &bad).is_err(), "{bad:?}");
    }
}

async fn handshake(reply: &'static [u8], target: SocketAddr) -> (io::Result<()>, String) {
    let proxy = UpstreamProxy::new(SECRET_URL, &domains(&["reddit.com"])).unwrap();
    let (mut ours, mut theirs) = tokio::io::duplex(4096);
    let server = tokio::spawn(async move {
        let mut seen = Vec::new();
        let mut byte = [0u8; 1];
        while !seen.ends_with(b"\r\n\r\n") {
            theirs.read_exact(&mut byte).await.unwrap();
            seen.push(byte[0]);
        }
        theirs.write_all(reply).await.unwrap();
        // The destination's first bytes must stay unread by the handshake.
        theirs.write_all(b"after").await.unwrap();
        (String::from_utf8(seen).unwrap(), theirs)
    });
    let result = open_tunnel(&mut ours, &proxy, target).await;
    let (request, _theirs) = server.await.unwrap();
    if result.is_ok() {
        let mut after = [0u8; 5];
        ours.read_exact(&mut after).await.unwrap();
        assert_eq!(&after, b"after");
    }
    (result, request)
}

#[tokio::test]
async fn the_tunnel_asks_for_the_approved_address_with_the_proxy_credentials() {
    let (result, request) = handshake(
        b"HTTP/1.1 200 Connection established\r\n\r\n",
        target("151.101.1.140:443"),
    )
    .await;
    result.unwrap();
    let expected = base64::engine::general_purpose::STANDARD.encode("res@user:p:ss-word");
    assert_eq!(
        request,
        format!(
            "CONNECT 151.101.1.140:443 HTTP/1.1\r\nHost: 151.101.1.140:443\r\nProxy-Authorization: Basic {expected}\r\n\r\n"
        )
    );
    let (result, request) = handshake(
        b"HTTP/1.0 200 OK\r\nVia: x\r\n\r\n",
        target("[2001:db8::7]:80"),
    )
    .await;
    result.unwrap();
    assert!(request.starts_with("CONNECT [2001:db8::7]:80 HTTP/1.1\r\n"));
}

#[tokio::test]
async fn a_refused_tunnel_names_only_the_status() {
    let (result, _) = handshake(
        b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\n\r\n",
        target("151.101.1.140:443"),
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("407"));
    assert!(!error.to_string().contains("ss-word"));

    let (result, _) = handshake(
        b"HTTP/1.1 502 Bad Gateway\r\n\r\n",
        target("151.101.1.140:443"),
    )
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    assert!(error.to_string().contains("502"));

    let (result, _) = handshake(b"SSH-2.0-nope\r\n\r\n", target("151.101.1.140:443")).await;
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn a_proxy_that_closes_or_floods_the_head_is_refused() {
    let proxy = UpstreamProxy::new(SECRET_URL, &domains(&["reddit.com"])).unwrap();
    let (mut ours, theirs) = tokio::io::duplex(64);
    drop(theirs);
    assert!(
        open_tunnel(&mut ours, &proxy, target("151.101.1.140:443"))
            .await
            .is_err()
    );

    let (mut ours, mut theirs) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let mut sink = [0u8; 512];
        let _ = theirs.read(&mut sink).await;
        let _ = theirs.write_all(&vec![b'a'; MAX_RESPONSE_HEAD + 16]).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let error = open_tunnel(&mut ours, &proxy, target("151.101.1.140:443"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[tokio::test(start_paused = true)]
async fn a_silent_proxy_times_out() {
    let proxy = UpstreamProxy::new(SECRET_URL, &domains(&["reddit.com"])).unwrap();
    let (mut ours, _theirs) = tokio::io::duplex(4096);
    let error = open_tunnel(&mut ours, &proxy, target("151.101.1.140:443"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}
