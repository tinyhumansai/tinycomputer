//! Tests for what a press still refused as covered says about its cover, and
//! for pressing an empty layer over a target where nothing lies beneath.

use serde_json::{Value, json};
use tinycomputer_bus::JevOperation;
use tinycomputer_core::surface::{Candidate, Surface};

use super::{Drawn, Harness, harness, node, shown_harness};
use crate::fake::{Fake, failure, ok};
use crate::surface::{COVER_JS, SPOT_GRID};

/// A page whose every click is refused as covered, whose target's box is
/// 100 by 40 at (10, 20), and whose cover script answers `cover`; every
/// other script answers `false`.
fn covering_fake(cover: Value) -> Fake {
    Fake::scripted(move |command| match command["action"].as_str().unwrap() {
        "click" => Some(failure(
            "Element is covered by <span.label> at its click point, so the input would land on that element instead.",
        )),
        "boundingbox" => Some(ok(
            &json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0}),
        )),
        "evaluate" if command["script"].as_str().unwrap().starts_with(COVER_JS) => {
            Some(ok(&json!({"result": cover})))
        }
        "evaluate" => Some(ok(&json!({"result": false}))),
        _ => None,
    })
}

/// The scripts sent that read what covers a target.
fn cover_scripts(fake: &Fake) -> Vec<String> {
    fake.sent()
        .iter()
        .filter_map(|command| command["script"].as_str())
        .filter(|script| script.starts_with(COVER_JS))
        .map(str::to_owned)
        .collect()
}

fn basket() -> Candidate {
    Candidate {
        name: Some("2".to_owned()),
        role: "button".to_owned(),
        ..node("seen:5", &["Click"])
    }
}

#[test]
fn a_press_still_covered_says_what_covers_it() {
    let Harness { fake, surface, .. } = harness(
        "covered-named",
        covering_fake(json!({"said": "button \"Select Location\"", "bare": false, "spot": null})),
    );
    let reply = surface.execute(JevOperation::Click, Some(basket()), None);
    let error = reply.error.unwrap();
    assert!(error.message.contains("is covered by"), "{}", error.message);
    let details = error.details.unwrap();
    assert_eq!(details["cover"], "button \"Select Location\"");
    assert_eq!(details["empty_layer"], false);
    // Read at the target's middle, by its mark, without a spot to press.
    assert_eq!(
        cover_scripts(&fake),
        [format!(
            r#"{COVER_JS}(60, 40, "[data-tc-seen=\"5\"]", false, {SPOT_GRID})"#
        )]
    );
    // The retry before it brought the target to the middle at once: a page
    // that scrolls smoothly would still be moving it.
    let centred = fake.sent().iter().any(|command| {
        command["script"].as_str().is_some_and(|script| {
            script.contains(
                "scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' })",
            ) && script.contains("focused.blur()")
        })
    });
    assert!(centred);
    assert!(
        !fake.pressed_by_position(),
        "naming a cover presses nothing"
    );

    // An empty layer is said to be one; a cover the page cannot say leaves
    // the refusal as it was.
    let Harness { surface, .. } = harness(
        "covered-empty",
        covering_fake(json!({"said": "an empty layer", "bare": true, "spot": null})),
    );
    let details = surface
        .execute(JevOperation::Click, Some(basket()), None)
        .error
        .unwrap()
        .details
        .unwrap();
    assert_eq!(details["empty_layer"], true);
    let Harness { surface, .. } = harness("covered-unsaid", covering_fake(Value::Null));
    let error = surface
        .execute(JevOperation::Click, Some(basket()), None)
        .error
        .unwrap();
    assert_eq!(error.code, "NOT_ACTIONABLE");
    assert!(error.details.is_none());
}

