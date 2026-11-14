//! The brief every choosing question carries, secret masking, page kinds,
//! and fitting a request under the size limit.

use super::*;

/// The brief a request's choosing questions carry; `Null` when none does.
fn brief_of(request: &EvaluationRequest) -> Value {
    request
        .questions
        .values()
        .find_map(|question| match question {
            Question::Choice(choice) => choice.instructions.get("brief").cloned(),
            _ => None,
        })
        .unwrap_or(Value::Null)
}

/// Whether any yes/no or scale question about the screen carries a brief.
fn judgements_are_briefed(request: &EvaluationRequest) -> bool {
    request
        .questions
        .iter()
        .any(|(id, question)| match question {
            Question::Noul(noul) => id != "confirm" && noul.instructions.get("brief").is_some(),
            Question::Score(score) => score.instructions.get("brief").is_some(),
            Question::Choice(_) | Question::Box(_) => false,
        })
}

#[tokio::test]
async fn every_question_is_briefed_on_the_goal_the_person_and_the_plan() {
    let run = run_with(
        App::default(),
        mail_flow(),
        |request| {
            request.brief = tinycomputer_bus::FlowBrief {
                goal: "move Thursday's sync with Sam to Friday".to_owned(),
                details: BTreeMap::from([
                    ("first name".to_owned(), "Alex".to_owned()),
                    ("date of birth".to_owned(), "2000-01-01".to_owned()),
                ]),
                secrets: vec!["card number".to_owned()],
                rules: vec!["never send without approval".to_owned()],
            };
        },
        |_, _, _| None,
    )
    .await;
    assert_ne!(
        run.requests,
        [] as [tinyinference_decisions::EvaluationRequest; 0]
    );
    assert!(
        !run.requests.iter().any(judgements_are_briefed),
        "judging the screen is left to the screen"
    );
    assert!(
        run.requests
            .iter()
            .all(|request| request.state.get("brief").is_none())
    );
    let choosing = run
        .requests
        .iter()
        .filter(|request| {
            request
                .questions
                .values()
                .any(|question| matches!(question, Question::Choice(_)))
        })
        .collect::<Vec<_>>();
    assert_ne!(
        choosing,
        [] as [&tinyinference_decisions::EvaluationRequest; 0]
    );
    for request in choosing {
        let brief = brief_of(request);
        assert_eq!(brief["goal"], "move Thursday's sync with Sam to Friday");
        assert_eq!(brief["for"]["date of birth"], "2000-01-01");
        assert_eq!(brief["secrets"]["names"], json!(["${card number}"]));
        assert_eq!(brief["rules"], json!(["never send without approval"]));
        assert_eq!(brief["plan"].as_array().unwrap().len(), 5);
    }
    let entering = run
        .requests
        .iter()
        .find(|request| request.questions.keys().any(|id| id.starts_with("slot_")))
        .unwrap();
    let plan = brief_of(entering)["plan"].clone();
    assert_eq!(plan[1], "2. [done] do: start a new email message");
    assert!(
        plan[2].as_str().unwrap().starts_with("3. [now] enter:"),
        "{plan}"
    );
    assert!(
        plan[3].as_str().unwrap().starts_with("4. [next] verify:"),
        "{plan}"
    );
    let sending = run
        .requests
        .iter()
        .rev()
        .find(|request| request.questions.contains_key("target"))
        .unwrap();
    assert_eq!(
        brief_of(sending)["so_far"],
        json!(["entered: message body, recipient, subject"])
    );
}

#[tokio::test]
async fn an_unbriefed_run_sends_no_brief_but_its_plan() {
    let run = run(
        App::default(),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
    )
    .await;
    assert!(
        run.requests
            .iter()
            .all(|request| brief_of(request).is_null()),
        "a one-step flow with no brief adds nothing to the state"
    );
}

