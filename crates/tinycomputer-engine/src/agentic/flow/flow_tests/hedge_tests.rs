//! Hedging a framing: one still running past its hedge delay gets a copy,
//! and the first answer of the two counts.

use super::*;
use crate::agentic::flow::hedge::hedged;

/// A Jev that answers each call after the wait scripted for it, in call
/// order, or fails it.
struct Paced {
    calls: Mutex<usize>,
    script: Vec<(Duration, bool)>,
}

impl Evaluator for Paced {
    fn evaluate<'a>(
        &'a self,
        _request: &'a EvaluationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EvaluationResult, EvaluationFailure>> + Send + 'a>>
    {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls - 1
        };
        let (wait, fails) = self.script.get(call).copied().unwrap_or_default();
        Box::pin(async move {
            tokio::time::sleep(wait).await;
            if fails {
                return Err(EvaluationFailure {
                    error: Box::new(tinyinference_decisions::Error::RateLimited),
                    attempts: 1,
                    latency: wait,
                });
            }
            Ok(EvaluationResult {
                response: EvaluationResponse {
                    model: format!("call {call}"),
                    answers: BTreeMap::new(),
                    usage: tinyinference_decisions::Usage::default(),
                },
                request_id: None,
                attempts: 1,
                latency: wait,
            })
        })
    }
}

fn paced(script: &[(u64, bool)]) -> (JevRuntime, Arc<Paced>) {
    paced_as(tinycomputer_bus::JevProvider::OpenRouter, script)
}

fn paced_as(
    provider: tinycomputer_bus::JevProvider,
    script: &[(u64, bool)],
) -> (JevRuntime, Arc<Paced>) {
    let paced = Arc::new(Paced {
        calls: Mutex::new(0),
        script: script
            .iter()
            .map(|(ms, fails)| (Duration::from_millis(*ms), *fails))
            .collect(),
    });
    let runtime = JevRuntime {
        client: paced.clone(),
        configuration: tinycomputer_bus::JevConfiguration {
            provider,
            model: "jev-latest".to_owned(),
            endpoint_url: None,
            fast: false,
        },
        pending: Arc::default(),
        journal: crate::agentic::journal::Journal::default(),
        copies: Arc::default(),
    };
    (runtime, paced)
}

/// A request of about `bytes` bytes.
fn request(bytes: usize) -> EvaluationRequest {
    ask::request(
        "jev-latest",
        json!({"visible_text": "x".repeat(bytes)}),
        ask::Questions::default(),
    )
}

#[tokio::test(start_paused = true)]
async fn a_stalled_framing_is_answered_by_its_copy() {
    // Live, one framing of a burst stalled 12–32 s while its siblings
    // answered in under a second.
    let (runtime, paced) = paced(&[(30_000, false), (600, false)]);
    let started = tokio::time::Instant::now();
    let answer = hedged(&runtime, "1", &request(1_000)).await.unwrap();
    assert_eq!(answer.response.model, "call 1", "the copy answered");
    assert_eq!(answer.attempts, 2, "the copy is an attempt of its own");
    assert_eq!(started.elapsed(), Duration::from_millis(4_600));
    assert_eq!(*paced.calls.lock().unwrap(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_framing_that_answers_in_time_gets_no_copy() {
    // Live, a slow evening's calls took up to 3.4 s and still answered.
    let (runtime, paced) = paced(&[(3_900, false)]);
    let answer = hedged(&runtime, "1", &request(1_000)).await.unwrap();
    assert_eq!(
        (answer.response.model.as_str(), answer.attempts),
        ("call 0", 1)
    );
    assert_eq!(*paced.calls.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_large_request_waits_longer_before_its_copy() {
    // A 32 KB request's p99.9 was 3.9 s live: at 4.9 s it still gets no copy.
    let (runtime, paced) = paced(&[(4_900, false)]);
    let answer = hedged(&runtime, "1", &request(40_000)).await.unwrap();
    assert_eq!(answer.attempts, 1);
    assert_eq!(*paced.calls.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_copy_gives_way_and_two_failures_fail() {
    // The copy fails at once; the slow first answer still counts.
    let (runtime, _) = paced(&[(6_000, false), (0, true)]);
    let started = tokio::time::Instant::now();
    let answer = hedged(&runtime, "1", &request(1_000)).await.unwrap();
    assert_eq!(answer.response.model, "call 0");
    assert_eq!(started.elapsed(), Duration::from_secs(6));

    // The first fails after its copy was sent; the copy's answer counts.
    let (runtime, _) = paced(&[(4_500, true), (1_000, false)]);
    let answer = hedged(&runtime, "1", &request(1_000)).await.unwrap();
    assert_eq!(answer.response.model, "call 1");

    let (runtime, _) = paced(&[(4_500, true), (1_000, true)]);
    assert!(hedged(&runtime, "1", &request(1_000)).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_sage_framing_gets_no_copy() {
    // Sage's calls take 5–6 s as a rule: a copy would double them.
    let (runtime, paced) = paced_as(
        tinycomputer_bus::JevProvider::Sage,
        &[(6_000, false), (600, false)],
    );
    let started = tokio::time::Instant::now();
    let answer = hedged(&runtime, "1", &request(1_000)).await.unwrap();
    assert_eq!(
        (answer.response.model.as_str(), answer.attempts),
        ("call 0", 1)
    );
    assert_eq!(started.elapsed(), Duration::from_secs(6));
    assert_eq!(*paced.calls.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn no_more_than_two_copies_are_in_flight() {
    // Three framings stall together, as on a slow gateway: two get a copy,
    // the third waits for its own answer.
    let (runtime, paced) = paced(&[
        (30_000, false),
        (30_000, false),
        (30_000, false),
        (600, false),
        (600, false),
    ]);
    let asked = request(1_000);
    let ask = || hedged(&runtime, "1", &asked);
    let (one, two, three) = tokio::join!(ask(), ask(), ask());
    let mut attempts = [one, two, three].map(|answer| answer.unwrap().attempts);
    // Which framing's delay ends first is the timer's to say.
    attempts.sort_unstable();
    assert_eq!(attempts, [1, 2, 2]);
    assert_eq!(*paced.calls.lock().unwrap(), 5);
    assert_eq!(
        runtime.copies.load(std::sync::atomic::Ordering::Acquire),
        0,
        "every place given back"
    );
}

#[tokio::test(start_paused = true)]
async fn the_journal_names_the_answer_that_counted() {
    let scratch = std::env::temp_dir().join(format!(
        "tinycomputer-hedge-journal-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let won = |script: &[(u64, bool)]| {
        let (mut runtime, _) = paced(script);
        runtime.journal = crate::agentic::journal::Journal::at(&scratch).fresh("hedge");
        let dir = runtime.journal.run_dir().unwrap();
        async move {
            let _ = hedged(&runtime, "1", &request(1_000)).await;
            let journal = std::fs::read_to_string(dir.join(crate::JOURNAL_FILE)).unwrap();
            let event: Value = journal
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|event| event["event"] == "hedge")
                .unwrap();
            (
                event["won"].as_str().unwrap().to_owned(),
                event["ok"].as_bool().unwrap(),
            )
        }
    };
    // The first finished first, failing; the copy's answer counted.
    assert_eq!(
        won(&[(4_500, true), (1_000, false)]).await,
        ("copy".to_owned(), true)
    );
    // The copy failed at once; the first's answer counted.
    assert_eq!(
        won(&[(6_000, false), (0, true)]).await,
        ("first".to_owned(), true)
    );
    assert_eq!(
        won(&[(4_500, true), (1_000, true)]).await,
        ("neither".to_owned(), false)
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
