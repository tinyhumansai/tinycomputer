//! Unit tests for the module's bus identity and its member list.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{INTERFACE, METHODS, OBJECT_PATH, methods};
use std::collections::BTreeSet;

#[test]
fn the_interface_and_object_path_are_pinned() {
    assert_eq!(INTERFACE, "ai.tinyhumans.tinycomputer.Desktop");
    assert_eq!(OBJECT_PATH, "/ai/tinyhumans/tinycomputer/Desktop");
}

#[test]
fn the_object_path_is_the_interface_in_path_form() {
    let expected = format!("/{}", INTERFACE.replace('.', "/"));
    assert_eq!(OBJECT_PATH, expected);
}

#[test]
fn every_member_name_is_listed_exactly_once() {
    let unique = METHODS.iter().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), METHODS.len());
}

#[test]
fn the_member_list_has_the_eighty_members_the_contract_documents() {
    assert_eq!(METHODS.len(), 88);
}

#[test]
fn every_member_name_is_pascal_case_and_non_empty() {
    for member in METHODS {
        assert!(!member.is_empty(), "a member name must not be empty");
        assert!(
            member
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_uppercase()),
            "{member} must start with an uppercase letter"
        );
        assert!(
            member.chars().all(|c| c.is_ascii_alphanumeric()),
            "{member} must be alphanumeric so it needs no escaping on the wire"
        );
    }
}

#[test]
fn the_families_appear_in_the_documented_order() {
    // Agentic goal members come first because they compose the primitive
    // families below. The list is also the asserted dispatch order.
    assert_eq!(METHODS.first(), Some(&methods::RESOLVE_INTENT));
    assert_eq!(
        METHODS.last(),
        Some(&crate::browser::names::methods::WAIT_DOWNLOAD)
    );
}

#[test]
fn the_four_reserved_hold_members_are_served_rather_than_omitted() {
    // They fail closed, but a structured, explained failure beats an
    // `UnknownMethod` a caller cannot interpret.
    for reserved in [
        methods::KEY_DOWN,
        methods::KEY_UP,
        methods::MOUSE_DOWN,
        methods::MOUSE_UP,
    ] {
        assert!(METHODS.contains(&reserved), "{reserved} must be served");
    }
}

#[test]
fn the_browser_members_close_the_list_in_their_own_order() {
    let browser = crate::browser::names::METHODS;
    assert_eq!(&METHODS[METHODS.len() - browser.len()..], browser);
}
