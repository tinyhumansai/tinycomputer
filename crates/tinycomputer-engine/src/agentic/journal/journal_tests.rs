//! Unit tests for the debug journal: switching it on, naming runs, and the
//! events a run writes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use serde_json::{Value, json};
use tinyinference_decisions::{
    Answer, EvaluationFailure, EvaluationRequest, EvaluationResponse, EvaluationResult, Noul,
    NoulAnswer, Question, Usage,
};

use super::{
    DEFAULT_DIR, JOURNAL_FILE, Journal, civil_from_days, fresh_id, millis, sanitize, timestamp,
};

/// A fresh directory for one test, removed by [`Scratch`]'s drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "tinycomputer-journal-{name}-{}-{nanos}",
            std::process::id()
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn events(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn request() -> EvaluationRequest {
    EvaluationRequest::jev(
        json!({"app": "Mail"}),
        [(
            "done".to_owned(),
            Question::Noul(Noul {
                instructions: json!("Is the step done?"),
                criteria: None,
            }),
        )]
        .into_iter()
        .collect(),
    )
}

#[test]
fn the_journal_is_off_unless_the_setting_turns_it_on() {
    for off in [
        None,
        Some(""),
        Some("0"),
        Some("false"),
        Some("OFF"),
        Some(" no "),
    ] {
        let journal = Journal::from_setting(off.map(OsString::from));
        assert!(journal.root.is_none(), "{off:?} must leave it off");
    }
    for on in ["1", "true", "On", "yes"] {
        let journal = Journal::from_setting(Some(OsString::from(on)));
        assert_eq!(journal.root.as_deref(), Some(&PathBuf::from(DEFAULT_DIR)));
    }
    let journal = Journal::from_setting(Some(OsString::from("/var/tmp/jev")));
    assert_eq!(
        journal.root.as_deref(),
        Some(&PathBuf::from("/var/tmp/jev"))
    );
}

#[test]
fn an_off_journal_writes_nothing_and_has_no_run() {
    let journal = Journal::default().begin("flow", "Mail", "jev-latest");
    assert!(journal.run_dir().is_none());
    journal.record("step", || panic!("an off journal must not build events"));
}

#[test]
fn a_begun_run_writes_a_run_event_then_each_exchange() {
    let scratch = Scratch::new("exchange");
    let journal = Journal::at(&scratch.0).begin("flow", "Mail: compose", "jev-latest");
    let dir = journal.run_dir().unwrap();
    assert!(dir.starts_with(&scratch.0));
    assert!(
        dir.file_name()
            .unwrap()
            .to_string_lossy()
            .contains("-flow-")
    );

    let answered = EvaluationResult {
        response: EvaluationResponse {
            model: "jev-1".to_owned(),
            answers: [("done".to_owned(), Answer::Noul(NoulAnswer { noul: 0.9 }))]
                .into_iter()
                .collect(),
            usage: Usage {
                input_tokens: Some(120),
                output_tokens: Some(4),
                cached_input_tokens: None,
                cost_usd_nanos: None,
            },
        },
        request_id: Some("req-1".to_owned()),
        attempts: 1,
        latency: Duration::from_millis(250),
    };
    journal.exchange(Some("2"), &request(), Ok(&answered));
    let failed = EvaluationFailure {
        error: Box::new(tinyinference_decisions::Error::Timeout),
        attempts: 3,
        latency: Duration::from_secs(9),
    };
    journal.exchange(None, &request(), Err(&failed));

    let events = events(&dir);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["event"], "run");
    assert_eq!(events[0]["kind"], "flow");
    assert_eq!(events[0]["label"], "Mail: compose");
    assert_eq!(events[0]["model"], "jev-latest");
    let seqs = events
        .iter()
        .map(|event| event["seq"].clone())
        .collect::<Vec<_>>();
    assert_eq!(seqs, [json!(0), json!(1), json!(2)]);

    let ok = &events[1];
    assert_eq!(ok["event"], "exchange");
    assert_eq!(ok["step"], "2");
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["latency_ms"], 250);
    assert_eq!(ok["attempts"], 1);
    assert_eq!(ok["request_id"], "req-1");
    assert_eq!(ok["input_tokens"], 120);
    assert_eq!(ok["questions"], json!(["done"]));
    assert_eq!(ok["request"]["state"]["app"], "Mail");
    assert_eq!(ok["answers"]["done"]["noul"], 0.9);
    assert!(ok["request_bytes"].as_u64().unwrap() > 0);
    assert!(ok["at"].as_str().unwrap().ends_with('Z'));

    let failed = &events[2];
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["step"], Value::Null);
    assert_eq!(failed["attempts"], 3);
    assert_eq!(failed["latency_ms"], 9000);
    assert_ne!(failed["error"].as_str().unwrap(), "");
}

