//! Voting: framings, their cost, and merging their answers.

use super::*;

/// A Jev that leans toward whichever option is listed first: the needle gets
/// 0.4, the first option 0.6 — enough to mislead any single asking.
fn first_biased(question: &Question, needle: &str) -> Answer {
    let Question::Choice(choice) = question else {
        panic!("a choice");
    };
    let right = choice
        .criteria
        .iter()
        .find(|(_, description)| {
            description
                .as_ref()
                .is_some_and(|description| description.to_string().contains(needle))
        })
        .map(|(key, _)| key.clone())
        .unwrap();
    let first = choice
        .criteria
        .keys()
        .find(|key| *key != "none")
        .unwrap()
        .clone();
    let probabilities = choice
        .criteria
        .keys()
        .map(|key| {
            let mut probability = 0.0;
            if *key == right {
                probability += 0.4;
            }
            if *key == first {
                probability += 0.6;
            }
            (key.clone(), probability)
        })
        .collect::<BTreeMap<String, f64>>();
    let choice = probabilities
        .iter()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .unwrap()
        .0
        .clone();
    Answer::Choice(ChoiceAnswer {
        choice,
        probabilities,
        confidence: 0.6,
    })
}

async fn biased_mail(votes: u32) -> Run {
    run_with(
        App::default(),
        json!({"app": "Mail", "steps": [
            {"open": "Mail"},
            "start a new email message",
            {"enter": {"subject": "Moving Thursday's sync"}}
        ]}),
        move |request| request.votes = votes,
        |id, question, _| {
            id.starts_with("slot_")
                .then(|| first_biased(question, needle_for(&text_of(question, "purpose"))))
        },
    )
    .await
}

#[tokio::test]
async fn a_vote_undoes_a_bias_that_misleads_a_single_asking() {
    let once = biased_mail(1).await;
    assert_ne!(
        once.app.sim().fields.get("Subject").map(String::as_str),
        Some("Moving Thursday's sync"),
        "asked once, the first field wins"
    );
    let voted = biased_mail(5).await;
    assert_eq!(
        voted.app.sim().fields["Subject"],
        "Moving Thursday's sync",
        "asked five ways, the right field wins"
    );
    assert!(voted.result.steps[2].loops.contains(&FlowLoop::Vote));
}

