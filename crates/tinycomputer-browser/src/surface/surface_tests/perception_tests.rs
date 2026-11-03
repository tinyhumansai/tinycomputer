//! Tests for reading a page by sight, and falling back to the tree.

use serde_json::json;
use tinycomputer_bus::JevOperation;
use tinycomputer_bus::browser::ElementRef;
use tinycomputer_core::surface::{Candidate, Depth, Surface};

use super::{Harness, harness, page_fake};
use crate::fake::{Fake, failure, ok};
use crate::surface::{Denoised, Perception};

#[test]
fn sight_targets_map_to_unique_current_native_refs_only() {
    let mut unique = Candidate {
        ref_id: "seen:1".to_owned(),
        role: "button".to_owned(),
        name: Some("Accept cookies".to_owned()),
        ..Candidate::default()
    };
    crate::surface::attach_unique_snapshot_ref(
        &mut unique,
        &[ElementRef {
            id: "e1".to_owned(),
            role: "button".to_owned(),
            name: "Accept cookies".to_owned(),
        }],
    );
    assert_eq!(unique.ref_id, "e1");

    let mut duplicate = Candidate {
        ref_id: "seen:2".to_owned(),
        role: "button".to_owned(),
        name: Some("Continue".to_owned()),
        ..Candidate::default()
    };
    crate::surface::attach_unique_snapshot_ref(
        &mut duplicate,
        &[
            ElementRef {
                id: "e2".to_owned(),
                role: "button".to_owned(),
                name: "Continue".to_owned(),
            },
            ElementRef {
                id: "e3".to_owned(),
                role: "button".to_owned(),
                name: "Continue".to_owned(),
            },
        ],
    );
    assert_eq!(duplicate.ref_id, "seen:2");

    let mut stale_shadow_ref = Candidate {
        ref_id: "e2".to_owned(),
        role: "button".to_owned(),
        name: Some("Allow Selection".to_owned()),
        ..Candidate::default()
    };
    assert!(!crate::surface::attach_unique_snapshot_ref(
        &mut stale_shadow_ref,
        &[ElementRef {
            id: "e2".to_owned(),
            role: "button".to_owned(),
            name: "Different target".to_owned(),
        }],
    ));

    let mut ambiguous_shadow_ref = Candidate {
        ref_id: "e3".to_owned(),
        role: "button".to_owned(),
        name: Some("Continue".to_owned()),
        ..Candidate::default()
    };
    assert!(!crate::surface::attach_unique_snapshot_ref(
        &mut ambiguous_shadow_ref,
        &[
            ElementRef {
                id: "e3".to_owned(),
                role: "button".to_owned(),
                name: "Continue".to_owned(),
            },
            ElementRef {
                id: "e4".to_owned(),
                role: "button".to_owned(),
                name: "Continue".to_owned(),
            },
        ],
    ));
}

/// A page read by sight: one field and one result link a card covers.
fn sighted_fake() -> Fake {
    Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "evaluate"
            if command["script"]
                .as_str()
                .unwrap()
                .contains("__tinycomputerSeen") =>
        {
            Some(ok(&json!({"result": {
                "ok": true,
                "title": "Flights",
                "surface": "window",
                "unreachable": 0,
                "denoised": {"ads": 2, "empty": 1, "hidden": 0},
                "nodes": [
                    {"id": "1", "role": "textbox", "name": "To", "states": [], "path": []},
                    {"id": "2", "role": "link", "name": "", "states": [], "path": []}
                ]
            }})))
        }
        "snapshot" => Some(ok(&json!({
            "snapshot": "- textbox \"To\" [ref=e1]\n- link [ref=e2]",
            "refs": {
                "e1": {"role": "textbox", "name": "To"},
                "e2": {"role": "link", "name": ""}
            }
        }))),
        "click" => Some(failure(
            "Element is covered by <div.layer> at its click point, so the input would land on that element instead.",
        )),
        "boundingbox" => Some(ok(
            &json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0}),
        )),
        "evaluate" => Some(ok(&json!({"result": true}))),
        _ => None,
    })
}

