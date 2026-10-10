//! Running one desktop action, charged to the budget and the step log.

use std::time::Instant;

use serde_json::{Value, json};
use tinycomputer_bus::{DesktopResponse, FlowActionRecord, FlowStopReason};

use super::{
    FlowRun, Halt, StepLog,
    backend::{AgentBackend, blocking},
    view::{Candidate, target_payload},
};
use crate::agentic::journal::millis;

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Runs one desktop action, charging it to the budget and the step log.
    pub(in crate::agentic::flow) async fn act<F>(
        &mut self,
        log: &mut StepLog,
        action: &str,
        target: Option<&Candidate>,
        call: F,
    ) -> Result<DesktopResponse, Halt>
    where
        F: FnOnce(B) -> DesktopResponse + Send + 'static,
    {
        if self.actions >= self.max_actions {
            return Err(Halt::Stop(FlowStopReason::ActionBudget));
        }
        self.actions = self.actions.saturating_add(1);
        if action != "wait" {
            let typing = ["fill", "type", "paste"]
                .iter()
                .any(|verb| action.starts_with(verb));
            self.typed_last = target.filter(|_| typing).cloned();
        }
        let started = Instant::now();
        let reply = self.backend_call(call).await;
        let acted_ms = millis(started.elapsed());
        // A press the page refused never reached it, so it opened nothing in
        // front. Live, a refused press of a store's basket button counted as
        // one, the controls that read as covered after it were taken for a
        // dialog it opened, and the step's failure told a rescue to answer it.
        if !refused(&reply) {
            self.front.act(action, target);
        }
        if let Some(url) = reply
            .data
            .as_ref()
            .and_then(|data| data.get("url"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        {
            self.location = Some(url.to_owned());
        }
        let note = match (&reply.error, &reply.data) {
            (Some(error), _) => error.code.clone(),
            (None, Some(_)) if still(&reply) => "nothing changed".to_owned(),
            (None, Some(data)) => data
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(|path| {
                    let verified = data
                        .get("verified")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);
                    format!("via {path}{}", if verified { "" } else { ", unverified" })
                })
                .unwrap_or_default(),
            (None, None) => String::new(),
        };
        log.actions.push(FlowActionRecord {
            action: action.to_owned(),
            target: target.map(target_payload),
            ok: reply.ok,
            note,
        });
        let settle_started = Instant::now();
        // A wait that saw the surface stay still has nothing to settle.
        let settles = reply.ok && !still(&reply);
        if settles {
            // Let the surface finish reacting, so the next look sees what the
            // action did rather than the moment before it took effect.
            let briefly = fetches_nothing(action);
            self.backend_call(move |backend| {
                if briefly {
                    backend.settle_briefly();
                } else {
                    backend.settle();
                }
                DesktopResponse::ok("settle", serde_json::json!({}))
            })
            .await;
        }
        self.runtime.journal.record("action", || {
            let record = log.actions.last();
            json!({
                "step": self.step,
                "action": action,
                "target": record.and_then(|record| record.target.as_ref()),
                "ok": reply.ok,
                "note": record.map(|record| record.note.as_str()),
                "wall_ms": acted_ms,
                "settle_ms": if settles { millis(settle_started.elapsed()) } else { 0 },
            })
        });
        Ok(reply)
    }

    /// Waits up to `ms` for the surface to change by itself
    /// (`Surface::await_change`), as one `wait` action charged to the budget
    /// and the step log: settled when the surface changed, and `false` when
    /// it stayed still, so a caller watching for something to appear stops.
    pub(in crate::agentic::flow) async fn await_change(
        &mut self,
        log: &mut StepLog,
        ms: u64,
    ) -> Result<bool, Halt> {
        let reply = self
            .act(log, "wait", None, move |backend| {
                let changed = backend.await_change(ms);
                DesktopResponse::ok("wait", json!({ "still": !changed }))
            })
            .await?;
        Ok(!still(&reply))
    }

    async fn backend_call<F>(&self, call: F) -> DesktopResponse
    where
        F: FnOnce(B) -> DesktopResponse + Send + 'static,
    {
        blocking(self.backend.clone(), call).await
    }
}

/// Whether `action` fetches nothing the next look must wait for, so the
/// surface settles briefly after it (`Surface::settle_briefly`): a launch,
/// which leaves an open page as it is, or Escape closing a layer. Live, 3%
/// of launches and 12% of Escapes settled with a request of the page still
/// running, against 39% of fills (fetching suggestions the next look reads)
/// and 70% of clicks, which settle in full.
fn fetches_nothing(action: &str) -> bool {
    action == "launch" || action.starts_with("launch ") || action.starts_with("press escape")
}

/// Whether the page refused `reply`'s action before it reached the page
/// (`NOT_ACTIONABLE`: covered, not visible, or not interactable). The
/// browser checks a target before it sends any input; its only press that
/// can land and still come back refused is one through a result card's own
/// cover whose last mouse event fails in transit, which presses that card's
/// own control.
fn refused(reply: &DesktopResponse) -> bool {
    reply
        .error
        .as_ref()
        .is_some_and(|error| error.code == "NOT_ACTIONABLE")
}

/// Whether `reply` is a wait's that saw the surface stay still.
fn still(reply: &DesktopResponse) -> bool {
    reply
        .data
        .as_ref()
        .and_then(|data| data.get("still"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
