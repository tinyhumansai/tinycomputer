//! Tests for deterministic Jev desktop-control policy.
//!
//! This root holds the fake desktops and the scripted Jev every topic shares;
//! the tests themselves live in `agentic_tests/`, one file per topic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod continuation_tests;
mod goal_tests;
mod observation_tests;
mod policy_tests;
mod resolve_tests;
mod scope_tests;
mod waiting_tests;
mod warm_tests;

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    Evaluator, JevRuntime,
    backend::{AgentBackend, execute_desktop},
    goal::{run_goal, run_goal_with, same_target},
    policy::{
        ACT, DESTRUCTIVE, FLOOR, action_space, choice, deterministic_destructive,
        exact_named_match, gate_with_evidence, noul, parse_operation, playing_goal_satisfied,
        positional_match, request, rerank_request, shortlist, target,
    },
    reply::{internal_error, provider_error, response as agent_response},
    resolve::{reason, resolve_intent, resolve_intent_with, target_payload, visible_completion},
    screen::{
        Candidate, NativeId, Screen, describe, fingerprint, observe, parse_reply, snapshot_request,
    },
};
use serde_json::json;
use tinycomputer_bus::{
    DesktopResponse, GoalContinuation, JevConfig, JevDecisionKind, JevOperation, JevProvider,
    JevStopReason, RunGoalRequest, VisiblePredicate,
};
use tinyinference_decisions::{Answer, ChoiceAnswer};

#[derive(Clone)]
struct FakeBackend {
    screens: Arc<Mutex<VecDeque<Screen>>>,
    operations: Arc<Mutex<Vec<JevOperation>>>,
    fail_execute: bool,
}

#[derive(Clone)]
struct RecordingTextBackend {
    inner: FakeBackend,
    values: Arc<Mutex<Vec<Option<String>>>>,
}

#[derive(Clone)]
struct WindowBoundBackend {
    inner: FakeBackend,
    requested: Arc<Mutex<Vec<Option<String>>>>,
}

#[derive(Clone)]
struct ReadinessBackend {
    inner: FakeBackend,
    attempts: Arc<Mutex<u32>>,
    first_error: &'static str,
}

#[derive(Clone)]
struct UnverifiedClickBackend {
    inner: FakeBackend,
}

impl AgentBackend for UnverifiedClickBackend {
    fn observe(
        &self,
        app: &str,
        window_id: Option<&str>,
        root: Option<&str>,
    ) -> Result<Screen, Box<DesktopResponse>> {
        self.inner.observe(app, window_id, root)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        let _ = self.inner.execute(operation, target, text);
        DesktopResponse::ok(
            "click",
            json!({"disposition":{"delivery":"delivered_unverified"}}),
        )
    }
}

impl AgentBackend for ReadinessBackend {
    fn observe(
        &self,
        app: &str,
        window_id: Option<&str>,
        root: Option<&str>,
    ) -> Result<Screen, Box<DesktopResponse>> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts == 1 {
            return Err(Box::new(DesktopResponse::err(
                "snapshot",
                tinycomputer_bus::DesktopError::new(self.first_error, "not ready"),
            )));
        }
        drop(attempts);
        self.inner.observe(app, window_id, root)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        self.inner.execute(operation, target, text)
    }
}

impl AgentBackend for WindowBoundBackend {
    fn observe(
        &self,
        app: &str,
        window_id: Option<&str>,
        root: Option<&str>,
    ) -> Result<Screen, Box<DesktopResponse>> {
        self.requested
            .lock()
            .unwrap()
            .push(window_id.map(str::to_owned));
        if window_id != Some("w-515619") {
            return Err(Box::new(DesktopResponse::err(
                "snapshot",
                tinycomputer_bus::DesktopError::new(
                    "WINDOW_NOT_FOUND",
                    "requested window is unavailable",
                ),
            )));
        }
        self.inner.observe(app, window_id, root)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        self.inner.execute(operation, target, text)
    }
}

impl AgentBackend for RecordingTextBackend {
    fn observe(
        &self,
        app: &str,
        window_id: Option<&str>,
        root: Option<&str>,
    ) -> Result<Screen, Box<DesktopResponse>> {
        self.inner.observe(app, window_id, root)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        self.values.lock().unwrap().push(text.clone());
        self.inner.execute(operation, target, text)
    }
}

impl AgentBackend for FakeBackend {
    fn observe(
        &self,
        _app: &str,
        _window_id: Option<&str>,
        _root: Option<&str>,
    ) -> Result<Screen, Box<DesktopResponse>> {
        self.screens
            .lock()
            .expect("screen lock")
            .pop_front()
            .ok_or_else(|| {
                Box::new(DesktopResponse::err(
                    "snapshot",
                    tinycomputer_bus::DesktopError::new("EMPTY", "no screen"),
                ))
            })
    }

    fn execute(
        &self,
        operation: JevOperation,
        _target: Option<Candidate>,
        _text: Option<String>,
    ) -> DesktopResponse {
        self.operations
            .lock()
            .expect("operation lock")
            .push(operation);
        if self.fail_execute {
            DesktopResponse::err(
                "fake",
                tinycomputer_bus::DesktopError::new("ACTION_FAILED", "fake failure"),
            )
        } else {
            DesktopResponse::ok("fake", json!({"delivery": "delivered_verified"}))
        }
    }
}