#[test]
fn every_run_of_one_named_journal_shares_its_file() {
    let scratch = Scratch::new("named");
    let root = Journal::at(&scratch.0);
    let first = root.named("task-7").begin("flow", "first", "jev-latest");
    let second = root.named("task-7").begin("flow", "second", "jev-latest");
    assert_eq!(first.run_dir(), second.run_dir());
    let labels = events(&first.run_dir().unwrap())
        .iter()
        .map(|event| event["label"].clone())
        .collect::<Vec<_>>();
    assert_eq!(labels, [json!("first"), json!("second")]);
}

#[test]
fn a_journal_that_cannot_be_opened_is_dropped_not_fatal() {
    let scratch = Scratch::new("blocked");
    std::fs::write(&scratch.0, "a file where a directory must go").unwrap();
    let journal = Journal::at(&scratch.0).begin("goal", "Calculator", "jev-latest");
    assert!(journal.run_dir().is_none());
    journal.record("step", || json!({}));
    let _ = std::fs::remove_file(&scratch.0);
}

#[test]
fn run_ids_are_safe_path_segments() {
    assert_eq!(sanitize("task-abc_1.2"), "task-abc_1.2");
    assert_eq!(sanitize("../../etc/passwd"), "-..-etc-passwd");
    assert_eq!(sanitize("..."), "run");
    assert_eq!(sanitize(""), "run");
    assert_eq!(sanitize(&"x".repeat(200)).len(), 96);
}

#[test]
fn fresh_ids_sort_by_time_and_name_their_kind() {
    let id = fresh_id("flow");
    // 20260928T101530Z-flow-a1b2c3
    assert_eq!(id.len(), "20260928T101530Z-flow-a1b2c3".len(), "{id}");
    assert_eq!(&id[8..9], "T");
    assert!(id[16..].starts_with("-flow-"), "{id}");
}

#[test]
fn timestamps_are_rfc3339_utc() {
    assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    let moment = UNIX_EPOCH + Duration::from_millis(1_790_591_730_123);
    assert_eq!(timestamp(moment), "2026-09-28T10:35:30.123Z");
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
}

#[test]
fn millis_saturate() {
    assert_eq!(millis(Duration::from_micros(2500)), 2);
    assert_eq!(millis(Duration::MAX), u64::MAX);
}

#[test]
fn a_fresh_run_is_named_for_its_kind_and_opened_only_when_the_journal_is_on() {
    let scratch = Scratch::new("fresh");
    let root = Journal::at(&scratch.0);
    assert!(!root.is_open(), "a root is no run");
    let plan = root.fresh("plan");
    assert!(plan.is_open());
    let dir = plan.run_dir().unwrap();
    assert!(
        dir.file_name()
            .unwrap()
            .to_string_lossy()
            .contains("Z-plan-"),
        "{}",
        dir.display()
    );
    plan.record("plan", || json!({"wall_ms": 12}));
    assert_eq!(events(&dir)[0]["wall_ms"], 12);
    assert!(!Journal::default().fresh("plan").is_open(), "off stays off");
}