#[tokio::test]
async fn every_framing_is_charged_and_the_budget_bounds_them() {
    let flow = json!({"app": "Mail", "steps": [{"open": "Mail"}, "start a new email message"]});
    let once = run_with(App::default(), flow.clone(), |_| {}, |_, _, _| None).await;
    let thrice = run_with(
        App::default(),
        flow.clone(),
        |request| request.votes = 3,
        |_, _, _| None,
    )
    .await;
    assert_eq!(thrice.result.metrics.calls, 3 * once.result.metrics.calls);
    assert_eq!(
        usize::try_from(thrice.result.metrics.calls).unwrap(),
        thrice.requests.len()
    );
    assert_eq!(
        thrice.result.trace.len(),
        once.result.trace.len(),
        "the trace keeps one merged exchange per decision"
    );
    let squeezed = run_with(
        App::default(),
        flow,
        |request| {
            request.votes = 9;
            request.max_model_calls = 2;
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(squeezed.result.metrics.calls, 2, "voting never overspends");
    assert_eq!(squeezed.result.stop, FlowStopReason::ModelBudget);
}

#[test]
fn framings_relabel_label_keys_and_keep_word_keys() {
    let request = ask::request(
        "jev-latest",
        json!({}),
        ask::Questions::default()
            .with(
                "target",
                ask::options(
                    json!({"task": "t"}),
                    ["1", "2", "3"].map(|key| (key.to_owned(), json!(format!("option {key}")))),
                ),
            )
            .with(
                "move",
                ask::options(
                    json!({"task": "m"}),
                    ["activate", "wait"].map(|key| (key.to_owned(), json!(key))),
                ),
            )
            .with("done", ask::completion("x")),
    );
    let framings = vote::framings(&request, 4);
    assert_eq!(framings.len(), 4);
    assert_eq!(
        framings[0].request, request,
        "the first framing is the request"
    );
    let firsts = framings
        .iter()
        .map(|framing| {
            let Question::Choice(choice) = &framing.request.questions["target"] else {
                panic!()
            };
            let first = choice.criteria.keys().next().unwrap();
            choice.criteria[first].clone().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        firsts,
        [
            json!("option 1"),
            json!("option 2"),
            json!("option 3"),
            json!("option 1")
        ],
        "each framing leads with another option"
    );
    let Question::Choice(target) = &framings[1].request.questions["target"] else {
        panic!()
    };
    assert_eq!(
        target.criteria.keys().collect::<Vec<_>>(),
        ["A", "B", "C", "none"]
    );
    assert_ne!(target.instructions["perspective"].as_str().unwrap(), "");
    let Question::Choice(moves) = &framings[1].request.questions["move"] else {
        panic!()
    };
    assert_eq!(
        moves.criteria.keys().collect::<Vec<_>>(),
        ["activate", "none", "wait"],
        "meaningful keys are kept"
    );
    assert_eq!(vote::framings(&request, 50).len(), vote::MAX_VOTES as usize);
    assert_eq!(vote::framings(&request, 0).len(), 1);
}

#[test]
fn merged_answers_average_under_the_original_keys() {
    let request = ask::request(
        "jev-latest",
        json!({}),
        ask::Questions::default()
            .with(
                "target",
                ask::options(
                    json!({"task": "t"}),
                    (1..=30).map(|key| (key.to_string(), json!(format!("option {key}")))),
                ),
            )
            .with("done", ask::completion("x"))
            .with("progress", ask::progress("x")),
    );
    let framings = vote::framings(&request, 2);
    let Question::Choice(second) = &framings[1].request.questions["target"] else {
        panic!()
    };
    let keys = second.criteria.keys().cloned().collect::<Vec<_>>();
    assert_eq!((keys[0].as_str(), keys[29].as_str()), ("AA", "BD"));
    let key_for = |framing: &vote::Framing, description: &str| {
        let Question::Choice(choice) = &framing.request.questions["target"] else {
            panic!()
        };
        choice
            .criteria
            .iter()
            .find(|(_, value)| value.as_ref() == Some(&json!(description)))
            .unwrap()
            .0
            .clone()
    };
    let answer = |framing: &vote::Framing, winner: &str, done: f64, top: usize| {
        let winner = key_for(framing, winner);
        BTreeMap::from([
            (
                "target".to_owned(),
                Answer::Choice(ChoiceAnswer {
                    probabilities: BTreeMap::from([(winner.clone(), 0.8)]),
                    choice: winner,
                    confidence: 0.8,
                }),
            ),
            ("done".to_owned(), noul(done)),
            ("progress".to_owned(), level(top)),
        ])
    };
    let answered = vec![
        (
            framings[0].clone(),
            answer(&framings[0], "option 7", 0.9, 4),
        ),
        (
            framings[1].clone(),
            answer(&framings[1], "option 7", 0.5, 2),
        ),
    ];
    let merged = vote::tally(&vote::ballots(&answered));
    let Answer::Choice(target) = &merged["target"] else {
        panic!()
    };
    assert_eq!(target.choice, "7");
    assert!((target.probabilities["7"] - 0.8).abs() < 1e-9);
    assert!((target.confidence - 1.0).abs() < 1e-9);
    assert!((ask::probability(&merged, "done").unwrap() - 0.7).abs() < 1e-9);
    assert!((ask::top_level(&merged, "progress").unwrap() - 0.5).abs() < 1e-9);

    let split = vote::tally(&vote::ballots(&[
        (
            framings[0].clone(),
            answer(&framings[0], "option 7", 0.9, 4),
        ),
        (
            framings[1].clone(),
            answer(&framings[1], "option 9", 0.9, 4),
        ),
    ]));
    let Answer::Choice(split) = &split["target"] else {
        panic!()
    };
    assert!((split.confidence - 0.5).abs() < 1e-9, "one of two agreed");
    assert!(vote::tally(&vote::ballots(&[])).is_empty());
}

#[test]
fn every_part_of_a_split_request_gets_a_ballot() {
    // Parts of a request split by its questions are asked, and answered,
    // side by side; the ballots once took their questions from the first
    // answer alone, dropping every other part's.
    let part = |id: &str| {
        ask::request(
            "jev-latest",
            json!({}),
            ask::Questions::default().with(id, ask::completion("x")),
        )
    };
    let answered = vote::framings(&part("done"), 2)
        .into_iter()
        .map(|framing| (framing, BTreeMap::from([("done".to_owned(), noul(0.9))])))
        .chain(
            vote::framings(&part("holds"), 2)
                .into_iter()
                .map(|framing| (framing, BTreeMap::from([("holds".to_owned(), noul(0.2))]))),
        )
        .collect::<Vec<_>>();
    let ballots = vote::ballots(&answered);
    assert_eq!(ballots.keys().collect::<Vec<_>>(), ["done", "holds"]);
    assert_eq!(ballots["holds"].len(), 2);
}

#[test]
fn box_questions_take_a_perspective_and_merge_to_the_surest_box() {
    let request = ask::request(
        "jev-preview",
        json!({"page": "shot"}),
        ask::Questions::default().with(
            "button",
            Question::Box(tinyinference_decisions::BoxQuestion {
                instructions: json!({"task": "the Verify button in `page`"}),
            }),
        ),
    );
    let framings = vote::framings(&request, 2);
    let Question::Box(framed) = &framings[1].request.questions["button"] else {
        panic!()
    };
    assert!(framed.instructions.get("perspective").is_some());
    let boxed = |left: f64, confidence: f64| {
        Answer::Box(tinyinference_decisions::BoxAnswer {
            edges: tinyinference_decisions::BoxEdges {
                top: 0.1,
                bottom: 0.2,
                left,
                right: left + 0.1,
            },
            confidence,
        })
    };
    let merged = vote::tally(&BTreeMap::from([(
        "button".to_owned(),
        vec![boxed(0.1, 0.4), boxed(0.5, 0.9), boxed(0.3, 0.6)],
    )]));
    let Answer::Box(surest) = &merged["button"] else {
        panic!()
    };
    assert!((surest.edges.left - 0.5).abs() < 1e-9);
    assert!((surest.confidence - 0.9).abs() < 1e-9);
}
