//! Tests for a task's surface on a page its allowed origins refuse: the
//! observation fails as blocked, and the page is left before it is read.
//! A page whose address cannot be read is not read either.

use std::sync::{Arc, Mutex};

use serde_json::json;
use tinycomputer_bus::browser::SessionOptions;
use tinycomputer_core::surface::{Depth, Surface};

use super::{Drawn, shown_harness};
use crate::fake::{Fake, ok};

/// A page that opens at `back` and is at `then` from the second time its
/// address is read (a page that moved on its own after the session
/// opened), until the session goes back to `back`.
fn page_at(then: &'static str, back: &'static str) -> Fake {
    let state = Arc::new(Mutex::new((0_u32, back.to_owned())));
    Fake::scripted(move |command| {
        let mut state = state.lock().unwrap();
        match command["action"].as_str().unwrap() {
            "back" => {
                back.clone_into(&mut state.1);
                None
            }
            "url" => {
                state.0 += 1;
                if state.0 == 2 {
                    then.clone_into(&mut state.1);
                }
                Some(ok(&json!({"url": state.1})))
            }
            _ => None,
        }
    })
}

fn within(origins: &[&str]) -> SessionOptions {
    SessionOptions {
        allowed_origins: origins.iter().map(|origin| (*origin).to_owned()).collect(),
        ..SessionOptions::default()
    }
}

#[test]
fn observing_a_refused_page_fails_as_blocked_and_leaves_it_unread() {
    let fake = page_at("https://evil.test/offer", "https://flights.test/");
    let harness = shown_harness(
        "origins-observe",
        fake,
        within(&[".flights.test"]),
        &Drawn::default(),
    );
    let refused = harness
        .surface
        .observe("", None, Depth::Full)
        .expect_err("a refused page is never read");
    let error = refused.error.as_ref().unwrap();
    assert_eq!(error.code, "BLOCKED_BY_POLICY");
    assert!(
        error.message.contains("https://evil.test/offer"),
        "{}",
        error.message
    );
    let actions = harness.fake.actions();
    assert!(actions.contains(&"back".to_owned()));
    assert!(
        !actions.iter().any(|action| action == "snapshot")
            && !harness
                .fake
                .sent()
                .iter()
                .any(|command| command["action"] == "evaluate"),
        "nothing was read on the refused page: {actions:?}"
    );
}

#[test]
fn an_admitted_page_is_read_as_before() {
    let fake = page_at("https://www.flights.test/search", "https://flights.test/");
    let harness = shown_harness(
        "origins-admitted",
        fake,
        within(&[".flights.test"]),
        &Drawn::default(),
    );
    let screen = harness
        .surface
        .observe("", None, Depth::Full)
        .expect("an admitted page is read");
    // The fake engine's page: one "Search" button.
    assert!(
        screen
            .candidates
            .iter()
            .any(|candidate| candidate.name.as_deref() == Some("Search")),
        "{:?}",
        screen.candidates
    );
    assert!(!harness.fake.actions().contains(&"back".to_owned()));
}

#[test]
fn a_page_whose_address_cannot_be_read_is_not_read() {
    let reads = Arc::new(Mutex::new(0_u32));
    let fake = Fake::scripted(move |command| {
        let mut reads = reads.lock().unwrap();
        match command["action"].as_str().unwrap() {
            // The address reads once as the session opens, then fails.
            "url" => {
                *reads += 1;
                Some(if *reads == 1 {
                    ok(&json!({"url": "https://flights.test/"}))
                } else {
                    crate::fake::failure("Execution context was destroyed")
                })
            }
            _ => None,
        }
    });
    let harness = shown_harness(
        "origins-unreadable",
        fake,
        within(&[".flights.test"]),
        &Drawn::default(),
    );
    assert!(harness.surface.observe("", None, Depth::Full).is_err());
    let actions = harness.fake.actions();
    assert!(
        !actions
            .iter()
            .any(|action| action == "snapshot" || action == "evaluate"),
        "nothing was read: {actions:?}"
    );
}
