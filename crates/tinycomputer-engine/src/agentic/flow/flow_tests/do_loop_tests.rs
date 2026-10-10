//! The `do` loop: obstacles, overlays, covered clicks, regressions, stalls,
//! refused controls, move outcomes, and disabled loops.

use super::*;

#[tokio::test]
async fn an_obstacle_is_dismissed_with_a_safe_control_only() {
    let run = run(
        App::with(|sim| sim.obstacle = true),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert!(run.result.steps[0].loops.contains(&FlowLoop::Obstacles));
    assert_eq!(run.app.sim().clicks, ["Keep Editing"]);
    let dismiss = run
        .requests
        .iter()
        .find_map(|request| request.questions.get("dismiss"))
        .unwrap();
    assert!(
        !serde_json::to_string(dismiss)
            .unwrap()
            .contains("Delete Draft"),
        "an irreversible control is never offered to clear an obstacle"
    );

    let escaped = run_with(
        App::with(|sim| sim.obstacle = true),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |_| {},
        |id, question, _| (id == "dismiss").then(|| pick(question, "escape", 0.9)),
    )
    .await;
    assert!(escaped.app.sim().presses.contains(&"escape".to_owned()));
}

/// Answers that press "Keep Editing" and never judge the step done, so only
/// the screen can end it.
fn press_keep_editing(id: &str, question: &Question, _: &Sim) -> Option<Answer> {
    match id {
        "done" | "blocked" => Some(noul(0.05)),
        "move" => Some(pick(question, "activate", 0.9)),
        _ if id == "target" || id == "region" || id.starts_with("group_") => {
            Some(pick(question, "Keep Editing", 0.9))
        }
        _ => None,
    }
}

#[tokio::test]
async fn pressing_the_named_control_that_closes_an_overlay_ends_the_step() {
    for step in [
        "close the dialog by keeping editing",
        "dismiss the save prompt",
    ] {
        let run = run_with(
            App::with(|sim| sim.obstacle = true),
            json!({"app": "Mail", "steps": [step]}),
            |_| {},
            press_keep_editing,
        )
        .await;
        assert_eq!(run.result.stop, FlowStopReason::Completed, "{step}");
        assert_eq!(run.app.sim().clicks, ["Keep Editing"], "{step}");
        assert!(
            run.result.steps[0].note.contains("closed"),
            "{}",
            run.result.steps[0].note
        );
    }
    let unrelated = run_with(
        App::with(|sim| sim.obstacle = true),
        json!({"app": "Mail", "steps": ["archive the message"]}),
        |request| request.max_actions = 1,
        press_keep_editing,
    )
    .await;
    assert_ne!(
        unrelated.result.stop,
        FlowStopReason::Completed,
        "closing an overlay the step never mentions does not finish it"
    );
}

#[tokio::test]
async fn a_covered_click_closes_what_covers_it_and_tries_again() {
    let run = run_with(
        App::quirky(Quirk::Drawer),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |_| {},
        |id, question, _| (id == "move").then(|| pick(question, "activate", 0.9)),
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    let sim = run.app.sim();
    assert!(sim.compose_open);
    assert_eq!(sim.presses, ["escape"]);
    let actions = &run.result.steps[0].actions;
    assert_eq!(
        actions
            .iter()
            .map(|action| action.action.as_str())
            .collect::<Vec<_>>(),
        ["click", "press escape (uncover)", "click"],
    );
    // The launch and Escape fetch nothing and settle briefly; the refused
    // click does not settle, and the one that went through settles in full.
    assert_eq!(sim.trail, ["settle briefly", "settle briefly", "settle"]);
}

#[tokio::test]
async fn an_empty_layer_escape_leaves_is_pressed_outside_and_the_press_tried_again() {
    // Live, a store's search box left its suggestions open with a backdrop
    // over the page that Escape left there. Its basket button was refused
    // under it, the refused press read as one that opened a dialog, and the
    // step's failure told a rescue to answer that dialog: it re-added items.
    let run = run_with(
        App::quirky(Quirk::Backdrop),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        |id, question, _| (id == "move").then(|| pick(question, "activate", 0.9)),
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    let sim = run.app.sim();
    assert!(sim.compose_open);
    assert_eq!(sim.presses, ["escape"]);
    assert_eq!(sim.clicks, ["the backdrop", "New Message"]);
    assert_eq!(
        run.result.steps[0]
            .actions
            .iter()
            .map(|action| action.action.as_str())
            .collect::<Vec<_>>(),
        [
            "click",
            "press escape (uncover)",
            "click",
            "press the cover (uncover)",
            "click"
        ],
    );
    let told = |text: &str| {
        run.requests.iter().any(|request| {
            serde_json::to_string(&request.state)
                .unwrap()
                .contains(text)
        })
    };
    assert!(told(
        "was covered by an empty layer; pressed escape to close it"
    ));
    assert!(told(
        "was still covered by an empty layer; pressed an empty part of it"
    ));
    assert!(
        !told("the last press opened a dialog"),
        "a press the page refused opened nothing, though the header it scrolled in is covered"
    );
}

#[tokio::test]
async fn a_layer_that_cannot_be_pressed_or_went_by_itself_is_handled_as_it_is() {
    let activate = |id: &str, question: &Question, _: &Sim| {
        (id == "move").then(|| pick(question, "activate", 0.9))
    };
    let told = |run: &Run, text: &str| {
        run.requests.iter().any(|request| {
            serde_json::to_string(&request.state)
                .unwrap()
                .contains(text)
        })
    };
    // A layer the surface will not press: the step fails naming it.
    let stuck = run_with(
        App::quirky(Quirk::StubbornBackdrop),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        activate,
    )
    .await;
    assert_eq!(stuck.result.stop, FlowStopReason::StepFailed);
    let step = &stuck.result.steps[0];
    assert!(
        step.note.ends_with(
            "its press of button \"New Message\" was refused because an empty layer lies over it"
        ),
        "{}",
        step.note
    );
    assert!(!told(&stuck, "pressed an empty part of it"));
    assert!(!stuck.app.sim().compose_open);

    // A layer gone by itself before it is pressed: the press is tried
    // again, and nothing claims the layer was pressed.
    let gone = run_with(
        App::quirky(Quirk::FleetingBackdrop),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        activate,
    )
    .await;
    assert_eq!(gone.result.stop, FlowStopReason::Completed);
    assert_eq!(
        gone.result.steps[0]
            .actions
            .iter()
            .map(|action| action.action.as_str())
            .collect::<Vec<_>>(),
        [
            "click",
            "press escape (uncover)",
            "click",
            "press the cover (uncover)",
            "click"
        ],
    );
    assert_eq!(gone.app.sim().clicks, ["New Message"]);
    assert!(!told(&gone, "pressed an empty part of it"));
}

#[tokio::test]
async fn a_press_a_control_still_covers_fails_naming_that_control() {
    let run = run_with(
        App::quirky(Quirk::ControlOver),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        |id, question, _| (id == "move").then(|| pick(question, "activate", 0.9)),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    let step = &run.result.steps[0];
    assert!(
        step.note.ends_with(
            "its press of button \"New Message\" was refused because button \"Select Location\" lies over it"
        ),
        "{}",
        step.note
    );
    assert!(
        !step
            .actions
            .iter()
            .any(|action| action.action == "press the cover (uncover)"),
        "a control is never pressed as an empty layer: {:?}",
        step.actions
    );
    assert!(run.requests.iter().any(|request| {
        serde_json::to_string(&request.state)
            .unwrap()
            .contains("is still covered by button \\\"Select Location\\\": deal with that first")
    }));
}

#[tokio::test]
async fn a_covered_click_closes_the_banner_in_front_with_its_own_button() {
    // Live, a consent banner lay over "Add To Cart"; Escape left it there,
    // and every press was refused. Its least committal button closes it.
    let run = run_with(
        App::quirky(Quirk::ConsentBanner),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        |id, question, _| (id == "move").then(|| pick(question, "activate", 0.9)),
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    let sim = run.app.sim();
    assert!(sim.compose_open);
    assert!(sim.presses.is_empty(), "no Escape: {:?}", sim.presses);
    assert!(
        sim.clicks.contains(&"Allow Selection".to_owned())
            && !sim.clicks.contains(&"Allow all".to_owned()),
        "{:?}",
        sim.clicks
    );
    let actions = &run.result.steps[0].actions;
    assert_eq!(
        actions
            .iter()
            .map(|action| action.action.as_str())
            .collect::<Vec<_>>(),
        ["click", "click (uncover)", "click"],
    );
}

#[tokio::test]
async fn a_regression_is_undone_and_the_element_is_not_tried_again() {
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.max_actions = 6,
        |id, question, sim| match id {
            "move" => Some(pick(
                question,
                if sim.presses.contains(&"escape".to_owned()) {
                    "shortcut"
                } else {
                    "activate"
                },
                0.9,
            )),
            "target" => Some(pick(question, "Archive", 0.9)),
            "progress" => Some(level(if sim.compose_open {
                4
            } else if sim.clicks.is_empty() {
                3
            } else {
                0
            })),
            _ => None,
        },
    )
    .await;
    let sim = run.app.sim();
    assert_eq!(sim.clicks, ["Archive"]);
    assert!(sim.presses.contains(&"escape".to_owned()));
    assert!(run.result.steps[0].loops.contains(&FlowLoop::Undo));
    assert_eq!(run.result.stop, FlowStopReason::Completed);
}

#[tokio::test]
async fn actions_that_change_nothing_fail_the_step() {
    let run = run(
        App::quirky(Quirk::Frozen),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert!(run.result.steps[0].note.contains("changed nothing"));
}

#[tokio::test]
async fn a_stalled_step_whose_result_already_shows_is_done() {
    // Live, "press Enter to search" pressed Enter three times over results a
    // live search had already listed, failed, and a rescue found its work
    // done ~18 s later: 25 of a day's 189 rescues were such steps.
    let run = run_with(
        App::quirky(Quirk::Frozen),
        json!({"app": "Mail", "steps": ["press the search button"]}),
        |_| {},
        |id, question, _| match id {
            "done" => Some(noul(0.05)),
            "move" => Some(pick(question, "activate", 0.9)),
            "holds" if text_of(question, "condition").contains("already shows the result") => {
                Some(noul(0.95))
            }
            _ => None,
        },
    )
    .await;
    let step = &run.result.steps[0];
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    assert_eq!(step.outcome, StepOutcome::AlreadyDone, "{}", step.note);
    assert!(
        step.note.contains("already shows what this step was for"),
        "{}",
        step.note
    );
}

#[tokio::test]
async fn an_irreversible_control_is_refused_inside_an_ordinary_step() {
    let run = run_with(
        App::with(|sim| sim.compose_open = true),
        json!({"app": "Mail", "steps": ["get rid of this draft"]}),
        |request| request.max_actions = 4,
        |id, question, _| match id {
            "move" => Some(pick(question, "activate", 0.9)),
            "target" => Some(pick(question, "Send", 0.95)),
            _ => None,
        },
    )
    .await;
    assert!(!run.app.sim().sent);
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert!(
        run.result.steps[0].actions.is_empty(),
        "a refused destructive click must never be recorded as an action the step took"
    );
    assert_eq!(
        run.result.steps[0].turns, 1,
        "a refused destructive click must fail the step immediately, not after it has been \
         mistaken for a no-op action and stalled out"
    );
}

#[tokio::test]
async fn move_outcomes_cover_finished_stuck_wait_and_a_missing_shortcut() {
    // With no completion judge to overrule it, "finished" ends the step.
    let finished = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |request| request.disabled_loops = vec![FlowLoop::Completion],
        |id, question, _| (id == "move").then(|| pick(question, "finished", 0.9)),
    )
    .await;
    assert_eq!(finished.result.stop, FlowStopReason::Completed);

    // A judge that sees the step undone overrules it: the loop acts instead
    // of skipping a step that was never done.
    let overruled = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |request| request.max_actions = 3,
        |id, question, _| (id == "move").then(|| pick(question, "finished", 0.9)),
    )
    .await;
    assert_ne!(overruled.result.stop, FlowStopReason::Completed);
    assert!(!overruled.app.sim().clicks.is_empty(), "it acted instead");
    assert!(overruled.requests.iter().any(|request| {
        serde_json::to_string(&request.state)
            .unwrap()
            .contains("does not yet clearly show this step done")
    }));

    let stuck = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |_| {},
        |id, question, _| (id == "move").then(|| pick(question, "stuck", 0.9)),
    )
    .await;
    assert_eq!(stuck.result.stop, FlowStopReason::StepFailed);

    let waited = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |request| request.max_actions = 2,
        |id, question, _| match id {
            "move" => Some(pick(question, "wait", 0.9)),
            "shortcut" => Some(pick(question, "none", 0.9)),
            _ => None,
        },
    )
    .await;
    assert_eq!(waited.result.stop, FlowStopReason::ActionBudget);

    let no_shortcut = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |request| request.max_model_calls = 4,
        |id, question, _| (id == "shortcut").then(|| pick(question, "none", 0.9)),
    )
    .await;
    assert_eq!(no_shortcut.result.stop, FlowStopReason::ModelBudget);
    assert_eq!(
        no_shortcut.app.sim().presses,
        [] as [std::string::String; 0]
    );
}

