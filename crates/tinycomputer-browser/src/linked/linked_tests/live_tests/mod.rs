//! Live Chromium egress tests that use local HTTP and WebSocket fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::EnforcedAgentBrowser;
use crate::engine::Launcher;
use crate::{BrowserDialTarget, DialGrant, EgressAuthorizer};
use serde_json::json;
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

mod denied_requests_tests;
mod dns_rebinding_tests;
mod navigation_tests;

#[cfg(live_browser_tests)]
#[derive(Debug)]
struct RejectAllButFixture {
    address: SocketAddr,
    authorizations: Arc<AtomicUsize>,
    denied_targets: Arc<Mutex<Vec<(String, u16)>>>,
}

#[cfg(live_browser_tests)]
#[derive(Debug)]
struct RebindingAuthorizer {
    allowed_address: SocketAddr,
    private_address: SocketAddr,
    authorizations: AtomicUsize,
    denied_targets: Arc<Mutex<Vec<(String, u16)>>>,
    observed_addresses: Arc<Mutex<Vec<SocketAddr>>>,
}

#[cfg(live_browser_tests)]
#[derive(Debug)]
struct AllowFixtureHosts {
    addresses: HashMap<String, SocketAddr>,
    authorizations: Arc<AtomicUsize>,
}

#[cfg(live_browser_tests)]
impl EgressAuthorizer for AllowFixtureHosts {
    fn authorize(
        &self,
        target: BrowserDialTarget,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<DialGrant>> + Send>> {
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        let address = self
            .addresses
            .get(&target.hostname)
            .copied()
            .filter(|address| address.port() == target.port);
        Box::pin(async move {
            match address {
                Some(address) => Ok(DialGrant {
                    lease: "live-cross-origin-fixture-lease".into(),
                    addresses: vec![address],
                }),
                None => Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            }
        })
    }

    fn consume(
        &self,
        _: BrowserDialTarget,
        grant: DialGrant,
        address: SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
        Box::pin(async move {
            if grant.addresses.contains(&address) {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        })
    }

    fn validate(
        &self,
        _: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<bool>> + Send>> {
        Box::pin(async { Ok(true) })
    }
}

#[cfg(live_browser_tests)]
impl EgressAuthorizer for RebindingAuthorizer {
    fn authorize(
        &self,
        target: BrowserDialTarget,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<DialGrant>> + Send>> {
        let attempt = self.authorizations.fetch_add(1, Ordering::SeqCst);
        let resolved_address = if attempt == 0 {
            self.allowed_address
        } else {
            self.private_address
        };
        self.observed_addresses
            .lock()
            .unwrap()
            .push(resolved_address);
        let allowed = attempt == 0
            && target.hostname == "fixture.test"
            && target.port == self.allowed_address.port();
        if !allowed {
            self.denied_targets
                .lock()
                .unwrap()
                .push((target.hostname, target.port));
        }
        Box::pin(async move {
            if allowed {
                Ok(DialGrant {
                    lease: "live-dns-rebinding-lease".into(),
                    addresses: vec![resolved_address],
                })
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "rebound private address denied",
                ))
            }
        })
    }

    fn consume(
        &self,
        _: BrowserDialTarget,
        grant: DialGrant,
        address: SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
        Box::pin(async move {
            if grant.addresses.contains(&address) {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        })
    }

    fn validate(
        &self,
        _: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<bool>> + Send>> {
        Box::pin(async { Ok(true) })
    }
}

#[cfg(live_browser_tests)]
#[derive(Debug)]
struct AuthorizeExactSocket(SocketAddr);

#[cfg(live_browser_tests)]
impl EgressAuthorizer for AuthorizeExactSocket {
    fn authorize(
        &self,
        _: BrowserDialTarget,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<DialGrant>> + Send>> {
        let address = self.0;
        Box::pin(async move {
            Ok(DialGrant {
                lease: "live-pinned-socket-lease".into(),
                addresses: vec![address],
            })
        })
    }

    fn consume(
        &self,
        _: BrowserDialTarget,
        grant: DialGrant,
        address: SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
        Box::pin(async move {
            if grant.addresses.contains(&address) {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        })
    }

    fn validate(
        &self,
        _: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<bool>> + Send>> {
        Box::pin(async { Ok(true) })
    }
}

#[cfg(live_browser_tests)]
async fn wait_for_denied_target(denied_targets: &Arc<Mutex<Vec<(String, u16)>>>, port: u16) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let was_denied = denied_targets
                .lock()
                .unwrap()
                .iter()
                .any(|(_, denied_port)| *denied_port == port);
            if was_denied {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Chromium should attempt the denied destination");
}

#[cfg(live_browser_tests)]
impl EgressAuthorizer for RejectAllButFixture {
    fn authorize(
        &self,
        target: BrowserDialTarget,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<DialGrant>> + Send>> {
        let address = self.address;
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        let allowed = target.hostname == "127.0.0.1" && target.port == address.port();
        if !allowed {
            self.denied_targets
                .lock()
                .unwrap()
                .push((target.hostname, target.port));
        }
        Box::pin(async move {
            if allowed {
                Ok(DialGrant {
                    lease: "live-fixture-lease".into(),
                    addresses: vec![address],
                })
            } else {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        })
    }

    fn consume(
        &self,
        _: BrowserDialTarget,
        grant: DialGrant,
        address: SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send>> {
        Box::pin(async move {
            if grant.addresses.contains(&address) {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        })
    }

    fn validate(
        &self,
        _: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<bool>> + Send>> {
        Box::pin(async { Ok(true) })
    }
}
