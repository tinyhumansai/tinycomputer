//! Tests for the mechanical TinyBus-to-TinySecurity browser egress adapter.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::sync::{Arc, Mutex};
use tinybus::{Connection, broker::Broker, transport::memory::MemoryBus};
use tinycomputer_bus::browser_egress::{
    self as contract, BrowserEgressConsumeRequest, BrowserEgressDecision, BrowserEgressGrant,
    BrowserEgressTarget, BrowserEgressValidateRequest,
};

#[derive(Clone)]
struct Host {
    addresses: Vec<String>,
    generation: u64,
    consume_allowed: bool,
    validate_allowed: bool,
    seen: Arc<Mutex<Vec<String>>>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.BrowserEgressHost")]
impl Host {
    async fn authorize(&self, request: BrowserEgressTarget) -> tinybus::Result<BrowserEgressGrant> {
        tokio::task::yield_now().await;
        self.seen.lock().unwrap().push(format!(
            "authorize:{}:{}:{}:{}:{}:{}",
            request.policy_id,
            request.expected_generation,
            request.session_id,
            request.scheme,
            request.hostname,
            request.port
        ));
        Ok(BrowserEgressGrant {
            generation: self.generation,
            authorization: "secret-lease".into(),
            addresses: self.addresses.clone(),
        })
    }

    async fn consume(
        &self,
        request: BrowserEgressConsumeRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        tokio::task::yield_now().await;
        self.seen.lock().unwrap().push(format!(
            "consume:{}:{}:{}:{}:{}:{}",
            request.target.policy_id,
            request.target.expected_generation,
            request.target.session_id,
            request.target.scheme,
            request.target.hostname,
            request.address
        ));
        Ok(BrowserEgressDecision {
            allowed: self.consume_allowed,
        })
    }

    async fn validate(
        &self,
        request: BrowserEgressValidateRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        tokio::task::yield_now().await;
        self.seen
            .lock()
            .unwrap()
            .push(format!("validate:{}", request.authorization));
        Ok(BrowserEgressDecision {
            allowed: self.validate_allowed,
        })
    }
}

async fn connect_host(host: Host) -> tinybus::Result<(BusAuthorizer, Connection)> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let connection = Connection::connect(bus.connect().await?).await?;
    connection
        .serve_at(contract::OBJECT_PATH.try_into()?, host)
        .await?;
    connection.request_name(contract::INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;
    Ok((
        BusAuthorizer::new(module, "policy-private".into(), 7),
        connection,
    ))
}

fn dial_target() -> BrowserDialTarget {
    BrowserDialTarget {
        scheme: "https".into(),
        hostname: "example.test".into(),
        port: 443,
        session_id: "session-private".into(),
    }
}

fn host(addresses: Vec<String>) -> Host {
    Host {
        addresses,
        generation: 7,
        consume_allowed: true,
        validate_allowed: true,
        seen: Arc::new(Mutex::new(Vec::new())),
    }
}

#[tokio::test]
async fn bus_adapter_delegates_exact_target_and_checks_the_fixed_socket() -> tinybus::Result<()> {
    let host = host(vec!["8.8.8.8:443".into()]);
    let seen = host.seen.clone();
    let (authorizer, _host_connection) = connect_host(host).await?;
    let target = dial_target();
    let grant = authorizer.authorize(target.clone()).await?;
    let address = grant.addresses[0];
    authorizer.consume(target, grant.clone(), address).await?;
    assert!(authorizer.validate(grant.lease).await?);
    let calls = seen.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[0].contains("policy-private:7:session-private:https:example.test:443"));
    assert!(calls[1].contains("policy-private:7:session-private:https:example.test:8.8.8.8:443"));
    assert!(calls[2].contains("validate:secret-lease"));
    Ok(())
}

#[tokio::test]
async fn bus_adapter_rejects_wrong_generation_port_and_ungranted_socket() -> tinybus::Result<()> {
    let (wrong_generation, _wrong_generation_connection) = connect_host(Host {
        generation: 8,
        ..host(vec!["8.8.8.8:443".into()])
    })
    .await?;
    assert!(wrong_generation.authorize(dial_target()).await.is_err());

    let (wrong_port, _wrong_port_connection) =
        connect_host(host(vec!["8.8.8.8:80".into()])).await?;
    assert!(wrong_port.authorize(dial_target()).await.is_err());

    let host = host(vec!["8.8.8.8:443".into()]);
    let seen = host.seen.clone();
    let (authorizer, _host_connection) = connect_host(host).await?;
    let target = dial_target();
    let grant = authorizer.authorize(target.clone()).await?;
    assert!(
        authorizer
            .consume(
                target,
                grant,
                std::net::SocketAddr::from(([1, 1, 1, 1], 443)),
            )
            .await
            .is_err()
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    Ok(())
}

#[tokio::test]
async fn bus_adapter_refuses_host_denials_and_malformed_grants() -> tinybus::Result<()> {
    let denied_host = Host {
        consume_allowed: false,
        validate_allowed: false,
        ..host(vec!["8.8.8.8:443".into()])
    };
    let (authorizer, _denied_host_connection) = connect_host(denied_host).await?;
    let target = dial_target();
    let grant = authorizer.authorize(target.clone()).await?;
    let address = grant.addresses[0];
    assert!(
        authorizer
            .consume(target, grant.clone(), address)
            .await
            .is_err()
    );
    assert!(!authorizer.validate(grant.lease).await?);

    for addresses in [
        vec![],
        (0..65).map(|_| "8.8.8.8:443".into()).collect(),
        vec!["not-a-socket".into()],
    ] {
        let (malformed, _malformed_connection) = connect_host(host(addresses)).await?;
        assert!(malformed.authorize(dial_target()).await.is_err());
    }
    Ok(())
}

mod upstream_tests;
