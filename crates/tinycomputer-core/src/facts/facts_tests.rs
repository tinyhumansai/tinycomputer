//! Tests for the facts store: lookup, shared and secret facts, redaction,
//! and masking secrets back into templates.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Facts, SecretFactHandle, is_sensitive_name};
use crate::error::Error;

fn traveller() -> Facts {
    Facts::new([
        ("first name", "Asha"),
        ("last name", "Raina"),
        ("email", "asha@example.com"),
        ("phone", "+91 98765 43210"),
        ("empty", ""),
    ])
}

#[test]
fn values_are_looked_up_by_name_and_debug_shows_names_only() {
    let facts = traveller();
    assert_eq!(facts.get("email"), Some("asha@example.com"));
    assert_eq!(facts.get("passport"), None);
    assert_eq!(
        facts.names(),
        ["email", "empty", "first name", "last name", "phone"]
    );
    let rendered = format!("{facts:?}");
    assert!(
        rendered.contains("email") && !rendered.contains("asha@"),
        "{rendered}"
    );
}

#[test]
fn missing_names_are_reported_in_order() {
    let wanted = ["passport", "email", "date of birth"].map(str::to_owned);
    assert_eq!(traveller().missing(&wanted), ["passport", "date of birth"]);
}

#[test]
fn redaction_replaces_values_with_their_names() {
    let text = "Booked for Asha Raina (asha@example.com), call +91 98765 43210.";
    assert_eq!(
        traveller().redact(text),
        "Booked for ‹first name› ‹last name› (‹email›), call ‹phone›."
    );
    let nested = Facts::new([("short", "Ann"), ("long", "Annabel")]);
    assert_eq!(nested.redact("Annabel and Ann"), "‹long› and ‹short›");
}

#[test]
fn ordinary_details_are_shared_and_sensitive_ones_secret() {
    let facts = Facts::new([
        ("first name", "Asha"),
        ("date of birth", "2000-01-01"),
        ("Card Number", "anything"),
        ("cvv", "123"),
        ("passport_number", "Z1234567"),
        ("notes", "4111 1111 1111 1111"),
    ]);
    assert_eq!(
        facts.secret_names(),
        ["Card Number", "cvv", "notes", "passport_number"]
    );
    assert_eq!(
        facts.shared().collect::<Vec<_>>(),
        [("date of birth", "2000-01-01"), ("first name", "Asha")]
    );
}

#[test]
fn ordinary_numbers_are_not_mistaken_for_cards() {
    for value in [
        "4111 1111 1111 1112",
        "98765 43210",
        "A1234567",
        "12345678901234567890",
        "2026-10-14",
    ] {
        assert!(
            !Facts::new([("reference", value)]).is_secret("reference"),
            "{value}"
        );
    }
}

#[test]
fn a_caller_can_make_any_fact_secret_but_not_a_missing_one() {
    let facts = Facts::with_secrets([("frequent flyer", "6E123456")], ["frequent flyer"]).unwrap();
    assert!(facts.is_secret("frequent flyer"));
    assert_eq!(facts.shared().count(), 0);
    assert_eq!(
        Facts::with_secrets([("frequent flyer", "6E123456")], ["frequent flier"]),
        Err(Error::UnknownSecret {
            name: "frequent flier".to_owned()
        })
    );
}

#[test]
fn masking_turns_secrets_back_into_templates_and_leaves_shared_values() {
    let facts = Facts::new([
        ("first name", "Asha"),
        ("card number", "4111111111111111"),
        ("cvv", "123"),
    ]);
    assert_eq!(
        facts.mask("Asha paid with 4111 1111 1111 1111 (cvv 123), ref 41111111"),
        "Asha paid with ${card number} (cvv ${cvv}), ref 41111111"
    );
    assert_eq!(
        facts.mask("card 4111-1111-1111-1111."),
        "card ${card number}."
    );
    assert_eq!(facts.mask("no digits here"), "no digits here");
}

#[test]
fn merged_facts_keep_every_secret() {
    let first = Facts::with_secrets([("code", "7788")], ["code"]).unwrap();
    let second = Facts::new([("email", "a@b.c"), ("code", "9900")]);
    let merged = first.merged(&second);
    assert_eq!(merged.get("code"), Some("9900"));
    assert!(merged.is_secret("code") && !merged.is_secret("email"));
}

#[test]
fn host_handles_are_secret_names_without_values() {
    let facts = Facts::with_handles(
        [("email", "person@example.test")],
        [(
            "card number",
            SecretFactHandle {
                handle: "vault-card-1".to_owned(),
                mask: "•••• 4242".to_owned(),
                kind: "payment_card".to_owned(),
            },
        )],
        std::iter::empty::<&str>(),
    )
    .unwrap();

    assert_eq!(facts.get("card number"), None);
    assert_eq!(facts.handle("card number").unwrap().handle, "vault-card-1");
    assert!(facts.is_secret("card number"));
    assert_eq!(
        facts.shared().collect::<Vec<_>>(),
        [("email", "person@example.test")]
    );
    assert_eq!(
        facts.mask("use vault-card-1 or 4242"),
        "use vault-card-1 or 4242"
    );
}

#[test]
fn host_handle_conflicts_and_empty_ids_are_refused() {
    let handle = SecretFactHandle {
        handle: "handle-1".to_owned(),
        mask: "••••".to_owned(),
        kind: "payment_card".to_owned(),
    };
    assert!(matches!(
        Facts::with_handles(
            [("card number", "4111")],
            [("card number", handle.clone())],
            std::iter::empty::<&str>(),
        ),
        Err(crate::Error::DuplicateFact { .. })
    ));
    assert!(matches!(
        Facts::with_handles(
            std::iter::empty::<(&str, &str)>(),
            [(
                "card number",
                SecretFactHandle {
                    handle: String::new(),
                    ..handle
                }
            )],
            std::iter::empty::<&str>(),
        ),
        Err(crate::Error::EmptyFactHandle { .. })
    ));
}

#[test]
fn merging_caller_input_cannot_replace_a_host_held_value() {
    let held = Facts::with_handles(
        std::iter::empty::<(&str, &str)>(),
        [(
            "card number",
            SecretFactHandle {
                handle: "vault-card-1".to_owned(),
                mask: "•••• 1111".to_owned(),
                kind: "payment_card".to_owned(),
            },
        )],
        std::iter::empty::<&str>(),
    )
    .unwrap();
    let supplied = Facts::new([("card number", "4111 1111 1111 1111")]);
    let merged = held.merged(&supplied);

    assert_eq!(merged.get("card number"), None);
    assert_eq!(merged.handle("card number").unwrap().handle, "vault-card-1");
}

#[test]
fn sensitive_names_match_whole_words_only() {
    assert!(is_sensitive_name("One-Time Password"));
    assert!(is_sensitive_name("PAN"));
    assert!(!is_sensitive_name("company"));
    assert!(!is_sensitive_name("spinach"));
    assert!(!is_sensitive_name("mobile number"));
}
