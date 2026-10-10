//! Tests pinning the Agent interface's wire form.
//!
//! A model reads and writes these frames directly, so field names and tags
//! are the contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use serde_json::json;

use super::{
    AgentError, AgentResponse, AwaitTaskRequest, ContinueTaskRequest, InputField, InputKind,
    PaymentMode, StartTaskRequest, SurfaceKind, TaskId, TaskReportRequest, TaskStatus, TaskView,
};

#[test]
fn a_bare_task_takes_safe_defaults() {
    let request: StartTaskRequest =
        serde_json::from_value(json!({"task": "book a flight"})).unwrap();
    assert_eq!(request.task.as_deref(), Some("book a flight"));
    assert!(request.flow.is_none());
    assert!(!request.constraints.allow_destructive);
    assert!(!request.constraints.headed);
    assert!(request.constraints.browser_executable.is_none());
    assert!(request.constraints.browser_profile.is_none());
    assert_eq!(request.constraints.surfaces, []);
    assert_eq!(request.constraints.payment, PaymentMode::StopAtPayment);
    assert!(request.secret_facts.is_empty() && request.budget.votes.is_none());
    assert!(!request.trace);
    let round_trip: StartTaskRequest =
        serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
    assert_eq!(round_trip, request);
}

#[test]
fn a_flow_and_constraints_are_accepted_as_written() {
    let request: StartTaskRequest = serde_json::from_value(json!({
        "flow": {"app": "Mail", "steps": [{"open": "Mail"}, "start a new email message"]},
        "constraints": {
            "surfaces": ["browser", "desktop"],
            "origins": ["https://.flights.test"],
            "browser_endpoint": "http://127.0.0.1:9222",
            "headed": true,
            "browser_executable": "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "browser_profile": "/Users/asha/.openhuman/chrome"
        },
        "budget": {"max_actions": 80}
    }))
    .unwrap();
    assert_eq!(
        request.constraints.browser_executable.as_deref(),
        Some("/Applications/Chromium.app/Contents/MacOS/Chromium")
    );
    assert_eq!(
        request.constraints.browser_profile.as_deref(),
        Some("/Users/asha/.openhuman/chrome")
    );
    assert_eq!(request.flow.unwrap().steps.len(), 2);
    assert_eq!(
        request.constraints.surfaces,
        [SurfaceKind::Browser, SurfaceKind::Desktop]
    );
    assert_eq!(request.budget.max_actions, Some(80));
    assert_eq!(request.budget.max_model_calls, None);
}

#[test]
fn secrets_and_the_payment_mode_pin_their_wire_form() {
    let request: StartTaskRequest = serde_json::from_value(json!({
        "task": "book and fill the card form",
        "facts": {"first name": "Asha", "frequent flyer": "6E1234"},
        "secret_facts": ["frequent flyer"],
        "constraints": {"payment": "fill_then_approve", "origins": ["https://.goindigo.in"]},
        "budget": {"votes": 7, "strategy": "wide", "deliberation": "standard"}
    }))
    .unwrap();
    assert_eq!(request.secret_facts, ["frequent flyer"]);
    assert_eq!(request.constraints.payment, PaymentMode::FillThenApprove);
    assert_eq!(request.budget.votes, Some(7));
    assert_eq!(request.budget.strategy, Some(crate::FlowStrategy::Wide));
    assert_eq!(
        request.budget.deliberation,
        Some(crate::Deliberation::Standard)
    );
    assert_eq!(
        serde_json::to_value(PaymentMode::StopAtPayment).unwrap(),
        json!("stop_at_payment")
    );
}

#[test]
fn statuses_are_tagged_by_state() {
    let fields = TaskStatus::NeedsInput {
        fields: vec![InputField {
            name: "cabin".to_owned(),
            why: "the fare depends on it".to_owned(),
            kind: InputKind::Choice,
            options: vec!["economy".to_owned(), "business".to_owned()],
        }],
    };
    assert_eq!(
        serde_json::to_value(&fields).unwrap(),
        json!({"state": "needs_input", "fields": [{
            "name": "cabin", "why": "the fare depends on it", "kind": "choice",
            "options": ["economy", "business"]
        }]})
    );
    assert_eq!(
        serde_json::to_value(TaskStatus::Running).unwrap(),
        json!({"state": "running"})
    );
    let checkpoint = TaskStatus::Checkpoint {
        reason: "reached the payment page".to_owned(),
        location: "https://flights.test/pay".to_owned(),
        screenshot: None,
        summary: "IndiGo 6E-2135, ₹6,840, traveller details filled".to_owned(),
        continuable: false,
    };
    let wire = serde_json::to_value(&checkpoint).unwrap();
    assert_eq!(wire["state"], "checkpoint");
    assert!(wire.get("screenshot").is_none());
    assert_eq!(
        serde_json::from_value::<TaskStatus>(wire).unwrap(),
        checkpoint
    );
}

