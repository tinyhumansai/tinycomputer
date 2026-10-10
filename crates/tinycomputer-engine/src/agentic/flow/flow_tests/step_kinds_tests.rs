//! The step kinds other than `do`, `choose`, `enter`, and `pick`: `read`,
//! `if`, `repeat_until`, `verify`, `wait_for`, `open`, and `browse`.

use super::*;

#[tokio::test]
async fn a_read_target_beyond_the_source_cap_is_still_found_by_paging() {
    // 72 candidates ("New Message", "Archive", and 70 message rows) exceed
    // `ask::MAX_READ_SOURCES` (60); a target past that cutoff must still be
    // reachable a page at a time rather than permanently dropped.
    let run = run_with(
        App::with(|sim| sim.extra_buttons = 70),
        json!({"app": "Mail", "steps": [
            {"read": {"what": "the row for message 65", "into": "row"}}
        ]}),
        |_| {},
        |id, question, _| (id == "source").then(|| pick(question, "Message 65", 0.9)),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.result.vars["row"], "Message 65");
}

#[tokio::test]
async fn a_read_can_take_an_elements_name_rather_than_its_value() {
    // The Subject field is named "Subject" and holds what was typed: asked
    // for the field's name, the read gets the name, not the value.
    let answers = |part: &'static str| {
        move |id: &str, question: &Question, _: &Sim| {
            (id == "source").then(|| pick(question, &format!("\"part\":\"{part}\""), 0.9))
        }
    };
    for (part, read) in [("name", "Subject"), ("value", "Moving Thursday")] {
        let run = run_with(
            App::default(),
            json!({"app": "Mail", "steps": [
                "start a new email message",
                {"enter": {"subject": "Moving Thursday"}},
                {"read": {"what": "the subject field", "into": "got"}}
            ]}),
            |_| {},
            answers(part),
        )
        .await;
        assert_eq!(run.result.stop, FlowStopReason::Completed, "{part}");
        assert_eq!(run.result.vars["got"], read, "{part}");
    }
}

