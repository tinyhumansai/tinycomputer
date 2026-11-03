//! Paused-task snapshots keep resumable state without carrying secret values.

use super::*;

#[tokio::test]
async fn a_paused_task_round_trips_into_a_fresh_controller() {
    let (tasks, _) = controller(Vec::new());
    let profile = std::env::temp_dir()
        .join("tinycomputer-paused-profile")
        .to_string_lossy()
        .into_owned();
    let missing = tasks.export_paused_task(&TaskId::new("t-900")).unwrap_err();
    assert!(matches!(&missing, crate::PausedTaskError::NoSuchTask(_)));
    assert!(missing.to_string().contains("t-900"));
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({
                "app": "browser",
                "steps": [
                    {"browse": "https://shop.test/cart"},
                    {"enter": {"email": "${email}"}},
                    {"read": {"what": "the confirmation", "into": "confirmation"}}
                ]
            }))),
            facts: BTreeMap::from([("password".to_owned(), "swordfish-secret".to_owned())]),
            constraints: TaskConstraints {
                browser_profile: Some(profile.clone()),
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(view.status, TaskStatus::NeedsInput { .. }));

    let cell = tasks.find(&view.id).unwrap();
    cell.state.lock().unwrap().usage.record_call(
        crate::ModelRole::Jev,
        Some(25),
        Some(5),
        Some(8),
        Some(120),
    );
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    let serialized = std::str::from_utf8(&encoded).unwrap();
    assert!(!serialized.contains("swordfish-secret"));
    let snapshot: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        snapshot
            .pointer("/task/constraints/browser_profile")
            .and_then(serde_json::Value::as_str),
        Some(profile.as_str())
    );

    drop(tasks);
    let (restored, script) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![step(
            "1",
            "browse",
            "https://shop.test/cart",
            StepOutcome::Done,
            "",
        )],
        &[("confirmation", "Order received")],
        None,
    )]);
    let imported = restored.import_paused_task(&encoded).unwrap();
    assert_eq!(imported.id, view.id);
    assert!(matches!(imported.status, TaskStatus::NeedsInput { .. }));
    assert_eq!(imported.usage.jev.calls, 1);
    assert_eq!(imported.usage.jev.input_tokens, Some(25));
    assert!(
        script.events.lock().unwrap().is_empty(),
        "import must not open a browser"
    );

    let continued = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        inputs: BTreeMap::from([("email".to_owned(), "asha@example.test".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    assert!(continued.ok, "{:?}", continued.error);
    let done = settle(&restored, &imported.id).await;
    assert!(matches!(done.status, TaskStatus::Done { .. }));
    assert_eq!(
        script.requests.lock().unwrap()[0].vars["email"],
        "asha@example.test"
    );
    assert_eq!(script.requests.lock().unwrap()[0].flow.steps.len(), 3);
    assert_eq!(
        script.constraints.lock().unwrap()[0]
            .browser_profile
            .as_deref(),
        Some(profile.as_str())
    );

    // Imported ids advance the local allocator so the next task cannot
    // replace the task just restored.
    let next = start(&restored, json!({"app": "Mail", "steps": ["a"]}), &[]);
    assert_eq!(next.id, TaskId::new("t-2"));
}

#[tokio::test]
async fn a_paused_task_with_a_host_value_leak_fails_closed() {
    let canary = "paused-profile-secret-canary";
    let (tasks, script) = controller(Vec::new());
    *script.host_secret.lock().unwrap() = Some(canary.to_owned());
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({
                "app": "browser",
                "steps": [{"enter": {"email": "${email}"}}]
            }))),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(view.status, TaskStatus::NeedsInput { .. }));

    let cell = tasks.find(&view.id).unwrap();
    cell.state.lock().unwrap().goal = format!("restored form context: {canary}");
    assert!(matches!(
        tasks.export_paused_task(&view.id),
        Err(crate::PausedTaskError::SecretValueInState)
    ));
}