#[tokio::test]
async fn a_secret_the_page_shows_back_is_masked_in_every_request() {
    let run = run_with(
        App::default(),
        json!({
            "app": "Mail",
            "steps": [
                {"open": "Mail"},
                "start a new email message",
                {"enter": {"message body": "${card number}"}},
                {"verify": "the draft shows the body"}
            ]
        }),
        |request| {
            request.vars =
                BTreeMap::from([("card number".to_owned(), "4111111111111111".to_owned())]);
            request.facts = BTreeSet::from(["card number".to_owned()]);
            // The body field shows what was typed, so the value is on screen.
            request.include_values = true;
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(run.app.sim().fields["Body"], "4111111111111111");
    let text = run
        .requests
        .iter()
        .map(|request| serde_json::to_string(request).unwrap())
        .collect::<String>();
    assert!(!text.contains("4111111111111111"), "the secret leaked");
    assert!(
        text.contains("${card number}"),
        "the page's copy reads as its template"
    );
    let traced = serde_json::to_string(&run.result.trace).unwrap();
    assert!(!traced.contains("4111111111111111"));
}

#[tokio::test]
async fn a_secret_longer_than_the_display_clip_is_still_fully_masked() {
    // A secret over 80 characters must still be masked whole: clipping the
    // held value to a readable length before `FlowRun::mask` sees it would
    // leave the first 80 characters — everything the exact-match and
    // digit-run masking can no longer find — sitting unmasked in the state.
    let long_secret = "4111".repeat(25); // 100 characters, all digits.
    let run = run_with(
        App::default(),
        json!({
            "app": "Mail",
            "steps": [
                {"open": "Mail"},
                "start a new email message",
                {"enter": {"message body": "${card number}"}},
                {"verify": "the draft shows the body"}
            ]
        }),
        |request| {
            request.vars = BTreeMap::from([("card number".to_owned(), long_secret.clone())]);
            request.facts = BTreeSet::from(["card number".to_owned()]);
            request.include_values = true;
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(run.app.sim().fields["Body"], long_secret);
    let text = run
        .requests
        .iter()
        .map(|request| serde_json::to_string(request).unwrap())
        .collect::<String>();
    assert!(
        !text.contains(&long_secret[..80]),
        "even the first 80 characters of a long secret must never appear unmasked"
    );
    assert!(
        text.contains("${card number}"),
        "the page's copy reads as its template"
    );
    let traced = serde_json::to_string(&run.result.trace).unwrap();
    assert!(!traced.contains(&long_secret[..80]));
}

#[tokio::test]
async fn a_shared_value_may_be_named_in_a_step_and_reaches_jev() {
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": [
            {"open": "Mail"},
            "start a new email message",
            {"verify": "the draft is addressed to ${to}"}
        ]}),
        |request| {
            request.vars = BTreeMap::from([("to".to_owned(), "sam@example.com".to_owned())]);
        },
        |_, _, _| None,
    )
    .await;
    assert!(run.requests.iter().any(|request| {
        text_of(
            request
                .questions
                .get("holds")
                .unwrap_or(&ask::condition("")),
            "condition",
        )
        .contains("sam@example.com")
    }));
}

#[tokio::test]
async fn a_web_page_is_named_and_the_name_briefs_the_next_question() {
    let web = run_with(
        App::default(),
        json!({"app": "browser", "steps": [
            {"browse": "https://flights.test"},
            {"verify": "flights are listed"},
            {"stop_before": "booking the flight"}
        ]}),
        |_| {},
        |id, question, _| match id {
            "page_kind" => Some(pick(question, "results", 0.9)),
            "holds" => Some(noul(0.9)),
            _ => None,
        },
    )
    .await;
    let named = web
        .requests
        .iter()
        .filter(|request| request.questions.contains_key("page_kind"))
        .count();
    assert!(named > 0, "a web page's questions carry the page kind");
    assert!(
        web.requests
            .iter()
            .any(|request| brief_of(request)["page"] == "results"),
        "a later question is told the page is a results page"
    );
    assert!(web.result.steps[1].loops.contains(&FlowLoop::PageKind));
    let desktop = run(App::default(), mail_flow()).await;
    assert!(
        desktop
            .requests
            .iter()
            .all(|request| !request.questions.contains_key("page_kind")),
        "a desktop application has no page kind"
    );
}

