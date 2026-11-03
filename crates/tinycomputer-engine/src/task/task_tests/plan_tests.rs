//! Tests for planning a plain-language task before running it.

use super::super::store::Run;
use super::failed_at_step_two;
use super::*;
use crate::planner::{LanguageModel, Planner, Turn};
use tinycomputer_bus::agent::SurfaceKind;

struct WarmUsageRunner(Arc<Script>);

impl FlowRunner for WarmUsageRunner {
    fn run(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
    ) -> FlowFuture {
        self.0.run(task, constraints, request)
    }

    fn run_with_usage(
        &self,
        task: &TaskId,
        constraints: &TaskConstraints,
        request: RunFlowRequest,
        usage: Arc<crate::UsageRecorder>,
    ) -> FlowFuture {
        usage.record_call(crate::ModelRole::Jev, Some(5), Some(1), Some(2), Some(3));
        self.0.run(task, constraints, request)
    }

    fn warm_with_usage(
        &self,
        task: &TaskId,
        votes: u32,
        usage: Arc<crate::UsageRecorder>,
    ) -> super::super::PrepareFuture {
        usage.record_call(crate::ModelRole::Jev, Some(40), Some(10), Some(2), Some(12));
        self.0.warm(task, votes)
    }
}

/// A model that always answers with the same text.
struct Fixed(Result<String, String>);

impl crate::planner::LanguageModel for Fixed {
    fn complete(&self, _turns: &[crate::planner::Turn]) -> crate::planner::Completion {
        let answer = self.0.clone();
        Box::pin(async move { answer })
    }
}

fn planned(replies: Vec<DesktopResponse>, answer: Result<&str, &str>) -> (Tasks, Arc<Script>) {
    let (tasks, script) = controller(replies);
    *script.origin.lock().unwrap() = Some("https://shop.example/".to_owned());
    let model = Arc::new(Fixed(answer.map(str::to_owned).map_err(str::to_owned)));
    (
        tasks.with_planner(crate::planner::Planner::new(model)),
        script,
    )
}

#[tokio::test]
async fn a_plain_language_task_is_planned_then_run() {
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(r#"{"app": "Mail", "steps": [{"enter": {"recipient": "${email}"}}]}"#),
    );
    assert!(tasks.planner_configured());
    let reply = tasks.start(&StartTaskRequest {
        task: Some("email Sam".to_owned()),
        facts: BTreeMap::from([("email".to_owned(), "sam@example.com".to_owned())]),
        ..StartTaskRequest::default()
    });
    assert!(reply.ok, "{:?}", reply.error);
    let started = reply.data.unwrap();
    assert_eq!(started.summary, "Planning the task.");
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests[0].flow.app, "Mail");
    assert_eq!(requests[0].vars["email"], "sam@example.com");
    assert_eq!(
        tasks
            .report(&started.id)
            .data
            .unwrap()
            .flow
            .unwrap()
            .steps
            .len(),
        1
    );
    let plans = journaled(&script, "plan");
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].0.as_ref(),
        Some(&started.id),
        "into the task's journal"
    );
    assert_eq!(plans[0].1["ok"], true);
    assert_eq!(plans[0].1["calls"], 1);
    assert_eq!(plans[0].1["steps"], 1);
    assert!(plans[0].1["wall_ms"].is_u64());
}

