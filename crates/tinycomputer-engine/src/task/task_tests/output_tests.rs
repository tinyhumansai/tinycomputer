//! Tests for shaping a finished task's answer into the caller's `output`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tinycomputer_bus::agent::{StartTaskRequest, TaskOutput, TaskStatus};
use tinycomputer_bus::{DesktopResponse, FlowStopReason};

use super::rescue_tests::Model;
use super::{controller, finished_run, flow, settle};
use crate::shape::Shaper;
use crate::task::Tasks;

fn shaped(replies: Vec<DesktopResponse>, answers: &[Result<&str, &str>]) -> (Tasks, Arc<Model>) {
    let (tasks, _) = controller(replies);
    let model = Arc::new(Model {
        answers: Mutex::new(
            answers
                .iter()
                .map(|answer| answer.map(str::to_owned).map_err(str::to_owned))
                .collect(),
        ),
        seen: Mutex::default(),
    });
    let tasks = tasks.with_shaper(Shaper::new(model.clone()));
    assert!(tasks.output_configured());
    (tasks, model)
}

fn chats_read() -> DesktopResponse {
    finished_run(
        FlowStopReason::Completed,
        vec![],
        &[
            ("chat_1", "Sam"),
            (
                "messages_1",
                r#"[["message, hi, 9:00, Received from Sam"],["Your message, see you at asha@example.com, 9:05"]]"#,
            ),
        ],
        None,
    )
}

fn request(schema: Option<serde_json::Value>) -> StartTaskRequest {
    StartTaskRequest {
        task: Some("read my newest chat".to_owned()),
        flow: Some(flow(
            json!({"app": "WhatsApp", "steps": ["open the newest chat"]}),
        )),
        facts: BTreeMap::from([("email".to_owned(), "asha@example.com".to_owned())]),
        output: Some(TaskOutput {
            instructions: "the chat's name and its messages' text".to_owned(),
            schema,
        }),
        ..StartTaskRequest::default()
    }
}

fn schema() -> serde_json::Value {
    json!({"type": "object", "required": ["name", "messages"], "properties": {
        "name": {"type": "string"},
        "messages": {"type": "array", "items": {"type": "string"}}
    }})
}

#[tokio::test]
async fn a_finished_task_returns_its_answer_in_the_shape_asked_for() {
    let (tasks, model) = shaped(
        vec![chats_read()],
        &[Ok(
            r#"{"name": "Sam", "messages": ["hi", "see you at ${email}"]}"#,
        )],
    );
    let view = tasks.start(&request(Some(schema()))).data.unwrap();
    let TaskStatus::Done {
        result, records, ..
    } = settle(&tasks, &view.id).await.status
    else {
        panic!("done");
    };
    assert_eq!(
        result,
        Some(json!({"name": "Sam", "messages": ["hi", "see you at ${email}"]}))
    );
    assert_eq!(records["messages_1"].len(), 2, "the raw records stay");
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let brief = &seen[0][1].text;
    assert!(brief.contains("Received from Sam"), "{brief}");
    assert!(
        !brief.contains("asha@example.com"),
        "a fact value never reaches the model: {brief}"
    );
}