#[tokio::test]
async fn a_control_the_flows_own_stop_before_names_is_refused_in_an_ordinary_step() {
    // "Archive" is not on the generic denylist, but this flow already plans
    // to stop in front of it later; an ordinary step must not press it first.
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": [
            "tidy up the inbox",
            {"stop_before": "archive the conversation"}
        ]}),
        |request| request.max_actions = 4,
        |id, question, _| match id {
            "move" => Some(pick(question, "activate", 0.9)),
            "target" => Some(pick(question, "Archive", 0.95)),
            _ => None,
        },
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert!(
        run.app.sim().clicks.is_empty(),
        "a control the flow's own stop_before names must never be clicked early"
    );
}

#[tokio::test]
async fn an_unrecognized_move_is_skipped_rather_than_clicked() {
    // A malformed or prompt-injected answer must never fall through to
    // `activate`'s default Click branch; only `activate`, `expand`, and
    // `scroll` may ground and act.
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |request| request.max_actions = 4,
        |id, _, _| {
            (id == "move").then(|| {
                Answer::Choice(ChoiceAnswer {
                    choice: "delete_everything".to_owned(),
                    probabilities: BTreeMap::from([("delete_everything".to_owned(), 0.9)]),
                    confidence: 0.9,
                })
            })
        },
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert!(
        run.app.sim().clicks.is_empty(),
        "an unrecognized move must never ground and click a control"
    );
}

