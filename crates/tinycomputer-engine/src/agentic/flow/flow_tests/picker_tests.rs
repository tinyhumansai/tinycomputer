//! Pop-ups and calendars a step works with: a closer that goes with its
//! pop-up (a "Close" or a consent bar's "Accept all"), a calendar the task picked in closed for a press behind it, in
//! its own step or a later one, a date picked from a calendar already
//! open, and a calendar paged no further than the heading of its month.

use super::*;

/// Answers that press the toast's "Close" and never judge the step done.
fn press_close(id: &str, question: &Question, _: &Sim) -> Option<Answer> {
    match id {
        "done" | "blocked" => Some(noul(0.05)),
        "move" => Some(pick(question, "activate", 0.9)),
        _ if id == "target" || id == "region" || id.starts_with("group_") => {
            Some(pick(question, "Close", 0.9))
        }
        _ => None,
    }
}

#[tokio::test]
async fn a_closer_that_goes_with_its_pop_up_ends_a_step_closing_it() {
    // Live, an offer pop-up drawn without a dialog's role closed at the
    // first press, and the step stalled: the judge could not tell whether
    // "declining optional cookies" was done with no cookie banner shown.
    let toast = || {
        App::with(|sim| {
            sim.quirks.insert(Quirk::PromoToast);
        })
    };
    let run = run_with(
        toast(),
        json!({"app": "Mail", "steps": ["close any login or offer pop-up, declining optional cookies"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        press_close,
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.app.sim().clicks, ["Close"]);
    assert!(
        run.result.steps[0]
            .note
            .contains("closed with what it was on"),
        "{}",
        run.result.steps[0].note
    );
    let unrelated = run_with(
        toast(),
        json!({"app": "Mail", "steps": ["archive the message"]}),
        |request| {
            request.disabled_loops.push(FlowLoop::Attention);
            request.max_actions = 1;
        },
        press_close,
    )
    .await;
    assert_ne!(
        unrelated.result.stop,
        FlowStopReason::Completed,
        "a step that closes nothing is not finished by a closer"
    );
}

/// Answers that press the cookie bar's "Accept all" and never judge the
/// step done.
fn press_accept(id: &str, question: &Question, _: &Sim) -> Option<Answer> {
    match id {
        "done" | "blocked" => Some(noul(0.05)),
        "move" => Some(pick(question, "activate", 0.9)),
        _ if id == "target" || id == "region" || id.starts_with("group_") => {
            Some(pick(question, "Accept all", 0.9))
        }
        _ => None,
    }
}

#[tokio::test]
async fn accepting_a_cookie_bar_ends_the_step_that_accepts_it() {
    // A consent bar drawn without a dialog's role closes on "Accept all"
    // as surely as a pop-up on "Close": the press going with the bar is
    // the evidence the judge cannot see.
    let run = run_with(
        App::with(|sim| {
            sim.quirks.insert(Quirk::CookieBar);
        }),
        json!({"app": "Mail", "steps": ["accept the cookie banner"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        press_accept,
    )
    .await;
    assert_eq!(run.result.stop, FlowStopReason::Completed);
    assert_eq!(run.app.sim().clicks, ["Accept all"]);
    assert!(
        run.result.steps[0]
            .note
            .contains("closed with what it was on"),
        "{}",
        run.result.steps[0].note
    );
}

/// Answers that open the calendar, pick a day, then press "Find flights",
/// judging the step done once that press went through.
fn pick_then_find(id: &str, question: &Question, sim: &Sim) -> Option<Answer> {
    let found = sim.clicks.iter().any(|click| click == "Find flights");
    let next = if sim.fields.contains_key("Departure") {
        "Find flights"
    } else if sim
        .booking
        .as_ref()
        .is_some_and(|booking| booking.calendar.is_some())
    {
        "12 September 2026"
    } else {
        "Departure"
    };
    match id {
        "done" => Some(noul(if found { 0.95 } else { 0.05 })),
        "blocked" => Some(noul(0.05)),
        "move" => Some(pick(question, "activate", 0.9)),
        _ if id == "target" || id == "region" || id.starts_with("group_") => {
            Some(pick(question, next, 0.9))
        }
        _ => None,
    }
}

#[tokio::test]
async fn a_calendar_the_task_picked_in_is_closed_for_a_press_behind_it() {
    // Live, a calendar the task opened stayed in front of the guests and
    // Search buttons once both dates were picked, and every press behind
    // it was refused as lying behind the task's own dialog.
    let run = run_with(
        App::with(|sim| {
            sim.booking = Some(Booking::default());
            sim.quirks.insert(Quirk::CalendarStaysOpen);
        }),
        json!({"app": "Mail", "steps": ["pick 12 September 2026 as the departure date, then press Find flights"]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        pick_then_find,
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    let sim = run.app.sim();
    assert_eq!(sim.presses, ["escape"]);
    assert_eq!(
        sim.clicks,
        ["Departure", "12 September 2026", "Find flights"]
    );
    assert!(
        run.result.steps[0]
            .actions
            .iter()
            .any(|action| action.action == "press escape (uncover)")
    );
}

#[tokio::test]
async fn a_calendar_a_step_before_picked_in_is_closed_for_a_press_behind_it() {
    // Live, the dates were picked a step each, and the calendar left open
    // in front of the Search button was handed back to the page with the
    // next step: every press behind it was refused, and no step closed it.
    let run = run_with(
        App::with(|sim| {
            sim.booking = Some(Booking::default());
            sim.quirks.insert(Quirk::CalendarStaysOpen);
        }),
        json!({"app": "Mail", "steps": [
            "pick 12 September 2026 as the departure date",
            "press Find flights"
        ]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        |id, question, sim| {
            if id != "done" {
                return pick_then_find(id, question, sim);
            }
            let finished = if text_of(question, "step").contains("find flights") {
                sim.clicks.iter().any(|click| click == "Find flights")
            } else {
                sim.fields.contains_key("Departure")
            };
            Some(noul(if finished { 0.95 } else { 0.05 }))
        },
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    let sim = run.app.sim();
    assert_eq!(sim.presses, ["escape"]);
    assert_eq!(
        sim.clicks,
        ["Departure", "12 September 2026", "Find flights"]
    );
}

#[tokio::test]
async fn a_date_whose_calendar_is_open_is_picked_without_pressing_its_button() {
    // Live, a check-out date's step found the calendar open from the
    // check-in, pressed its button, which closed it, four times, and never
    // picked the day.
    let run = run_with(
        App::with(|sim| {
            sim.booking = Some(Booking {
                calendar: Some(8),
                ..Booking::default()
            });
            sim.quirks.insert(Quirk::CalendarStaysOpen);
        }),
        json!({"app": "Mail", "steps": [{"enter": {"departure date": "12 September 2026"}}]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        |_, _, _| None,
    )
    .await;
    assert_eq!(
        run.result.stop,
        FlowStopReason::Completed,
        "{:?}",
        run.result.steps
    );
    let sim = run.app.sim();
    assert_eq!(sim.clicks, ["12 September 2026"]);
    assert_eq!(sim.fields["Departure"], "12 September 2026");
}

#[test]
fn a_calendars_heading_names_the_month_it_shows() {
    use super::steps::heads_month_of;
    let date = "23 October 2026";
    assert!(heads_month_of("October 2026", date));
    assert!(heads_month_of("Oct 2026", date));
    // One heading over two months, as a two-month picker draws it.
    assert!(heads_month_of(
        "September 2026 Mo Tu We Th Fr Sa Su October 2026 Mo Tu We Th Fr Sa Su",
        date
    ));
    assert!(!heads_month_of("November 2026", date));
    assert!(
        !heads_month_of("October 2027", date),
        "another year's October"
    );
    assert!(
        heads_month_of("October 2027", "23 October"),
        "a date with no year is in the first October shown"
    );
    assert!(!heads_month_of("October", date), "a month with no year");
    assert!(
        !heads_month_of("Check-in 18 Oct 2026 Sunday", date),
        "a field's date is no heading"
    );
    assert!(
        !heads_month_of("Next month, October 2026", date),
        "an arrow names the month it turns to"
    );
    assert!(
        !heads_month_of("Lowest fares in October 2026", date),
        "a sentence is no heading"
    );
    assert!(
        heads_month_of("« October 2026 »", date),
        "arrows' glyphs aside"
    );
}

#[test]
fn only_a_heading_beside_the_calendars_arrow_names_its_month() {
    use super::steps::heads_its_month;
    let placed = |name: &str, role: &str, order: usize| Candidate {
        order,
        ..node(name, role, &["Click"], &["main"], 0.0)
    };
    let next = placed("Next Month", "button", 40);
    // A week of days that show a number and a fare, no month.
    let bare_days = (1..=7_u8)
        .map(|day| {
            placed(
                &format!("{day} 6,0{day}5"),
                "gridcell",
                50 + usize::from(day),
            )
        })
        .collect::<Vec<_>>();
    let screen = |candidates: Vec<Candidate>, text_nodes: Vec<Candidate>| Screen {
        app: "browser".to_owned(),
        window: None,
        surface: "window".to_owned(),
        candidates: candidates
            .into_iter()
            .chain([next.clone()])
            .chain(bare_days.clone())
            .collect(),
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes,
    };
    let date = "23 October 2026";
    assert!(heads_its_month(
        &screen(Vec::new(), vec![placed("October 2026", "text", 41)]),
        &next,
        date
    ));
    // A caption drawn as a button before the arrows.
    assert!(heads_its_month(
        &screen(
            vec![placed("October 2026 Mo Tu We Th Fr Sa Su", "button", 39)],
            Vec::new()
        ),
        &next,
        date
    ));
    // Months to fly in, listed elsewhere on the page, say nothing of the
    // month the calendar shows, nor does a month menu's choice beside it.
    assert!(!heads_its_month(
        &screen(vec![placed("October 2026", "button", 3)], Vec::new()),
        &next,
        date
    ));
    assert!(!heads_its_month(
        &screen(vec![placed("October 2026", "option", 41)], Vec::new()),
        &next,
        date
    ));
    // A results list's page numbers, far from the arrow, are no calendar's
    // days.
    let paged = Screen {
        candidates: (1..=7_u8)
            .map(|day| placed(&day.to_string(), "button", 400 + usize::from(day)))
            .chain([next.clone()])
            .collect(),
        text_nodes: vec![placed("October 2026", "text", 41)],
        ..screen(Vec::new(), Vec::new())
    };
    assert!(!heads_its_month(&paged, &next, date));
    // A calendar whose days name their month is paged by them alone.
    let dated = Screen {
        candidates: (1..=7_u8)
            .map(|day| placed(&format!("{day} September 2026"), "gridcell", 50))
            .chain([next.clone()])
            .collect(),
        text_nodes: vec![placed("October 2026", "text", 41)],
        ..screen(Vec::new(), Vec::new())
    };
    assert!(!heads_its_month(&dated, &next, date));
}

#[tokio::test]
async fn a_calendar_whose_days_name_no_month_is_paged_no_further_than_its_heading() {
    // Live, a hotel site's open days showed a number and a fare: no day
    // read as the date, and its calendar was paged a year past October.
    let run = run_with(
        App::with(|sim| {
            sim.booking = Some(Booking {
                calendar: Some(8),
                ..Booking::default()
            });
            sim.quirks.insert(Quirk::BareCalendarDays);
        }),
        json!({"app": "Mail", "steps": [{"enter": {"departure date": "18 October 2026"}}]}),
        |request| request.disabled_loops.push(FlowLoop::Attention),
        |id, question, sim| match id {
            "move" => Some(pick(question, "activate", 0.9)),
            "done" => Some(noul(
                if sim
                    .booking
                    .as_ref()
                    .is_some_and(|booking| booking.calendar.is_some())
                {
                    0.9
                } else {
                    0.05
                },
            )),
            _ => None,
        },
    )
    .await;
    let sim = run.app.sim();
    // Replayed: the departure box opens the calendar on September, and
    // each "Next Month" turns it a month on.
    let furthest = sim
        .clicks
        .iter()
        .fold((8, 8), |(month, furthest), click| {
            let month = match click.as_str() {
                "Departure" => 8,
                "Next Month" => month + 1,
                _ => month,
            };
            (month, furthest.max(month))
        })
        .1;
    assert_eq!(
        furthest, 9,
        "paged to October and never past it: {:?}",
        sim.clicks
    );
    assert_eq!(
        sim.booking.as_ref().and_then(|booking| booking.calendar),
        Some(9),
        "the calendar shows October"
    );
    // A day that names no month is never pressed on its number alone: the
    // step ends there, its date unpicked, rather than guessing at a day.
    assert_eq!(
        run.result.stop,
        FlowStopReason::StepFailed,
        "{:?}",
        run.result.steps
    );
    assert!(!sim.fields.contains_key("Departure"), "{:?}", sim.fields);
}
