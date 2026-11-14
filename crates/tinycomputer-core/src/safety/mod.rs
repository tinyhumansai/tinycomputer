//! What an action would commit to, and whether a page is asking for payment.
//!
//! Two deterministic checks every surface runs before it acts, independently
//! of anything a model decided:
//!
//! - [`consequence`] classifies a control by its label. A payment is never
//!   made on its own — it is stopped at, or held for approval; an irreversible action (send, delete, publish, confirm a
//!   booking) needs explicit approval; everything else proceeds. Stepping
//!   through a booking — "Book", "Select", "Continue" — is deliberately
//!   *reversible*: those lead to further forms, and the payment check stops
//!   the run before anything is charged.
//! - [`payment_evidence`] looks at the page itself — card fields, `cc-*`
//!   autocomplete attributes, a checkout URL with a pay button — so a payment
//!   step is caught even when its button says only "Continue".
//! - [`screen_payment_evidence`] builds `payment_evidence`'s inputs from a
//!   surface-agnostic [`Screen`](crate::surface::Screen), so every surface's
//!   destructive-click gate can apply the same page-level check.

mod consequence;
mod gates;
mod payment;

pub use consequence::{Consequence, adjusts_a_count, consequence};
pub use gates::{PROVE_YOU_ARE_HUMAN, SOLVE_THE_CAPTCHA, human_needed, is_bot_check};
pub use payment::{FieldHint, PaymentEvidence, payment_evidence, screen_payment_evidence};

/// Lower-case words separated by single spaces, padded so whole-word phrases
/// can be matched with `contains`.
fn normalize(text: &str) -> String {
    let words = text
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    format!(
        " {} ",
        words.split_whitespace().collect::<Vec<_>>().join(" ")
    )
}

fn has_phrase(words: &str, phrase: &str) -> bool {
    words.contains(&format!(" {phrase} "))
}

fn contains_any(words: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| has_phrase(words, phrase))
}

#[cfg(test)]
mod safety_tests;