#[tokio::test]
async fn later_steps_remember_what_earlier_steps_saved() {
    // A flow that walks a list must know which items it has already done:
    // every state after a read recalls the value, and an extract's rows as
    // their count.
    let app = flights();
    let run = run_with(
        app,
        json!({"app": "Mail", "steps": [
            {"read": {"what": "the window heading", "into": "heading"}},
            {"extract": {"what": "the flight results", "into": "flights"}},
            {"read": {"what": "the first Select button", "into": "again"}}
        ]}),
        |_| {},
        |id, question, _| (id == "source").then(|| pick(question, "Select", 0.9)),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    let first = run.requests.first().unwrap();
    assert!(
        first.state.get("already_collected").is_none(),
        "nothing is recalled before anything is saved"
    );
    let last = run.requests.last().unwrap();
    let recalled = &last.state["already_collected"]["untrusted_accessibility_data"];
    assert_eq!(recalled["heading"], run.result.vars["heading"]);
    assert!(
        recalled["flights"]
            .as_str()
            .unwrap()
            .starts_with("3 items; the first: IndiGo 6E-2135"),
        "{recalled}"
    );
}

#[tokio::test]
async fn control_steps_branch_repeat_read_and_wait() {
    let run = run_with(
        App::default(),
        json!({
            "app": "Mail",
            "steps": [
                {"if": {"condition": "a compose window is open",
                        "then": [{"verify": "never taken"}],
                        "else": ["start a new email message"]}},
                {"repeat_until": {"condition": "a compose window is open",
                                  "steps": ["start a new email message"], "max": 2}},
                {"wait_for": "a compose window is open"},
                {"read": {"what": "the window heading", "into": "heading"}},
                {"enter": {"subject": "About ${heading}"}}
            ]
        }),
        |_| {},
        |id, question, _| (id == "source").then(|| pick(question, "New Message heading", 0.9)),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.result.vars["heading"], "New Message heading");
    assert_eq!(run.app.sim().fields["Subject"], "About New Message heading");
    let paths = run
        .result
        .steps
        .iter()
        .map(|step| step.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["1", "1.1", "2", "3", "4", "5"]);
}

#[tokio::test]
async fn a_wait_for_stops_when_the_page_says_it_found_nothing() {
    // Live, a store's "No Results Found" page was checked ten times over,
    // three times in one task, and each rescue was told only that the
    // condition never held, so it guessed at the search's wording.
    let flow = json!({"app": "Mail", "steps": [{"wait_for": "search results are listed"}]});
    let empty = run(
        App::with(|sim| sim.hint = Some("No results found for \"Amul Taaza\"")),
        flow.clone(),
    )
    .await;
    assert_eq!(empty.result.stop, FlowStopReason::StepFailed);
    let note = &empty.result.steps[0].note;
    assert!(note.contains("\"no results\""), "{note}");
    let waiting = run(App::default(), flow).await;
    assert_eq!(waiting.result.stop, FlowStopReason::StepFailed);
    assert!(
        waiting.result.steps[0].note.contains("still not true"),
        "a page that says nothing of the kind is waited on in full"
    );
    assert!(
        asked(&empty.requests, "holds") < asked(&waiting.requests, "holds"),
        "it stopped waiting early"
    );
}

#[tokio::test]
async fn a_repeat_that_never_holds_and_a_failing_verify_fail_the_flow() {
    let repeat = run(
        App::quirky(Quirk::Frozen),
        json!({"app": "Mail", "steps": [
            {"repeat_until": {"condition": "a compose window is open",
                              "steps": [{"wait_for": "nothing"}], "max": 1}}
        ]}),
    )
    .await;
    assert_eq!(repeat.result.stop, FlowStopReason::StepFailed);

    let verify = run(
        App::default(),
        json!({"app": "Mail", "steps": [{"verify": "a compose window is open"}]}),
    )
    .await;
    assert_eq!(verify.result.stop, FlowStopReason::StepFailed);
    assert!(verify.result.steps[0].note.contains("does not hold"));

    let branch_then = run(
        App::with(|sim| sim.compose_open = true),
        json!({"app": "Mail", "steps": [
            {"if": {"condition": "a compose window is open", "then": [{"verify": "a compose window is open"}]}},
            {"repeat_until": {"condition": "a compose window is open", "steps": ["x"]}}
        ]}),
    )
    .await;
    assert_eq!(branch_then.result.stop, FlowStopReason::Completed);
}

#[tokio::test]
async fn a_repeat_conditions_trace_is_attributed_to_the_repeat_step_not_its_last_child() {
    // Round 0 runs its child ("start a new email message", path "1.r1.1"),
    // which opens the compose window. Round 1's condition check must then be
    // traced to "1", the repeat_until step itself, not left tagged with the
    // path of the child that last ran.
    let run = run(
        App::default(),
        json!({"app": "Mail", "steps": [
            {"repeat_until": {"condition": "a compose window is open",
                              "steps": ["start a new email message"], "max": 2}}
        ]}),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(
        run.result
            .trace
            .last()
            .map(|exchange| exchange.step.as_str()),
        Some("1"),
        "the condition check that ended the loop belongs to the repeat_until step"
    );
}

#[tokio::test]
async fn fields_in_a_truncated_subtree_are_found_by_exploring_it() {
    let run = run(
        App::with(|sim| {
            sim.compose_open = true;
            sim.quirks.insert(Quirk::HiddenEditor);
        }),
        json!({"app": "Mail", "steps": [{"enter": {"subject": "Found it"}}]}),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.app.sim().fields["Subject"], "Found it");
}

#[tokio::test]
async fn an_app_that_never_shows_a_window_is_reported_as_opened_but_unreadable() {
    let run = run(
        App::quirky(Quirk::FailObserve),
        json!({"app": "Mail", "steps": [{"open": "Mail"}]}),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert!(run.result.steps[0].note.contains("no readable window yet"));
}

#[tokio::test]
async fn browse_opens_the_address_and_moves_the_flow_onto_the_page() {
    let run = run(
        App::default(),
        json!({
            "app": "Mail",
            "vars": {"to": "Srinagar"},
            "steps": [{"browse": "https://flights.test/to/${to}"}]
        }),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.app.sim().launched, ["Mail", "browser"]);
    assert_eq!(
        run.app.sim().navigated,
        ["https://flights.test/to/Srinagar"]
    );
    assert_eq!(
        run.result.steps[0].note,
        "https://flights.test/to/Srinagar is open (Flights)"
    );
}

#[tokio::test]
async fn browse_fails_the_flow_where_there_is_no_browser_or_no_page() {
    for (quirk, code) in [
        (Quirk::FailLaunch, "APP_NOT_FOUND"),
        (Quirk::NoAddresses, "ACTION_NOT_SUPPORTED"),
    ] {
        let run = run(
            App::quirky(quirk),
            json!({"app": "browser", "steps": [{"browse": "https://flights.test"}]}),
        )
        .await;
        assert_eq!(run.result.stop, FlowStopReason::StepFailed, "{quirk:?}");
        assert!(
            run.result.steps[0].note.contains(code),
            "{}",
            run.result.steps[0].note
        );
    }
    // The note keeps the failure's own words after its code, so a browser
    // that could not be started says what to set.
    let unopened = run(
        App::quirky(Quirk::FailLaunch),
        json!({"app": "browser", "steps": [{"browse": "https://flights.test"}]}),
    )
    .await;
    assert_eq!(
        unopened.result.steps[0].note,
        "https://flights.test could not be opened: APP_NOT_FOUND (no such app)"
    );
    let unreadable = run(
        App::quirky(Quirk::FailObserve),
        json!({"app": "browser", "steps": [{"browse": "https://flights.test"}]}),
    )
    .await;
    assert!(
        unreadable.result.steps[0]
            .note
            .contains("no readable page yet")
    );
}

#[test]
fn a_failure_is_noted_by_its_code_and_the_first_line_of_its_words() {
    use super::steps::failure;
    let refused = |message: &str| {
        tinycomputer_bus::DesktopResponse::err(
            "launch",
            tinycomputer_bus::DesktopError::new("BROWSER_UNAVAILABLE", message),
        )
    };
    assert_eq!(
        failure(&refused(
            "browser unavailable: no Chrome or Chromium was found on this machine; give the path of the browser to use\nChecked: /Applications"
        )),
        "BROWSER_UNAVAILABLE (browser unavailable: no Chrome or Chromium was found on this machine; give the path of the browser to use)"
    );
    assert_eq!(failure(&refused("  ")), "BROWSER_UNAVAILABLE");
    let long = failure(&refused(&"x".repeat(300)));
    assert_eq!(long, format!("BROWSER_UNAVAILABLE ({}…)", "x".repeat(200)));
    assert_eq!(
        failure(&tinycomputer_bus::DesktopResponse::ok("launch", json!({}))),
        "unknown error"
    );
}

#[test]
fn a_plain_step_that_types_is_read_as_the_enter_it_means() {
    use super::steps::typing;
    let slot = |intent: &str| typing(intent).map(|slot| (slot.slot, slot.text));
    assert_eq!(
        slot("enter 560001 into the pincode field"),
        Some(("pincode".to_owned(), "560001".to_owned()))
    );
    assert_eq!(
        slot("Type 'Maggi' in the search box."),
        Some(("search".to_owned(), "Maggi".to_owned()))
    );
    assert_eq!(
        slot("fill in the pincode with 560001"),
        Some(("pincode".to_owned(), "560001".to_owned()))
    );
    assert_eq!(
        slot("enter Amul Taaza milk in 1 litre packs in the search bar"),
        Some((
            "search".to_owned(),
            "Amul Taaza milk in 1 litre packs".to_owned()
        )),
        "the last \" in \" splits a text that has \"in\" in it"
    );
    assert_eq!(
        slot("enter Bengaluru as the city"),
        Some(("city".to_owned(), "Bengaluru".to_owned()))
    );
    assert_eq!(
        slot("Type in 560001 into the pincode box"),
        Some(("pincode".to_owned(), "560001".to_owned()))
    );
    assert_eq!(
        slot("enter the ${otp} in the code box"),
        Some(("code".to_owned(), "${otp}".to_owned())),
        "words that only describe a lone name are not typed"
    );
    assert_eq!(
        slot("type 'shoes' in the search box in the header"),
        Some(("search box in the header".to_owned(), "shoes".to_owned())),
        "a quoted text ends at its quote"
    );
    assert_eq!(
        slot("type made in india in the search box"),
        Some(("search".to_owned(), "made in india".to_owned()))
    );
    for plain in [
        "enter the store",
        "press enter in the search box",
        "type in the search box",
        "enter your name in the name field",
        "open the cart",
        // "Enter" also means going into something.
        "Enter Reader mode in Safari",
        "enter full screen in the player",
        // A step that does more than type is two steps.
        "type 'Maggi' in the search box and press Enter",
        "enter 560001 in the pincode field then search",
    ] {
        assert_eq!(slot(plain), None, "{plain}");
    }
}

#[test]
fn the_items_chosen_together_in_one_list_read_as_one_source() {
    // Live, a seat table marked two seats "Selected", and a read of "the
    // selected seats" could take only one piece of text, naming neither.
    let seat = |number: u32, status: &str, chosen: bool| Candidate {
        ref_id: format!("seen:{number}"),
        role: "gridcell".to_owned(),
        name: Some(if chosen { "Selected" } else { "Select" }.to_owned()),
        states: if chosen {
            vec!["checked".to_owned()]
        } else {
            Vec::new()
        },
        path: vec![
            "dialog \"Seats\"".to_owned(),
            "grid \"Row A\"".to_owned(),
            format!("row \"{number:02} {status}\" #{number}"),
        ],
        ..Candidate::default()
    };
    let mut screen = Screen {
        app: "browser".to_owned(),
        window: None,
        surface: "sheet".to_owned(),
        candidates: vec![
            seat(1, "Handicapped", false),
            seat(2, "Companion", true),
            seat(3, "Available", true),
        ],
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    };
    let sources = steps::chosen_together(&screen, true);
    assert_eq!(sources.len(), 1, "{sources:?}");
    assert_eq!(sources[0].2, "02 Companion; 03 Available");
    assert!(
        sources[0]
            .1
            .to_string()
            .contains("02 Companion; 03 Available")
    );
    let masked = steps::chosen_together(&screen, false);
    assert!(
        masked[0].1.to_string().contains("26 characters"),
        "{masked:?}"
    );

    // One chosen item is already a source of its own.
    screen.candidates.truncate(2);
    assert_eq!(steps::chosen_together(&screen, true).len(), 0);
}

#[test]
fn a_stop_before_names_only_signing_in_or_something_irreversible() {
    for login in [
        "signing in",
        "logging in to your account",
        "Log in",
        "signing in with OTP",
    ] {
        assert!(steps::only_signs_in(login), "{login}");
    }
    for gate in [
        "paying or logging in",
        "proceeding to checkout",
        "entering a phone number",
        "booking the cab",
        "placing the order",
        "the login fee payment",
        // Creating an account hands a person's details to the site.
        "sign up or log in",
        "creating an account",
        "registering",
    ] {
        assert!(!steps::only_signs_in(gate), "{gate}");
    }
}

#[tokio::test]
async fn a_stop_before_signing_in_lets_the_flow_go_on() {
    // Live, a plan for "do not log in" stopped short of the cart, in front
    // of a header's "Hello, sign in": a login wall pauses on its own.
    let run = run(
        App::default(),
        json!({"app": "Mail", "steps": [
            {"stop_before": "signing in"},
            "start a new email message"
        ]}),
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{}",
        run.result.steps[0].note
    );
    assert_eq!(run.result.steps[0].outcome, StepOutcome::Done);
    assert!(run.app.sim().compose_open);
}

#[tokio::test]
async fn a_plain_typing_step_types_a_value_that_names_a_fact_as_written() {
    // A value read off a page can say `${card number}`. Typed through a
    // plain step, it must stay that text: substituted twice, it became the
    // caller's secret, typed wherever the page's step pointed.
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": [
            {"open": "Mail"},
            "start a new email message",
            "type ${code} into the message body field"
        ]}),
        |request| {
            request.vars = BTreeMap::from([
                ("card number".to_owned(), "4111111111111111".to_owned()),
                ("code".to_owned(), "${card number}".to_owned()),
            ]);
            request.facts = BTreeSet::from(["card number".to_owned()]);
            request.include_values = true;
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(
        run.app.sim().fields.get("Body").map(String::as_str),
        Some("${card number}"),
        "{:?}",
        run.result.steps
    );
}

#[tokio::test]
async fn a_value_read_before_a_rescue_outranks_the_flows_empty_declaration() {
    // A planner declares each read's variable up front, empty, and a flow
    // resumed after a rescue declares it again: `${total}` read before the
    // rescue expanded to nothing after it.
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "vars": {"total": ""}, "steps": [
            {"open": "Mail"},
            "start a new email message",
            {"enter": {"subject": "Total ${total}"}}
        ]}),
        |request| {
            request.collected = BTreeMap::from([("total".to_owned(), "₹95".to_owned())]);
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(
        run.app.sim().fields.get("Subject").map(String::as_str),
        Some("Total ₹95"),
        "{:?}",
        run.result.steps
    );
}
