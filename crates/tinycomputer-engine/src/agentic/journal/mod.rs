//! The debug journal: every Jev exchange, and what each loop spent its time
//! on, written to disk grouped by run.
//!
//! The journal is off unless it is switched on, and switched on it never
//! changes what a run does: every write is best effort, and a write that
//! fails is dropped rather than failing the run. It is a tool for reading a
//! run afterwards — what Jev was asked, what it answered, how long each call
//! took — and for measuring where a loop's wall time goes.
//!
//! Switch it on with the environment variable [`JOURNAL_ENV`] (`1` writes
//! under [`DEFAULT_DIR`] in the working directory; any other value is the
//! directory to write under), or in code with
//! [`JevRuntime::with_journal`](super::JevRuntime::with_journal).
//!
//! Each run gets one directory, `<dir>/<run id>/`, holding one JSON Lines
//! file, `journal.jsonl`. Every line is an event with an `event` kind, a
//! sequence number, a wall-clock `at`, and `elapsed_ms` since the run's
//! journal opened. The kinds and their fields are described in
//! `README.md` next to this file; `docs/technical/jev-journal.md` shows how to read
//! and summarise one.
//!
//! A journal holds what Jev was shown: screen text, element names, and the
//! caller's goal. Secrets are masked before a request is built, so they are
//! not in it, but personal data on screen is. The default directory is
//! git-ignored for that reason.

#[cfg(test)]
mod journal_tests;

use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use tinyinference_decisions::{EvaluationFailure, EvaluationRequest, EvaluationResult};

/// The environment variable that switches the journal on.
pub const JOURNAL_ENV: &str = "TINYCOMPUTER_JEV_JOURNAL";

/// Where the journal is written when [`JOURNAL_ENV`] is `1`, relative to the
/// working directory.
pub const DEFAULT_DIR: &str = ".jev-journal";

/// The file each run directory holds.
pub const JOURNAL_FILE: &str = "journal.jsonl";

/// A journal: where runs are written, and the run being written, if any.
///
/// Cloning is cheap; clones of a journal inside a run write to the same file.
#[derive(Clone, Default)]
pub(crate) struct Journal {
    root: Option<Arc<PathBuf>>,
    run: Option<Arc<RunJournal>>,
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Journal")
            .field("root", &self.root)
            .field("run", &self.run.as_ref().map(|run| run.id.as_str()))
            .finish()
    }
}

/// One run's open journal file.
struct RunJournal {
    id: String,
    file: Mutex<File>,
    seq: AtomicU64,
    opened: Instant,
}

impl Journal {
    /// A journal configured from [`JOURNAL_ENV`]; off when it is unset.
    pub(crate) fn from_env() -> Self {
        Self::from_setting(std::env::var_os(JOURNAL_ENV))
    }

    /// A journal configured from the value of [`JOURNAL_ENV`].
    fn from_setting(value: Option<OsString>) -> Self {
        let root = value.and_then(|value| {
            match value.to_string_lossy().trim().to_ascii_lowercase().as_str() {
                "" | "0" | "false" | "off" | "no" => None,
                "1" | "true" | "on" | "yes" => Some(PathBuf::from(DEFAULT_DIR)),
                _ => Some(PathBuf::from(value)),
            }
        });
        Self {
            root: root.map(Arc::new),
            run: None,
        }
    }