#[tokio::test]
async fn return_is_refused_while_a_dialog_is_showing() {
    let run = run_with(
        App::with(|sim| sim.obstacle = true),
        json!({"app": "Mail", "steps": ["confirm the name"]}),
        |request| {
            request.disabled_loops = vec![FlowLoop::Obstacles];
            request.max_model_calls = 6;
        },
        |id, question, _| match id {
            "shortcut" => Some(pick(question, "confirm", 0.9)),
            _ => None,
        },
    )
    .await;
    assert!(!run.app.sim().presses.contains(&"return".to_owned()));
    let confirmed = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["confirm the name"]}),
        // 2: one for the implicit launch, one for the press itself.
        |request| request.max_actions = 2,
        |id, question, _| match id {
            "shortcut" => Some(pick(question, "confirm", 0.9)),
            _ => None,
        },
    )
    .await;
    assert_eq!(confirmed.app.sim().presses, ["return"]);
}

#[tokio::test]
async fn disabled_loops_are_not_asked_and_the_move_falls_back_to_pressing() {
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| {
            request.disabled_loops = vec![
                FlowLoop::Moves,
                FlowLoop::Progress,
                FlowLoop::Obstacles,
                FlowLoop::Consistency,
                FlowLoop::Corroboration,
                FlowLoop::Undo,
                FlowLoop::Memory,
                FlowLoop::Narrowing,
                FlowLoop::Slots,
            ];
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.app.sim().clicks, ["New Message"]);
    let asked = run
        .requests
        .iter()
        .flat_map(|request| request.questions.keys().cloned())
        .collect::<BTreeSet<_>>();
    assert!(!asked.contains("move") && !asked.contains("progress") && !asked.contains("blocked"));

    let blind = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| {
            request.disabled_loops = vec![
                FlowLoop::Moves,
                FlowLoop::Progress,
                FlowLoop::Obstacles,
                FlowLoop::Completion,
            ];
        },
        |_, _, _| None,
    )
    .await;
    assert_eq!(
        blind.result.stop,
        FlowStopReason::StepFailed,
        "without the completion judge a step cannot recognise its own success"
    );
    assert_eq!(blind.app.sim().clicks, ["New Message"]);
}

