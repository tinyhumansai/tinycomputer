//! Tests for intent flows against a simulated mail app and a scripted Jev.
//!
//! `Sim` is a tiny stateful application: it shows an inbox with a New Message
//! button, opens a compose window on click or cmd+n, holds field values, and
//! records every press. `Oracle` answers Jev questions from the same state, the
//! way a well-behaved decision model would, and each test overrides only the
//! answers it is about.
//!
//! This root holds the harness every test runs through; the simulator, its
//! screens, and the oracle live beside it in `flow_tests/`, as does each
//! topic's tests in its own `<topic>_tests.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod oracle;
mod places;
mod screens;
mod simulator;

mod attention_tests;
mod backtrack_tests;
mod brief_tests;
mod budget_tests;
mod choose_tests;
mod deliberation_tests;
mod do_loop_tests;
mod end_to_end_tests;
mod enter_tests;
mod grounding_tests;
mod hedge_tests;
mod helpers_tests;
mod journal_tests;
mod pick_tests;
mod picker_tests;
mod quorum_tests;
mod reflection_tests;
mod secret_tests;
mod split_tests;
mod step_kinds_tests;
mod suggestion_tests;
mod survey_tests;
mod tree_tests;
mod validation_tests;
mod vote_tests;
mod wide_tests;

use oracle::*;
use places::*;
use screens::*;
use simulator::*;

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tinycomputer_bus::{
    Deliberation, DesktopResponse, Flow, FlowLoop, FlowRunResult, FlowStopReason, GroundingHint,
    JevExchange, JevOperation, RunFlowRequest, StepOutcome, ValidateFlowRequest,
};
use tinyinference_decisions::{
    Answer, ChoiceAnswer, EvaluationFailure, EvaluationRequest, EvaluationResponse,
    EvaluationResult, NoulAnswer, Question, ScoreAnswer,
};

use super::{
    super::{Evaluator, JevRuntime},
    act, ask,
    backend::AgentBackend,
    decide::fit,
    enter, flow_guide, ground, memory, run_flow, run_flow_with,
    steps::{
        self, already_chosen, already_holds, in_region, lists_more_than, looks_like_date, redacted,
    },
    survey, validate, validate_flow,
    view::{self, Candidate, Depth, Screen},
    vote, wide,
};
use crate::{SecretHost, run_flow_with_host};

fn runtime(client: impl Evaluator + 'static) -> JevRuntime {
    JevRuntime {
        client: Arc::new(client),
        configuration: tinycomputer_bus::JevConfiguration {
            provider: tinycomputer_bus::JevProvider::OpenRouter,
            model: "jev-latest".to_owned(),
            endpoint_url: None,
            fast: false,
        },
        #[cfg(feature = "desktop")]
        pending: Arc::default(),
        journal: crate::agentic::journal::Journal::default(),
        copies: Arc::default(),
        usage: None,
    }
}

struct Run {
    result: FlowRunResult,
    app: App,
    requests: Vec<EvaluationRequest>,
}

async fn run_with(
    app: App,
    flow: Value,
    configure: impl FnOnce(&mut RunFlowRequest),
    hook: impl Fn(&str, &Question, &Sim) -> Option<Answer> + Send + Sync + 'static,
) -> Run {
    run_with_secret_host(app, flow, configure, None, hook).await
}