    /// A journal writing runs under `dir`.
    pub(crate) fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(Arc::new(dir.into())),
            run: None,
        }
    }

    /// The directory of the run being written, if any.
    pub(crate) fn run_dir(&self) -> Option<PathBuf> {
        Some(self.root.as_ref()?.join(&self.run.as_ref()?.id))
    }

    /// This journal, writing to the run named `id`: a new one, or the end of
    /// an existing one, so every run of one task shares a file.
    ///
    /// A journal that is off stays off.
    pub(crate) fn named(&self, id: &str) -> Self {
        let Some(root) = &self.root else {
            return self.clone();
        };
        let id = sanitize(id);
        Self {
            root: Some(root.clone()),
            run: RunJournal::open(root, &id).map(Arc::new),
        }
    }

    /// Whether a run is open to write to.
    pub(crate) fn is_open(&self) -> bool {
        self.run.is_some()
    }

    /// Whether the journal is on: it has a folder to write runs under.
    pub(crate) fn is_on(&self) -> bool {
        self.root.is_some()
    }

    /// This journal, writing to a new run named for the time and `kind`. A
    /// journal that is off stays off.
    pub(crate) fn fresh(&self, kind: &str) -> Self {
        if !self.is_on() {
            return self.clone();
        }
        self.named(&fresh_id(kind))
    }

    /// This journal with a run begun: a `run` event in the current run, or,
    /// when there is none yet, in a new one named for the time and `kind`.
    pub(crate) fn begin(&self, kind: &str, label: &str, model: &str) -> Self {
        let journal = if self.run.is_some() || !self.is_on() {
            self.clone()
        } else {
            self.named(&fresh_id(kind))
        };
        journal.record(
            "run",
            || json!({"kind": kind, "label": label, "model": model, "pid": std::process::id()}),
        );
        journal
    }

    /// Writes one event of `kind`; `fields` is only built when recording.
    pub(crate) fn record(&self, kind: &str, fields: impl FnOnce() -> Value) {
        let Some(run) = &self.run else {
            return;
        };
        let mut line = Map::new();
        line.insert("event".to_owned(), json!(kind));
        line.insert(
            "seq".to_owned(),
            json!(run.seq.fetch_add(1, Ordering::Relaxed)),
        );
        line.insert("at".to_owned(), json!(timestamp(SystemTime::now())));
        line.insert("elapsed_ms".to_owned(), json!(millis(run.opened.elapsed())));
        if let Value::Object(fields) = fields() {
            line.extend(fields);
        }
        run.write(&Value::Object(line));
    }

    /// Writes one Jev exchange: what was sent, what came back or why nothing
    /// did, and how long it took.
    pub(crate) fn exchange(
        &self,
        step: Option<&str>,
        request: &EvaluationRequest,
        outcome: Result<&EvaluationResult, &EvaluationFailure>,
    ) {
        self.record("exchange", || {
            let request_bytes = serde_json::to_vec(request).map_or(0, |json| json.len());
            let mut fields = json!({
                "step": step,
                "questions": request.questions.keys().collect::<Vec<_>>(),
                "request_bytes": request_bytes,
                "request": without_image_data(request),
            });
            let outcome = match outcome {
                Ok(result) => json!({
                    "ok": true,
                    "latency_ms": millis(result.latency),
                    "attempts": result.attempts,
                    "request_id": result.request_id,
                    "model": result.response.model,
                    "input_tokens": result.response.usage.input_tokens,
                    "output_tokens": result.response.usage.output_tokens,
                    "answers": result.response.answers,
                }),
                Err(failure) => json!({
                    "ok": false,
                    "latency_ms": millis(failure.latency),
                    "attempts": failure.attempts,
                    "error": failure.to_string(),
                }),
            };
            if let (Value::Object(fields), Value::Object(outcome)) = (&mut fields, outcome) {
                fields.extend(outcome);
            }
            fields
        });
    }
}

/// `request` as the journal records it: every attached image's base64 is
/// replaced by its length, so a captcha run's journal stays readable and
/// small. The exchange still records the request's full size.
fn without_image_data(request: &EvaluationRequest) -> Value {
    let mut logged = serde_json::to_value(request).unwrap_or(Value::Null);
    if let Some(assets) = logged.get_mut("assets").and_then(Value::as_object_mut) {
        for asset in assets.values_mut() {
            if let Some(data) = asset.get_mut("data_base64") {
                let chars = data.as_str().map_or(0, str::len);
                *data = Value::from(format!("[{chars} base64 characters]"));
            }
        }
    }
    logged
}

impl RunJournal {
    fn open(root: &Path, id: &str) -> Option<Self> {
        let dir = root.join(id);
        let opened = std::fs::create_dir_all(&dir).and_then(|()| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(JOURNAL_FILE))
        });
        match opened {
            Ok(file) => Some(Self {
                id: id.to_owned(),
                file: Mutex::new(file),
                seq: AtomicU64::new(0),
                opened: Instant::now(),
            }),
            Err(error) => {
                // The journal is opt-in debugging; a developer who asked for
                // it needs to know it is not being written.
                eprintln!("jev journal: cannot open {}: {error}", dir.display());
                None
            }
        }
    }

    fn write(&self, line: &Value) {
        let Ok(mut encoded) = serde_json::to_vec(line) else {
            return;
        };
        encoded.push(b'\n');
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(&encoded);
        }
    }
}

/// A run id that sorts by start time: `20260928T101530Z-flow-a1b2c3`.
fn fresh_id(kind: &str) -> String {
    let mut nonce = [0_u8; 3];
    let _ = getrandom::fill(&mut nonce);
    let stamp = timestamp(SystemTime::now());
    // `2026-09-28T10:15:30.123Z` -> `20260928T101530Z`
    let compact = stamp.get(..19).unwrap_or_default().replace(['-', ':'], "");
    sanitize(&format!(
        "{compact}Z-{kind}-{:02x}{:02x}{:02x}",
        nonce[0], nonce[1], nonce[2]
    ))
}

/// `id` made safe as one path segment: letters, digits, `.`, `_`, and `-`,
/// never empty and never starting with a dot.
fn sanitize(id: &str) -> String {
    let clean = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '-'
            }
        })
        .take(96)
        .collect::<String>();
    let clean = clean.trim_start_matches('.');
    if clean.is_empty() {
        "run".to_owned()
    } else {
        clean.to_owned()
    }
}

/// Whole milliseconds in `duration`, saturating.
pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// `time` as an RFC 3339 UTC timestamp with milliseconds.
fn timestamp(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let of_day = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60,
        since.subsec_millis()
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01, after Howard
/// Hinnant's `civil_from_days`.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
