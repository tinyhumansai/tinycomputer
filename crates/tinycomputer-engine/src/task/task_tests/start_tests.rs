//! Tests for starting a task: finishing with its reads, asking for missing
//! values first, refusing what cannot start, and what each run is given.

use super::*;

#[tokio::test]
async fn a_finished_flow_is_done_with_its_reads_and_no_fact_values() {
    let (tasks, script) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![
            step("1", "browse", "https://flights.test", StepOutcome::Done, ""),
            step(
                "2",
                "enter",
                "email asha@example.com",
                StepOutcome::Done,
                "",
            ),
        ],
        &[
            ("email", "asha@example.com"),
            ("cheapest", "IndiGo ₹6,840 for asha@example.com"),
        ],
        None,
    )]);
    let view = start(
        &tasks,
        json!({"app": "browser", "steps": [
            {"browse": "https://flights.test"},
            {"enter": {"email": "${email}"}}
        ]}),
        &[("email", "asha@example.com")],
    );
    assert_eq!(view.status, TaskStatus::Running);
    assert_eq!(view.next, ["AwaitTask", "CancelTask"]);
    let done = settle(&tasks, &view.id).await;
    let TaskStatus::Done {
        answer, records, ..
    } = &done.status
    else {
        panic!("{:?}", done.status);
    };
    assert!(
        answer.contains("cheapest: IndiGo ₹6,840 for ‹email›"),
        "{answer}"
    );
    assert!(!done.summary.contains("asha@"));
    assert_eq!(
        records["cheapest"][0]["value"],
        "IndiGo ₹6,840 for asha@example.com"
    );
    assert!(!records.contains_key("email"), "facts are not records");
    assert!((done.progress - 1.0).abs() < f32::EPSILON);
    assert_eq!(done.step.as_ref().unwrap().intent, "email ‹email›");
    assert_eq!(done.step.as_ref().unwrap().surface, "browser");
    assert_eq!(done.next, ["TaskReport"]);

    let request = &script.requests.lock().unwrap()[0];
    assert_eq!(request.vars["email"], "asha@example.com");
    assert!(
        request.include_values,
        "Jev reads what fields hold; the runtime masks secrets"
    );
    assert!(!request.allow_destructive);
    assert_eq!(
        (request.max_actions, request.max_model_calls, request.votes),
        (120, 6000, 7)
    );
    assert!(request.facts.is_empty(), "an email is shared, not secret");
    assert_eq!(request.brief.details["email"], "asha@example.com");
    assert!(
        request
            .brief
            .rules
            .iter()
            .any(|rule| rule.contains("Never pay")),
        "{:?}",
        request.brief.rules
    );

    let report = tasks.report(&view.id).data.unwrap();
    assert_eq!(report.steps.len(), 2);
    assert!(report.flow.is_some());
    assert_eq!(tasks.list().data.unwrap()[0].id, view.id);

    let full = tinycomputer_bus::agent::TaskReportRequest::new(view.id.clone());
    assert_eq!(tasks.report_for(&full).data.unwrap(), report);
    let lean = tinycomputer_bus::agent::TaskReportRequest {
        trace: false,
        ..full
    };
    let lean = tasks.report_for(&lean).data.unwrap();
    assert_eq!(lean.trace, [] as [tinycomputer_bus::JevExchange; 0]);
    assert_eq!(lean.steps, report.steps);
}

