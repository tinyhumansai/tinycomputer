//! Ending a decision on a quorum: once all but its last two framings have
//! answered and agree plainly, the rest are not waited for.

use super::*;
use crate::agentic::flow::quorum::{self, QUORUM_TOP, SURE_NO, SURE_YES};
use crate::agentic::flow::{FlowRun, StepLog};

/// A target Choice whose options each framing relabels and reorders, a
/// yes/no, and the page kind.
fn request() -> EvaluationRequest {
    let choice = |options: &[&str], labelled: bool| {
        Question::Choice(tinyinference_decisions::Choice {
            instructions: json!({"question": "which"}),
            criteria: options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    if labelled {
                        ((index + 1).to_string(), Some(json!(option)))
                    } else {
                        ((*option).to_owned(), None)
                    }
                })
                .collect(),
        })
    };
    EvaluationRequest {
        state: json!("a product page"),
        model: "jev-latest".to_owned(),
        questions: BTreeMap::from([
            (
                "target".to_owned(),
                choice(&["Add to Cart", "Buy Now", "Wishlist"], true),
            ),
            (
                "done".to_owned(),
                Question::Noul(tinyinference_decisions::Noul {
                    instructions: json!({"question": "done?"}),
                    criteria: None,
                }),
            ),
            ("page_kind".to_owned(), choice(&["product", "cart"], false)),
        ]),
    }
}

/// One framing's answers: `target` (an option's text) at `sure`, `done`
/// at `done`, and the page kind `kind` at a weak 0.6, in the framing's own
/// keys.
fn answers(
    framing: &vote::Framing,
    target: &str,
    sure: f64,
    done: f64,
    kind: &str,
) -> BTreeMap<String, Answer> {
    let Question::Choice(choice) = &framing.request.questions["target"] else {
        panic!("target is a choice");
    };
    let key = choice
        .criteria
        .iter()
        .find(|(_, text)| text.as_ref() == Some(&json!(target)))
        .map(|(key, _)| key.clone())
        .unwrap();
    let others = (1.0 - sure) / 2.0;
    let other = if kind == "product" { "cart" } else { "product" };
    BTreeMap::from([
        (
            "target".to_owned(),
            Answer::Choice(ChoiceAnswer {
                choice: key.clone(),
                probabilities: choice
                    .criteria
                    .keys()
                    .map(|option| (option.clone(), if *option == key { sure } else { others }))
                    .collect(),
                confidence: sure,
            }),
        ),
        ("done".to_owned(), noul(done)),
        (
            "page_kind".to_owned(),
            Answer::Choice(ChoiceAnswer {
                choice: kind.to_owned(),
                probabilities: BTreeMap::from([(kind.to_owned(), 0.6), (other.to_owned(), 0.4)]),
                confidence: 0.6,
            }),
        ),
    ])
}

/// Whether five framings answering as `each` says, framing by framing,
/// settle the decision.
fn settles(each: impl Fn(usize) -> (&'static str, f64, f64, &'static str)) -> bool {
    let framings = vote::framings(&request(), 7);
    let answered = (0..5)
        .map(|index| {
            let (target, sure, done, kind) = each(index);
            (index, answers(&framings[index], target, sure, done, kind))
        })
        .collect::<Vec<_>>();
    quorum::settled(
        &framings,
        answered.iter().map(|(index, answers)| (*index, answers)),
        5,
    )
}

#[test]
fn only_a_decision_asked_seven_ways_or_more_ends_on_a_quorum() {
    assert_eq!(quorum::size(7), Some(5));
    assert_eq!(quorum::size(9), Some(7));
    assert_eq!(quorum::size(6), None);
    assert_eq!(quorum::size(1), None);
}

#[test]
fn five_plain_answers_settle_a_decision_however_each_framing_labels_it() {
    // Each framing names "Add to Cart" by a key of its own; the page kind
    // needs only to be the same.
    assert!(settles(|_| ("Add to Cart", 0.95, 0.99, "product")));
    assert!(settles(|_| ("Add to Cart", QUORUM_TOP, SURE_NO, "product")));
    assert!(settles(|_| ("Add to Cart", 0.95, SURE_YES, "product")));
}