#[tokio::test]
async fn an_approval_requiring_a_secret_asks_for_it_again_before_approval() {
    let (tasks, _) = controller(vec![gated_for_persistence("Send", "sending the email")]);
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({
                "app": "Mail",
                "vars": {"password alias": "${password}"},
                "steps": [
                    {"enter": {
                        "password field": "${password alias}",
                        "one time code field": "${one time code}"
                    }},
                    {"stop_before": "sending the email"},
                    "check the sent message"
                ]
            }))),
            facts: BTreeMap::from([
                ("password".to_owned(), "do-not-persist-this".to_owned()),
                ("one time code".to_owned(), "123456".to_owned()),
            ]),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let paused = settle(&tasks, &view.id).await;
    assert!(matches!(paused.status, TaskStatus::NeedsApproval { .. }));
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    assert!(!String::from_utf8_lossy(&encoded).contains("do-not-persist-this"));

    drop(tasks);
    let (restored, _) = controller(vec![
        finished_run(FlowStopReason::Completed, vec![], &[], None),
        finished_run(FlowStopReason::Completed, vec![], &[], None),
    ]);
    let imported = restored.import_paused_task(&encoded).unwrap();
    let TaskStatus::NeedsInput { fields } = &imported.status else {
        panic!("secret facts must be re-entered: {:?}", imported.status);
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["one time code", "password"]
    );

    let partial = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        inputs: BTreeMap::from([("password".to_owned(), "new-secret".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    assert!(matches!(
        partial.data.unwrap().status,
        TaskStatus::NeedsInput { .. }
    ));
    let halfway = restored.export_paused_task(&imported.id).unwrap();
    drop(restored);

    let (restored, script) = controller(vec![
        finished_run(FlowStopReason::Completed, vec![], &[], None),
        finished_run(FlowStopReason::Completed, vec![], &[], None),
    ]);
    let imported = restored.import_paused_task(&halfway).unwrap();
    let TaskStatus::NeedsInput { fields } = &imported.status else {
        panic!(
            "partial secret re-entry must stay paused: {:?}",
            imported.status
        );
    };
    assert_eq!(
        fields.len(),
        2,
        "all omitted secret values must be re-entered"
    );
    let reentered = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        inputs: BTreeMap::from([
            ("password".to_owned(), "new-secret".to_owned()),
            ("one time code".to_owned(), "654321".to_owned()),
        ]),
        ..ContinueTaskRequest::default()
    });
    assert!(matches!(
        reentered.data.unwrap().status,
        TaskStatus::NeedsApproval { .. }
    ));
    assert!(script.requests.lock().unwrap().is_empty());

    let approved = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        approve: Some(true),
        ..ContinueTaskRequest::default()
    });
    assert!(approved.ok);
    assert!(matches!(
        settle(&restored, &imported.id).await.status,
        TaskStatus::Done { .. }
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].vars["password"], "new-secret");
    assert!(requests[0].allow_destructive);
}