#[tokio::test]
async fn a_completed_prefix_hands_the_same_task_and_surface_to_the_planner() {
    let prefix = flow(json!({
        "app": "browser",
        "steps": ["dismiss the cookie banner"]
    }));
    let (tasks, script) = planned(
        vec![
            finished_run(
                FlowStopReason::Completed,
                vec![step(
                    "1",
                    "do",
                    "dismiss the cookie banner",
                    StepOutcome::Done,
                    "",
                )],
                &[],
                None,
            ),
            finished_run(
                FlowStopReason::Completed,
                vec![step(
                    "1",
                    "do",
                    "continue the requested purchase",
                    StepOutcome::Done,
                    "",
                )],
                &[],
                None,
            ),
        ],
        Ok(r#"{"app":"browser","steps":["continue the requested purchase"]}"#),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp from shop.example".to_owned()),
            flow: Some(prefix.clone()),
            handoff_to_planner: true,
            budget: tinycomputer_bus::agent::TaskBudget {
                max_model_calls: Some(1),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                origins: vec![".shop.example".to_owned()],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();

    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].max_model_calls, 0,
        "the continuation planner call is charged to the same task budget"
    );
    assert_eq!(requests[0].flow, prefix);
    assert_eq!(requests[1].flow.app, "browser");
    assert!(
        requests[1]
            .flow
            .steps
            .iter()
            .all(|step| !matches!(step.action(), tinycomputer_bus::FlowAction::Browse(_)))
    );
    assert_eq!(
        script.task_ids.lock().unwrap().as_slice(),
        [started.id.clone(), started.id.clone()],
        "the prefix and planned task share one task id"
    );
    assert!(
        script.opened.lock().unwrap().is_empty(),
        "handoff does not reopen a page"
    );
    assert!(
        script.prepared.lock().unwrap().is_empty(),
        "handoff does not prepare a new session"
    );
    let report = tasks.report(&started.id).data.unwrap();
    assert_eq!(
        report.steps.len(),
        2,
        "both prefix and planner steps are reported"
    );
}

#[tokio::test]
async fn a_failed_native_handoff_spends_its_last_call_before_host_recovery() {
    let prefix = flow(json!({"app": "browser", "steps": ["safe prefix"]}));
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Err("planner failed"),
    );
    *script.plan_recovery.lock().unwrap() = Some(PlanRecoveryResult::Plan(flow(json!({
        "app": "browser",
        "steps": ["host continuation"]
    }))));
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp".to_owned()),
            flow: Some(prefix),
            handoff_to_planner: true,
            budget: tinycomputer_bus::agent::TaskBudget {
                max_model_calls: Some(1),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();

    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Failed { .. }
    ));
    assert!(
        script.plan_requests.lock().unwrap().is_empty(),
        "the failed native call consumes the last budget unit before host recovery"
    );
}

