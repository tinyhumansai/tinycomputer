//! Tests for the flow's own policy: which controls it must not press.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;

use super::{
    Candidate, Screen, destructive_label, is_destructive, named_first, named_in_stop_before,
};

fn clickable_screen() -> Screen {
    Screen {
        app: "Spotify".to_owned(),
        window: Some("Liked Songs".to_owned()),
        surface: "window".to_owned(),
        context: Vec::new(),
        unexplored: Vec::new(),
        text_nodes: Vec::new(),
        candidates: vec![Candidate {
            ref_id: "@s1:e1".to_owned(),
            role: "button".to_owned(),
            name: Some("Play First Song by Artist".to_owned()),
            available_actions: vec!["Click".to_owned()],
            bounds: Some(json!({"x": 10.0, "y": 100.0})),
            ..Candidate::default()
        }],
    }
}

#[test]
fn the_denylist_names_irreversible_labels() {
    assert!(destructive_label("send"));
    assert!(destructive_label("delete draft"));
    assert!(!destructive_label("new message"));
}

#[test]
fn a_stop_before_phrase_names_a_control_the_denylist_does_not_cover() {
    let phrases = vec!["discard the draft".to_owned()];
    assert!(named_in_stop_before("Discard", &phrases));
    assert!(named_in_stop_before("discard", &phrases));
    assert!(!named_in_stop_before("Reply", &phrases));
    // Too short to mean anything on its own; must never match by accident.
    assert!(!named_in_stop_before("Go", &phrases));
    assert!(!named_in_stop_before("Reply", &[]));
}

#[test]
fn a_stop_before_phrase_names_a_control_only_from_the_start_of_a_word() {
    let phrases = vec![
        "sending the email".to_owned(),
        "paying the current bill".to_owned(),
    ];
    // A label that starts a word still names it, inflection and all.
    assert!(named_in_stop_before("Send", &phrases));
    assert!(named_in_stop_before("Pay", &phrases));
    assert!(named_in_stop_before("the current", &phrases));
    // One that only sits inside another word does not.
    assert!(!named_in_stop_before("Rent", &phrases));
    assert!(!named_in_stop_before("Ending", &phrases));
}

fn tab(name: &str) -> Candidate {
    Candidate {
        role: "tab".to_owned(),
        name: Some(name.to_owned()),
        ..Candidate::default()
    }
}

#[test]
fn a_tab_is_navigation_unless_its_own_label_reads_irreversible() {
    // IndiGo keeps its flight search behind a tab labelled "Book", and a flow
    // stopping before "paying for the booking" named it (tinycomputer#62).
    let stop_before = vec!["paying for the booking".to_owned()];
    let mut screen = clickable_screen();
    assert!(!is_destructive(&tab("Book"), &screen, &stop_before));
    // The role is only the page's claim: a tab labelled like a payment stays
    // gated, with or without card fields on the screen.
    assert!(is_destructive(&tab("Pay ₹7,346"), &screen, &[]));
    // The same words on a button are still gated.
    let book = Candidate {
        role: "button".to_owned(),
        name: Some("Book".to_owned()),
        ..Candidate::default()
    };
    assert!(is_destructive(&book, &screen, &stop_before));
    // On a payment page, choosing how to pay is filling the form, not paying.
    screen.candidates.push(Candidate {
        role: "textbox".to_owned(),
        name: Some("Card number".to_owned()),
        available_actions: vec!["SetValue".to_owned()],
        ..Candidate::default()
    });
    assert!(!is_destructive(&tab("UPI"), &screen, &[]));
    assert!(is_destructive(&tab("Pay now"), &screen, &[]));
    let pay = Candidate {
        role: "button".to_owned(),
        name: Some("Pay ₹7,346".to_owned()),
        ..Candidate::default()
    };
    assert!(is_destructive(&pay, &screen, &[]));
    // An unnamed tab in a confirmation sheet is still the sheet's default.
    screen.surface = "sheet".to_owned();
    let unnamed = Candidate {
        role: "tab".to_owned(),
        ..Candidate::default()
    };
    assert!(is_destructive(&unnamed, &screen, &[]));
}