#[tokio::test]
async fn an_action_judged_unhelpful_is_undone_and_not_tried_again() {
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["start a new email message"]}),
        |request| request.max_actions = 6,
        |id, question, sim| match id {
            "move" => Some(pick(
                question,
                if sim.presses.contains(&"escape".to_owned()) {
                    "shortcut"
                } else {
                    "activate"
                },
                0.9,
            )),
            "target" => Some(pick(question, "Archive", 0.9)),
            "helped" => Some(noul(if sim.clicks.is_empty() { 0.9 } else { 0.05 })),
            _ => None,
        },
    )
    .await;
    let sim = run.app.sim();
    assert_eq!(sim.clicks, ["Archive"]);
    assert!(sim.presses.contains(&"escape".to_owned()));
    assert!(run.result.steps[0].loops.contains(&FlowLoop::Undo));
    assert_eq!(run.result.stop, FlowStopReason::Completed);
}

#[tokio::test]
async fn waits_that_change_nothing_are_not_a_stall_and_stop_being_offered() {
    let run = run_with(
        App::quirky(Quirk::Frozen),
        json!({"app": "Mail", "steps": ["open the search results"]}),
        |_| {},
        |id, question, _| (id == "move").then(|| pick(question, "wait", 0.9)),
    )
    .await;
    let step = &run.result.steps[0];
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert!(
        step.note.contains("not accomplished after"),
        "a settled page is not a stall: {}",
        step.note
    );
    let waits = step
        .actions
        .iter()
        .filter(|action| action.action == "wait")
        .count();
    assert_eq!(waits, 2, "no third wait on a settled page");
    let told = run.requests.iter().any(|request| {
        serde_json::to_string(&request.state)
            .unwrap()
            .contains("the page has finished loading and nothing changed")
    });
    assert!(told, "Jev is told the page has settled");
}

