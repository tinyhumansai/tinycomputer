//! Tests for the planner over a scripted language model.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tinycomputer_bus::agent::{InputKind, SurfaceKind};

use super::{Completion, LanguageModel, ModelUse, Planner, REPAIRS, Role, Turn};

#[cfg(feature = "planner")]
struct RestoreNetworkGuard;

#[cfg(feature = "planner")]
impl Drop for RestoreNetworkGuard {
    fn drop(&mut self) {
        tinyinference_llm::allow_network_models();
    }
}

/// Answers from a queue and records every conversation it was shown.
#[derive(Default)]
struct Scripted {
    answers: Mutex<VecDeque<Result<String, String>>>,
    seen: Mutex<Vec<Vec<Turn>>>,
}

impl LanguageModel for Scripted {
    fn complete(&self, turns: &[Turn]) -> Completion {
        self.seen.lock().unwrap().push(turns.to_vec());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err("no more answers".to_owned()));
        Box::pin(async move { answer })
    }
}

fn scripted(answers: &[Result<&str, &str>]) -> (Planner, Arc<Scripted>) {
    let model = Arc::new(Scripted {
        answers: Mutex::new(
            answers
                .iter()
                .map(|answer| answer.map(str::to_owned).map_err(str::to_owned))
                .collect(),
        ),
        seen: Mutex::default(),
    });
    (Planner::new(model.clone()), model)
}

const FLIGHT: &str = r#"Here is the plan:
```json
{"app": "browser", "steps": [
  {"browse": "https://www.google.com/travel/flights"},
  {"enter": {"where to": "Srinagar", "email": "${email}", "birth date": "${date of birth}"}},
  "search for flights",
  {"stop_before": "paying for the booking"}
]}
```"#;

#[tokio::test]
async fn a_valid_plan_comes_back_with_its_questions() {
    let (planner, model) = scripted(&[Ok(FLIGHT)]);
    let plan = planner
        .plan(
            "book the cheapest flight to Srinagar",
            &["email".to_owned()],
            &[],
            &[SurfaceKind::Browser],
        )
        .await
        .unwrap();
    assert_eq!(plan.flow.steps.len(), 4);
    assert_eq!(plan.questions.len(), 1);
    assert_eq!(plan.questions[0].name, "date of birth");
    assert_eq!(plan.questions[0].kind, InputKind::Date);
    assert_eq!(plan.notes.len(), 1, "the stop before paying is noted");

    let seen = &model.seen.lock().unwrap()[0];
    assert_eq!(seen[0].role, Role::System);
    assert!(seen[0].text.contains("\"browse\""), "the guide is included");
    assert!(seen[1].text.contains("Shared facts you may use: ${email}."));
    assert!(seen[1].text.contains("only ever an `enter` value: none."));
    assert!(seen[1].text.contains("Available: the web browser."));
}

#[tokio::test]
async fn a_shared_fact_may_be_named_in_steps_but_a_secret_only_typed() {
    let shared = r#"{"app": "browser", "steps": [
      {"browse": "https://airline.test"},
      {"choose": {"what": "title", "option": "${title}"}},
      {"enter": {"card number": "${card number}"}},
      {"stop_before": "paying for the booking"}
    ]}"#;
    let leaky = r#"{"app": "browser", "steps": [
      {"browse": "https://airline.test"},
      "type ${card number} into the card field",
      {"stop_before": "paying for the booking"}
    ]}"#;
    let (planner, model) = scripted(&[Ok(leaky), Ok(shared)]);
    let names = ["title".to_owned(), "card number".to_owned()];
    let plan = planner
        .plan("pay for the booking", &names, &[], &[SurfaceKind::Browser])
        .await
        .unwrap();
    assert_eq!(plan.flow.steps.len(), 4);
    {
        let seen = model.seen.lock().unwrap();
        let brief = &seen[1][1].text;
        assert!(
            brief.contains("Shared facts you may use: ${title}."),
            "{brief}"
        );
        assert!(
            brief.contains("only ever an `enter` value: ${card number}."),
            "{brief}"
        );
        assert!(
            seen[1][3].text.contains("`${card number}` is a secret"),
            "{}",
            seen[1][3].text
        );
    }

    let (planner, _) = scripted(&[Ok(shared)]);
    let names = ["title".to_owned(), "frequent flyer".to_owned()];
    let secret_title = planner.plan("x", &names, &["title".to_owned()], &[]).await;
    assert!(
        secret_title.is_err(),
        "a caller's secret is kept out of step text"
    );
}