#[test]
fn an_empty_layer_is_pressed_where_nothing_lies_beneath_it() {
    let Harness { fake, surface, .. } = harness(
        "cover-pressed",
        covering_fake(json!({"said": "an empty layer", "bare": true, "spot": [300.0, 450.0]})),
    );
    let reply = surface.dismiss_cover(&basket());
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.data.unwrap()["dismissed"], "an empty layer");
    assert_eq!(
        cover_scripts(&fake),
        [format!(
            r#"{COVER_JS}(60, 40, "[data-tc-seen=\"5\"]", true, {SPOT_GRID})"#
        )]
    );
    let pressed = fake
        .sent()
        .into_iter()
        .filter(|command| command["action"] == "mouse")
        .map(|command| {
            (
                command["eventType"].as_str().unwrap().to_owned(),
                command["x"].as_f64().unwrap(),
                command["y"].as_f64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pressed,
        [
            ("mouseMoved".to_owned(), 300.0, 450.0),
            ("mousePressed".to_owned(), 300.0, 450.0),
            ("mouseReleased".to_owned(), 300.0, 450.0)
        ]
    );
    // A ref of the tree is read by its point alone.
    let Harness { fake, surface, .. } = harness(
        "cover-pressed-tree",
        covering_fake(json!({"said": "an empty layer", "bare": true, "spot": [300.0, 450.0]})),
    );
    assert!(surface.dismiss_cover(&node("e9", &["Click"])).ok);
    assert!(cover_scripts(&fake)[0].ends_with(&format!("(60, 40, null, true, {SPOT_GRID})")));
}

#[test]
fn a_cover_that_is_no_empty_layer_or_has_no_clear_spot_is_never_pressed() {
    for (cover, why) in [
        (
            json!({"said": "button \"Select Location\"", "bare": false, "spot": null}),
            "it is no empty layer",
        ),
        (
            json!({"said": "an empty layer", "bare": true, "spot": null}),
            "every spot on it lies over something pressable",
        ),
    ] {
        let said = cover["said"].clone();
        let Harness { fake, surface, .. } = harness("cover-refused", covering_fake(cover));
        let reply = surface.dismiss_cover(&basket());
        let error = reply.error.unwrap();
        assert_eq!(error.code, "NOT_ACTIONABLE");
        assert!(error.message.contains(why), "{}", error.message);
        assert!(
            !error.message.contains("is covered by"),
            "{}",
            error.message
        );
        assert_eq!(error.details.unwrap()["cover"], said);
        assert!(!fake.pressed_by_position());
    }
}

#[test]
fn a_cover_that_cannot_be_read_is_never_pressed() {
    // Nothing shows at the target's middle, or the script cannot run.
    for fake in [
        covering_fake(json!({"unread": true})),
        Fake::scripted(|command| match command["action"].as_str().unwrap() {
            "boundingbox" => Some(ok(
                &json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0}),
            )),
            "evaluate" => Some(failure("Evaluation failed: CSP")),
            _ => None,
        }),
    ] {
        let Harness { fake, surface, .. } = harness("cover-unread", fake);
        let error = surface.dismiss_cover(&basket()).error.unwrap();
        assert_eq!(error.code, "NOT_ACTIONABLE");
        assert!(
            error
                .message
                .starts_with("what lies over the element could not be read"),
            "{}",
            error.message
        );
        assert!(!fake.pressed_by_position());
    }
    // Nor is a press refused under it named.
    let Harness { surface, .. } =
        harness("cover-unread-named", covering_fake(json!({"unread": true})));
    let error = surface
        .execute(JevOperation::Click, Some(basket()), None)
        .error
        .unwrap();
    assert!(error.details.is_none());
}

#[test]
fn a_visible_session_glides_the_cursor_to_the_spot_it_presses() {
    let drawn = Drawn::default();
    let fake = Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "boundingbox" => Some(ok(
            &json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0}),
        )),
        "evaluate" if command["script"].as_str().unwrap().starts_with(COVER_JS) => Some(ok(
            &json!({"result": {"said": "an empty layer", "bare": true, "spot": [300.0, 450.0]}}),
        )),
        // The window at (100, 50), with 80 points of toolbars above the page.
        "evaluate" => Some(ok(
            &json!({"result": [100.0, 50.0, 1280.0, 880.0, 1280.0, 800.0]}),
        )),
        _ => None,
    });
    let headed = tinycomputer_bus::browser::SessionOptions {
        headless: false,
        ..tinycomputer_bus::browser::SessionOptions::default()
    };
    let Harness { surface, .. } = shown_harness("cover-cursor", fake, headed, &drawn);
    assert!(surface.dismiss_cover(&basket()).ok);
    let glides = drawn.glides();
    let [_, x, y] = *glides.last().unwrap().last().unwrap();
    assert!(
        (x - 400.0).abs() < 2.0 && (y - 580.0).abs() < 2.0,
        "the spot, in screen points: {x}, {y}"
    );
}

#[test]
fn nothing_over_a_target_leaves_nothing_to_press() {
    let Harness { fake, surface, .. } = harness("cover-gone", covering_fake(Value::Null));
    let reply = surface.dismiss_cover(&basket());
    assert!(reply.ok);
    assert_eq!(reply.data.unwrap()["dismissed"], Value::Null);
    assert!(!fake.pressed_by_position());

    let Harness { surface, .. } = harness("cover-no-target", Fake::new());
    assert_eq!(
        surface
            .dismiss_cover(&Candidate::default())
            .error
            .unwrap()
            .code,
        "INVALID_TARGET"
    );
}