#[tokio::test]
async fn opaque_secret_handles_survive_without_resolving_their_values() {
    let (tasks, _) = controller(Vec::new());
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({"app": "Shop", "steps": [
                {"enter": {"password field": "${password}", "email field": "${email}"}}
            ]}))),
            fact_handles: BTreeMap::from([(
                "password".to_owned(),
                tinycomputer_bus::agent::SecretHandle {
                    handle: "secret-handle-7".to_owned(),
                    mask: "••••1234".to_owned(),
                    kind: "password".to_owned(),
                },
            )]),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    let serialized = String::from_utf8_lossy(&encoded);
    assert!(serialized.contains("secret-handle-7"));
    assert!(!serialized.contains("actual-password-value"));
    drop(tasks);

    let (restored, script) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[],
        None,
    )]);
    let imported = restored.import_paused_task(&encoded).unwrap();
    let continued = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        inputs: BTreeMap::from([("email".to_owned(), "asha@example.test".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    assert!(continued.ok);
    settle(&restored, &imported.id).await;
    assert_eq!(
        script.requests.lock().unwrap()[0].fact_handles["password"].handle,
        "secret-handle-7"
    );
}

#[tokio::test]
async fn payment_checkpoints_are_not_exported_as_resumable_tasks() {
    let (tasks, _) = controller(vec![gated_for_persistence(
        "Pay now",
        "paying for the order",
    )]);
    let view = start(
        &tasks,
        json!({"app": "Shop", "steps": [
            "open the order",
            {"stop_before": "paying for the order"}
        ]}),
        &[],
    );
    let checkpoint = settle(&tasks, &view.id).await;
    assert!(matches!(
        checkpoint.status,
        TaskStatus::Checkpoint {
            continuable: false,
            ..
        }
    ));
    let refused = tasks.export_paused_task(&view.id).unwrap_err();
    assert!(matches!(&refused, crate::PausedTaskError::NotResumable));
    assert!(refused.to_string().contains("resumable pause"));
    assert_eq!(
        code(&tasks.continue_task(ContinueTaskRequest {
            id: view.id,
            approve: Some(true),
            ..ContinueTaskRequest::default()
        })),
        "NOT_WAITING"
    );

    let (pauses, _) = controller(Vec::new());
    let input = start(&pauses, json!({"app": "Mail", "steps": ["${email}"]}), &[]);
    let encoded = pauses.export_paused_task(&input.id).unwrap();
    let mut checkpoint: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    checkpoint["task"]["view"]["status"] = json!({
        "state": "checkpoint",
        "reason": "payment",
        "location": "checkout",
        "summary": "awaiting payment",
        "continuable": false
    });
    let (fresh, _) = controller(Vec::new());
    assert!(matches!(
        fresh.import_paused_task(&serde_json::to_vec(&checkpoint).unwrap()),
        Err(crate::PausedTaskError::NotResumable)
    ));
}

#[tokio::test]
async fn a_human_pause_keeps_its_retry_state_after_import() {
    let (tasks, script) = controller(vec![
        failed_at_step_two(),
        finished_run(FlowStopReason::Completed, vec![], &[], None),
    ]);
    *script.screen.lock().unwrap() = vec![
        "Security check".to_owned(),
        "Verify you are human".to_owned(),
    ];
    let view = start(
        &tasks,
        json!({"app": "browser", "steps": [
            {"browse": "https://flights.test"},
            "search for flights",
            "open the cheapest result"
        ]}),
        &[],
    );
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::NeedsHuman { .. }
    ));
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    drop(tasks);

    let (restored, runner) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[],
        None,
    )]);
    let imported = restored.import_paused_task(&encoded).unwrap();
    assert!(matches!(imported.status, TaskStatus::NeedsHuman { .. }));
    let reply = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        answer: Some("done".to_owned()),
        ..ContinueTaskRequest::default()
    });
    assert!(reply.ok);
    assert!(matches!(
        settle(&restored, &imported.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(runner.requests.lock().unwrap()[0].flow.steps.len(), 2);
}

#[tokio::test]
async fn partial_secret_reentry_keeps_a_human_pause_when_retry_still_needs_input() {
    let (tasks, script) = controller(vec![failed_at_step_two()]);
    *script.screen.lock().unwrap() = vec![
        "Security check".to_owned(),
        "Verify you are human".to_owned(),
    ];
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({
                "app": "browser",
                "steps": [
                    {"browse": "https://flights.test"},
                    {"enter": {"email": "${email}"}},
                    {"enter": {"phone": "${phone}"}}
                ]
            }))),
            facts: BTreeMap::from([
                ("email".to_owned(), "a@example.test".to_owned()),
                ("phone".to_owned(), "555-0100".to_owned()),
            ]),
            secret_facts: vec!["email".to_owned(), "phone".to_owned()],
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::NeedsHuman { .. }
    ));
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    drop(tasks);

    // Preserve the saved retry step that needs phone, but make the exported
    // current flow mention only email, matching a retry whose missing secret
    // exists only in its pending continuation.
    let mut snapshot: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    snapshot["task"]["flow"]["steps"]
        .as_array_mut()
        .unwrap()
        .truncate(2);
    let (restored, _) = controller(Vec::new());
    let imported = restored
        .import_paused_task(&serde_json::to_vec(&snapshot).unwrap())
        .unwrap();
    assert!(matches!(imported.status, TaskStatus::NeedsInput { .. }));

    let partial = restored.continue_task(ContinueTaskRequest {
        id: imported.id.clone(),
        inputs: BTreeMap::from([("email".to_owned(), "new@example.test".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    let TaskStatus::NeedsInput { fields } = partial.data.unwrap().status else {
        panic!("the task must keep asking for the retry's missing phone secret");
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["phone"]
    );
}

#[tokio::test]
async fn process_local_screenshot_handles_are_dropped_from_snapshots() {
    let (tasks, _) = controller(vec![gated_for_persistence("Send", "sending the email")]);
    let view = start(
        &tasks,
        json!({"app": "Mail", "steps": [{"stop_before": "sending the email"}]}),
        &[],
    );
    settle(&tasks, &view.id).await;
    let screenshot = tinycomputer_bus::browser::OutputRef {
        id: tinycomputer_bus::browser::OutputId::new("o-local"),
        total_bytes: 3,
        sha256: "abc".to_owned(),
        media_type: "image/png".to_owned(),
        width: 1,
        height: 1,
    };
    let cell = tasks.find(&view.id).unwrap();
    cell.state
        .lock()
        .unwrap()
        .artifacts
        .push(screenshot.clone());
    cell.view.send_modify(|view| {
        if let TaskStatus::NeedsApproval {
            screenshot: held, ..
        } = &mut view.status
        {
            *held = Some(screenshot);
        }
    });

    let encoded = tasks.export_paused_task(&view.id).unwrap();
    assert!(!String::from_utf8_lossy(&encoded).contains("o-local"));
    drop(tasks);
    let (restored, _) = controller(Vec::new());
    let imported = restored.import_paused_task(&encoded).unwrap();
    assert!(matches!(
        imported.status,
        TaskStatus::NeedsApproval {
            screenshot: None,
            ..
        }
    ));
    assert_eq!(
        restored.report(&imported.id).data.unwrap().artifacts,
        Vec::<tinycomputer_bus::browser::OutputRef>::new()
    );
}

#[tokio::test]
async fn import_rejects_unknown_versions_and_inconsistent_pending_state() {
    let (tasks, _) = controller(Vec::new());
    let view = start(&tasks, json!({"app": "Mail", "steps": ["${email}"]}), &[]);
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    let mut unknown: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    unknown["version"] = json!(3);
    let (fresh, _) = controller(Vec::new());
    let unsupported = fresh
        .import_paused_task(&serde_json::to_vec(&unknown).unwrap())
        .unwrap_err();
    assert!(matches!(
        &unsupported,
        crate::PausedTaskError::UnsupportedVersion(3)
    ));
    assert!(unsupported.to_string().contains("version 3"));
    let mut legacy: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    legacy["version"] = json!(1);
    legacy["task"]
        .as_object_mut()
        .unwrap()
        .remove("handoff_to_planner");
    let (legacy_tasks, _) = controller(Vec::new());
    assert!(
        legacy_tasks
            .import_paused_task(&serde_json::to_vec(&legacy).unwrap())
            .is_ok()
    );
    let malformed = fresh
        .import_paused_task(br#"{"version":1,"task":{}}"#)
        .unwrap_err();
    assert!(std::error::Error::source(&malformed).is_some());
    assert!(malformed.to_string().contains("paused-task JSON"));
    assert_eq!(
        fresh.list().data.unwrap(),
        Vec::<tinycomputer_bus::agent::TaskView>::new()
    );
}

#[tokio::test]
async fn import_refuses_invalid_ids_duplicate_tasks_and_unmatched_approval_state() {
    let (tasks, _) = controller(Vec::new());
    let view = start(&tasks, json!({"app": "Mail", "steps": ["${email}"]}), &[]);
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    let (fresh, _) = controller(Vec::new());
    let imported = fresh.import_paused_task(&encoded).unwrap();
    assert_eq!(imported.id, view.id);
    assert!(fresh.import_paused_task(&encoded).is_err());

    let mut invalid_id: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    invalid_id["task"]["id"] = json!("nonsense");
    invalid_id["task"]["view"]["id"] = json!("nonsense");
    let (fresh, _) = controller(Vec::new());
    assert!(
        fresh
            .import_paused_task(&serde_json::to_vec(&invalid_id).unwrap())
            .is_err()
    );
    assert_eq!(
        fresh.list().data.unwrap(),
        Vec::<tinycomputer_bus::agent::TaskView>::new()
    );

    let (tasks, _) = controller(vec![gated_for_persistence("Send", "sending the email")]);
    let view = start(
        &tasks,
        json!({"app": "Mail", "steps": [{"stop_before": "sending the email"}]}),
        &[],
    );
    settle(&tasks, &view.id).await;
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    let mut malformed: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    malformed["task"]["resume"] = serde_json::Value::Null;
    let (fresh, _) = controller(Vec::new());
    assert!(
        fresh
            .import_paused_task(&serde_json::to_vec(&malformed).unwrap())
            .is_err()
    );
    assert_eq!(
        fresh.list().data.unwrap(),
        Vec::<tinycomputer_bus::agent::TaskView>::new()
    );
}

#[tokio::test]
async fn an_alias_id_cannot_replace_a_registered_task_or_advance_the_allocator() {
    let (source, _) = controller(Vec::new());
    let source_task = start(&source, json!({"app": "Mail", "steps": ["${email}"]}), &[]);
    let encoded = source.export_paused_task(&source_task.id).unwrap();
    let mut alias: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    alias["task"]["id"] = json!("t-01");
    alias["task"]["view"]["id"] = json!("t-01");
    let alias = serde_json::to_vec(&alias).unwrap();

    let (tasks, _) = controller(Vec::new());
    let first = start(&tasks, json!({"app": "Mail", "steps": ["${first}"]}), &[]);
    let second = start(&tasks, json!({"app": "Mail", "steps": ["${second}"]}), &[]);
    assert_eq!(first.id, TaskId::new("t-1"));
    assert_eq!(second.id, TaskId::new("t-2"));

    assert!(tasks.import_paused_task(&alias).is_err());
    let ids = tasks
        .list()
        .data
        .unwrap()
        .into_iter()
        .map(|view| view.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [second.id, first.id]);
    let next = start(&tasks, json!({"app": "Mail", "steps": ["${third}"]}), &[]);
    assert_eq!(next.id, TaskId::new("t-3"));

    let mut exhausted: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let max_id = format!("t-{}", u64::MAX);
    exhausted["task"]["id"] = json!(max_id);
    exhausted["task"]["view"]["id"] = json!(max_id);
    let exhausted = serde_json::to_vec(&exhausted).unwrap();
    let (tasks, _) = controller(Vec::new());
    let first = start(&tasks, json!({"app": "Mail", "steps": ["${first}"]}), &[]);
    assert_eq!(first.id, TaskId::new("t-1"));
    assert!(tasks.import_paused_task(&exhausted).is_err());
    let next = start(&tasks, json!({"app": "Mail", "steps": ["${second}"]}), &[]);
    assert_eq!(next.id, TaskId::new("t-2"));
}

#[test]
fn task_id_allocator_refuses_to_wrap() {
    let (tasks, _) = controller(Vec::new());
    tasks
        .counter
        .store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    let reply = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "Mail", "steps": ["finish"]}))),
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&reply), "TOO_MANY_TASKS");
    assert_eq!(
        tasks.list().data.unwrap(),
        Vec::<tinycomputer_bus::agent::TaskView>::new()
    );
}