#[tokio::test]
async fn after_acting_a_finished_move_stands_unless_the_judge_leans_undone() {
    let run = run_with(
        App::default(),
        json!({"app": "Mail", "steps": ["tidy up"]}),
        |_| {},
        |id, question, sim| match id {
            "move" => Some(pick(
                question,
                if sim.clicks.is_empty() {
                    "activate"
                } else {
                    "finished"
                },
                0.9,
            )),
            "done" => Some(noul(if sim.clicks.is_empty() { 0.05 } else { 0.6 })),
            _ => None,
        },
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(
        run.app.sim().clicks.len(),
        1,
        "no click after the step was done"
    );
}

/// Three result cards, each with its own "Select".
fn three_cards(sim: &mut Sim) {
    sim.results = vec![
        ("Row A seat 03", "Available", "₹200"),
        ("Row A seat 04", "Available", "₹200"),
        ("Row A seat 05", "Available", "₹200"),
    ];
}

/// Picks the first option offering a "Select" its card does not already
/// show chosen, as a person choosing seats goes on to the next free one.
fn pick_unselected(question: &Question) -> Answer {
    let Question::Choice(choice) = question else {
        panic!("a choice question");
    };
    let key = choice
        .criteria
        .iter()
        .find(|(_, description)| {
            description.as_ref().is_some_and(|description| {
                let text = description.to_string();
                text.contains("Select") && !text.contains("selected")
            })
        })
        .map_or_else(|| "none".to_owned(), |(key, _)| key.clone());
    pick(question, &key, 0.9)
}

/// Answers that press a card's "Select" until `wanted` different cards
/// are pressed, and only then judge the step done.
fn select_until(wanted: usize) -> impl Fn(&str, &Question, &Sim) -> Option<Answer> {
    move |id, question, sim| match id {
        "done" => {
            let distinct = sim.picked.iter().collect::<BTreeSet<_>>().len();
            Some(noul(if distinct >= wanted { 0.95 } else { 0.05 }))
        }
        "blocked" => Some(noul(0.05)),
        "move" => Some(pick(question, "activate", 0.9)),
        _ if id == "target" || id == "region" || id.starts_with("group_") => {
            Some(pick_unselected(question))
        }
        _ => None,
    }
}

#[tokio::test]
async fn a_step_choosing_several_items_presses_each_ones_own_copy() {
    // Live, a seat table's "Select" was pressed for one seat, and the
    // second seat's "Select" was struck off as another item's copy.
    let run = run_with(
        App::with(three_cards),
        json!({"app": "Mail", "steps": ["choose 2 adjacent available seats"]}),
        |_| {},
        select_until(2),
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{}",
        run.result.steps[0].note
    );
    assert_eq!(run.app.sim().picked, ["@s:select-1", "@s:select-2"]);
}

#[tokio::test]
async fn a_step_adding_one_item_leaves_the_other_items_copies_alone() {
    // A count of one item ("2 packets of milk") is raised on that item:
    // another card's copy of its button adds a different product.
    let run = run_with(
        App::with(three_cards),
        json!({"app": "Mail", "steps": ["add 2 packets of the milk"]}),
        |_| {},
        select_until(2),
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::StepFailed);
    assert_eq!(run.app.sim().picked, ["@s:select-1"]);
}

#[test]
fn several_items_are_asked_for_by_a_choosing_verb_and_a_counted_plural() {
    for several in [
        "choose 2 adjacent available seats in the cheapest section",
        "select three files",
        "pick two seats together",
        "please choose 4 tickets",
    ] {
        assert!(act::asks_for_several(several), "{several}");
    }
    for one in [
        "add 2 packets of Amul Taaza milk to the cart",
        "choose Wednesday 7 October 2026 in the date selector",
        "choose 2D",
        "select the 1 kg pack",
        "choose 2 in the quantity box",
        "open the first movie",
    ] {
        assert!(!act::asks_for_several(one), "{one}");
    }
}

#[tokio::test]
async fn a_step_finding_nothing_to_press_answers_the_dialog_the_task_opened() {
    // Live, a date step found nothing to press for four turns while the
    // format dialog a booking button had opened offered "2D", and a rescue
    // was spent pressing it.
    let run = run_with(
        App::with(|sim| sim.obstacle = true),
        json!({"app": "browser", "steps": ["choose Wednesday 7 October 2026 in the date picker"]}),
        |request| request.dialog_left_open = true,
        |id, question, sim| {
            let answering = serde_json::to_string(question)
                .unwrap()
                .contains("click to answer the dialog");
            match id {
                "done" => Some(noul(if sim.obstacle { 0.05 } else { 0.95 })),
                "blocked" => Some(noul(0.05)),
                "move" => Some(pick(question, "activate", 0.9)),
                _ if id == "target" || id == "region" || id.starts_with("group_") => Some(pick(
                    question,
                    if answering {
                        "Keep Editing"
                    } else {
                        "no such control"
                    },
                    0.9,
                )),
                _ => None,
            }
        },
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{}",
        run.result.steps[0].note
    );
    assert_eq!(run.app.sim().clicks, ["Keep Editing"]);
}

#[tokio::test]
async fn a_control_the_dialogs_own_bar_covers_is_pressed_and_one_behind_it_is_not() {
    // Live, a seat table's lower rows sat under its "Pay" bar and were never
    // offered; a press scrolls such a control out from under the bar. A
    // browser run takes a dialog at its first look as the task's own when
    // the run before it left that dialog open.
    let pressing = |wanted: &'static str| {
        move |id: &str, question: &Question, sim: &Sim| match id {
            "done" => Some(noul(if sim.obstacle { 0.05 } else { 0.95 })),
            "blocked" => Some(noul(0.05)),
            "move" => Some(pick(question, "activate", 0.9)),
            _ if id == "target" || id == "region" || id.starts_with("group_") => {
                Some(pick(question, wanted, 0.9))
            }
            _ => None,
        }
    };
    let in_the_dialog = run_with(
        App::with(|sim| {
            sim.obstacle = true;
            sim.quirks.insert(Quirk::BarOverSheet);
        }),
        json!({"app": "browser", "steps": ["keep editing the draft"]}),
        |request| request.dialog_left_open = true,
        pressing("Keep Editing"),
    )
    .await;
    let asked = in_the_dialog
        .requests
        .iter()
        .flat_map(|request| request.questions.keys().cloned())
        .collect::<Vec<_>>();
    let (clicks, presses) = {
        let sim = in_the_dialog.app.sim();
        (sim.clicks.clone(), sim.presses.clone())
    };
    assert_eq!(
        clicks,
        ["Keep Editing"],
        "{} {asked:?} {presses:?}",
        in_the_dialog.result.steps[0].note
    );

    // What the dialog itself covers on the page behind it stays out.
    let behind = run_with(
        App::with(|sim| {
            sim.obstacle = true;
            sim.quirks.insert(Quirk::Covered);
        }),
        json!({"app": "browser", "steps": ["start a new email message"]}),
        |request| request.dialog_left_open = true,
        pressing("New Message"),
    )
    .await;
    assert!(
        !behind.app.sim().clicks.contains(&"New Message".to_owned()),
        "{:?}",
        behind.app.sim().clicks
    );
}