#[test]
fn an_output_shape_and_its_result_pin_their_wire_form() {
    let request: StartTaskRequest = serde_json::from_value(json!({
        "task": "read my newest chats",
        "output": {
            "instructions": "each chat's name and last message",
            "schema": {"type": "object", "required": ["chats"]}
        }
    }))
    .unwrap();
    let output = request.output.clone().unwrap();
    assert_eq!(output.instructions, "each chat's name and last message");
    assert_eq!(output.schema.unwrap()["required"][0], "chats");
    // Without an output, the request's wire form is as it was before 2.5.
    let plain = serde_json::to_value(StartTaskRequest::default()).unwrap();
    assert!(plain.get("output").is_none());

    let done = TaskStatus::Done {
        answer: "Finished all 3 steps.".to_owned(),
        records: BTreeMap::new(),
        result: Some(json!({"chats": []})),
    };
    let wire = serde_json::to_value(&done).unwrap();
    assert_eq!(wire["result"], json!({"chats": []}));
    assert_eq!(serde_json::from_value::<TaskStatus>(wire).unwrap(), done);
    // A 2.4 module's `done` carries no result, and still reads.
    let older: TaskStatus =
        serde_json::from_value(json!({"state": "done", "answer": "ok", "records": {}})).unwrap();
    assert!(matches!(older, TaskStatus::Done { result: None, .. }));
    let bare = serde_json::to_value(TaskStatus::Done {
        answer: "ok".to_owned(),
        records: BTreeMap::new(),
        result: None,
    })
    .unwrap();
    assert!(bare.get("result").is_none());
}

#[test]
fn only_settled_statuses_are_final() {
    let done = TaskStatus::Done {
        answer: "done".to_owned(),
        records: BTreeMap::new(),
        result: None,
    };
    let failed = TaskStatus::Failed {
        step: Some(2),
        reason: "no results".to_owned(),
        hint: "try another date".to_owned(),
        recoverable: true,
    };
    let payment = TaskStatus::Checkpoint {
        reason: String::new(),
        location: String::new(),
        screenshot: None,
        summary: String::new(),
        continuable: false,
    };
    let review = TaskStatus::Checkpoint {
        reason: String::new(),
        location: String::new(),
        screenshot: None,
        summary: String::new(),
        continuable: true,
    };
    for status in [done, failed, payment, TaskStatus::Cancelled] {
        assert!(status.is_final(), "{status:?}");
    }
    for status in [
        TaskStatus::Running,
        review,
        TaskStatus::NeedsHuman {
            reason: "solve the captcha".to_owned(),
            screenshot: None,
        },
        TaskStatus::NeedsPlan {
            guide: String::new(),
        },
        TaskStatus::NeedsApproval {
            action: "send the email".to_owned(),
            target: "Send".to_owned(),
            screenshot: None,
        },
    ] {
        assert!(!status.is_final(), "{status:?}");
    }
}

#[test]
fn replies_carry_data_or_an_actionable_error() {
    let view = TaskView {
        id: TaskId::new("t-1"),
        status: TaskStatus::Running,
        summary: "Searching for flights.".to_owned(),
        step: None,
        progress: 0.25,
        next: vec!["AwaitTask".to_owned()],
    };
    let ok = serde_json::to_value(AgentResponse::ok(view)).unwrap();
    assert_eq!(ok["ok"], true);
    assert!(ok.get("error").is_none());
    assert!(ok["data"].get("step").is_none());

    let failed: AgentResponse<TaskView> = AgentResponse::err(AgentError::new(
        "NO_SUCH_TASK",
        "task t-9 does not exist",
        "call ListTasks for current task ids",
        false,
    ));
    let wire = serde_json::to_value(&failed).unwrap();
    assert_eq!(wire["error"]["code"], "NO_SUCH_TASK");
    assert!(wire.get("data").is_none());
    assert_eq!(
        serde_json::from_value::<AgentResponse<TaskView>>(wire).unwrap(),
        failed
    );
}