#[test]
fn an_oversized_request_is_fitted_under_the_limit() {
    let brief = json!({"goal": "g".repeat(500)});
    let mut questions = ask::Questions::default();
    for group in 0..10 {
        questions = questions.with(
            &format!("group_{group}"),
            ask::options(
                json!({"task": "t", "brief": brief}),
                [("1".to_owned(), json!("a")), ("2".to_owned(), json!("b"))],
            ),
        );
    }
    let state = json!({
        "visible_text": {"untrusted_accessibility_data": (0..400).map(|line| format!("line {line} {}", "x".repeat(40))).collect::<Vec<_>>()},
        "elements": {"untrusted_accessibility_data": (0..50).map(|line| format!("button {line}")).collect::<Vec<_>>()},
    });
    let mut request = ask::request("jev-latest", state, questions);
    let before = serde_json::to_vec(&request).unwrap().len();
    fit(&mut request, 12_000);
    let after = serde_json::to_vec(&request).unwrap().len();
    assert!(before > 12_000 && after <= 12_000, "{before} -> {after}");
    let briefed = request
        .questions
        .values()
        .filter(|question| matches!(question, Question::Choice(choice) if choice.instructions.get("brief").is_some()))
        .count();
    assert_eq!(briefed, 1, "the brief stays on one question");
    let text = request.state["visible_text"]["untrusted_accessibility_data"]
        .as_array()
        .unwrap();
    assert_eq!(
        text[0].as_str().unwrap(),
        format!("line 0 {}", "x".repeat(40)),
        "the top of the screen is kept"
    );
    assert_eq!(
        request.state["elements"]["untrusted_accessibility_data"]
            .as_array()
            .unwrap()
            .len(),
        50
    );

    let mut small = ask::request(
        "jev-latest",
        json!({"a": [1, 2]}),
        ask::Questions::default().with("done", ask::completion("x")),
    );
    let untouched = serde_json::to_value(&small).unwrap();
    fit(&mut small, 12_000);
    assert_eq!(serde_json::to_value(&small).unwrap(), untouched);
    let mut unshrinkable = ask::request(
        "jev-latest",
        json!({"a": "y".repeat(500)}),
        ask::Questions::default().with("done", ask::completion("x")),
    );
    fit(&mut unshrinkable, 100);
    assert_eq!(
        unshrinkable.state["a"].as_str().unwrap().len(),
        500,
        "nothing to cut is left as is"
    );
}

#[tokio::test]
async fn a_long_goal_is_clipped_in_the_brief() {
    let run = run_with(
        App::default(),
        mail_flow(),
        |request| {
            request.brief = tinycomputer_bus::FlowBrief {
                goal: "g".repeat(2000),
                ..tinycomputer_bus::FlowBrief::default()
            };
        },
        |_, _, _| None,
    )
    .await;
    let goal = run
        .requests
        .iter()
        .map(brief_of)
        .find(|brief| !brief.is_null())
        .unwrap()["goal"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(goal.chars().count(), 601);
    assert!(goal.ends_with('…'));
}

/// A browser screen of `candidates`, in the order given.
fn long_screen(candidates: Vec<Candidate>) -> Screen {
    Screen {
        app: "browser".to_owned(),
        window: None,
        surface: "sheet".to_owned(),
        candidates,
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    }
}

/// `count` buttons named `prefix` and their number, each `covered` or not.
fn buttons(prefix: &str, count: u32, covered: bool) -> Vec<Candidate> {
    (0..count)
        .map(|index| {
            let mut control = node(
                &format!("{prefix} {index}"),
                "button",
                &["Click"],
                &["main"],
                f64::from(index),
            );
            if covered {
                control.states = vec!["covered".to_owned()];
            }
            control
        })
        .collect()
}

/// The element lines Jev is shown for `screen`.
fn element_lines(screen: &Screen) -> Vec<String> {
    ask::state(screen, "x", &[], false)["elements"]["untrusted_accessibility_data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_long_screen_shows_jev_what_is_in_view_and_keeps_room_for_the_rest() {
    // Live, a sign-up pop-up a long page drew at the end of its document
    // fell outside the first 120 elements, behind the covered page.
    let mut candidates = buttons("Page", 130, true);
    for name in ["close", "Enter your Mobile Number"] {
        candidates.push(node(name, "button", &["Click"], &["dialog"], 200.0));
    }
    let lines = element_lines(&long_screen(candidates));
    assert_eq!(lines.len(), 120);
    assert!(lines[0].contains("Page 0"), "kept in screen order");
    assert!(lines[118].contains("close"), "{:?}", &lines[115..]);
    assert!(lines[119].contains("Enter your Mobile Number"));

    // A calendar open in front fills no more than three quarters of the
    // room: the guests button it covers, early on the page, stays.
    let mut candidates = buttons("Search form", 3, true);
    candidates.extend(buttons("Day", 140, false));
    candidates.extend(buttons("Footer", 50, true));
    let lines = element_lines(&long_screen(candidates));
    assert_eq!(lines.len(), 120);
    assert!(lines[0].contains("Search form 0"), "{:?}", &lines[..4]);
    assert_eq!(lines.iter().filter(|line| line.contains("Day")).count(), 90);
}