#[tokio::test]
async fn host_redaction_covers_output_model_inputs_and_results() {
    const CARD: &str = "4111 1111 1111 1111";
    let model = Arc::new(Model {
        answers: Mutex::new(vec![Ok(format!(r#"{{"card":"{CARD}"}}"#))].into()),
        seen: Mutex::default(),
    });
    let response = finished_run(
        FlowStopReason::Completed,
        vec![],
        &[("card_text", CARD)],
        None,
    );
    let runner = Arc::new(super::Script {
        replies: Mutex::new(vec![response].into()),
        host_secret: Mutex::new(Some(CARD.to_owned())),
        ..super::Script::default()
    });
    let tasks = Tasks::new(runner).with_shaper(Shaper::new(model.clone()));
    let request = StartTaskRequest {
        flow: Some(flow(json!({
            "app": "browser",
            "steps": [{"read": {"what": "card echo", "into": "card_text"}}]
        }))),
        output: Some(TaskOutput {
            instructions: "return the card echo".to_owned(),
            schema: None,
        }),
        ..StartTaskRequest::default()
    };
    let view = tasks.start(&request).data.unwrap();
    let finished = settle(&tasks, &view.id).await;
    let TaskStatus::Done { result, .. } = finished.status else {
        panic!("expected a completed shaped task");
    };

    let sent = format!("{:?}", model.seen.lock().unwrap());
    assert!(!sent.contains(CARD));
    assert!(sent.contains("•••• 1111"));
    assert_eq!(result.unwrap()["card"], "•••• 1111");
}

#[tokio::test]
async fn an_answer_that_never_fits_fails_the_task_with_its_records_kept() {
    let (tasks, _) = shaped(vec![chats_read()], &[Ok(r#"{"name": 3}"#); 3]);
    let view = tasks.start(&request(Some(schema()))).data.unwrap();
    let TaskStatus::Failed {
        reason,
        recoverable,
        step,
        ..
    } = settle(&tasks, &view.id).await.status
    else {
        panic!("failed");
    };
    assert!(reason.contains("could not be shaped"), "{reason}");
    assert!(
        reason.contains("the result.name must be of type"),
        "{reason}"
    );
    assert!(!recoverable);
    assert_eq!(step, Some(1));
    let report = tasks.report(&view.id).data.unwrap();
    assert_eq!(report.records["messages_1"].len(), 2);
}

#[tokio::test]
async fn an_output_is_refused_without_a_shaper_or_with_an_unsupported_schema() {
    let (tasks, _) = controller(Vec::new());
    assert!(!tasks.output_configured());
    let refused = tasks.start(&request(None));
    assert_eq!(refused.error.unwrap().code, "OUTPUT_UNAVAILABLE");

    let (tasks, _) = shaped(Vec::new(), &[]);
    let refused = tasks.start(&request(Some(json!({"type": "object", "oneOf": []}))));
    let error = refused.error.unwrap();
    assert_eq!(error.code, "INVALID_OUTPUT");
    assert!(error.message.contains("`oneOf`"), "{}", error.message);
}

#[tokio::test]
async fn without_an_output_no_model_is_asked() {
    let (tasks, model) = shaped(vec![chats_read()], &[]);
    let mut plain = request(None);
    plain.output = None;
    let view = tasks.start(&plain).data.unwrap();
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::Done { result: None, .. }
    ));
    assert!(model.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_value_read_into_a_declared_variable_is_reported() {
    // Live, the planner declared each read's variable up front, empty
    // ("total": ""), and every one was dropped as the caller's own input:
    // the brand, price, and bag total a task was asked for never came back.
    let (tasks, _) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[("total", "Rs. 264"), ("city", "Pune")],
        None,
    )]);
    let view = tasks
        .start(&StartTaskRequest {
            task: Some("read the bag total".to_owned()),
            flow: Some(flow(json!({
                "app": "browser",
                "vars": {"total": "", "city": "Pune"},
                "steps": [{"read": {"what": "the bag total", "into": "total"}}]
            }))),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let TaskStatus::Done { records, .. } = settle(&tasks, &view.id).await.status else {
        panic!("done");
    };
    assert_eq!(
        records["total"],
        [BTreeMap::from([("value".to_owned(), "Rs. 264".to_owned())])]
    );
    assert!(
        !records.contains_key("city"),
        "a value the flow was given is not a read"
    );
}

#[tokio::test]
async fn a_variable_defined_from_a_fact_is_never_reported_as_a_read() {
    // A flow may define a variable from the caller's values, and the run
    // expands it when it starts: its value then differs from the flow's own
    // `${email}`, but it is the caller's input, and when the fact is secret
    // it must never come back in the records.
    let (tasks, _) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[("recipient", "asha@example.com"), ("total", "Rs. 264")],
        None,
    )]);
    let view = tasks
        .start(&StartTaskRequest {
            task: Some("read the bag total".to_owned()),
            flow: Some(flow(json!({
                "app": "browser",
                "vars": {"recipient": "${email}", "total": ""},
                "steps": [
                    {"enter": {"email": "${recipient}"}},
                    {"read": {"what": "the bag total", "into": "total"}}
                ]
            }))),
            facts: BTreeMap::from([("email".to_owned(), "asha@example.com".to_owned())]),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let TaskStatus::Done { records, .. } = settle(&tasks, &view.id).await.status else {
        panic!("done");
    };
    assert!(
        !records.contains_key("recipient"),
        "a variable defined from a fact is the caller's input: {records:?}"
    );
    assert!(records.contains_key("total"));
}
