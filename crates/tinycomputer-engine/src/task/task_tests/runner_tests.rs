//! Tests for what a `FlowRunner` gets by default when it only runs flows.

use super::*;

/// A runner that implements only `run`, leaving the rest to the trait.
struct RunsOnly;

impl FlowRunner for RunsOnly {
    fn run(
        &self,
        _task: &TaskId,
        _constraints: &TaskConstraints,
        _request: RunFlowRequest,
    ) -> FlowFuture {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn a_runner_that_only_runs_flows_reads_nothing_and_holds_nothing() {
    let task = TaskId::new("t-1");
    assert_eq!(
        RunsOnly.visible_text(&task).await,
        [] as [std::string::String; 0]
    );
    assert!(RunsOnly.capture(&task).await.is_none());
    // Releasing a task the runner never held is a no-op, not a failure.
    RunsOnly.release(&task);
    // Nothing to get ready, and no journal to write to.
    RunsOnly.prepare(&task, &TaskConstraints::default()).await;
    RunsOnly.open_page(&task, "https://mail.test").await;
    RunsOnly.warm(&task, 7).await;
    assert_eq!(RunsOnly.redact(&task, "ordinary text"), "ordinary text");
    RunsOnly.journal(Some(&task), "plan", &|| serde_json::json!({"wall_ms": 1}));
    RunsOnly.journal(None, "plan", &|| serde_json::json!({}));
    assert_eq!(
        RunsOnly.visible_text(&task).await,
        [] as [std::string::String; 0]
    );
}

#[tokio::test]
async fn task_report_redacts_host_secret_echoes_from_every_section() {
    const CARD: &str = "4111 1111 1111 1111";
    let trace = tinycomputer_bus::JevExchange {
        step: "1".to_owned(),
        state: serde_json::json!({(CARD): CARD, "numeric": 4_111_111_111_111_111_u64}),
        questions: serde_json::json!({"field": CARD}),
        answers: serde_json::json!({"field": CARD}),
    };
    let reply = finished_run(
        FlowStopReason::Completed,
        vec![step("1", "verify", CARD, StepOutcome::Done, CARD)],
        &[],
        None,
    );
    let mut data = reply.data.unwrap();
    data["trace"] = serde_json::json!([trace]);
    let runner = Arc::new(Script {
        replies: Mutex::new(vec![DesktopResponse::ok("run-flow", data)].into()),
        host_secret: Mutex::new(Some(CARD.to_owned())),
        ..Script::default()
    });
    let tasks = Tasks::new(runner);
    let started = start(&tasks, json!({"app": "Mail", "steps": ["finish"]}), &[]);
    let finished = settle(&tasks, &started.id).await;
    assert!(matches!(finished.status, TaskStatus::Done { .. }));

    let report = tasks.report(&started.id);
    assert!(report.ok, "{:?}", report.error);
    let report = serde_json::to_string(&report.data.unwrap()).unwrap();
    assert!(!report.contains(CARD));
    assert!(report.contains("•••• 1111"));
}

#[test]
fn task_payload_redaction_covers_strings_numeric_echoes_and_object_keys() {
    struct SecretFilter;
    impl FlowRunner for SecretFilter {
        fn run(
            &self,
            _task: &TaskId,
            _constraints: &TaskConstraints,
            _request: RunFlowRequest,
        ) -> FlowFuture {
            Box::pin(std::future::pending())
        }

        fn redact(&self, _task: &TaskId, text: &str) -> String {
            text.replace("4111 1111 1111 1111", "•••• 1111")
                .replace("4111111111111111", "•••• 1111")
        }
    }

    let mut value = serde_json::json!({
        "4111 1111 1111 1111": "card key",
        "4111111111111111": "number key",
        "numeric": 4_111_111_111_111_111_u64,
        "value": "4111 1111 1111 1111",
        "count": 1
    });
    super::super::redact_task_value(&SecretFilter, &TaskId::new("t-redact"), &mut value);

    assert_eq!(value["numeric"], "•••• 1111");
    assert_eq!(value["value"], "•••• 1111");
    assert_eq!(value["count"], 1);
    let collisions = ["•••• 1111", "•••• 1111#2"].map(|key| value[key].as_str().unwrap());
    assert_eq!(
        std::collections::BTreeSet::from(collisions),
        std::collections::BTreeSet::from(["card key", "number key"])
    );
    assert!(!value.to_string().contains("4111 1111 1111 1111"));
}