#[tokio::test]
async fn missing_values_are_asked_for_before_anything_runs() {
    let (tasks, script) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[],
        None,
    )]);
    let view = start(
        &tasks,
        json!({"app": "browser", "steps": [
            {"enter": {
                "email": "${email}",
                "birth date": "${date of birth}",
                "phone": "${phone}",
                "traveller count": "${travellers}"
            }},
            {"read": {"what": "the fare", "into": "fare"}},
            {"verify": "the fare ${fare} is shown"}
        ]}),
        &[("phone", "+91 98765 43210")],
    );
    let TaskStatus::NeedsInput { fields } = &view.status else {
        panic!("{:?}", view.status);
    };
    let asked = fields
        .iter()
        .map(|field| (field.name.as_str(), field.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        asked,
        [
            ("date of birth", InputKind::Date),
            ("email", InputKind::Email),
            ("travellers", InputKind::Number)
        ],
        "first-use order (json! sorts the slots), and never a value a read step defines"
    );
    assert_eq!(view.next, ["ContinueTask", "CancelTask"]);
    assert!(script.requests.lock().unwrap().is_empty());

    let partial = tasks.continue_task(ContinueTaskRequest {
        id: view.id.clone(),
        inputs: BTreeMap::from([("email".to_owned(), "a@b.c".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    let TaskStatus::NeedsInput { fields } = &partial.data.unwrap().status else {
        panic!("still missing values");
    };
    assert_eq!(fields.len(), 2);

    // A card number is taken, as a secret.
    let card = tasks.continue_task(ContinueTaskRequest {
        id: view.id.clone(),
        inputs: BTreeMap::from([("card number".to_owned(), "4111".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    assert!(matches!(
        card.data.unwrap().status,
        TaskStatus::NeedsInput { .. }
    ));

    let complete = tasks.continue_task(ContinueTaskRequest {
        id: view.id.clone(),
        inputs: BTreeMap::from([
            ("date of birth".to_owned(), "1990-04-02".to_owned()),
            ("travellers".to_owned(), "1".to_owned()),
        ]),
        ..ContinueTaskRequest::default()
    });
    assert_eq!(complete.data.unwrap().status, TaskStatus::Running);
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::Done { .. }
    ));
    let request = &script.requests.lock().unwrap()[0];
    assert_eq!(request.vars["email"], "a@b.c");
    assert_eq!(request.vars["phone"], "+91 98765 43210");
    assert_eq!(request.vars["card number"], "4111");
    assert_eq!(request.facts, BTreeSet::from(["card number".to_owned()]));
    assert_eq!(request.brief.secrets, ["card number"]);
    assert!(!request.brief.details.contains_key("card number"));
    assert_eq!(request.brief.details["date of birth"], "1990-04-02");
}

#[tokio::test]
async fn a_value_supplied_for_a_missing_fact_still_fails_fast_if_it_leaks() {
    // `passport number` is not declared at `StartTask`, so the flow only
    // looks like it is missing a plain value; once `ContinueTask` supplies
    // it, it becomes a secret the same as one declared up front, and the
    // `verify` step that reads it is exactly as invalid as if it had been
    // declared from the start. This must fail before the task spawns.
    let (tasks, script) = controller(Vec::new());
    let view = start(
        &tasks,
        json!({"app": "Mail", "steps": [
            {"verify": "shows ${passport number}"}
        ]}),
        &[],
    );
    let TaskStatus::NeedsInput { fields } = &view.status else {
        panic!("{:?}", view.status);
    };
    assert_eq!(fields[0].name, "passport number");

    let supplied = tasks.continue_task(ContinueTaskRequest {
        id: view.id,
        inputs: BTreeMap::from([("passport number".to_owned(), "Z1234567".to_owned())]),
        ..ContinueTaskRequest::default()
    });
    assert_eq!(code(&supplied), "INVALID_FLOW");
    assert!(
        supplied.error.unwrap().message.contains("is a secret"),
        "a secret supplied to answer a missing-input prompt is still a secret"
    );
    assert!(
        script.requests.lock().unwrap().is_empty(),
        "the invalid flow must never be run"
    );
}

#[tokio::test]
async fn constraints_that_cannot_start_a_task_are_refused() {
    let (tasks, _) = controller(Vec::new());
    // `*` names no site card details may be typed on, alone or beside one
    // that does: with it, every public site is admitted.
    for origins in [vec!["*"], vec!["https://.pay.test", "*"]] {
        let any_site = tasks.start(&StartTaskRequest {
            flow: Some(flow(json!({"app": "Mail", "steps": ["x"]}))),
            constraints: TaskConstraints {
                payment: PaymentMode::FillThenApprove,
                origins: origins.iter().map(|origin| (*origin).to_owned()).collect(),
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        });
        assert_eq!(code(&any_site), "ORIGINS_REQUIRED", "{origins:?}");
    }
    let relative_profile = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "Mail", "steps": ["x"]}))),
        constraints: TaskConstraints {
            browser_profile: Some("chrome-profile".to_owned()),
            ..TaskConstraints::default()
        },
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&relative_profile), "INVALID_REQUEST");
    assert!(
        relative_profile
            .error
            .unwrap()
            .message
            .contains("absolute folder")
    );
}

#[tokio::test]
async fn a_task_browser_starts_only_with_a_binary_and_profile_it_can_launch() {
    let (tasks, _) = controller(Vec::new());
    let refused = |constraints: TaskConstraints| {
        tasks.start(&StartTaskRequest {
            flow: Some(flow(json!({"app": "Mail", "steps": ["x"]}))),
            constraints,
            ..StartTaskRequest::default()
        })
    };
    // A binary is the absolute path of an executable file on this machine:
    // never a name looked up on the `PATH`, a folder, a file that is not
    // there or would not run, or a path with a space before it (a
    // different, relative path). Which browser it is stays the host's
    // choice, so any such file will do.
    let here = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let profile = std::env::temp_dir()
        .join("tinycomputer-profile")
        .to_string_lossy()
        .into_owned();
    // A file that is there but would not run (no executable mark).
    let plain =
        std::env::temp_dir().join(format!("tinycomputer-not-a-binary-{}", std::process::id()));
    std::fs::write(&plain, "not a browser").unwrap();
    let mut binaries = vec![
        "  ".to_owned(),
        "chrome".to_owned(),
        std::env::temp_dir().to_string_lossy().into_owned(),
        "/nonexistent/tinycomputer/chrome".to_owned(),
        format!(" {here}"),
    ];
    if cfg!(unix) {
        binaries.push(plain.to_string_lossy().into_owned());
    }
    for binary in binaries {
        let reply = refused(TaskConstraints {
            browser_executable: Some(binary.clone()),
            ..TaskConstraints::default()
        });
        assert_eq!(code(&reply), "INVALID_REQUEST", "{binary:?}");
    }
    assert_eq!(
        code(&refused(TaskConstraints {
            browser_profile: Some(format!(" {profile}")),
            ..TaskConstraints::default()
        })),
        "INVALID_REQUEST",
        "a space before the folder makes it relative"
    );
    // A browser attached to is not launched: no binary or profile goes with it.
    for constraints in [
        TaskConstraints {
            browser_endpoint: Some("ws://127.0.0.1:9222".to_owned()),
            browser_profile: Some(profile.clone()),
            ..TaskConstraints::default()
        },
        TaskConstraints {
            browser_endpoint: Some("ws://127.0.0.1:9222".to_owned()),
            browser_executable: Some(here.clone()),
            ..TaskConstraints::default()
        },
    ] {
        let reply = refused(constraints);
        assert_eq!(code(&reply), "INVALID_REQUEST");
        assert!(
            reply
                .error
                .unwrap()
                .message
                .contains("browser_endpoint attaches")
        );
    }
    // A binary that is there, and an absolute folder, start the task.
    let started = refused(TaskConstraints {
        browser_executable: Some(here),
        browser_profile: Some(profile),
        ..TaskConstraints::default()
    });
    assert!(started.ok, "{:?}", started.error);
    let _ = std::fs::remove_file(&plain);
}

#[tokio::test]
async fn requests_that_cannot_start_are_refused_with_a_hint() {
    let (tasks, _) = controller(Vec::new());
    let misspelt = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "Mail", "steps": ["x"]}))),
        facts: BTreeMap::from([("passport no".to_owned(), "Z1234567".to_owned())]),
        secret_facts: vec!["passport number".to_owned()],
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&misspelt), "UNKNOWN_SECRET");
    let anywhere = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "Mail", "steps": ["x"]}))),
        constraints: TaskConstraints {
            payment: PaymentMode::FillThenApprove,
            ..TaskConstraints::default()
        },
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&anywhere), "ORIGINS_REQUIRED");
    let nothing = tasks.start(&StartTaskRequest::default());
    assert_eq!(code(&nothing), "INVALID_REQUEST");
    let invalid = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "", "steps": []}))),
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&invalid), "INVALID_FLOW");
    assert_ne!(invalid.error.unwrap().hint, "");

    let fact_leak = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "Mail", "steps": [
            {"verify": "shows ${email}"}
        ]}))),
        facts: BTreeMap::from([("email".to_owned(), "sam@example.com".to_owned())]),
        secret_facts: vec!["email".to_owned()],
        ..StartTaskRequest::default()
    });
    assert_eq!(code(&fact_leak), "INVALID_FLOW");
    assert!(
        fact_leak.error.unwrap().message.contains("is a secret"),
        "a secret referenced outside an enter step fails fast"
    );
    let shared = tasks.start(&StartTaskRequest {
        flow: Some(flow(json!({"app": "Mail", "steps": [
            {"verify": "shows ${email}"}
        ]}))),
        facts: BTreeMap::from([("email".to_owned(), "sam@example.com".to_owned())]),
        ..StartTaskRequest::default()
    });
    assert!(
        shared.ok,
        "a shared fact may be named in any step: {:?}",
        shared.error
    );

    let planless = tasks.start(&StartTaskRequest {
        task: Some("book the cheapest flight to Srinagar".to_owned()),
        ..StartTaskRequest::default()
    });
    let view = planless.data.unwrap();
    assert!(matches!(view.status, TaskStatus::NeedsPlan { ref guide } if guide.contains("browse")));
    assert_eq!(view.next, ["StartTask"]);
    assert_eq!(
        code(&tasks.continue_task(ContinueTaskRequest {
            id: view.id,
            ..ContinueTaskRequest::default()
        })),
        "NOT_WAITING"
    );
}