fn clickable_screen() -> Screen {
    Screen {
        app: "Spotify".to_owned(),
        window: Some("Liked Songs".to_owned()),
        window_id: None,
        surface: "window".to_owned(),
        root: None,
        candidates: vec![Candidate {
            ref_id: "@s1:e1".to_owned(),
            role: "button".to_owned(),
            name: Some("Play First Song by Artist".to_owned()),
            available_actions: vec!["Click".to_owned()],
            bounds: Some(json!({"x": 10.0, "y": 100.0})),
            ..Candidate::default()
        }],
        observed: Vec::new(),
    }
}

fn two_candidate_screen() -> Screen {
    let mut screen = clickable_screen();
    screen.candidates.push(Candidate {
        ref_id: "@s1:e2".to_owned(),
        role: "button".to_owned(),
        name: Some("Play Second Song by Artist".to_owned()),
        available_actions: vec!["Click".to_owned()],
        bounds: Some(json!({"x": 10.0, "y": 160.0})),
        ..Candidate::default()
    });
    screen
}

fn response(
    operation: &str,
    probability: f64,
    target: &str,
) -> tinyinference_decisions::EvaluationResult {
    response_with(operation, probability, target, 0.9, 0.05)
}

fn response_with(
    operation: &str,
    probability: f64,
    target: &str,
    selected_target_probability: f64,
    destructive: f64,
) -> tinyinference_decisions::EvaluationResult {
    let remainder = (1.0 - probability) / 3.0;
    let target_probability = if target == "1" {
        selected_target_probability
    } else {
        1.0 - selected_target_probability
    };
    evaluation(json!({
        "model": "typesafe/jev-1.13-20260917",
        "answers": {
            "operation": {
                "type": "choice", "choice": operation, "confidence": 0.4,
                "probabilities": {
                    "CLICK": if operation == "CLICK" { probability } else { remainder },
                    "WAIT": if operation == "WAIT" { probability } else { remainder },
                    "DONE": if operation == "DONE" { probability } else { remainder },
                    "BLOCKED": if operation == "BLOCKED" { probability } else { remainder }
                }
            },
            "click_target": {
                "type": "choice", "choice": target, "confidence": 0.4,
                "probabilities": {"1": target_probability, "none": 1.0 - target_probability}
            },
            "destructive": {"type": "noul", "noul": destructive}
        },
        "usage": {"input_tokens": 10, "output_tokens": 2}
    }))
}

fn evaluation(value: serde_json::Value) -> tinyinference_decisions::EvaluationResult {
    let response = serde_json::from_value(value).expect("mock response decodes");
    tinyinference_decisions::EvaluationResult {
        response,
        request_id: Some("mock-request".to_owned()),
        attempts: 1,
        latency: Duration::from_millis(1),
    }
}

struct MockEvaluator {
    results: Mutex<VecDeque<tinyinference_decisions::EvaluationResult>>,
    requests: Arc<Mutex<Vec<tinyinference_decisions::EvaluationRequest>>>,
}

impl Evaluator for MockEvaluator {
    fn evaluate<'a>(
        &'a self,
        request: &'a tinyinference_decisions::EvaluationRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        tinyinference_decisions::EvaluationResult,
                        tinyinference_decisions::EvaluationFailure,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.requests
                .lock()
                .expect("request lock")
                .push(request.clone());
            Ok(self
                .results
                .lock()
                .expect("evaluation lock")
                .pop_front()
                .expect("mock evaluation"))
        })
    }
}

fn runtime(results: Vec<tinyinference_decisions::EvaluationResult>) -> JevRuntime {
    runtime_recording(results).0
}

fn runtime_recording(
    results: Vec<tinyinference_decisions::EvaluationResult>,
) -> (
    JevRuntime,
    Arc<Mutex<Vec<tinyinference_decisions::EvaluationRequest>>>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        JevRuntime {
            client: Arc::new(MockEvaluator {
                results: Mutex::new(VecDeque::from(results)),
                requests: Arc::clone(&requests),
            }),
            configuration: tinycomputer_bus::JevConfiguration {
                provider: tinycomputer_bus::JevProvider::OpenRouter,
                model: "jev-latest".to_owned(),
                endpoint_url: None,
                fast: false,
            },
            pending: Arc::new(Mutex::new(std::collections::HashMap::new())),
            journal: super::journal::Journal::default(),
            copies: Arc::default(),
            usage: None,
        },
        requests,
    )
}

fn backend(screen_count: usize) -> (FakeBackend, Arc<Mutex<Vec<JevOperation>>>) {
    let operations = Arc::new(Mutex::new(Vec::new()));
    (
        FakeBackend {
            screens: Arc::new(Mutex::new(VecDeque::from(
                (0..screen_count)
                    .map(|_| clickable_screen())
                    .collect::<Vec<_>>(),
            ))),
            operations: Arc::clone(&operations),
            fail_execute: false,
        },
        operations,
    )
}
