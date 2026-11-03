//! The host's `ConsequenceHost` rules on an approval before the task takes
//! its action, and a host that cannot answer never lets one through.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use tinybus::Connection;
use tinycomputer_bus::agent::{ContinueTaskRequest, HostContext, TaskId};

use super::*;

/// What a fake host answers.
#[derive(Debug, Clone)]
enum Answer {
    Allow,
    Deny(Option<&'static str>),
    NotServed,
    Broken,
}

#[derive(Debug, Clone)]
struct FakeHost {
    answer: Answer,
    seen: Arc<Mutex<Vec<ReviewActionRequest>>>,
}

impl FakeHost {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            seen: Arc::default(),
        }
    }
}

impl ConsequencePort for FakeHost {
    fn review(&self, request: ReviewActionRequest) -> PortFuture<'_, ReviewActionResponse> {
        self.seen.lock().unwrap().push(request);
        Box::pin(async move {
            match &self.answer {
                Answer::Allow => Ok(ReviewActionResponse {
                    allow: true,
                    reason: None,
                }),
                Answer::Deny(reason) => Ok(ReviewActionResponse {
                    allow: false,
                    reason: reason.map(str::to_owned),
                }),
                Answer::NotServed => Err(PortError::NotServed),
                Answer::Broken => Err(PortError::Failed("timed out".to_owned())),
            }
        })
    }
}

fn approval(approve: Option<bool>) -> ContinueTaskRequest {
    ContinueTaskRequest {
        id: TaskId::new("t1"),
        inputs: std::collections::BTreeMap::new(),
        fact_handles: std::collections::BTreeMap::new(),
        approve,
        answer: None,
        host_context: Some(HostContext::new("opaque-context")),
    }
}

fn pending() -> (String, String) {
    ("send the message".to_owned(), "Send".to_owned())
}

async fn screened(answer: Answer, approve: Option<bool>) -> (Option<bool>, FakeHost) {
    let host = FakeHost::new(answer);
    let link = ConsequenceLink::new(Box::new(host.clone()));
    let mut request = approval(approve);
    link.screen(&mut request, Some(pending())).await;
    (request.approve, host)
}

#[test]
fn a_payment_label_on_either_side_makes_the_action_a_payment() {
    assert_eq!(kind_of("pay now", "Continue"), ConsequenceKind::Payment);
    assert_eq!(
        kind_of("press the button", "Place order"),
        ConsequenceKind::Payment
    );
    assert_eq!(
        kind_of("send the message", "Send"),
        ConsequenceKind::Irreversible
    );
}

#[test]
fn a_control_that_reads_as_reversible_still_counts_as_irreversible() {
    assert_eq!(kind_of("continue", "Next"), ConsequenceKind::Irreversible);
}

#[tokio::test]
async fn an_allowed_approval_goes_through() {
    let (approve, host) = screened(Answer::Allow, Some(true)).await;
    assert_eq!(approve, Some(true));
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].task, "t1");
    assert_eq!(seen[0].target, "Send");
    assert_eq!(seen[0].kind, ConsequenceKind::Irreversible);
    assert_eq!(
        seen[0].host_context.as_ref().map(HostContext::as_str),
        Some("opaque-context")
    );
}

#[tokio::test]
async fn a_denied_approval_becomes_a_decline() {
    let (approve, _) = screened(Answer::Deny(Some("policy: no sends")), Some(true)).await;
    assert_eq!(approve, Some(false));
    let (approve, _) = screened(Answer::Deny(None), Some(true)).await;
    assert_eq!(approve, Some(false));
}

#[tokio::test]
async fn a_host_that_fails_to_answer_declines_the_action() {
    let (approve, _) = screened(Answer::Broken, Some(true)).await;
    assert_eq!(approve, Some(false));
}

