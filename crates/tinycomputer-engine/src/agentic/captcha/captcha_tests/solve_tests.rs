//! The solver's loop against a scripted page and a scripted Jev.

use super::*;

fn checkbox() -> CaptchaWidget {
    widget(CaptchaRole::Checkbox, 20.0, 100.0, 304.0, 78.0)
}

fn challenge() -> CaptchaWidget {
    widget(CaptchaRole::Challenge, 50.0, 20.0, 300.0, 270.0)
}

/// Jev reading a 3 by 3 grid at the middle of the challenge, its verify
/// button bottom right, and yes to tiles 1 and 5.
fn grid_jev(
    layout: &'static str,
) -> impl Fn(&EvaluationRequest) -> Option<BTreeMap<String, Answer>> + Send + Sync + 'static {
    move |request| {
        if request.questions.contains_key("layout") {
            return Some(BTreeMap::from([
                ("layout".to_owned(), chose(layout)),
                ("grid".to_owned(), found(0.1, 0.2, 0.9, 0.8, 0.9)),
                ("verify".to_owned(), found(0.7, 0.9, 0.9, 1.0, 0.9)),
            ]));
        }
        Some(
            request
                .questions
                .keys()
                .map(|id| {
                    let sure = matches!(id.as_str(), "tile_1" | "tile_5");
                    (id.clone(), yes(if sure { 0.9 } else { 0.1 }))
                })
                .collect(),
        )
    }
}

#[tokio::test]
async fn a_checkbox_is_pressed_where_jev_finds_it_and_a_token_means_solved() {
    let page = ScriptedPage::new(vec![
        Some(scan(vec![checkbox()], false)),
        Some(scan(vec![checkbox()], true)),
    ]);
    let (jev, asked) = jev(|_| {
        Some(BTreeMap::from([(
            "checkbox".to_owned(),
            found(0.05, 0.3, 0.15, 0.7, 0.8),
        )]))
    });
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert_eq!(outcome, CaptchaOutcome::Solved { attempts: 0 });
    let presses = page.presses();
    assert_eq!(presses.len(), 1);
    assert!(near(presses[0], (50.4, 139.0)), "{presses:?}");
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].assets.len(), 1, "the widget is shown to Jev");
    assert!(asked[0].needs_image_opt_in());
}

#[tokio::test]
async fn an_unsure_jev_leaves_the_checkbox_at_its_usual_place() {
    let page = ScriptedPage::new(vec![
        Some(scan(vec![checkbox()], false)),
        Some(scan(vec![], false)),
    ]);
    let (jev, _) = jev(|_| {
        Some(BTreeMap::from([(
            "checkbox".to_owned(),
            found(0.6, 0.6, 0.9, 0.9, 0.1),
        )]))
    });
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert_eq!(
        outcome,
        CaptchaOutcome::Solved { attempts: 0 },
        "the widget went away once pressed"
    );
    assert!(near(page.presses()[0], (48.0, 139.0)));
}

#[tokio::test]
async fn a_grid_challenge_presses_the_tiles_jev_is_sure_of_then_verify() {
    let page = ScriptedPage::new(vec![
        Some(scan(vec![checkbox(), challenge()], false)),
        Some(scan(vec![checkbox()], true)),
    ]);
    let (jev, asked) = jev(grid_jev("grid_3x3"));
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert_eq!(outcome, CaptchaOutcome::Solved { attempts: 1 });
    let presses = page.presses();
    assert_eq!(presses.len(), 3, "{presses:?}");
    assert!(near(presses[0], (120.0, 101.0)), "tile 1: {presses:?}");
    assert!(near(presses[1], (200.0, 155.0)), "tile 5: {presses:?}");
    assert!(near(presses[2], (290.0, 276.5)), "verify: {presses:?}");
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1].questions.len(), 9);
    assert_eq!(
        asked[1].assets.len(),
        10,
        "the challenge and its nine tiles"
    );
    asked[1].validate().unwrap();
}

#[tokio::test]
async fn a_four_by_four_grid_is_cut_into_sixteen_tiles() {
    let page = ScriptedPage::new(vec![
        Some(scan(vec![challenge()], false)),
        Some(scan(vec![], true)),
    ]);
    let (jev, asked) = jev(grid_jev("grid_4x4"));
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert_eq!(outcome, CaptchaOutcome::Solved { attempts: 1 });
    assert_eq!(asked.lock().unwrap()[1].questions.len(), 16);
}