#[tokio::test]
async fn return_in_a_search_box_runs_the_search_with_a_sheet_in_front() {
    // Live, a store's search opened as a full-window sheet, and the step to
    // press Enter in the box just typed into was refused 154 times, as if
    // Return would press the sheet's default button.
    let pressing_return = |id: &str, question: &Question, sim: &Sim| match id {
        "done" => Some(noul(if sim.presses.iter().any(|key| key == "return") {
            0.95
        } else {
            0.05
        })),
        "move" => Some(pick(question, "shortcut", 0.9)),
        "shortcut" => Some(pick(question, "confirm", 0.95)),
        _ => None,
    };
    let run = run_with(
        App::with(|sim| {
            sim.quirks.insert(Quirk::SearchBehindLink);
            sim.quirks.insert(Quirk::SearchOpen);
            sim.quirks.insert(Quirk::SearchSheet);
        }),
        json!({"app": "Mail", "steps": [
            {"enter": {"search": "invoices"}},
            "press Enter in the search box"
        ]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        pressing_return,
    )
    .await;
    let presses = run.app.sim().presses.clone();
    assert!(presses.iter().any(|key| key == "return"), "{presses:?}");
    assert_eq!(run.result.stop, FlowStopReason::Completed);

    // Without a search box typed into, Return in front of a sheet stays
    // refused: it would press the sheet's default button.
    let run = run_with(
        App::with(|sim| {
            sim.quirks.insert(Quirk::SearchBehindLink);
            sim.quirks.insert(Quirk::SearchOpen);
            sim.quirks.insert(Quirk::SearchSheet);
        }),
        json!({"app": "Mail", "steps": ["press Enter"]}),
        |request| {
            request.disabled_loops.push(FlowLoop::Attention);
            request.max_actions = 3;
        },
        pressing_return,
    )
    .await;
    let presses = run.app.sim().presses.clone();
    assert!(!presses.iter().any(|key| key == "return"), "{presses:?}");
}

/// A page at `surface`, its `covered` controls drawn under something.
fn page_at(surface: &str, covered: usize) -> Screen {
    Screen {
        app: "browser".to_owned(),
        window: Some("Flights".to_owned()),
        surface: surface.to_owned(),
        candidates: (0..covered + 2)
            .map(|index| {
                let mut control = node(
                    &format!("Control {index}"),
                    "button",
                    &["Click"],
                    &["main"],
                    f64::from(u32::try_from(index).unwrap()),
                );
                if index < covered {
                    control.states = vec!["covered".to_owned()];
                }
                control
            })
            .collect(),
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    }
}

#[test]
fn a_dialog_the_task_worked_in_is_in_the_way_of_the_next_step() {
    // A date field's calendar, left open after its day was chosen, stayed
    // "the task's own" for every later step, so it was never cleared out of
    // the way of the class button under it.
    use crate::agentic::flow::front::Front;
    let at = Some("https://flights.test/");
    let (window, sheet) = (page_at("window", 0), page_at("sheet", 3));
    let press = node("Control 4", "button", &["Click"], &["main"], 4.0);
    let mut front = Front::default();
    front.act("browse https://flights.test/", None);
    assert!(front.look(&window, at).is_none());
    front.act("click", Some(&press));
    assert!(front.look(&sheet, at).is_some(), "the press opened it");
    front.next_step();
    assert!(front.opened_dialog(), "the next step answers what it asks");
    front.act("click", Some(&press));
    front.look(&sheet, at);
    assert!(
        front.opened_dialog(),
        "still its own within the step that works in it"
    );
    front.next_step();
    assert!(!front.opened_dialog(), "a step later, it is in the way");
    front.look(&sheet, at);
    assert!(
        !front.opened_dialog(),
        "and it does not become the task's again"
    );

    // A scroll or the run's own housekeeping opens no dialog of the task's.
    for action in ["scroll", "click (clear distraction)", "click (dismiss)"] {
        let mut front = Front::default();
        front.act("browse https://flights.test/", None);
        front.look(&window, at);
        front.act(action, Some(&press));
        assert!(front.look(&sheet, at).is_none(), "{action}");
        assert!(!front.opened_dialog(), "{action}");
    }

    // Opening an address leaves what was in front behind.
    let mut front = Front::default();
    front.act("browse https://flights.test/", None);
    front.look(&window, at);
    front.act("click", Some(&press));
    front.look(&sheet, at);
    front.act("browse https://flights.test/next", None);
    assert!(!front.opened_dialog());
}

#[test]
fn a_dialog_at_a_runs_first_look_is_the_tasks_only_when_the_run_before_left_it() {
    use crate::agentic::flow::front::Front;
    let at = Some("https://flights.test/");
    let sheet = page_at("sheet", 3);
    // A pop-up the page opened itself, at a rescue's first look.
    let mut front = Front::new(false);
    assert!(front.look(&sheet, at).is_none());
    assert!(
        !front.opened_dialog(),
        "the page's own, cleared like any other"
    );
    // The dialog the task's run before left open.
    let mut front = Front::new(true);
    assert!(front.look(&sheet, at).is_some());
    assert!(front.opened_dialog(), "the task's current stage");
}

/// A sheet of ten grid cells named by bare numbers, under `month` when
/// given.
fn days_at(month: Option<&str>) -> Screen {
    let mut screen = page_at("sheet", 3);
    screen.candidates.extend((1..=10).map(|day: u32| {
        node(
            &day.to_string(),
            "gridcell",
            &["Click"],
            &["main"],
            f64::from(100 + day),
        )
    }));
    screen.context.extend(month.map(str::to_owned));
    screen
}

#[test]
fn turning_a_calendars_month_answers_nothing_it_asks() {
    // An arrow pressed is no day chosen: the calendar still asks for one,
    // and nothing it covers may be pressed through it yet.
    use crate::agentic::flow::front::Front;
    let at = Some("https://flights.test/");
    let calendar = days_at(Some("October 2026"));
    let mut front = Front::default();
    front.act("browse https://flights.test/", None);
    front.look(&page_at("window", 0), at);
    front.act(
        "click",
        Some(&node("Departure", "button", &["Click"], &["main"], 0.0)),
    );
    assert!(front.look(&calendar, at).is_some(), "the press opened it");
    front.act(
        "click",
        Some(&node("Next month", "button", &["Click"], &["main"], 90.0)),
    );
    front.look(&calendar, at);
    assert!(!front.served_calendar(), "a month turned is no day chosen");
    assert!(front.opened_dialog());
    front.act(
        "click",
        Some(&node("5", "gridcell", &["Click"], &["main"], 105.0)),
    );
    front.look(&calendar, at);
    assert!(front.served_calendar(), "a day chosen serves its field");
}

#[test]
fn a_grid_of_bare_numbers_is_a_calendar_only_beside_a_month() {
    // A seat map's cells are numbers in a grid too: pressed in, it is no
    // calendar that has served its field and may be closed for a press
    // behind it.
    use crate::agentic::flow::front::Front;
    let at = Some("https://cinema.test/");
    for (month, calendar) in [(None, false), (Some("October 2026"), true)] {
        let grid = days_at(month);
        let mut front = Front::default();
        front.act("browse https://cinema.test/", None);
        front.look(&page_at("window", 0), at);
        front.act(
            "click",
            Some(&node("Select seats", "button", &["Click"], &["main"], 0.0)),
        );
        front.look(&grid, at);
        front.act(
            "click",
            Some(&node("5", "gridcell", &["Click"], &["main"], 105.0)),
        );
        front.look(&grid, at);
        assert_eq!(front.served_calendar(), calendar, "{month:?}");
    }
}

#[test]
fn a_calendar_reads_its_days_from_labels_that_name_their_month() {
    // Day cells named whole dates, the day not first ("Thu Oct 01 2026",
    // "Choose Thursday, October 22nd, 2026"), are a calendar's days too.
    use crate::agentic::flow::front::Front;
    let at = Some("https://stays.test/");
    let mut calendar = page_at("sheet", 3);
    calendar.candidates.extend((1..=7).map(|day: u32| {
        node(
            &format!("Choose Thursday, October {day}th, 2026"),
            "button",
            &["Click"],
            &["main"],
            f64::from(100 + day),
        )
    }));
    let mut front = Front::default();
    front.act("browse https://stays.test/", None);
    front.look(&page_at("window", 0), at);
    front.act(
        "click",
        Some(&node("Check-in", "button", &["Click"], &["main"], 0.0)),
    );
    front.look(&calendar, at);
    // An arrow named "Next" and described "next month" turns the month.
    let next = Candidate {
        description: Some("next month".to_owned()),
        ..node("Next", "button", &["Click"], &["main"], 90.0)
    };
    front.act("click", Some(&next));
    front.look(&calendar, at);
    assert!(!front.served_calendar(), "a month turned is no day chosen");
    front.act(
        "click",
        Some(&node(
            "Choose Thursday, October 2th, 2026",
            "button",
            &["Click"],
            &["main"],
            102.0,
        )),
    );
    front.look(&calendar, at);
    assert!(front.served_calendar(), "its days are read as a calendar's");
}

#[test]
fn a_search_box_typed_into_counts_only_while_it_shows_uncovered() {
    use crate::agentic::flow::act::still_shows;
    let field = node("Search Lenskart", "textbox", &["SetValue"], &["main"], 0.0);
    let mut screen = page_at("sheet", 0);
    screen.candidates.push(field.clone());
    assert!(still_shows(&screen, &field));
    screen.candidates.last_mut().unwrap().states = vec!["covered".to_owned()];
    assert!(
        !still_shows(&screen, &field),
        "a dialog over it takes the keys"
    );
    screen.candidates.pop();
    assert!(!still_shows(&screen, &field), "nor once it is gone");
}

#[test]
fn a_run_that_never_looked_hands_on_the_dialog_it_was_left() {
    use crate::agentic::flow::front::Front;
    assert!(Front::new(true).left_open(), "nothing changed in front");
    assert!(!Front::new(false).left_open());
    let mut front = Front::new(true);
    front.look(&page_at("window", 0), Some("https://flights.test/"));
    assert!(!front.left_open(), "the window is in front once it looked");
}

#[tokio::test]
async fn answering_the_task_dialog_never_presses_what_commits() {
    // The fallback presses what serves no step's words, so it never
    // presses what a person would approve: "Confirm 2 tickets? Yes".
    let run = run_with(
        App::with(|sim| {
            sim.obstacle = true;
            sim.quirks.insert(Quirk::YesOnSheet);
        }),
        json!({"app": "browser", "steps": ["choose seat A5"]}),
        |_| {},
        |id, question, sim| {
            let answering = serde_json::to_string(question)
                .unwrap()
                .contains("click to answer the dialog");
            match id {
                "done" => Some(noul(if sim.obstacle { 0.05 } else { 0.95 })),
                "blocked" => Some(noul(0.05)),
                "move" => Some(pick(question, "activate", 0.9)),
                _ if id == "target" || id == "region" || id.starts_with("group_") => Some(pick(
                    question,
                    if answering { "Yes" } else { "no such control" },
                    0.9,
                )),
                _ => None,
            }
        },
    )
    .await;
    assert!(
        !run.app.sim().clicks.contains(&"Yes".to_owned()),
        "{:?}",
        run.result.steps
    );
}

#[test]
fn a_pressed_controls_copies_are_only_on_the_other_items_of_its_list() {
    // Live, the main "Add to cart" was refused and the sticky bar's own
    // "Add to cart" was struck off with the copies.
    let select = |list: &str, card: &str| {
        let mut path = vec!["main".to_owned(), list.to_owned()];
        if !card.is_empty() {
            path.push(card.to_owned());
        }
        Candidate {
            ref_id: format!("@s:{list}-{card}"),
            path,
            ..node("Add to cart", "button", &["Click"], &[], 10.0)
        }
    };
    let screen = Screen {
        app: "browser".to_owned(),
        window: None,
        surface: "window".to_owned(),
        candidates: vec![
            select("list \"Results\"", "listitem #1"),
            select("list \"Results\"", "listitem #2"),
            select("list \"Results\"", "listitem #3"),
            select("region \"Sticky\"", ""),
        ],
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
    };
    let copies = act::copies_of(&screen, &screen.candidates[0]);
    assert_eq!(copies.len(), 2, "{copies:?}");
    assert!(
        act::copies_of(&screen, &screen.candidates[3]).is_empty(),
        "the bar's own has none"
    );
}