#[test]
fn sight_reads_the_page_and_its_refs_reach_their_marks() {
    let Harness { fake, surface, .. } = harness("sight", sighted_fake());
    let screen = surface.observe("flights", None, Depth::Skeleton).unwrap();
    assert!(fake.actions().iter().any(|action| action == "snapshot"));
    assert_eq!(screen.app, "flights");
    let field = screen.candidates[0].clone();
    assert_eq!(field.ref_id, "e1");

    let typed = surface.execute(
        JevOperation::TypeText,
        Some(field),
        Some("Srinagar".to_owned()),
    );
    assert!(typed.ok, "{:?}", typed.error);
    let fill = fake.last("fill");
    assert_eq!(fill["selector"], "@e1", "{fill}");
    assert_eq!(fake.last("focus")["selector"], "@e1");

    // An unnamed link a card covers is still clicked through its card: its
    // mark names it exactly.
    let link = screen.candidates[1].clone();
    let reply = surface.execute(JevOperation::Click, Some(link), None);
    assert!(reply.ok, "{:?}", reply.error);
    // A link's press is then checked for having gone anywhere, by a later
    // script: the card's click-through is the one that reads the point.
    let script = fake
        .sent()
        .iter()
        .filter(|command| command["action"] == "evaluate")
        .filter_map(|command| command["script"].as_str())
        .find(|script| script.contains("elementsFromPoint"))
        .unwrap()
        .to_owned();
    assert!(
        script.ends_with(r#"(60, 40, "", "[data-tc-seen=\"2\"]")"#),
        "{script}"
    );

    surface.observe("", Some("seen:2"), Depth::Full).unwrap();
    let sent = fake.sent();
    let scoped = sent
        .iter()
        .filter(|command| command["action"] == "evaluate")
        .filter_map(|command| command["script"].as_str())
        .find(|script| {
            script.contains("__tinycomputerSeen") && script.contains("data-tc-seen=\\\"2\\\"")
        })
        .expect("the scoped Sight script uses the retained Sight ref");
    assert!(scoped.contains(r#"("[data-tc-seen=\"2\"]", {"#), "{scoped}");
}

#[test]
fn the_surface_keeps_what_its_last_sight_reading_left_out_as_noise() {
    let Harness { surface, .. } = harness("sight-denoised", sighted_fake());
    assert_eq!(surface.denoised(), Denoised::default(), "nothing read yet");
    surface.observe("", None, Depth::Full).unwrap();
    assert_eq!(
        surface.denoised(),
        Denoised {
            ads: 2,
            empty: 1,
            hidden: 0
        }
    );

    let Harness { surface, .. } = harness("tree-denoised", page_fake());
    surface.observe("", None, Depth::Full).unwrap();
    assert_eq!(
        surface.denoised(),
        Denoised::default(),
        "a page read by the tree was not denoised"
    );
}

#[test]
fn the_tree_is_read_when_sight_fails_or_is_turned_off() {
    let Harness { fake, surface, .. } = harness("sight-fallback", page_fake());
    let screen = surface.observe("", None, Depth::Full).unwrap();
    assert_eq!(screen.candidates[0].ref_id, "e1");
    assert!(fake.actions().iter().any(|action| action == "evaluate"));

    let Harness { fake, surface, .. } = harness("sight-off", sighted_fake());
    let surface = surface.with_perception(Perception::Tree);
    surface.observe("", None, Depth::Full).unwrap();
    // The tree's own look for deep controls runs; sight's reading never does.
    let read_by_sight = fake.sent().iter().any(|command| {
        command["action"] == "evaluate"
            && command["script"]
                .as_str()
                .is_some_and(|script| script.contains("__tinycomputerSeen"))
    });
    assert!(!read_by_sight);
    assert!(format!("{surface:?}").contains("Tree"));
}

/// A page read by sight whose `shadows` show controls: a covered "Add To
/// Cart", and under a host the tree reads a consent banner's buttons, or
/// fails to when `subtree_fails`.
fn shadowed_fake(shadows: serde_json::Value, subtree_fails: bool) -> Fake {
    Fake::scripted(move |command| match command["action"].as_str().unwrap() {
        "evaluate"
            if command["script"]
                .as_str()
                .unwrap()
                .contains("__tinycomputerSeen") =>
        {
            Some(ok(&json!({"result": {
                "ok": true,
                "title": "Glasses",
                "surface": "window",
                "unreachable": 0,
                "shadows": shadows,
                "denoised": {"ads": 0, "empty": 0, "hidden": 0},
                "nodes": [
                    {"id": "1", "role": "button", "name": "Add To Cart", "states": ["covered"], "path": ["main"]},
                    {"text": "Limited Period Offer", "path": ["main"]}
                ]
            }})))
        }
        "snapshot" if command.get("selector").is_some() && subtree_fails => {
            Some(failure("no such element"))
        }
        "snapshot" if command.get("selector").is_some() => Some(ok(&json!({
            "snapshot": "- generic\n  - paragraph\n    - StaticText \"We value your privacy\"\n  - button \"Allow Selection\" [ref=e2]\n  - button \"Allow all\" [ref=e3]",
            "refs": {
                "e2": {"role": "button", "name": "Allow Selection"},
                "e3": {"role": "button", "name": "Allow all"}
            }
        }))),
        "snapshot" => Some(ok(&json!({
            "snapshot": "- button \"Add To Cart\" [ref=e1]\n- button \"Allow Selection\" [ref=e2]\n- button \"Allow all\" [ref=e3]",
            "refs": {
                "e1": {"role": "button", "name": "Add To Cart"},
                "e2": {"role": "button", "name": "Allow Selection"},
                "e3": {"role": "button", "name": "Allow all"}
            }
        }))),
        _ => None,
    })
}

#[test]
fn a_shadow_roots_controls_are_read_by_the_tree_beside_sight() {
    // Live, a consent banner in a shadow root lay over "Add To Cart": sight
    // could not read it, and giving the whole page to the tree read the
    // rest of the page worse.
    let banner = json!([{"id": "9", "label": "popover \"We value your privacy\""}]);
    let Harness { fake, surface, .. } = harness("shadow-merged", shadowed_fake(banner, false));
    let screen = surface.observe("", None, Depth::Full).unwrap();
    let names = screen
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.ref_id.as_str(),
                candidate.name.as_deref().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            ("e1", "Add To Cart"),
            ("e2", "Allow Selection"),
            ("e3", "Allow all")
        ]
    );
    let allow = &screen.candidates[1];
    assert_eq!(
        allow.path[0], "popover \"We value your privacy\"",
        "{:?}",
        allow.path
    );
    assert!(
        allow.order > screen.candidates[0].order,
        "read after sight's nodes"
    );
    assert!(
        screen
            .context
            .iter()
            .any(|line| line.contains("We value your privacy"))
    );
    let sent = fake.sent();
    let subtree = sent
        .iter()
        .find(|command| command["action"] == "snapshot" && command.get("selector").is_some())
        .unwrap();
    assert_eq!(subtree["selector"], r#"[data-tc-seen="9"]"#, "{subtree}");

    // A shadow root that draws no layer keeps the tree's own places.
    let plain = json!([{"id": "9", "label": null}]);
    let Harness { surface, .. } = harness("shadow-plain", shadowed_fake(plain, false));
    let screen = surface.observe("", None, Depth::Full).unwrap();
    assert!(
        !screen.candidates[1]
            .path
            .first()
            .is_some_and(|label| label.starts_with("popover")),
        "{:?}",
        screen.candidates[1].path
    );
}

#[test]
fn the_tree_reads_the_page_when_two_shadow_roots_show_or_one_cannot_be_read() {
    let two = json!([{"id": "9", "label": null}, {"id": "10", "label": null}]);
    let Harness { fake, surface, .. } = harness("shadow-two", shadowed_fake(two, false));
    let screen = surface.observe("", None, Depth::Full).unwrap();
    assert!(
        screen
            .candidates
            .iter()
            .all(|candidate| !candidate.ref_id.starts_with("seen:"))
    );
    assert!(
        fake.last("snapshot").get("selector").is_none(),
        "the whole page"
    );

    let one = json!([{"id": "9", "label": null}]);
    let Harness { fake, surface, .. } = harness("shadow-failed", shadowed_fake(one, true));
    let screen = surface.observe("", None, Depth::Full).unwrap();
    assert!(
        screen
            .candidates
            .iter()
            .all(|candidate| !candidate.ref_id.starts_with("seen:"))
    );
    assert!(
        fake.last("snapshot").get("selector").is_none(),
        "the whole page"
    );
}