#[tokio::test]
async fn a_typed_challenge_is_unsupported_and_nothing_is_pressed() {
    let page = ScriptedPage::new(vec![Some(scan(vec![challenge()], false))]);
    let (jev, _) = jev(grid_jev("typed"));
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsupported { reason } if reason.contains("typed")),
        "{outcome:?}"
    );
    assert!(page.presses().is_empty());
}

#[tokio::test]
async fn a_challenge_that_keeps_coming_back_is_unsolved_after_the_attempts() {
    let page = ScriptedPage::new(vec![Some(scan(vec![challenge()], false))]);
    let (jev, asked) = jev(grid_jev("grid_3x3"));
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::with_attempts(2)).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsolved { attempts: 2, reason } if reason.contains("2 tries")),
        "{outcome:?}"
    );
    assert_eq!(asked.lock().unwrap().len(), 4, "two questions a round");
}

#[tokio::test]
async fn a_jev_that_cannot_answer_leaves_the_captcha_unsolved() {
    let page = ScriptedPage::new(vec![Some(scan(vec![challenge()], false))]);
    let (jev, _) = jev(|_| None);
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::with_attempts(1)).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsolved { reason, .. } if reason.contains("could not read")),
        "{outcome:?}"
    );
    assert!(page.presses().is_empty());
}

#[tokio::test]
async fn a_jev_that_cannot_find_the_tiles_presses_nothing() {
    let page = ScriptedPage::new(vec![Some(scan(vec![challenge()], false))]);
    let (jev, _) = jev(|_| {
        Some(BTreeMap::from([
            ("layout".to_owned(), chose("grid_3x3")),
            ("grid".to_owned(), found(0.1, 0.2, 0.9, 0.8, 0.05)),
            ("verify".to_owned(), found(0.7, 0.9, 0.9, 1.0, 0.05)),
        ]))
    });
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::with_attempts(1)).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsolved { reason, .. } if reason.contains("tiles")),
        "{outcome:?}"
    );
    assert!(page.presses().is_empty());
}

#[tokio::test]
async fn a_checkbox_that_will_not_pass_is_pressed_twice_then_left() {
    let page = ScriptedPage::new(vec![Some(scan(vec![checkbox()], false))]);
    let (jev, _) = jev(|_| None);
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsolved { attempts: 0, reason } if reason.contains("checkbox")),
        "{outcome:?}"
    );
    assert_eq!(page.presses().len(), 2);
}

#[tokio::test]
async fn a_refused_press_ends_the_try() {
    let mut page = ScriptedPage::new(vec![Some(scan(vec![checkbox()], false))]);
    page.presses_work = false;
    let (jev, _) = jev(|_| None);
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::default()).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsolved { reason, .. } if reason.contains("could not be pressed")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_page_with_no_known_widget_or_no_scan_is_left_alone() {
    let page = ScriptedPage::new(vec![Some(scan(vec![], false))]);
    let (jev, asked) = jev(|_| None);
    assert!(matches!(
        solve_captcha(&page, &jev, &CaptchaSettings::default()).await,
        CaptchaOutcome::Unsupported { .. }
    ));
    let page = ScriptedPage::new(vec![None]);
    assert!(matches!(
        solve_captcha(&page, &jev, &CaptchaSettings::default()).await,
        CaptchaOutcome::Unsolved { attempts: 0, .. }
    ));
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_page_that_cannot_be_photographed_is_not_guessed_at() {
    let mut page = ScriptedPage::new(vec![Some(scan(vec![challenge()], false))]);
    page.shot = None;
    let (jev, asked) = jev(grid_jev("grid_3x3"));
    let outcome = solve_captcha(&page, &jev, &CaptchaSettings::with_attempts(1)).await;
    assert!(
        matches!(&outcome, CaptchaOutcome::Unsolved { reason, .. } if reason.contains("photographed")),
        "{outcome:?}"
    );
    assert!(asked.lock().unwrap().is_empty());
    assert!(page.presses().is_empty());
}