#[test]
fn is_destructive_covers_the_denylist_stop_before_phrases_and_unnamed_sheet_buttons() {
    let mut screen = clickable_screen();
    let discard = Candidate {
        name: Some("Discard".to_owned()),
        ..Candidate::default()
    };
    // Neither on the denylist nor named by any stop_before phrase.
    assert!(!is_destructive(&discard, &screen, &[]));
    // The flow's own words name it, even though the denylist does not.
    assert!(is_destructive(
        &discard,
        &screen,
        &["discard the draft".to_owned()]
    ));
    // The denylist alone is still enough, with no stop_before phrases at all.
    let send = Candidate {
        name: Some("Send".to_owned()),
        ..Candidate::default()
    };
    assert!(is_destructive(&send, &screen, &[]));
    // An unnamed button is only gated inside a confirmation sheet.
    let unnamed = Candidate::default();
    assert!(!is_destructive(&unnamed, &screen, &[]));
    screen.surface = "sheet".to_owned();
    assert!(is_destructive(&unnamed, &screen, &[]));
    assert!(
        !is_destructive(&discard, &screen, &[]),
        "a named, non-denylisted control in a sheet is still safe"
    );
}

#[test]
fn is_destructive_gates_a_generic_control_on_a_payment_screen() {
    let mut screen = clickable_screen();
    let continue_button = Candidate {
        name: Some("Continue".to_owned()),
        ..Candidate::default()
    };
    // No payment evidence yet: an unremarkable control is not gated.
    assert!(!is_destructive(&continue_button, &screen, &[]));
    // A card field on the same screen makes it a payment step, so even a
    // control worded only "Continue" must not be pressed by an ordinary step.
    screen.candidates.push(Candidate {
        role: "textbox".to_owned(),
        name: Some("Card number".to_owned()),
        available_actions: vec!["SetValue".to_owned()],
        ..Candidate::default()
    });
    assert!(is_destructive(&continue_button, &screen, &[]));
    // Filling that form commits to nothing, so its fields and choices are
    // not gated: only the button that submits it is.
    for (role, name) in [
        ("textbox", "Card number"),
        ("combobox", "Expiry month"),
        ("option", "12"),
        ("radio", "Saved card ending 1111"),
    ] {
        let control = Candidate {
            role: role.to_owned(),
            name: Some(name.to_owned()),
            ..Candidate::default()
        };
        assert!(!is_destructive(&control, &screen, &[]), "{role} {name}");
    }
    let pay = Candidate {
        role: "button".to_owned(),
        name: Some("Pay ₹7,346".to_owned()),
        ..Candidate::default()
    };
    assert!(is_destructive(&pay, &screen, &[]));
}

#[test]
fn a_counters_minus_button_is_not_destructive() {
    assert!(!destructive_label("remove adult, 2 adult remaining"));
    assert!(destructive_label("remove"));
}

fn button(name: &str) -> Candidate {
    Candidate {
        ref_id: format!("@e:{name}"),
        role: "button".to_owned(),
        name: Some(name.to_owned()),
        ..Candidate::default()
    }
}

#[test]
fn the_elements_a_purpose_names_are_offered_first_in_their_own_order() {
    let mut pool = vec![
        button("Payment"),
        button("Card number"),
        button("Pay ₹6,840"),
        button("Booking summary"),
    ];
    named_first("perform: paying for the booking", &mut pool);
    let names = pool
        .iter()
        .map(|candidate| candidate.name.clone().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["Pay ₹6,840", "Booking summary", "Payment", "Card number"],
        "every element sharing a stem moves ahead, in page order; \"payment\" and \"paying\" share only three letters"
    );

    let mut untouched = vec![button("B"), button("A")];
    named_first("click to perform: the step", &mut untouched);
    assert_eq!(
        untouched[0].name.as_deref(),
        Some("B"),
        "filler names nothing"
    );
}

#[test]
fn posting_transferring_checking_out_paying_deleting_logging_out_and_signing_are_destructive() {
    let screen = clickable_screen();
    let pressable: Vec<&str> = [
        "Post",
        "Transfer $500",
        "Checkout",
        "Check out",
        "Pay",
        "Delete",
        "Log out",
        "Sign",
        "Sign up",
    ]
    .into_iter()
    .filter(|name| !is_destructive(&button(name), &screen, &[]))
    .collect();
    assert!(
        pressable.is_empty(),
        "pressed with no approval: {pressable:?}"
    );
    // Signing in commits to nothing, and a word that only contains one of
    // these is not one of them.
    for name in [
        "Sign in",
        "Sign-in with Google",
        "Postcode",
        "Design",
        "Logo",
    ] {
        assert!(!is_destructive(&button(name), &screen, &[]), "{name}");
    }
}
