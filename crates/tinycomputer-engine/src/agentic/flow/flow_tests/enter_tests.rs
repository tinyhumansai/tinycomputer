//! The `enter` step: revealing fields, matching slots without previewing
//! values, reading fields back, flagged fields, and fields that refuse text.

use super::*;

#[tokio::test]
async fn enter_reveals_fields_and_fails_for_a_slot_with_no_field() {
    let revealed = run(
        App::default(),
        json!({"app": "Mail", "steps": [{"enter": {"subject": "Hi"}}]}),
    )
    .await;
    assert_eq!(revealed.result.stop, FlowStopReason::Completed);
    assert_eq!(revealed.app.sim().fields["Subject"], "Hi");

    let missing = run_with(
        App::with(|sim| sim.compose_open = true),
        json!({"app": "Mail", "steps": [{"enter": {"shoe size": "11"}}]}),
        // Every way to find a field is tried before the step fails.
        |request| request.max_actions = 20,
        |id, question, _| {
            (id.starts_with("slot_") || id == "target").then(|| pick(question, "none", 0.9))
        },
    )
    .await;
    assert_eq!(missing.result.stop, FlowStopReason::StepFailed);

    let remembered = run_with(
        App::with(|sim| sim.compose_open = true),
        json!({"app": "Mail", "steps": [{"enter": {"subject": "Remembered"}}]}),
        |request| {
            request.memory = vec![GroundingHint {
                app: "Mail".to_owned(),
                key: "subject".to_owned(),
                role: "textfield".to_owned(),
                name: Some("Subject".to_owned()),
                path: vec![
                    "window \"New Message\"".to_owned(),
                    "group \"Header\"".to_owned(),
                ],
            }];
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(remembered.app.sim().fields["Subject"], "Remembered");
    assert!(
        remembered
            .requests
            .iter()
            .all(|request| !request.questions.contains_key("slot_0")),
        "a remembered field needs no slot question"
    );
}

#[tokio::test]
async fn entered_values_are_never_previewed_in_a_slot_matching_question() {
    // A slot's text can be a password, token, or private message; the model
    // only needs to know which field to type it into, not a preview of it.
    let run = run_with(
        App::with(|sim| sim.compose_open = true),
        json!({"app": "Mail", "steps": [{"enter": {"shoe size": "hunter2 super secret token"}}]}),
        // Every way to find a field is tried before the step fails.
        |request| request.max_actions = 20,
        |id, question, _| {
            (id.starts_with("slot_") || id == "target").then(|| pick(question, "none", 0.9))
        },
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert!(
        run.requests.iter().all(|request| request
            .questions
            .values()
            .all(|question| !text_of(question, "purpose").contains("hunter2"))),
        "the entered value must never reach a slot-matching question"
    );
}

#[test]
fn editable_fields_are_found_by_action_or_role_in_reading_order() {
    let mut screen = App::with(|sim| sim.compose_open = true).screen();
    screen.candidates.push(Candidate {
        role: "combobox".to_owned(),
        name: Some("From".to_owned()),
        ..Candidate::default()
    });
    screen.candidates.push(Candidate {
        role: "webarea".to_owned(),
        name: Some("message body".to_owned()),
        available_actions: vec!["SetFocus".to_owned()],
        ..Candidate::default()
    });
    let names = enter::editable(&screen)
        .into_iter()
        .map(|field| field.name.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["To", "Subject", "Body", "From", "message body"]);
}

#[test]
fn a_rich_text_area_reports_the_text_inside_it_as_its_contents() {
    // The token label and the rich-text body lines are ref-less, as they are
    // in a real snapshot, so they live in `text_nodes` rather than
    // `candidates`; `order` places them in the document position they would
    // really occupy.
    let mut screen = App::with(|sim| {
        sim.compose_open = true;
        sim.fields.insert("Subject".to_owned(), "Hello".to_owned());
    })
    .screen();
    for (index, candidate) in screen.candidates.iter_mut().enumerate() {
        candidate.order = index * 10;
    }
    let body = Candidate {
        role: "webarea".to_owned(),
        name: Some("message body".to_owned()),
        available_actions: vec!["SetFocus".to_owned()],
        order: screen.candidates.len() * 10,
        ..Candidate::default()
    };
    let area = super::view::label(&body);
    screen.candidates.push(body);
    for (offset, line) in ["Hi Sam,", "See you Friday."].iter().enumerate() {
        screen.text_nodes.push(Candidate {
            role: "statictext".to_owned(),
            value: Some(json!(line)),
            path: vec![area.clone()],
            order: screen.candidates.len() * 10 + offset + 1,
            ..Candidate::default()
        });
    }
    // The "To" field turned into a token; its accessible name sits in a
    // ref-less static text node immediately after it, as it does for real.
    screen.candidates[0].value = Some(json!("\u{fffc}"));
    screen.text_nodes.push(Candidate {
        role: "statictext".to_owned(),
        name: Some("sam@example.com".to_owned()),
        order: 1,
        ..Candidate::default()
    });
    let state = ask::state(&screen, "check the draft", &[], true);
    let fields = state["field_contents"]["untrusted_accessibility_data"]
        .as_array()
        .unwrap();
    assert!(fields.contains(&json!({"field": "textfield \"Subject\"", "holds": "Hello"})));
    assert!(fields.contains(&json!({"field": "textfield \"To\"", "holds": "sam@example.com"})));
    assert!(fields.contains(&json!({"field": area, "holds": "Hi Sam,\nSee you Friday."})));
    assert!(
        ask::state(&screen, "x", &[], false)
            .get("field_contents")
            .is_none()
    );
    // The body's text must never appear in `visible_text`, which every
    // request shares regardless of `include_values`; only the gated
    // `field_contents` above may carry it.
    for include_values in [false, true] {
        let visible_text =
            ask::state(&screen, "x", &[], include_values)["visible_text"].to_string();
        assert!(!visible_text.contains("Hi Sam,"));
        assert!(!visible_text.contains("sam@example.com"));
    }
}

#[test]
fn a_named_control_with_a_numeric_value_reads_as_its_name() {
    let radio = |value: &str| Candidate {
        role: "radiobutton".to_owned(),
        name: Some("Dark".to_owned()),
        value: Some(json!(value)),
        ..Candidate::default()
    };
    assert_eq!(super::steps::readable(&radio("1")).as_deref(), Some("Dark"));
    assert_eq!(
        super::steps::readable(&radio("Night")).as_deref(),
        Some("Night")
    );
    let blank = Candidate {
        role: "group".to_owned(),
        name: Some(" ".to_owned()),
        ..Candidate::default()
    };
    assert!(super::steps::readable(&blank).is_none());
}

#[tokio::test]
async fn a_field_the_form_flags_is_entered_again_and_then_gives_up() {
    let flow = json!({"app": "Mail", "steps": [
        {"open": "Mail"},
        "start a new email message",
        {"enter": {"subject": "Moving Thursday's sync"}}
    ]});
    let asked = Arc::new(Mutex::new(0));
    let counter = asked.clone();
    let recovered = run_with(
        App::default(),
        flow.clone(),
        |_| {},
        move |id, _, _| {
            (id == "error_0").then(|| {
                let mut asked = counter.lock().unwrap();
                *asked += 1;
                noul(if *asked == 1 { 0.9 } else { 0.05 })
            })
        },
    )
    .await;
    assert_eq!(recovered.result.stop, FlowStopReason::Completed);
    assert!(
        recovered.result.steps[2]
            .loops
            .contains(&FlowLoop::Validation)
    );
    let fills = recovered.result.steps[2]
        .actions
        .iter()
        .filter(|action| action.action.starts_with("fill"))
        .count();
    assert_eq!(fills, 2, "the flagged field is entered twice");

    let stuck = run_with(
        App::default(),
        flow,
        |_| {},
        |id, _, _| (id == "error_0").then(|| noul(0.9)),
    )
    .await;
    assert_eq!(stuck.result.stop, FlowStopReason::StepFailed);
    assert!(
        stuck.result.steps[2]
            .note
            .contains("still shows an error about: subject"),
        "{}",
        stuck.result.steps[2].note
    );
}

#[tokio::test]
async fn a_date_is_typed_in_the_layout_the_page_asks_for() {
    let run = run(
        App::with(|sim| {
            sim.compose_open = true;
            sim.hint = Some("Please enter the date in (DD-MM-YYYY) format");
        }),
        json!({"app": "Mail", "steps": [{"enter": {"subject": "2000-01-31"}}]}),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.app.sim().fields["Subject"], "31-01-2000");
}

#[tokio::test]
async fn a_detail_the_form_does_not_ask_for_is_skipped_not_typed_blindly() {
    let run = run_with(
        App::with(|sim| sim.compose_open = true),
        json!({"app": "Mail", "steps": [{"enter": {"subject": "Hi", "title": "Ms"}}]}),
        |_| {},
        |id, question, _| match id {
            "asks_1" => Some(noul(0.1)),
            _ if id.starts_with("slot_") && text_of(question, "purpose").contains("title") => {
                Some(pick(question, "none", 0.9))
            }
            _ => None,
        },
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    let step = &run.result.steps[0];
    assert!(
        step.note.contains("the form does not ask for: title"),
        "{}",
        step.note
    );
    let sim = run.app.sim();
    assert_eq!(sim.fields["Subject"], "Hi", "no blind typing spoiled it");
    assert!(
        !sim.fields.values().any(|value| value.contains("Ms")),
        "{:?}",
        sim.fields
    );
    assert!(
        step.actions
            .iter()
            .all(|action| action.action != "type to filter"),
        "a slot with no field is never typed into the focus"
    );
}

#[tokio::test]
async fn a_field_that_refuses_the_text_is_struck_and_the_real_one_is_used() {
    // A page gives each suggested city a `combobox` role and no name, each
    // holding its city as a value; Jev takes one for "the destination
    // search", as it did live. It refuses the text, and the step must strike
    // every row of its kind, not try them one by one.
    let run = run_with(
        App::quirky(Quirk::CityRows),
        json!({"app": "Mail", "steps": [{"enter": {"destination search": "Srinagar"}}]}),
        |_| {},
        |id, question, _| {
            id.starts_with("slot_").then(|| {
                let offered = serde_json::to_string(question).unwrap();
                let needle = if offered.contains(r#""what":"combobox""#) {
                    r#""what":"combobox""#
                } else {
                    "Search"
                };
                pick(question, needle, 0.9)
            })
        },
    )
    .await;
    let step = &run.result.steps[0];
    assert_eq!(step.outcome, StepOutcome::Done, "{}", step.note);
    // The waits for a place box's late suggestions have no target.
    let fills = step
        .actions
        .iter()
        .filter_map(|action| {
            action
                .target
                .as_ref()
                .map(|target| (target.ref_id.clone(), action.ok))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        fills,
        [
            ("@s:city-0".to_owned(), false),
            ("@s:Search".to_owned(), true)
        ],
        "one refusal strikes every row of its kind, then the real field"
    );
    assert_eq!(run.app.sim().fields["Search"], "Srinagar");
}

#[tokio::test]
async fn a_row_that_refused_the_text_is_never_pressed_while_revealing_a_field() {
    // Live, after a city row refused the text, the reveal loop pressed that
    // row and chose a city nobody asked for.
    let run = run_with(
        App::quirky(Quirk::CityRows),
        json!({"app": "Mail", "steps": [{"enter": {"destination search": "Srinagar"}}]}),
        |_| {},
        |id, question, _| {
            let offered = serde_json::to_string(question).unwrap();
            let row = r#""what":"combobox""#;
            match id {
                // Jev keeps wanting a row: for the text, and to reveal.
                _ if id.starts_with("slot_") || id == "target" => Some(if offered.contains(row) {
                    pick(question, row, 0.9)
                } else {
                    pick(question, "none", 0.9)
                }),
                "move" => Some(pick(question, "activate", 0.9)),
                _ => None,
            }
        },
    )
    .await;
    let step = &run.result.steps[0];
    let rows = step
        .actions
        .iter()
        .filter(|action| {
            action
                .target
                .as_ref()
                .is_some_and(|target| target.ref_id.starts_with("@s:city-"))
        })
        .map(|action| action.action.clone())
        .collect::<Vec<_>>();
    assert_eq!(rows, ["fill destination search"], "{:?}", step.actions);
    assert_eq!(step.outcome, StepOutcome::Failed);
    assert_eq!(
        step.note,
        "no field that takes text was found for: destination search; 1 element(s) the page offered as fields refused the text"
    );
}

#[tokio::test]
async fn a_control_named_by_the_slot_opens_its_box_only_when_jev_agrees() {
    // A link sharing the slot's word ("Search mail" for "search") shows the
    // box, but a shared word alone is no reason to press: "Email us"
    // shares "email", and pressing it left the form.
    let confirming =
        |yes: f64| move |id: &str, _: &Question, _: &Sim| (id == "confirm").then(|| noul(yes));
    let opened = run_with(
        App::with(|sim| {
            sim.quirks.insert(Quirk::SearchBehindLink);
        }),
        json!({"app": "Mail", "steps": [{"enter": {"search": "invoices"}}]}),
        |_| {},
        confirming(0.95),
    )
    .await;
    assert_eq!(
        opened.app.sim().fields.get("Search").map(String::as_str),
        Some("invoices"),
        "{:?}",
        opened.result.steps
    );
    assert!(
        opened.result.steps[0]
            .actions
            .iter()
            .any(|action| action.action == "click (show the field)"),
        "{:?}",
        opened.result.steps[0].actions
    );

    let refused = run_with(
        App::with(|sim| {
            sim.quirks.insert(Quirk::SearchBehindLink);
        }),
        json!({"app": "Mail", "steps": [{"enter": {"search": "invoices"}}]}),
        |_| {},
        confirming(0.2),
    )
    .await;
    assert!(
        !refused.result.steps[0]
            .actions
            .iter()
            .any(|action| action.action == "click (show the field)"),
        "pressed without Jev agreeing: {:?}",
        refused.result.steps[0].actions
    );
}

#[test]
fn a_short_place_word_names_its_box_only_as_a_labels_first_word() {
    // Live, a flight form drew its place boxes as buttons ("From DEL",
    // "To BLR"), and the "to" box was never opened: "to" is too short a
    // word to look for anywhere in a label ("Tap to add a return date").
    use super::enter::named_opener;
    let slots = vec![tinycomputer_bus::Slot {
        slot: "to".to_owned(),
        text: "Mumbai".to_owned(),
    }];
    let screen = Screen {
        app: "browser".to_owned(),
        window: None,
        surface: "window".to_owned(),
        candidates: vec![
            node(
                "From DEL, Delhi Airport India",
                "button",
                &["Click"],
                &["form"],
                0.0,
            ),
            node(
                "Return Tap to add a return date",
                "button",
                &["Click"],
                &["form"],
                1.0,
            ),
            node(
                "To BLR, Bengaluru Airport India",
                "button",
                &["Click"],
                &["form"],
                2.0,
            ),
        ],
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    };
    let opener = named_opener(&screen, &slots, &BTreeSet::from([0])).unwrap();
    assert_eq!(
        opener.name.as_deref(),
        Some("To BLR, Bengaluru Airport India")
    );
    // However the surface spells the role.
    let mut screen = screen;
    for candidate in &mut screen.candidates {
        candidate.role = "Button".to_owned();
    }
    let opener = named_opener(&screen, &slots, &BTreeSet::from([0])).unwrap();
    assert_eq!(
        opener.name.as_deref(),
        Some("To BLR, Bengaluru Airport India")
    );
}

#[test]
fn the_opener_is_the_box_most_particular_to_the_slot() {
    // Live, for the slot "from city" a trip-type tab "Multi City" came
    // before the "From DEL" box, and a button wrapping the whole form came
    // before the box inside it. A control that only shares the slot's
    // plainer word ("Date Change", "City Guides") is no opener either.
    use super::enter::named_opener;
    let screen = |names: &[(&str, bool)]| Screen {
        app: "browser".to_owned(),
        window: None,
        surface: "window".to_owned(),
        candidates: names
            .iter()
            .map(|(name, away)| Candidate {
                states: if *away {
                    vec!["offscreen".to_owned()]
                } else {
                    Vec::new()
                },
                ..node(name, "button", &["Click"], &["form"], 0.0)
            })
            .collect(),
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    };
    let opener = |screen: &Screen, slot: &str| {
        let slots = [tinycomputer_bus::Slot {
            slot: slot.to_owned(),
            text: "x".to_owned(),
        }];
        named_opener(screen, &slots, &BTreeSet::from([0])).and_then(|opener| opener.name)
    };
    let flights = screen(&[
        ("Multi City", false),
        ("City Guides", false),
        ("Date Change", false),
        (
            "From DEL, Delhi Airport India \u{21cc} To BLR, Bengaluru Airport India Departure \
             23 Oct 26 Friday Return Tap to add a return date",
            false,
        ),
        ("From DEL, Delhi Airport India", false),
        ("To BLR, Bengaluru Airport India", false),
        ("Departure 23 Oct 26 Friday", false),
    ]);
    assert_eq!(
        opener(&flights, "from city").as_deref(),
        Some("From DEL, Delhi Airport India")
    );
    assert_eq!(
        opener(&flights, "to city").as_deref(),
        Some("To BLR, Bengaluru Airport India")
    );
    assert_eq!(
        opener(&flights, "departure date").as_deref(),
        Some("Departure 23 Oct 26 Friday")
    );
    let hotel = screen(&[
        ("Check availability", false),
        ("Check-in 18 Oct 2026 Sunday", true),
        ("Check-in 18 Oct 2026", false),
    ]);
    assert_eq!(
        opener(&hotel, "check-in date").as_deref(),
        Some("Check-in 18 Oct 2026"),
        "on screen, and holding more of the slot's words"
    );
}