#[tokio::test]
async fn an_invalid_answer_is_repaired_with_the_errors() {
    let (planner, model) = scripted(&[
        Ok("I would search for flights."),
        Ok(r#"{"app": "", "steps": []}"#),
        Ok(r#"{"app": "Mail", "steps": ["start a new email message"]}"#),
    ]);
    let plan = planner.plan("write an email", &[], &[], &[]).await.unwrap();
    assert_eq!(plan.flow.app, "Mail");
    assert_eq!(plan.notes, [] as [std::string::String; 0]);
    let seen = model.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen[0][1].text.contains("Shared facts you may use: none."));
    assert!(
        seen[0][1]
            .text
            .contains("the web browser and desktop applications")
    );
    let repairs = seen[2]
        .iter()
        .filter(|turn| turn.role == Role::User)
        .map(|turn| turn.text.as_str())
        .collect::<Vec<_>>();
    assert!(repairs[1].contains("was not a flow"));
    assert!(repairs[2].contains("invalid"));
}

#[tokio::test]
async fn a_plan_that_never_validates_or_a_failed_model_is_an_error() {
    let never = [Ok(r#"{"app": "", "steps": []}"#); REPAIRS + 1];
    let (planner, _) = scripted(&never);
    let error = planner
        .plan("x", &[], &[], &[SurfaceKind::Desktop])
        .await
        .unwrap_err();
    assert!(error.contains("did not produce a valid flow"), "{error}");

    let (planner, _) = scripted(&[Err("rate limited")]);
    assert_eq!(
        planner.plan("x", &[], &[], &[]).await.unwrap_err(),
        "rate limited"
    );
    assert!(format!("{planner:?}").contains("Planner"));

    let (planner, _) = scripted(&[Ok(r#"{"steps": "not a list"}"#), Ok("{"), Ok("{}")]);
    assert!(planner.plan("x", &[], &[], &[]).await.is_err());
}

#[tokio::test]
async fn a_measured_plan_counts_each_call_and_what_the_first_one_sent() {
    let (planner, model) = scripted(&[
        Ok("I would search for flights."),
        Ok(r#"{"app": "Mail", "steps": ["start a new email message"]}"#),
    ]);
    let (plan, used) = planner.plan_measured("write an email", &[], &[], &[]).await;
    assert_eq!(plan.unwrap().flow.app, "Mail");
    let first = model.seen.lock().unwrap()[0].clone();
    assert_eq!(
        used,
        ModelUse {
            calls: 2,
            sent_bytes: first.iter().map(|turn| turn.text.len()).sum(),
        },
        "one repair after the refused answer"
    );

    let (planner, _) = scripted(&[Ok("{"), Err("rate limited")]);
    let (plan, used) = planner.plan_measured("x", &[], &[], &[]).await;
    assert_eq!(plan.unwrap_err(), "rate limited");
    assert_eq!(used.calls, 2, "a failed call still counts");

    let never = [Ok(r#"{"app": "", "steps": []}"#); REPAIRS + 1];
    let (planner, _) = scripted(&never);
    let (plan, used) = planner.plan_measured("x", &[], &[], &[]).await;
    assert!(plan.is_err());
    assert_eq!(used.calls, u32::try_from(REPAIRS).unwrap() + 1);
}

#[cfg(feature = "planner")]
#[tokio::test]
async fn the_open_router_planner_needs_a_key_and_never_prints_it() {
    use super::{PlannerConfig, open_router};

    let _network_test = crate::MODEL_NETWORK_TEST_MUTEX.lock().await;

    let empty = PlannerConfig {
        route: super::ModelRoute {
            api_key: " ".to_owned(),
            ..super::ModelRoute::default()
        },
        model: None,
        rescue_model: None,
        output_model: None,
        rescue_route: None,
    };
    assert!(open_router(&empty).unwrap_err().contains("api_key"));
    assert!(
        super::open_router_rescuer(&empty)
            .unwrap_err()
            .contains("api_key")
    );
    assert!(
        super::open_router_shaper(&empty)
            .unwrap_err()
            .contains("api_key")
    );

    let config: PlannerConfig =
        serde_json::from_value(serde_json::json!({"api_key": "secret-key", "model": ""})).unwrap();
    assert!(!format!("{config:?}").contains("secret-key"));
    let planner = open_router(&config).unwrap();

    // No network in tests: the guard makes the model call fail fast, which
    // exercises the adapter without reaching OpenRouter.
    tinyinference_llm::deny_network_models();
    let _restore_network = RestoreNetworkGuard;
    let failed = planner.plan("x", &[], &[], &[]).await.unwrap_err();
    assert!(!failed.contains("secret-key"));

    let config: PlannerConfig = serde_json::from_value(serde_json::json!(
        {"api_key": "secret-key", "rescue_model": "openai/gpt-6-luna-pro"}
    ))
    .unwrap();
    assert!(format!("{config:?}").contains("gpt-6-luna-pro"));
    let rescuer = super::open_router_rescuer(&config).unwrap();
    let failed = rescuer
        .guide(&crate::rescue::Briefing::default())
        .await
        .unwrap_err();
    assert!(!failed.contains("secret-key"));

    // The shaper takes its own model, and goes through the same adapter.
    let config: PlannerConfig = serde_json::from_value(serde_json::json!(
        {"api_key": "secret-key", "output_model": "openai/gpt-6-luna-pro"}
    ))
    .unwrap();
    assert!(format!("{config:?}").contains("output_model"));
    let shaper = super::open_router_shaper(&config).unwrap();
    let failed = shaper
        .shape(&crate::shape::Harvest::default())
        .await
        .unwrap_err();
    assert!(!failed.contains("secret-key"));
}

#[cfg(feature = "planner")]
mod retry_tests;
#[cfg(feature = "planner")]
mod route_tests;