#[tokio::test]
async fn persistence_refuses_secret_leaks_and_host_bound_browser_endpoints() {
    let (tasks, _) = controller(Vec::new());
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({"app": "Mail", "steps": ["${email}"]}))),
            facts: BTreeMap::from([("password".to_owned(), "hidden-value".to_owned())]),
            constraints: TaskConstraints {
                browser_endpoint: Some("wss://user:token@browser.test/session".to_owned()),
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let endpoint = tasks.export_paused_task(&view.id).unwrap_err();
    assert!(matches!(
        &endpoint,
        crate::PausedTaskError::HostBoundEndpoint
    ));
    assert!(endpoint.to_string().contains("browser_endpoint"));

    let (tasks, runner) = controller(Vec::new());
    let view = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(json!({"app": "Mail", "steps": ["${email}"]}))),
            facts: BTreeMap::from([("password".to_owned(), "hidden-value".to_owned())]),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    runner
        .host_secrets
        .lock()
        .unwrap()
        .push("runner-held-secret".to_owned());
    let cell = tasks.find(&view.id).unwrap();
    cell.state.lock().unwrap().reads.insert(
        "confirmation".to_owned(),
        "page echoed runner-held-secret".to_owned(),
    );
    assert!(matches!(
        tasks.export_paused_task(&view.id),
        Err(crate::PausedTaskError::SecretValueInState)
    ));

    let clean = tasks.export_paused_task(&view.id).unwrap_err();
    assert!(clean.to_string().contains("secret value"));

    let (tasks, _) = controller(Vec::new());
    let view = start(&tasks, json!({"app": "Mail", "steps": ["${email}"]}), &[]);
    let encoded = tasks.export_paused_task(&view.id).unwrap();
    let mut endpoint_snapshot: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    endpoint_snapshot["task"]["constraints"]["browser_endpoint"] =
        json!("wss://user:token@browser.test/session");
    let (fresh, _) = controller(Vec::new());
    assert!(matches!(
        fresh.import_paused_task(&serde_json::to_vec(&endpoint_snapshot).unwrap()),
        Err(crate::PausedTaskError::HostBoundEndpoint)
    ));
    assert_eq!(
        fresh.list().data.unwrap(),
        Vec::<tinycomputer_bus::agent::TaskView>::new()
    );
}

fn gated_for_persistence(target: &str, phrase: &str) -> DesktopResponse {
    finished_run(
        FlowStopReason::StoppedBeforeDestructive,
        vec![step(
            "1",
            "stop_before",
            phrase,
            StepOutcome::Gated,
            "found it",
        )],
        &[],
        Some(target),
    )
}