#[tokio::test]
async fn handoff_rejects_a_planner_flow_that_reopens_the_current_shop_homepage() {
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(
            r#"{"app":"browser","steps":[{"if":{"condition":"the cart is open","then":[{"browse":"https://shop.example/"}]}}]}"#,
        ),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp from shop.example".into()),
            flow: Some(flow(json!({"app": "browser", "steps": ["safe prefix"]}))),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Failed { ref reason, .. } if reason.contains("reopen the current shop's start page")
    ));
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_handoff_treats_a_different_shop_port_as_a_different_origin() {
    let (tasks, script) = planned(
        vec![
            finished_run(FlowStopReason::Completed, vec![], &[], None),
            finished_run(FlowStopReason::Completed, vec![], &[], None),
        ],
        Ok(r#"{"app":"browser","steps":[{"browse":"https://shop.example/"}]}"#),
    );
    // The retained page is hosted on a non-default HTTPS port.
    *script.origin.lock().unwrap() = Some("https://shop.example:8443/checkout".to_owned());
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("continue the order".into()),
            flow: Some(flow(json!({"app": "browser", "steps": ["safe prefix"]}))),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
}

#[tokio::test]
async fn a_paused_prefix_keeps_its_handoff_phase_across_export_and_import() {
    let prefix = flow(json!({
        "app": "browser",
        "steps": [{"enter": {"email": "${email}"}}]
    }));
    let (tasks, _) = planned(
        Vec::new(),
        Ok(r#"{"app":"browser","steps":["read the current page"]}"#),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp from shop.example".to_owned()),
            flow: Some(prefix),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                origins: vec![".shop.example".to_owned()],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(started.status, TaskStatus::NeedsInput { .. }));
    let exported = tasks.export_paused_task(&started.id).unwrap();
    let encoded: serde_json::Value = serde_json::from_slice(&exported).unwrap();
    assert_eq!(encoded["task"]["handoff_to_planner"], true);

    let (restored, script) = planned(
        vec![
            finished_run(
                FlowStopReason::Completed,
                vec![step("1", "enter", "email", StepOutcome::Done, "")],
                &[],
                None,
            ),
            finished_run(
                FlowStopReason::Completed,
                vec![step(
                    "1",
                    "do",
                    "read the current page",
                    StepOutcome::Done,
                    "",
                )],
                &[],
                None,
            ),
        ],
        Ok(r#"{"app":"browser","steps":["read the current page"]}"#),
    );
    let imported = restored.import_paused_task(&exported).unwrap();
    assert_eq!(imported.id, started.id);
    assert!(
        restored
            .continue_task(ContinueTaskRequest {
                id: imported.id.clone(),
                inputs: BTreeMap::from([("email".to_owned(), "buyer@example.test".to_owned())]),
                ..ContinueTaskRequest::default()
            })
            .ok
    );
    assert!(matches!(
        settle(&restored, &imported.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(
        script.task_ids.lock().unwrap().as_slice(),
        [imported.id.clone(), imported.id.clone()]
    );
}

#[tokio::test]
async fn a_payment_checkpoint_consumes_prefix_handoff_without_planning() {
    let (tasks, script) = planned(
        vec![finished_run(
            FlowStopReason::StoppedBeforeDestructive,
            vec![step(
                "1",
                "stop_before",
                "Place order",
                StepOutcome::Gated,
                "payment control located",
            )],
            &[],
            Some("Place order"),
        )],
        Ok(r#"{"app":"browser","steps":["read the receipt"]}"#),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp".to_owned()),
            flow: Some(flow(
                json!({"app": "browser", "steps": [{"stop_before": "Place order"}]}),
            )),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                origins: vec![".shop.example".to_owned()],
                payment: PaymentMode::StopAtPayment,
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Checkpoint { .. }
    ));
    assert!(
        !tasks
            .find(&started.id)
            .unwrap()
            .state
            .lock()
            .unwrap()
            .handoff_to_planner
    );
    assert_eq!(
        journaled(&script, "plan"),
        Vec::<(Option<TaskId>, serde_json::Value)>::new()
    );
}

#[tokio::test]
async fn a_failed_prefix_does_not_handoff_to_planning() {
    let (tasks, script) = planned(
        vec![finished_run(
            FlowStopReason::StepFailed,
            vec![step(
                "1",
                "do",
                "dismiss the cookie banner",
                StepOutcome::Failed,
                "missing",
            )],
            &[],
            None,
        )],
        Ok(r#"{"app":"browser","steps":["continue the requested purchase"]}"#),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp".to_owned()),
            flow: Some(flow(
                json!({"app": "browser", "steps": ["dismiss the cookie banner"]}),
            )),
            handoff_to_planner: true,
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Failed { .. }
    ));
    assert_eq!(script.requests.lock().unwrap().len(), 1);
    assert_eq!(
        journaled(&script, "plan"),
        Vec::<(Option<TaskId>, serde_json::Value)>::new()
    );
}

#[tokio::test]
async fn a_handoff_waits_until_every_queued_prefix_run_finishes() {
    let prefix = flow(json!({"app": "browser", "steps": ["first safe recipe step"]}));
    let tail = flow(json!({"app": "browser", "steps": ["remaining recipe steps"]}));
    let (tasks, script) = planned(
        vec![
            finished_run(FlowStopReason::Completed, vec![], &[], None),
            finished_run(FlowStopReason::Completed, vec![], &[], None),
            finished_run(FlowStopReason::Completed, vec![], &[], None),
        ],
        Ok(r#"{"app":"browser","steps":["finish the purchase"]}"#),
    );
    let request = StartTaskRequest {
        task: Some("buy a lamp from shop.example".into()),
        flow: Some(prefix.clone()),
        handoff_to_planner: true,
        ..StartTaskRequest::default()
    };
    let cell = tasks
        .register(&prefix, tinycomputer_core::Facts::default(), &request)
        .unwrap();
    let id = cell.view.borrow().id.clone();
    tasks.spawn(
        &cell,
        vec![
            Run {
                flow: prefix.clone(),
                allow_destructive: false,
                rescue: None,
            },
            Run {
                flow: tail.clone(),
                allow_destructive: false,
                rescue: None,
            },
        ],
    );

    assert!(matches!(
        settle(&tasks, &id).await.status,
        TaskStatus::Done { .. }
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].flow, prefix);
    assert_eq!(requests[1].flow, tail);
    assert_eq!(
        script.task_ids.lock().unwrap().as_slice(),
        [id.clone(), id.clone(), id.clone()],
        "planner starts after the queued prefix tail on the same task"
    );
}

#[tokio::test]
async fn a_planner_question_continues_without_releasing_the_prefix_session() {
    let (tasks, script) = planned(
        vec![
            finished_run(FlowStopReason::Completed, vec![], &[], None),
            finished_run(FlowStopReason::Completed, vec![], &[], None),
        ],
        Ok(r#"{"app":"browser","steps":[{"enter":{"email":"${email}"}}]}"#),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp from shop.example".into()),
            flow: Some(flow(json!({"app": "browser", "steps": ["safe prefix"]}))),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let paused = settle(&tasks, &started.id).await;
    assert!(matches!(paused.status, TaskStatus::NeedsInput { .. }));
    assert!(script.released.lock().unwrap().is_empty());
    assert!(
        tasks
            .continue_task(ContinueTaskRequest {
                id: started.id.clone(),
                inputs: BTreeMap::from([("email".to_owned(), "buyer@example.test".to_owned())]),
                ..ContinueTaskRequest::default()
            })
            .ok
    );
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(
        script.task_ids.lock().unwrap().as_slice(),
        [started.id.clone(), started.id.clone()],
        "planner answer and flow remain in the same task"
    );
    assert_eq!(script.released.lock().unwrap().as_slice(), [started.id]);
}

struct CapturedPlan(Mutex<Vec<Vec<Turn>>>);

impl LanguageModel for CapturedPlan {
    fn complete(&self, turns: &[Turn]) -> crate::planner::Completion {
        self.0.lock().unwrap().push(turns.to_vec());
        Box::pin(async { Ok(r#"{"app":"browser","steps":["finish the purchase"]}"#.to_owned()) })
    }
}

#[tokio::test]
async fn handoff_planner_gets_safe_origin_and_prefix_context_without_page_text() {
    let script = Arc::new(Script {
        replies: Mutex::new(
            vec![
                finished_run(
                    FlowStopReason::Completed,
                    vec![step(
                        "1",
                        "do",
                        "PRIVATE RAW STEP LABEL",
                        StepOutcome::Done,
                        "",
                    )],
                    &[],
                    None,
                ),
                finished_run(FlowStopReason::Completed, vec![], &[], None),
            ]
            .into(),
        ),
        host_secret: Mutex::new(Some("private page marker".into())),
        origin: Mutex::new(Some("https://shop.example/".into())),
        screen: Mutex::new(vec!["cart on current page private page marker".into()]),
        ..Script::default()
    });
    let model = Arc::new(CapturedPlan(Mutex::default()));
    let tasks = Tasks::new(script.clone()).with_planner(Planner::new(model.clone()));
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp from shop.example".into()),
            flow: Some(flow(json!({"app": "browser", "steps": ["safe prefix"]}))),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    let prompt = model
        .0
        .lock()
        .unwrap()
        .iter()
        .flatten()
        .map(|turn| turn.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        prompt.contains("Completed safe-prefix structure: 1 completed browser interaction step(s)")
    );
    assert!(prompt.contains("Do not repeat these steps"));
    assert!(prompt.contains("https://shop.example/"));
    assert!(prompt.contains("Do not open or browse to the shop start page again"));
    assert!(!prompt.contains("private page marker"));
    assert!(!prompt.contains("cart on current page"));
    assert!(!prompt.contains("PRIVATE RAW STEP LABEL"));
}

#[tokio::test]
async fn host_recovers_a_failed_planner_without_replacing_the_task() {
    let (tasks, script) = planned(
        vec![finished_run(
            FlowStopReason::Completed,
            vec![step(
                "1",
                "do",
                "open the shop home page",
                StepOutcome::Done,
                "",
            )],
            &[],
            None,
        )],
        Err("the configured planner is unavailable"),
    );
    *script.plan_recovery.lock().unwrap() = Some(crate::task::PlanRecoveryResult::Plan(flow(
        json!({"app":"browser","steps":["open the shop home page"]}),
    )));
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("find a blue wool sweater".to_owned()),
            constraints: TaskConstraints {
                surfaces: vec![tinycomputer_bus::agent::SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();

    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(script.requests.lock().unwrap().len(), 1);
    assert_eq!(script.plan_requests.lock().unwrap()[0].task, started.id);
}

#[tokio::test]
async fn plan_recovery_and_a_later_failed_step_share_one_host_call_budget() {
    let plan = flow(json!({"app":"browser","steps":[
        {"browse":"https://flights.test"},
        "search for flights",
        "open the cheapest result"
    ]}));
    let (tasks, script) = planned(vec![failed_at_step_two()], Err("planner failed"));
    *script.plan_recovery.lock().unwrap() = Some(PlanRecoveryResult::Plan(plan));
    *script.reserve_host_calls.lock().unwrap() = true;
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("find the cheapest flight".to_owned()),
            budget: tinycomputer_bus::agent::TaskBudget {
                max_model_calls: Some(1),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();

    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Failed { .. }
    ));
    assert_eq!(
        script.plan_requests.lock().unwrap()[0].remaining_model_calls,
        1
    );
    assert_eq!(script.requests.lock().unwrap()[0].max_model_calls, 0);
    let recovery = script.recoveries.lock().unwrap();
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].remaining_model_calls, 0);
    assert_eq!(
        *script.host_call_reservations.lock().unwrap(),
        [true, false]
    );
    let cell = tasks.cells.lock().unwrap().values().next().unwrap().clone();
    assert_eq!(cell.state.lock().unwrap().spent.model_calls, 1);
}

#[tokio::test]
async fn failed_native_handoff_charges_host_plan_elapsed_once() {
    let prefix = flow(json!({"app":"browser","steps":["safe prefix"]}));
    let (tasks, script) = planned(
        vec![
            finished_run(FlowStopReason::Completed, vec![], &[], None),
            finished_run(FlowStopReason::Completed, vec![], &[], None),
        ],
        Err("planner failed"),
    );
    *script.plan_recovery.lock().unwrap() = Some(PlanRecoveryResult::Plan(flow(json!({
        "app":"browser","steps":["host continuation"]
    }))));
    let delay = std::time::Duration::from_millis(40);
    *script.plan_recovery_delay.lock().unwrap() = delay;
    let max_elapsed_ms = 10_000;
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("buy a lamp".to_owned()),
            flow: Some(prefix),
            handoff_to_planner: true,
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            budget: tinycomputer_bus::agent::TaskBudget {
                max_elapsed_ms: Some(max_elapsed_ms),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));

    let remaining_before_host = script.plan_requests.lock().unwrap()[0]
        .remaining_ms
        .unwrap();
    let spent_after_host = tasks
        .cells
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .state
        .lock()
        .unwrap()
        .spent
        .elapsed_ms;
    let already_spent = max_elapsed_ms.saturating_sub(remaining_before_host);
    let host_elapsed = spent_after_host.saturating_sub(already_spent);
    assert!(
        (u64::try_from(delay.as_millis()).unwrap()..=100).contains(&host_elapsed),
        "host plan elapsed must be charged once after the native PlanningDebit, got {host_elapsed} ms"
    );
}

#[tokio::test]
async fn a_retained_plan_scope_cannot_reserve_after_its_guard_is_dropped() {
    let (tasks, script) = controller(vec![finished_run(
        FlowStopReason::StoppedBeforeDestructive,
        vec![step(
            "1",
            "stop_before",
            "pay now",
            StepOutcome::Gated,
            "found payment",
        )],
        &[],
        Some("Pay now"),
    )]);
    *script.plan_recovery.lock().unwrap() = Some(PlanRecoveryResult::Plan(flow(json!({
        "app":"browser","steps":[{"stop_before":"pay now"}]
    }))));
    let reply = tasks.start(&StartTaskRequest {
        task: Some("buy a lamp".to_owned()),
        constraints: TaskConstraints {
            payment: tinycomputer_bus::agent::PaymentMode::FillThenApprove,
            surfaces: vec![SurfaceKind::Browser],
            origins: vec![".shop.example".to_owned()],
            ..TaskConstraints::default()
        },
        budget: tinycomputer_bus::agent::TaskBudget {
            max_elapsed_ms: Some(1_000),
            ..tinycomputer_bus::agent::TaskBudget::default()
        },
        ..StartTaskRequest::default()
    });
    assert!(reply.ok, "{:?}", reply.error);
    let started = reply.data.unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::NeedsApproval { .. }
    ));

    let scope = script.plan_requests.lock().unwrap()[0].host_budget.clone();
    assert_eq!(scope.remaining_model_calls(), 0);
    assert_eq!(scope.remaining_ms(), Some(0));
    assert!(!scope.try_reserve_model_call());
}

#[tokio::test]
async fn a_host_scope_cannot_reserve_when_the_task_time_budget_is_exhausted() {
    let (tasks, _) = controller(vec![finished_run(
        FlowStopReason::StoppedBeforeDestructive,
        vec![step(
            "1",
            "stop_before",
            "sending the email",
            StepOutcome::Gated,
            "found Send",
        )],
        &[],
        Some("Send"),
    )]);
    let max_elapsed_ms = 10_000;
    let started = tasks
        .start(&StartTaskRequest {
            flow: Some(flow(
                json!({"app":"Mail","steps":[{"stop_before":"sending the email"}]}),
            )),
            budget: tinycomputer_bus::agent::TaskBudget {
                max_elapsed_ms: Some(max_elapsed_ms),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::NeedsApproval { .. }
    ));

    let cell = tasks.cells.lock().unwrap().values().next().unwrap().clone();
    let budget = cell.state.lock().unwrap().usage.task_call_budget().unwrap();
    let work = budget
        .begin_work()
        .expect("a paused non-terminal task permits a host-work scope");
    let scope = work.scope();
    cell.state.lock().unwrap().spent.elapsed_ms = max_elapsed_ms;

    assert_eq!(scope.remaining_ms(), Some(0));
    assert_eq!(scope.remaining_model_calls(), 0);
    assert!(!scope.try_reserve_model_call());
}

#[tokio::test]
async fn a_planned_task_reports_usage_from_warmup_and_run() {
    let script = Arc::new(Script {
        replies: VecDeque::from([finished_run(FlowStopReason::Completed, vec![], &[], None)])
            .into(),
        ..Script::default()
    });
    let model = Arc::new(Fixed(Ok(
        r#"{"app":"Mail","steps":[{"enter":{"recipient":"${email}"}}]}"#.to_owned(),
    )));
    let tasks = Tasks::new(Arc::new(WarmUsageRunner(Arc::clone(&script))))
        .with_planner(crate::Planner::new(model));
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("email Sam".to_owned()),
            facts: BTreeMap::from([("email".to_owned(), "sam@example.com".to_owned())]),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();

    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    let report = tasks.report(&started.id).data.unwrap();
    assert_eq!(report.usage.jev.calls, 2);
    assert_eq!(report.usage.jev.input_tokens, Some(45));
    assert_eq!(report.usage.jev.cached_input_tokens, Some(11));
    assert_eq!(report.usage.jev.output_tokens, Some(4));
    assert_eq!(report.usage.jev.cost_usd_nanos, Some(15));
}

#[tokio::test]
async fn a_plan_that_needs_values_asks_and_a_failed_plan_says_so() {
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(r#"{"app": "browser", "steps": [{"enter": {"phone": "${phone}"}}]}"#),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("fill my phone".to_owned()),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let waiting = settle(&tasks, &started.id).await;
    assert!(
        matches!(waiting.status, TaskStatus::NeedsInput { ref fields } if fields[0].name == "phone")
    );
    assert_eq!(
        *script.released.lock().unwrap(),
        std::slice::from_ref(&started.id),
        "a task waiting on a person holds no browser"
    );
    // An answer that still leaves a value missing keeps the task waiting:
    // no wait is over yet.
    assert!(
        tasks
            .continue_task(ContinueTaskRequest {
                id: started.id.clone(),
                ..ContinueTaskRequest::default()
            })
            .ok
    );
    assert_eq!(journaled(&script, "resume").len(), 0, "no resume yet");
    assert!(
        tasks
            .continue_task(ContinueTaskRequest {
                id: started.id.clone(),
                inputs: BTreeMap::from([("phone".to_owned(), "+91 98765 43210".to_owned())]),
                ..ContinueTaskRequest::default()
            })
            .ok
    );
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(
        script.requests.lock().unwrap()[0].vars["phone"],
        "+91 98765 43210"
    );
    let resumes = journaled(&script, "resume");
    assert_eq!(
        resumes.len(),
        1,
        "the answer journals how long the task waited"
    );
    assert_eq!(resumes[0].1["state"], "needs_input");
    assert!(resumes[0].1["waited_ms"].is_u64());

    let (tasks, _) = planned(Vec::new(), Err("the model is down"));
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("anything".to_owned()),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    let status = settle(&tasks, &started.id).await.status;
    assert!(
        matches!(
            &status,
            TaskStatus::Failed { reason, recoverable: true, .. }
                if reason.starts_with("the model is down; host planning recovery failed:")
        ),
        "{status:?}"
    );
}

#[tokio::test]
async fn plan_task_drafts_without_acting() {
    use tinycomputer_bus::agent::PlanTaskRequest;

    let request = PlanTaskRequest {
        task: "email Sam".to_owned(),
        ..PlanTaskRequest::default()
    };
    let (tasks, _) = controller(Vec::new());
    assert_eq!(code(&tasks.plan(&request).await), "PLANNER_NOT_CONFIGURED");
    let (tasks, script) = planned(
        Vec::new(),
        Ok(r#"{"app": "Mail", "steps": ["start a new email message"]}"#),
    );
    assert_eq!(tasks.plan(&request).await.data.unwrap().flow.app, "Mail");
    assert!(
        script.requests.lock().unwrap().is_empty(),
        "planning never runs anything"
    );
    let plans = journaled(&script, "plan");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].0, None, "no task yet: a run of its own");
    assert_eq!(plans[0].1["ok"], true);
    let (tasks, script) = planned(Vec::new(), Err("down"));
    assert_eq!(code(&tasks.plan(&request).await), "PLAN_FAILED");
    let plans = journaled(&script, "plan");
    assert_eq!(plans[0].1["ok"], false);
    assert_eq!(plans[0].1["error"], "down");
}

#[tokio::test]
async fn a_browser_only_task_gets_its_browser_ready_while_it_is_planned() {
    use tinycomputer_bus::agent::{SurfaceKind, TaskConstraints};

    let mail = r#"{"app": "browser", "steps": [{"browse": "https://mail.test"}]}"#;
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(mail),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("open my mail".to_owned()),
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(
        *script.prepared.lock().unwrap(),
        std::slice::from_ref(&started.id)
    );

    // A task that may also use the desktop could be planned for either, so
    // nothing is opened before the plan says which.
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(mail),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("open my mail".to_owned()),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    settle(&tasks, &started.id).await;
    assert!(script.prepared.lock().unwrap().is_empty());

    // A plan that fails lets go of what was made ready for it.
    let (tasks, script) = planned(Vec::new(), Err("the model is down"));
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("open my mail".to_owned()),
            constraints: TaskConstraints {
                surfaces: vec![SurfaceKind::Browser],
                ..TaskConstraints::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Failed { .. }
    ));
    assert_eq!(
        *script.prepared.lock().unwrap(),
        std::slice::from_ref(&started.id)
    );
    assert_eq!(
        *script.released.lock().unwrap(),
        std::slice::from_ref(&started.id)
    );
}

#[tokio::test]
async fn a_task_warms_jev_while_it_is_planned_and_never_waits_for_it() {
    // The script's warm-up never ends: the task finishes all the same.
    let flow = r#"{"app": "Mail", "steps": ["start a new email message"]}"#;
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(flow),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("start an email".to_owned()),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    assert!(matches!(
        settle(&tasks, &started.id).await.status,
        TaskStatus::Done { .. }
    ));
    assert_eq!(*script.warmed.lock().unwrap(), [(started.id.clone(), 7)]);

    // Asked as many ways as the task's budget says.
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(flow),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("start an email".to_owned()),
            budget: tinycomputer_bus::agent::TaskBudget {
                votes: Some(3),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    settle(&tasks, &started.id).await;
    assert_eq!(*script.warmed.lock().unwrap(), [(started.id.clone(), 3)]);

    // A budget capping its Jev calls is not spent on warming.
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(flow),
    );
    let started = tasks
        .start(&StartTaskRequest {
            task: Some("start an email".to_owned()),
            budget: tinycomputer_bus::agent::TaskBudget {
                max_model_calls: Some(40),
                ..tinycomputer_bus::agent::TaskBudget::default()
            },
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    settle(&tasks, &started.id).await;
    assert!(script.warmed.lock().unwrap().is_empty());

    // A flow handed over whole is not planned, and not warmed.
    let (tasks, script) = controller(vec![finished_run(
        FlowStopReason::Completed,
        vec![],
        &[],
        None,
    )]);
    let started = tasks
        .start(&StartTaskRequest {
            flow: Some(serde_json::from_str(flow).unwrap()),
            ..StartTaskRequest::default()
        })
        .data
        .unwrap();
    settle(&tasks, &started.id).await;
    assert!(script.warmed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_browser_only_task_loads_the_page_it_names_while_it_is_planned() {
    use tinycomputer_bus::agent::{SurfaceKind, TaskConstraints};

    let mail = r#"{"app": "browser", "steps": [{"browse": "https://mail.test"}]}"#;
    let start = |tasks: &Tasks, task: &str, surfaces: Vec<SurfaceKind>| {
        tasks
            .start(&StartTaskRequest {
                task: Some(task.to_owned()),
                constraints: TaskConstraints {
                    surfaces,
                    ..TaskConstraints::default()
                },
                ..StartTaskRequest::default()
            })
            .data
            .unwrap()
    };
    let (tasks, script) = planned(
        vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
        Ok(mail),
    );
    let started = start(
        &tasks,
        "Go to https://mail.test and open my inbox.",
        vec![SurfaceKind::Browser],
    );
    settle(&tasks, &started.id).await;
    assert_eq!(
        *script.opened.lock().unwrap(),
        [(started.id.clone(), "https://mail.test".to_owned())]
    );
    assert_eq!(
        *script.prepared.lock().unwrap(),
        std::slice::from_ref(&started.id)
    );
    assert_eq!(
        script
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(**event, "prepare" | "open_page"))
            .collect::<Vec<_>>(),
        [&"prepare", &"open_page"],
        "in the browser opened for it"
    );

    // Two pages leave no one to start on; a task that may use the desktop
    // gets no browser early, nor a page.
    for (task, surfaces) in [
        (
            "Compare https://a.test with https://b.test",
            vec![SurfaceKind::Browser],
        ),
        ("Go to https://mail.test and open my inbox.", Vec::new()),
    ] {
        let (tasks, script) = planned(
            vec![finished_run(FlowStopReason::Completed, vec![], &[], None)],
            Ok(mail),
        );
        let started = start(&tasks, task, surfaces);
        settle(&tasks, &started.id).await;
        assert_eq!(script.opened.lock().unwrap().len(), 0, "{task}");
    }
}
