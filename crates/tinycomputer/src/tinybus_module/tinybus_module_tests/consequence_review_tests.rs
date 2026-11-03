//! The host's consequence review, at the service: an approval the host
//! denies reaches the task as a decline.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tinybus::Interface;
use tinycomputer_bus::agent::{AgentResponse, TaskConstraints, TaskId, TaskStatus, TaskView};
use tinycomputer_bus::consequence_host::{ReviewActionRequest, ReviewActionResponse};
use tinycomputer_bus::{
    DesktopResponse, FlowRunResult, FlowStopReason, JevMetrics, JevTarget, RunFlowRequest,
    StepOutcome, StepReport, names,
};
use tinycomputer_engine::{FlowFuture, FlowRunner, Tasks};

use super::service;
use crate::tinybus_module::consequence_gate::{ConsequencePort, PortFuture};

/// A runner whose first flow stops before pressing "Send", and every later
/// one completes. It records whether each run was allowed to act.
#[derive(Default)]
struct GatedOnce {
    replies: Mutex<VecDeque<FlowRunResult>>,
    allowed: Mutex<Vec<bool>>,
}

fn run(stop: FlowStopReason, steps: Vec<StepReport>, pending: Option<&str>) -> FlowRunResult {
    FlowRunResult {
        stop,
        steps,
        vars: std::collections::BTreeMap::new(),
        pending: pending.map(|name| JevTarget {
            ref_id: "e9".to_owned(),
            role: "button".to_owned(),
            name: Some(name.to_owned()),
        }),
        learned: Vec::new(),
        actions: 1,
        metrics: JevMetrics::default(),
        trace: Vec::new(),
        dialog_left_open: false,
    }
}

impl FlowRunner for GatedOnce {
    fn run(&self, _: &TaskId, _: &TaskConstraints, request: RunFlowRequest) -> FlowFuture {
        self.allowed.lock().unwrap().push(request.allow_destructive);
        let result = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| run(FlowStopReason::Completed, Vec::new(), None));
        let reply = DesktopResponse::ok("run-flow", serde_json::to_value(result).unwrap());
        Box::pin(async move { reply })
    }
}

#[derive(Debug, Default)]
struct DenyingHost {
    seen: Mutex<Vec<ReviewActionRequest>>,
}

impl ConsequencePort for Arc<DenyingHost> {
    fn review(&self, request: ReviewActionRequest) -> PortFuture<'_, ReviewActionResponse> {
        self.seen.lock().unwrap().push(request);
        Box::pin(async {
            Ok(ReviewActionResponse {
                allow: false,
                reason: Some("not today".to_owned()),
            })
        })
    }
}

async fn awaited(service: &crate::tinybus_module::DesktopService, id: &TaskId) -> TaskView {
    let settled = service
        .call(
            &names::methods::AWAIT_TASK.try_into().unwrap(),
            json!([{"id": id, "timeout_ms": 5_000}]),
        )
        .await
        .unwrap();
    serde_json::from_value::<AgentResponse<TaskView>>(settled)
        .unwrap()
        .data
        .unwrap()
}

#[tokio::test]
async fn a_host_denial_turns_an_approval_into_a_decline() {
    let runner = Arc::new(GatedOnce::default());
    runner.replies.lock().unwrap().push_back(run(
        FlowStopReason::StoppedBeforeDestructive,
        vec![StepReport {
            path: "1".to_owned(),
            kind: "stop_before".to_owned(),
            text: "sending the email".to_owned(),
            outcome: StepOutcome::Gated,
            turns: 1,
            jev_calls: 1,
            actions: Vec::new(),
            loops: Vec::new(),
            confidence: None,
            note: "found it".to_owned(),
        }],
        Some("Send"),
    ));
    let host = Arc::new(DenyingHost::default());
    let mut service = service().with_consequence_review(Box::new(host.clone()));
    service.tasks = Arc::new(Tasks::new(runner.clone()));

    let started = service
        .call(
            &names::methods::START_TASK.try_into().unwrap(),
            json!([{"flow": {"app": "Mail", "steps": [
                {"stop_before": "sending the email"},
                "start another email message"
            ]}}]),
        )
        .await
        .unwrap();
    let id = serde_json::from_value::<AgentResponse<TaskView>>(started)
        .unwrap()
        .data
        .unwrap()
        .id;
    let paused = awaited(&service, &id).await;
    assert!(
        matches!(paused.status, TaskStatus::NeedsApproval { ref target, .. } if target == "Send"),
        "{:?}",
        paused.status
    );

    service
        .call(
            &names::methods::CONTINUE_TASK.try_into().unwrap(),
            json!([{"id": id, "approve": true}]),
        )
        .await
        .unwrap();

    assert_eq!(awaited(&service, &id).await.status, TaskStatus::Cancelled);
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "the host is asked once");
    assert_eq!(seen[0].target, "Send");
    assert!(
        !runner
            .allowed
            .lock()
            .unwrap()
            .iter()
            .any(|allowed| *allowed),
        "no run was allowed to take the denied action"
    );
}