#[tokio::test]
async fn extracted_rows_become_structured_records() {
    let (tasks, _) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[("flights", r#"[["IndiGo","₹6,840"],["Vistara","₹7,210"]]"#)],
        None,
    )]);
    let view = start(&tasks, json!({"app": "browser", "steps": ["a"]}), &[]);
    let TaskStatus::Done { records, .. } = settle(&tasks, &view.id).await.status else {
        panic!("done");
    };
    assert_eq!(records["flights"].len(), 2);
    assert_eq!(records["flights"][1]["field 2"], "₹7,210");
}

#[tokio::test]
async fn a_run_gets_the_callers_values_and_the_flow_keeps_its_own_definitions() {
    let (tasks, script) = controller(Vec::new());
    start(
        &tasks,
        json!({"app": "browser", "vars": {"first_name": "${first name}"}, "steps": [
            {"enter": {"first name": "${first_name}"}}
        ]}),
        &[("first name", "Asha")],
    );
    for _ in 0..50 {
        if !script.requests.lock().unwrap().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let request = &script.requests.lock().unwrap()[0];
    assert_eq!(request.vars["first name"], "Asha");
    assert!(
        !request.vars.contains_key("first_name"),
        "a definition passed as a caller value would shadow its expansion"
    );
    assert_eq!(request.flow.vars["first_name"], "${first name}");
}