#[tokio::test]
async fn a_host_that_serves_nothing_leaves_the_approval_alone() {
    let (approve, host) = screened(Answer::NotServed, Some(true)).await;
    assert_eq!(approve, Some(true));
    assert_eq!(host.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_decline_or_an_unanswered_pause_is_not_reviewed() {
    for approve in [Some(false), None] {
        let (after, host) = screened(Answer::Deny(None), approve).await;
        assert_eq!(after, approve);
        assert!(host.seen.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn an_approval_of_nothing_pending_is_not_reviewed() {
    let host = FakeHost::new(Answer::Deny(None));
    let link = ConsequenceLink::new(Box::new(host.clone()));
    let mut request = approval(Some(true));
    link.screen(&mut request, None).await;
    assert_eq!(request.approve, Some(true));
    assert!(host.seen.lock().unwrap().is_empty());
}

/// A host serving `ConsequenceHost` over a real bus.
#[derive(Clone)]
struct BusHost(FakeHost);

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.ConsequenceHost")]
impl BusHost {
    async fn review(&self, request: ReviewActionRequest) -> tinybus::Result<ReviewActionResponse> {
        self.0
            .review(request)
            .await
            .map_err(|error| tinybus::Error::failed(format!("{error:?}")))
    }
}

#[tokio::test]
async fn the_bus_port_reaches_the_hosts_interface() -> tinybus::Result<()> {
    use tinybus::broker::Broker;
    use tinybus::transport::memory::MemoryBus;

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let fake = FakeHost::new(Answer::Deny(Some("no")));
    host_connection
        .serve_at(OBJECT_PATH.try_into()?, BusHost(fake.clone()))
        .await?;
    host_connection.request_name(INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;

    let link = ConsequenceLink::new(Box::new(BusPort::new(module, None)));
    let verdict = link.review("t1", "pay now", "Pay", None).await;
    assert_eq!(verdict, Verdict::Denied("no".to_owned()));
    assert_eq!(fake.seen.lock().unwrap()[0].kind, ConsequenceKind::Payment);
    Ok(())
}

#[tokio::test]
async fn the_bus_port_never_sends_host_context_to_an_unconfidential_host() -> tinybus::Result<()> {
    use tinybus::broker::Broker;
    use tinybus::transport::memory::MemoryBus;

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await?).await?;
    let fake = FakeHost::new(Answer::Allow);
    host_connection
        .serve_at(OBJECT_PATH.try_into()?, BusHost(fake.clone()))
        .await?;
    host_connection.request_name(INTERFACE).await?;
    let module = Connection::connect(bus.connect().await?).await?;

    let link = ConsequenceLink::new(Box::new(BusPort::new(module, None)));
    let verdict = link
        .review(
            "t1",
            "send",
            "Send",
            Some(HostContext::new("opaque-context")),
        )
        .await;

    assert!(matches!(verdict, Verdict::Denied(ref reason) if reason.contains("confidential")));
    assert!(fake.seen.lock().unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn the_bus_port_treats_a_host_that_serves_nothing_as_unreviewed() -> tinybus::Result<()> {
    use tinybus::broker::Broker;
    use tinybus::transport::memory::MemoryBus;

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let module = Connection::connect(bus.connect().await?).await?;
    let link = ConsequenceLink::new(Box::new(BusPort::new(module, None)));
    assert_eq!(
        link.review("t1", "send", "Send", None).await,
        Verdict::Unreviewed
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_configured_callback_is_a_failure_not_an_unreviewed_action() -> tinybus::Result<()>
{
    use tinybus::broker::Broker;
    use tinybus::transport::memory::MemoryBus;

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let host = Connection::connect(bus.connect().await?).await?;
    let destination = host
        .unique_name()
        .expect("connected host has a unique name");
    host.serve_at(OBJECT_PATH.try_into()?, UnrelatedObject)
        .await?;
    let module = Connection::connect(bus.connect().await?).await?;
    let link = ConsequenceLink::new(Box::new(BusPort::new(module, Some(destination))));

    assert!(matches!(
        link.review("t1", "send", "Send", None).await,
        Verdict::Denied(_)
    ));
    Ok(())
}

struct UnrelatedObject;

#[tinybus::interface(name = "org.example.Unrelated")]
impl UnrelatedObject {
    async fn ping(&self) -> tinybus::Result<()> {
        std::future::ready(Ok(())).await
    }
}

#[test]
fn a_configured_callback_that_is_not_served_is_a_failure() {
    let error = tinybus::Error::UnknownInterface {
        path: OBJECT_PATH.parse().unwrap(),
        interface: INTERFACE.parse().unwrap(),
    };
    assert!(matches!(
        BusPort::classify_error(true, &error),
        PortError::Failed(_)
    ));
}
