//! Tests for walls only a person can pass, and for releasing a task's
//! surfaces when it ends another way.

use super::*;

#[tokio::test]
async fn a_human_wall_pauses_for_a_person_and_the_step_runs_again() {
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
    let paused = settle(&tasks, &view.id).await;
    let TaskStatus::NeedsHuman { reason, .. } = &paused.status else {
        panic!("{:?}", paused.status);
    };
    assert!(reason.starts_with("prove you are human"), "{reason}");
    assert_eq!(paused.next, ["ContinueTask", "CancelTask", "TaskReport"]);
    assert!(paused.summary.contains("A person is needed"));
    assert!(
        script.released.lock().unwrap().is_empty(),
        "the page stays open for the person"
    );

    let resumed = tasks.continue_task(ContinueTaskRequest {
        id: view.id.clone(),
        answer: Some("done".to_owned()),
        ..ContinueTaskRequest::default()
    });
    assert_eq!(resumed.data.unwrap().status, TaskStatus::Running);
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::Done { .. }
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests[1].flow.app, "browser");
    assert_eq!(
        requests[1].flow.steps.len(),
        2,
        "the failed step and the rest"
    );
    assert_eq!(
        *script.released.lock().unwrap(),
        std::slice::from_ref(&view.id)
    );
    let waits = journaled(&script, "resume");
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].0.as_ref(), Some(&view.id));
    assert_eq!(waits[0].1["state"], "needs_human");
}

#[tokio::test]
async fn an_ordinary_failure_or_cancel_releases_the_tasks_surfaces() {
    let (tasks, script) = controller(vec![failed_at_step_two()]);
    *script.screen.lock().unwrap() = vec!["Flights from Delhi".to_owned()];
    let failed = start(&tasks, json!({"app": "browser", "steps": ["a", "b"]}), &[]);
    assert!(matches!(
        settle(&tasks, &failed.id).await.status,
        TaskStatus::Failed { .. }
    ));
    let retry = tasks.continue_task(ContinueTaskRequest {
        id: failed.id.clone(),
        answer: Some("done".to_owned()),
        ..ContinueTaskRequest::default()
    });
    assert_eq!(code(&retry), "NOT_WAITING");

    let running = start(&tasks, json!({"app": "Mail", "steps": ["a"]}), &[]);
    assert!(tasks.cancel(&running.id).ok);
    assert_eq!(*script.released.lock().unwrap(), [failed.id, running.id]);
}

#[tokio::test]
async fn a_budget_failure_is_never_mistaken_for_a_human_wall() {
    let (tasks, script) = controller(vec![finished_run(
        FlowStopReason::ActionBudget,
        vec![],
        &[],
        None,
    )]);
    *script.screen.lock().unwrap() = vec!["Enter the OTP".to_owned()];
    let view = start(&tasks, json!({"app": "Mail", "steps": ["a"]}), &[]);
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::Failed { .. }
    ));
}

#[tokio::test]
async fn a_solved_captcha_runs_the_failed_step_again_without_a_person() {
    let (tasks, script) = controller(vec![
        failed_at_step_two(),
        finished_run(FlowStopReason::Completed, vec![], &[], None),
    ]);
    *script.screen.lock().unwrap() = vec!["Verify you are human".to_owned()];
    script
        .captcha
        .lock()
        .unwrap()
        .push_back(crate::CaptchaOutcome::Solved { attempts: 1 });
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
        TaskStatus::Done { .. }
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].flow.steps.len(),
        2,
        "the failed step and the rest"
    );
    let tries = journaled(&script, "captcha");
    assert_eq!(tries.len(), 1);
    assert_eq!(tries[0].1["outcome"], "solved");
}

#[tokio::test]
async fn an_unsolved_captcha_still_asks_a_person_and_says_what_was_tried() {
    let (tasks, script) = controller(vec![failed_at_step_two()]);
    *script.screen.lock().unwrap() = vec!["Please complete the reCAPTCHA".to_owned()];
    script
        .captcha
        .lock()
        .unwrap()
        .push_back(crate::CaptchaOutcome::Unsolved {
            attempts: 3,
            reason: "still unsolved after 3 tries".to_owned(),
        });
    let view = start(&tasks, json!({"app": "browser", "steps": ["a", "b"]}), &[]);
    let paused = settle(&tasks, &view.id).await;
    let TaskStatus::NeedsHuman { reason, .. } = &paused.status else {
        panic!("{:?}", paused.status);
    };
    assert!(reason.starts_with("solve the captcha"), "{reason}");
    assert!(reason.contains("tried 3 time(s) automatically"), "{reason}");
    assert_eq!(journaled(&script, "captcha")[0].1["outcome"], "unsolved");
}

#[tokio::test]
async fn an_unsupported_captcha_names_why_and_a_missing_solver_says_nothing() {
    let (tasks, script) = controller(vec![failed_at_step_two(), failed_at_step_two()]);
    *script.screen.lock().unwrap() = vec!["Type the characters: captcha".to_owned()];
    script
        .captcha
        .lock()
        .unwrap()
        .push_back(crate::CaptchaOutcome::Unsupported {
            reason: "the challenge is not an image grid (typed)".to_owned(),
        });
    let view = start(&tasks, json!({"app": "browser", "steps": ["a", "b"]}), &[]);
    let TaskStatus::NeedsHuman { reason, .. } = settle(&tasks, &view.id).await.status else {
        panic!("a person is needed");
    };
    assert!(reason.contains("not one the solver can answer"), "{reason}");

    let other = start(&tasks, json!({"app": "browser", "steps": ["a", "b"]}), &[]);
    let TaskStatus::NeedsHuman { reason, .. } = settle(&tasks, &other.id).await.status else {
        panic!("a person is needed");
    };
    assert_eq!(reason, "solve the captcha, then continue the task");
}

#[tokio::test]
async fn a_one_time_code_is_never_handed_to_the_captcha_solver() {
    let (tasks, script) = controller(vec![failed_at_step_two()]);
    *script.screen.lock().unwrap() = vec!["Enter the OTP we sent".to_owned()];
    let view = start(&tasks, json!({"app": "browser", "steps": ["a", "b"]}), &[]);
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::NeedsHuman { .. }
    ));
    assert_eq!(
        script
            .captcha_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn a_captcha_that_keeps_returning_goes_to_a_person_after_three_solves() {
    let replies = (0..4).map(|_| failed_at_step_two()).collect();
    let (tasks, script) = controller(replies);
    *script.screen.lock().unwrap() = vec!["Verify you are human".to_owned()];
    for _ in 0..5 {
        script
            .captcha
            .lock()
            .unwrap()
            .push_back(crate::CaptchaOutcome::Solved { attempts: 1 });
    }
    // Each retry starts at the failed step, so the flow is long enough for
    // every scripted failure to land on its second step.
    let view = start(
        &tasks,
        json!({"app": "browser", "steps": ["a", "b", "c", "d", "e", "f"]}),
        &[],
    );
    assert!(matches!(
        settle(&tasks, &view.id).await.status,
        TaskStatus::NeedsHuman { .. }
    ));
    assert_eq!(
        script
            .captcha_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        crate::task::drive::MAX_SOLVED_WALLS
    );
    assert_eq!(script.requests.lock().unwrap().len(), 4);
}
