//! Asking the host before an approved irreversible or paying action goes
//! ahead, through its `ConsequenceHost`.
//!
//! A task pauses before a control the module classifies as irreversible or
//! paying (`tinycomputer_core::safety`). Answering that pause with
//! `ContinueTask { approve: true }` used to be the whole gate. With the host
//! serving `ConsequenceHost`, the module first sends it a `Review` and
//! honours the approval only on `allow`; otherwise the task is declined, as
//! if `approve` had been `false`.
//!
//! - A host that does not serve the interface leaves the pause as the only
//!   gate ([`Verdict::Unreviewed`]).
//! - A host that serves it but errors, times out or is unreachable denies:
//!   an action nobody could rule on is never assumed safe.

use std::future::Future;
use std::pin::Pin;

use tinybus::Connection;
use tinycomputer_bus::agent::{ContinueTaskRequest, HostContext};
use tinycomputer_bus::consequence_host::{
    ConsequenceKind, INTERFACE, OBJECT_PATH, ReviewActionRequest, ReviewActionResponse, methods,
};
use tinycomputer_core::{Consequence, consequence};

/// The deadline for one call to the host.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The bus errors that identify a missing service on the legacy well-known
/// lookup. An explicitly configured destination is expected to be served and
/// every failure from it denies the action.
const NOT_SERVED: [&str; 3] = [
    "ai.tinyhumans.tinybus.Error.NameHasNoOwner",
    "ai.tinyhumans.tinybus.Error.UnknownObject",
    "ai.tinyhumans.tinybus.Error.UnknownInterface",
];

/// Why a call to the host failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PortError {
    /// Nothing serves the interface.
    NotServed,
    /// The host serves it and the call failed.
    Failed(String),
}

/// A pending call to the host.
pub(crate) type PortFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, PortError>> + Send + 'a>>;

/// The host's `ConsequenceHost`, as the module calls it.
pub(crate) trait ConsequencePort: Send + Sync + std::fmt::Debug {
    /// `Review`: rules on one action.
    fn review(&self, request: ReviewActionRequest) -> PortFuture<'_, ReviewActionResponse>;
}

/// The port over the bus connection the module was set up on.
#[derive(Clone)]
pub(crate) struct BusPort {
    connection: Connection,
    destination: Option<tinybus::BusName>,
}

impl std::fmt::Debug for BusPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("BusPort").finish_non_exhaustive()
    }
}

impl BusPort {
    pub(crate) fn new(connection: Connection, destination: Option<tinybus::BusName>) -> Self {
        Self {
            connection,
            destination,
        }
    }

    fn classify_error(configured_destination: bool, error: &tinybus::Error) -> PortError {
        if !configured_destination && NOT_SERVED.contains(&error.wire_name()) {
            PortError::NotServed
        } else {
            PortError::Failed(error.to_string())
        }
    }
}

impl ConsequencePort for BusPort {
    fn review(&self, request: ReviewActionRequest) -> PortFuture<'_, ReviewActionResponse> {
        if request.host_context.is_some() && self.destination.is_none() {
            return Box::pin(async {
                Err(PortError::Failed(
                    "host context requires a confidential host callback".to_owned(),
                ))
            });
        }
        Box::pin(async move {
            let proxy = self
                .connection
                .proxy(
                    self.destination
                        .as_ref()
                        .map_or(INTERFACE, tinybus::BusName::as_str),
                    OBJECT_PATH,
                    INTERFACE,
                )
                .map(|proxy| proxy.with_timeout(CALL_TIMEOUT))
                .map_err(|error| PortError::Failed(error.to_string()))?;
            let reply = if self.destination.is_some() {
                proxy.call_confidential(methods::REVIEW, (request,)).await
            } else {
                proxy.call(methods::REVIEW, (request,)).await
            };
            reply.map_err(|error| Self::classify_error(self.destination.is_some(), &error))
        })
    }
}

/// What the host ruled on an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The host does not review actions; the task's own pause is the gate.
    Unreviewed,
    /// The host allows the action.
    Allowed,
    /// The host, or its failure to answer, denies the action.
    Denied(String),
}

/// What `action` and `target` commit to, the worse of the two readings. A
/// control the module paused on is never treated as harmless, so one that
/// reads as reversible still counts as irreversible.
pub(crate) fn kind_of(action: &str, target: &str) -> ConsequenceKind {
    if consequence(action) == Consequence::Payment || consequence(target) == Consequence::Payment {
        ConsequenceKind::Payment
    } else {
        ConsequenceKind::Irreversible
    }
}

/// The module's link to the host's consequence review.
#[derive(Debug)]
pub(crate) struct ConsequenceLink {
    port: Box<dyn ConsequencePort>,
}

impl ConsequenceLink {
    pub(crate) fn new(port: Box<dyn ConsequencePort>) -> Self {
        Self { port }
    }

    /// Asks the host whether task `task` may go ahead with `action` on
    /// `target`.
    pub(crate) async fn review(
        &self,
        task: &str,
        action: &str,
        target: &str,
        host_context: Option<HostContext>,
    ) -> Verdict {
        let kind = kind_of(action, target);
        let request = ReviewActionRequest {
            task: task.to_owned(),
            action: action.to_owned(),
            target: target.to_owned(),
            kind,
            host_context,
        };
        match self.port.review(request).await {
            Ok(reply) if reply.allow => Verdict::Allowed,
            Ok(reply) => Verdict::Denied(
                reply
                    .reason
                    .unwrap_or_else(|| "the host denied the action".to_owned()),
            ),
            Err(PortError::NotServed) => Verdict::Unreviewed,
            Err(PortError::Failed(error)) => {
                Verdict::Denied(format!("the host could not review the action: {error}"))
            }
        }
    }
}

impl ConsequenceLink {
    /// Rules on an answer to a paused task. `paused` is the `(action,
    /// target)` the task is waiting on, when it waits on an approval. An
    /// approval the host denies becomes a decline, so the task never takes
    /// the action; a decline, an answer to nothing pending, an allow, and a
    /// host that does not review actions leave `request` as it was.
    pub(crate) async fn screen(
        &self,
        request: &mut ContinueTaskRequest,
        paused: Option<(String, String)>,
    ) {
        if request.approve != Some(true) {
            return;
        }
        let Some((action, target)) = paused else {
            return;
        };
        if let Verdict::Denied(_) = self
            .review(
                &request.id.to_string(),
                &action,
                &target,
                request.host_context.clone(),
            )
            .await
        {
            request.approve = Some(false);
        }
    }
}

#[cfg(test)]
mod consequence_gate_tests;