async fn run_with_secret_host(
    app: App,
    flow: Value,
    configure: impl FnOnce(&mut RunFlowRequest),
    secret_host: Option<Arc<dyn SecretHost>>,
    hook: impl Fn(&str, &Question, &Sim) -> Option<Answer> + Send + Sync + 'static,
) -> Run {
    let oracle = Arc::new(Oracle {
        app: app.clone(),
        hook: Box::new(hook),
        requests: Mutex::new(Vec::new()),
        fail: false,
    });
    let runtime = JevRuntime {
        client: oracle.clone(),
        configuration: tinycomputer_bus::JevConfiguration {
            provider: tinycomputer_bus::JevProvider::OpenRouter,
            model: "jev-latest".to_owned(),
            endpoint_url: None,
            fast: false,
        },
        #[cfg(feature = "desktop")]
        pending: Arc::default(),
        journal: crate::agentic::journal::Journal::default(),
        copies: Arc::default(),
        usage: None,
    };
    // One framing per decision, so every test that counts requests counts
    // decisions; voting has its own tests.
    let mut request = RunFlowRequest {
        flow: serde_json::from_value(flow).unwrap(),
        include_values: true,
        trace: true,
        votes: 1,
        ..RunFlowRequest::default()
    };
    configure(&mut request);
    if secret_host.is_some() {
        request.task_id = Some("t-secret".to_owned());
    }
    let reply = run_flow_with_host(app.clone(), &runtime, request, secret_host).await;
    assert!(reply.ok, "flow run failed: {:?}", reply.error);
    let requests = oracle.requests.lock().unwrap().clone();
    Run {
        result: serde_json::from_value(reply.data.unwrap()).unwrap(),
        app,
        requests,
    }
}

async fn run(app: App, flow: Value) -> Run {
    run_with(app, flow, |_| {}, |_, _, _| None).await
}

fn mail_flow() -> Value {
    json!({
        "app": "Mail",
        "vars": {"to": "sam@example.com"},
        "steps": [
            {"open": "Mail"},
            "start a new email message",
            {"enter": {
                "recipient": "${to}",
                "subject": "Moving Thursday's sync",
                "message body": "Hi Sam,\n\nCould we move it to Friday?\n\nAlex"
            }},
            {"verify": "the draft shows the recipient, subject and body"},
            {"stop_before": "sending the email"}
        ]
    })
}

fn outcomes(result: &FlowRunResult) -> Vec<(String, StepOutcome)> {
    result
        .steps
        .iter()
        .map(|step| (step.path.clone(), step.outcome))
        .collect()
}

fn choice_sizes(requests: &[EvaluationRequest]) -> Vec<usize> {
    requests
        .iter()
        .flat_map(|request| request.questions.values())
        .filter_map(|question| match question {
            Question::Choice(choice) => Some(choice.criteria.len()),
            _ => None,
        })
        .collect()
}

/// Two result cards whose "Select" buttons look alike: they differ only by
/// the card they sit in.
fn lookalikes() -> App {
    App::with(|sim| {
        sim.results = vec![("IndiGo", "₹5,000", "06:00"), ("IndiGo", "₹5,200", "09:00")];
    })
}

fn shop() -> App {
    App::with(|sim| sim.pages = vec![EXTRAS])
}

fn loops(run: &Run, index: usize) -> &[FlowLoop] {
    &run.result.steps[index].loops
}

fn wide(request: &mut RunFlowRequest) {
    request.strategy = tinycomputer_bus::FlowStrategy::Wide;
}

/// Answers `move` with "activate", so a step is done by pressing a control.
fn activate_moves(id: &str, question: &Question, _: &Sim) -> Option<Answer> {
    (id == "move").then(|| pick(question, "activate", 0.9))
}

fn asked(requests: &[EvaluationRequest], id: &str) -> usize {
    requests
        .iter()
        .filter(|request| request.questions.contains_key(id))
        .count()
}

fn asked_prefix(requests: &[EvaluationRequest], prefix: &str) -> usize {
    requests
        .iter()
        .filter(|request| request.questions.keys().any(|id| id.starts_with(prefix)))
        .count()
}

fn flights() -> App {
    App::with(|sim| {
        sim.results = vec![
            ("IndiGo 6E-2135", "₹6,840", "6:45 PM"),
            ("Vistara UK-707", "₹7,210", "09:10"),
            ("Air India AI-825", "₹8,050", "05:30"),
        ];
    })
}