#[test]
fn waiting_and_continuing_have_forgiving_defaults() {
    let wait: AwaitTaskRequest = serde_json::from_value(json!({"id": "t-1"})).unwrap();
    assert_eq!(wait.timeout_ms, 30_000);
    let resume: ContinueTaskRequest = serde_json::from_value(json!({
        "id": "t-1", "inputs": {"date of birth": "1990-04-02"}
    }))
    .unwrap();
    assert_eq!(resume.id.to_string(), "t-1");
    assert_eq!(resume.inputs["date of birth"], "1990-04-02");
    assert_eq!(resume.approve, None);
}

#[test]
fn rescues_pin_their_wire_form() {
    let request: StartTaskRequest = serde_json::from_value(json!({
        "task": "book a flight",
        "budget": {"max_rescues": 1}
    }))
    .unwrap();
    assert_eq!(request.budget.max_rescues, Some(1));
    let rescue = super::Rescue {
        step: 2,
        failure: "the class button is covered".to_owned(),
        reason: "the calendar is still open".to_owned(),
        steps: vec![crate::FlowStep::Intent("close the calendar".to_owned())],
        covers: 1,
        outcome: super::RescueOutcome::FailedAgain,
    };
    let value = serde_json::to_value(&rescue).unwrap();
    assert_eq!(
        value,
        json!({
            "step": 2,
            "failure": "the class button is covered",
            "reason": "the calendar is still open",
            "steps": ["close the calendar"],
            "covers": 1,
            "outcome": "failed_again"
        })
    );
    assert_eq!(
        serde_json::from_value::<super::Rescue>(value).unwrap(),
        rescue
    );
    for (outcome, wire) in [
        (super::RescueOutcome::Running, "running"),
        (super::RescueOutcome::Recovered, "recovered"),
        (super::RescueOutcome::GaveUp, "gave_up"),
    ] {
        assert_eq!(serde_json::to_value(outcome).unwrap(), json!(wire));
    }
    // A report or a description from before 2.4 still reads.
    let report: super::TaskReport = serde_json::from_value(json!({
        "view": {"id": "t-1", "status": {"state": "running"}, "summary": "",
                 "progress": 0.0, "next": []},
        "steps": [], "records": {}, "artifacts": [], "learned": [], "trace": []
    }))
    .unwrap();
    assert_eq!(report.rescues, []);
    assert!(
        !serde_json::to_value(&report)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("rescues")
    );
}

#[test]
fn a_report_request_always_carries_its_trace_flag() {
    // A confidential body holding a bare `{"id": "..."}` is refused by the
    // TinyBus client as a stream handle, so `trace` is never skipped.
    let request = TaskReportRequest::new(TaskId::new("t-1"));
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"id": "t-1", "trace": true})
    );
    let bare: TaskReportRequest = serde_json::from_value(json!({"id": "t-1"})).unwrap();
    assert_eq!(bare, request);
    let lean: TaskReportRequest =
        serde_json::from_value(json!({"id": "t-1", "trace": false})).unwrap();
    assert!(!lean.trace);
}

#[test]
fn capabilities_from_before_model_summaries_still_decode() {
    use super::{Capabilities, LanguageModelConfiguration, LanguageModelProvider};

    let older = json!({
        "contract_version": [2, 6],
        "surfaces": [],
        "jev_configured": true,
        "planner_configured": true,
        "step_kinds": [],
        "guide": "",
        "members": [],
        "examples": []
    });
    let decoded: Capabilities = serde_json::from_value(older).unwrap();
    assert!(decoded.decision_model.is_none());
    assert!(decoded.planner_model.is_none());
    assert!(decoded.rescue_model.is_none());
    assert!(decoded.output_model.is_none());

    let rescue = LanguageModelConfiguration {
        provider: LanguageModelProvider::TinyHumans,
        model: "openai/gpt-6-luna".into(),
        endpoint_url: None,
    };
    assert_eq!(
        serde_json::to_value(&rescue).unwrap(),
        json!({"provider": "tiny_humans", "model": "openai/gpt-6-luna"})
    );
    let routed: LanguageModelConfiguration = serde_json::from_value(json!({
        "provider": "open_router",
        "model": "anthropic/claude-sonnet-5",
        "endpoint_url": "https://openrouter.ai/api/v1"
    }))
    .unwrap();
    assert_eq!(routed.provider, LanguageModelProvider::OpenRouter);
    assert_eq!(
        LanguageModelProvider::default(),
        LanguageModelProvider::OpenRouter
    );
}