#[test]
fn a_decision_any_answer_leaves_open_is_not_settled() {
    let weak_pick = |index: usize| {
        (
            "Add to Cart",
            if index == 3 { 0.85 } else { 0.95 },
            0.99,
            "product",
        )
    };
    assert!(!settles(weak_pick), "a pick under QUORUM_TOP");
    let other_pick = |index: usize| {
        (
            if index == 2 { "Buy Now" } else { "Add to Cart" },
            0.95,
            0.99,
            "product",
        )
    };
    assert!(!settles(other_pick), "a framing picked another option");
    let unsure = |index: usize| {
        (
            "Add to Cart",
            0.95,
            if index == 4 { 0.95 } else { 0.99 },
            "product",
        )
    };
    assert!(!settles(unsure), "a yes/no short of SURE_YES");
    let split = |index: usize| {
        (
            "Add to Cart",
            0.95,
            if index == 0 { 0.02 } else { 0.99 },
            "product",
        )
    };
    assert!(!settles(split), "a yes/no answered both ways");
    let page = |index: usize| {
        (
            "Add to Cart",
            0.95,
            0.99,
            if index == 1 { "cart" } else { "product" },
        )
    };
    assert!(!settles(page), "the page kind read two ways");
}

#[test]
fn a_decision_short_of_its_quorum_is_not_settled() {
    let framings = vote::framings(&request(), 7);
    let plain = |index: usize| answers(&framings[index], "Add to Cart", 0.95, 0.99, "product");
    let four = (0..4)
        .map(|index| (index, plain(index)))
        .collect::<Vec<_>>();
    assert!(!quorum::settled(
        &framings,
        four.iter().map(|(index, answers)| (*index, answers)),
        5
    ));
    // Five answered, but one left a question out.
    let mut five = (0..5)
        .map(|index| (index, plain(index)))
        .collect::<Vec<_>>();
    five[2].1.remove("done");
    assert!(!quorum::settled(
        &framings,
        five.iter().map(|(index, answers)| (*index, answers)),
        5
    ));
}

