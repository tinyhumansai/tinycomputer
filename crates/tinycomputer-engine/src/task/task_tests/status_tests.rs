//! Tests for what a task reports: failures and their hints, awaiting and
//! cancelling, unknown ids, the bounded store, and the next calls offered.

use super::*;

#[tokio::test]
async fn failures_carry_the_step_the_reason_and_what_to_change() {
    let failure = |reply: DesktopResponse| async move {
        let (tasks, _) = controller(vec![reply]);
        let view = start(&tasks, json!({"app": "Mail", "steps": ["a", "b"]}), &[]);
        settle(&tasks, &view.id).await.status
    };
    let step_failed = failure(finished_run(
        FlowStopReason::StepFailed,
        vec![
            step("1", "do", "a", StepOutcome::Done, ""),
            step(
                "2",
                "do",
                "b",
                StepOutcome::Failed,
                "no search field was found",
            ),
        ],
        &[],
        None,
    ))
    .await;
    assert!(matches!(
        step_failed,
        TaskStatus::Failed { step: Some(1), ref reason, recoverable: true, .. }
            if reason == "no search field was found"
    ));
    for (stop, hint) in [
        (FlowStopReason::ActionBudget, "max_actions"),
        (FlowStopReason::ModelBudget, "max_model_calls"),
        (FlowStopReason::Invalid, "guide"),
    ] {
        let status = failure(finished_run(stop, vec![], &[], None)).await;
        let TaskStatus::Failed { hint: got, .. } = status else {
            panic!("{stop:?}");
        };
        assert!(got.contains(hint), "{stop:?}: {got}");
    }
    let mut error = DesktopError::new("JEV_NOT_CONFIGURED", "jev is not configured");
    error.suggestion = Some("send the module a jev config".to_owned());
    let unconfigured = failure(DesktopResponse::err("run-flow", error)).await;
    assert!(matches!(
        unconfigured,
        TaskStatus::Failed { ref hint, recoverable: false, .. } if hint == "send the module a jev config"
    ));
    let bare = failure(DesktopResponse::err(
        "run-flow",
        DesktopError::new("X", "broken"),
    ))
    .await;
    assert!(matches!(bare, TaskStatus::Failed { ref hint, .. } if hint.contains("Jev")));
    let unreadable = failure(DesktopResponse::ok("run-flow", json!({"nonsense": true}))).await;
    assert!(
        matches!(unreadable, TaskStatus::Failed { ref reason, .. } if reason.contains("unreadable"))
    );
    let mut no_error = DesktopResponse::err("run-flow", DesktopError::new("X", "x"));
    no_error.error = None;
    let silent = failure(no_error).await;
    assert!(
        matches!(silent, TaskStatus::Failed { ref reason, .. } if reason == "the flow could not run")
    );
}

#[tokio::test]
async fn a_running_task_can_be_awaited_briefly_and_cancelled() {
    let (tasks, _) = controller(Vec::new());
    let view = start(&tasks, json!({"app": "Mail", "steps": ["a"]}), &[]);
    let cell = tasks.find(&view.id).unwrap();
    cell.state.lock().unwrap().usage.record_call(
        crate::ModelRole::Jev,
        Some(40),
        Some(12),
        Some(7),
        Some(300),
    );
    let waited = tasks
        .await_task(AwaitTaskRequest {
            id: view.id.clone(),
            timeout_ms: 10,
        })
        .await
        .data
        .unwrap();
    assert_eq!(waited.status, TaskStatus::Running);
    assert_eq!(waited.usage.jev.input_tokens, Some(40));
    let cancelled = tasks.cancel(&view.id).data.unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);
    assert_eq!(cancelled.usage.jev.calls, 1);
    let report = tasks.report(&view.id).data.unwrap();
    assert_eq!(report.usage, cancelled.usage);
    assert_eq!(report.view.usage, cancelled.usage);
    assert_eq!(
        tasks.cancel(&view.id).data.unwrap().status,
        TaskStatus::Cancelled
    );
    assert_eq!(settle(&tasks, &view.id).await.status, TaskStatus::Cancelled);
}

#[tokio::test]
async fn unknown_tasks_are_named_as_such() {
    let (tasks, _) = controller(Vec::new());
    for id in ["t-99", "nonsense"] {
        let id = TaskId::new(id);
        assert_eq!(code(&tasks.cancel(&id)), "NO_SUCH_TASK");
        assert_eq!(code(&tasks.report(&id)), "NO_SUCH_TASK");
        assert_eq!(
            code(&tasks.continue_task(ContinueTaskRequest {
                id: id.clone(),
                ..ContinueTaskRequest::default()
            })),
            "NO_SUCH_TASK"
        );
        let waited = tasks
            .await_task(AwaitTaskRequest {
                id: id.clone(),
                timeout_ms: 1,
            })
            .await;
        assert_eq!(code(&waited), "NO_SUCH_TASK");
    }
}

#[tokio::test]
async fn the_store_is_bounded_and_drops_finished_tasks_first() {
    let (tasks, _) = controller(Vec::new());
    let waiting = json!({"app": "Mail", "steps": [{"verify": "${x} holds"}]});
    let first = start(&tasks, waiting.clone(), &[]);
    for _ in 1..MAX_TASKS {
        start(&tasks, waiting.clone(), &[]);
    }
    let full = tasks.start(&StartTaskRequest {
        flow: Some(flow(waiting.clone())),
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&full), "TOO_MANY_TASKS");
    assert!(tasks.cancel(&first.id).ok);
    let admitted = start(&tasks, waiting, &[]);
    assert_eq!(tasks.list().data.unwrap().len(), MAX_TASKS);
    assert_eq!(code(&tasks.report(&first.id)), "NO_SUCH_TASK");
    assert_eq!(tasks.list().data.unwrap()[0].id, admitted.id);
    assert!(format!("{tasks:?}").contains("Tasks"));
}

#[test]
fn input_kinds_and_next_calls_follow_the_status() {
    assert_eq!(input_kind("mobile"), InputKind::Phone);
    assert_eq!(input_kind("dob"), InputKind::Date);
    assert_eq!(input_kind("number of rooms"), InputKind::Number);
    assert_eq!(input_kind("passport"), InputKind::Text);
    assert_eq!(
        next_calls(&TaskStatus::NeedsHuman {
            reason: "captcha".to_owned(),
            screenshot: None
        }),
        ["ContinueTask", "CancelTask", "TaskReport"]
    );
    assert_eq!(
        next_calls(&TaskStatus::Checkpoint {
            reason: String::new(),
            location: String::new(),
            screenshot: None,
            summary: String::new(),
            continuable: true
        }),
        ["ContinueTask", "TaskReport"]
    );
    assert_eq!(
        next_calls(&TaskStatus::Failed {
            step: None,
            reason: String::new(),
            hint: String::new(),
            recoverable: true
        }),
        ["TaskReport", "StartTask"]
    );
}

#[test]
fn the_app_in_front_is_the_last_opened_before_a_step() {
    let steps = flow(json!({"app": "Notes", "steps": [
        {"open": "Mail"}, {"browse": "https://x.test"}, "a", "b"
    ]}));
    assert_eq!(app_at(&steps, 0), "Notes");
    assert_eq!(app_at(&steps, 1), "Mail");
    assert_eq!(app_at(&steps, 3), "browser");
    assert_eq!(app_at(&steps, 99), "browser");
}