/// Framing tasks for `framings`, framing `index` answering after `plan`'s
/// wait with its answers, or failing; each counts itself in `finished` as
/// it ends.
fn paced(
    framings: &[vote::Framing],
    plan: &[(u64, Option<(&'static str, f64)>)],
    finished: &Arc<AtomicU64>,
) -> Vec<tokio::task::JoinHandle<Result<EvaluationResult, EvaluationFailure>>> {
    framings
        .iter()
        .zip(plan)
        .map(|(framing, (wait, answer))| {
            let answers =
                answer.map(|(target, done)| answers(framing, target, 0.95, done, "product"));
            let (wait, finished) = (Duration::from_millis(*wait), Arc::clone(finished));
            tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                finished.fetch_add(1, Ordering::SeqCst);
                answers
                    .map(|answers| EvaluationResult {
                        response: EvaluationResponse {
                            model: "typesafe/jev-test".to_owned(),
                            answers,
                            usage: tinyinference_decisions::Usage::default(),
                        },
                        request_id: None,
                        attempts: 1,
                        latency: wait,
                    })
                    .ok_or_else(|| EvaluationFailure {
                        error: Box::new(tinyinference_decisions::Error::RateLimited),
                        attempts: 1,
                        latency: wait,
                    })
            })
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_decision_ends_once_five_plain_answers_are_in_and_the_rest_still_run() {
    let framings = vote::framings(&request(), 7);
    let finished = Arc::new(AtomicU64::new(0));
    let agreeing = Some(("Add to Cart", 0.99));
    let plan = [
        (500, agreeing),
        (100, agreeing),
        (300, agreeing),
        (200, agreeing),
        (400, agreeing),
        (900, agreeing),
        (3_000, agreeing),
    ];
    let handles = paced(&framings, &plan, &finished);
    let started = tokio::time::Instant::now();
    let gathered = quorum::gather(framings, handles, quorum::size(7)).await;
    assert_eq!(
        started.elapsed(),
        Duration::from_millis(500),
        "the fifth answer"
    );
    assert_eq!(gathered.left, 2);
    assert!(gathered.failure.is_none());
    // In framing order, as the ballots read them.
    assert_eq!(
        gathered
            .answered
            .iter()
            .map(|(_, evaluation)| evaluation.latency.as_millis())
            .collect::<Vec<_>>(),
        [500, 100, 300, 200, 400]
    );
    // The two left are not cut off: each runs to its end.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(finished.load(Ordering::SeqCst), 7);
}

#[tokio::test(start_paused = true)]
async fn a_decision_waits_for_every_framing_unless_five_agree_plainly() {
    let finished = Arc::new(AtomicU64::new(0));
    let agreeing = Some(("Add to Cart", 0.99));
    let started = tokio::time::Instant::now();
    // One of the first five in picks another option: all seven are waited
    // for. (One that answers after five agreeing ones is not.)
    let framings = vote::framings(&request(), 7);
    let mut plan = [(100, agreeing); 7];
    plan[2] = (50, Some(("Buy Now", 0.99)));
    plan[6] = (3_000, agreeing);
    let handles = paced(&framings, &plan, &finished);
    let gathered = quorum::gather(framings, handles, quorum::size(7)).await;
    assert_eq!(started.elapsed(), Duration::from_secs(3));
    assert_eq!((gathered.answered.len(), gathered.left), (7, 0));

    // A failed framing is no answer; five others are a quorum still.
    let started = tokio::time::Instant::now();
    let framings = vote::framings(&request(), 7);
    let mut plan = [(100, agreeing); 7];
    plan[0] = (50, None);
    plan[6] = (3_000, agreeing);
    let handles = paced(&framings, &plan, &finished);
    let gathered = quorum::gather(framings, handles, quorum::size(7)).await;
    assert_eq!(started.elapsed(), Duration::from_millis(100));
    assert!(gathered.failure.is_some());
    assert_eq!((gathered.answered.len(), gathered.left), (5, 1));

    // Asked fewer than seven ways, a decision waits for every framing.
    let started = tokio::time::Instant::now();
    let framings = vote::framings(&request(), 6);
    let mut plan = [(100, agreeing); 6];
    plan[5] = (2_000, agreeing);
    let handles = paced(&framings, &plan, &finished);
    let gathered = quorum::gather(framings, handles, quorum::size(6)).await;
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!((gathered.answered.len(), gathered.left), (6, 0));
}

/// The oracle, with each decision's framings answering after the waits in
/// `pace`, in the order they are asked.
struct Staggered {
    oracle: Oracle,
    pace: [u64; 7],
    calls: Mutex<usize>,
}

impl Evaluator for Staggered {
    fn evaluate<'a>(
        &'a self,
        request: &'a EvaluationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<EvaluationResult, EvaluationFailure>> + Send + 'a>>
    {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls - 1
        };
        let wait = Duration::from_millis(self.pace[call % self.pace.len()]);
        Box::pin(async move {
            tokio::time::sleep(wait).await;
            self.oracle.evaluate(request).await
        })
    }
}

/// A `verify` run asked seven ways, its last two framings 2 s behind the
/// rest, with Jev `sure` the inbox shows; how long it took, its result, and
/// its journal's decisions.
async fn verify_staggered(sure: f64) -> (Duration, FlowRunResult, Vec<Value>) {
    let scratch = std::env::temp_dir().join(format!(
        "tinycomputer-quorum-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let app = App::with(|_| {});
    let staggered = Staggered {
        oracle: Oracle {
            app: app.clone(),
            hook: Box::new(move |id: &str, _: &Question, _: &Sim| {
                (id == "holds").then(|| noul(sure))
            }),
            requests: Mutex::new(Vec::new()),
            fail: false,
        },
        pace: [10, 10, 10, 10, 10, 2_000, 2_000],
        calls: Mutex::new(0),
    };
    let mut runtime = runtime(staggered);
    runtime.journal = crate::agentic::journal::Journal::at(&scratch).fresh("quorum");
    let dir = runtime.journal.run_dir().unwrap();
    let started = tokio::time::Instant::now();
    let reply = run_flow_with(
        app,
        &runtime,
        RunFlowRequest {
            flow: serde_json::from_value(json!({
                "app": "Mail",
                "steps": [{"verify": "the inbox shows"}]
            }))
            .unwrap(),
            votes: 7,
            ..RunFlowRequest::default()
        },
    )
    .await;
    let took = started.elapsed();
    let decisions = std::fs::read_to_string(dir.join(crate::JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|event| event["event"] == "decision")
        .collect();
    let _ = std::fs::remove_dir_all(&scratch);
    (
        took,
        serde_json::from_value(reply.data.unwrap()).unwrap(),
        decisions,
    )
}

#[tokio::test(start_paused = true)]
async fn a_run_whose_framings_agree_plainly_does_not_wait_for_its_slowest() {
    let (took, result, decisions) = verify_staggered(0.99).await;
    assert_eq!(result.stop, FlowStopReason::Completed, "{:?}", result.steps);
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_ne!(decisions.len(), 0);
    for decision in &decisions {
        assert_eq!(
            (
                &decision["framings"],
                &decision["answered"],
                &decision["left"]
            ),
            (&json!(7), &json!(5), &json!(2)),
            "{decision}"
        );
    }
    // Every framing sent is a call made, waited for or not.
    assert_eq!(
        usize::try_from(result.metrics.calls).unwrap(),
        decisions.len() * 7
    );

    // Jev only fairly sure: every decision waits for all seven.
    let (took, result, decisions) = verify_staggered(0.9).await;
    assert_eq!(result.stop, FlowStopReason::Completed, "{:?}", result.steps);
    assert!(took >= Duration::from_secs(2), "{took:?}");
    assert!(decisions.iter().all(|decision| decision["left"] == 0));
}

#[tokio::test(start_paused = true)]
async fn a_decision_ended_on_a_quorum_widens_past_every_framing_it_asked() {
    // A press nothing undoes is vouched for, and the vouching always widens.
    let app = App::with(|_| {});
    let staggered = Staggered {
        oracle: Oracle {
            app: app.clone(),
            hook: Box::new(|id: &str, _: &Question, _: &Sim| match id {
                "is_0" => Some(noul(0.99)),
                "only_near_0" => Some(noul(0.02)),
                _ => None,
            }),
            requests: Mutex::new(Vec::new()),
            fail: false,
        },
        pace: [10, 10, 10, 10, 10, 2_000, 2_000],
        calls: Mutex::new(0),
    };
    let runtime = runtime(staggered);
    let request = RunFlowRequest {
        flow: serde_json::from_value(json!({
            "app": "Mail",
            "steps": [{"stop_before": "sending the email"}]
        }))
        .unwrap(),
        votes: 7,
        ..RunFlowRequest::default()
    };
    let mut run = FlowRun::new(app, &runtime, &request);
    let mut log = StepLog::default();
    let yes_no = |question: &str| {
        Question::Noul(tinyinference_decisions::Noul {
            instructions: json!({"question": question}),
            criteria: None,
        })
    };
    let vouching = EvaluationRequest {
        state: json!("the draft, its Send button in view"),
        model: "jev-latest".to_owned(),
        questions: BTreeMap::from([
            ("is_0".to_owned(), yes_no("is it the Send button?")),
            ("only_near_0".to_owned(), yes_no("is it only near it?")),
        ]),
    };
    run.ask(&mut log, vouching.clone()).await.unwrap();
    assert_eq!(
        (run.ballot("is_0").len(), run.asked_in("is_0")),
        (5, 7),
        "ended on a quorum"
    );
    run.widen(&mut log, &vouching).await.unwrap().unwrap();
    // The eighth and ninth framings: not the sixth and seventh again, which
    // were asked and left.
    assert_eq!((run.ballot("is_0").len(), run.asked_in("is_0")), (7, 9));
    assert_eq!(log.calls, 9, "seven, and two more");
}
