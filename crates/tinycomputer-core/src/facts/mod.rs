//! The facts a task may type: the caller's values, held locally by name.
//!
//! A fact is either **shared** or **secret**.
//!
//! - A shared fact — a traveller's name, date of birth, email, phone — is
//!   part of the brief a decision model works from, so it can tell "Ms" from
//!   "Mr" or pick "Female" from a list. [`Facts::shared`] lists them.
//! - A secret fact — a card number, a passport number, a password, a
//!   one-time code — is a template variable: a model only ever sees
//!   `${card number}`, a surface looks the value up with [`Facts::get`] at the
//!   moment it types it, and [`Facts::mask`] turns any appearance of the
//!   value, in any text leaving the machine, back into `${name}`.
//!
//! A fact is secret when the caller says so, when its name labels something
//! sensitive ([`is_sensitive_name`]), or when its value is a plausible card
//! number. A caller can make any fact secret but can never make one of those
//! shared.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};

/// Words that only ever label a value to keep secret, matched as whole
/// words in a fact's name.
const SENSITIVE_NAMES: &[&str] = &[
    "card number",
    "credit card",
    "debit card",
    "card holder",
    "cardholder",
    "name on card",
    "cvv",
    "cvc",
    "cvv2",
    "security code",
    "card expiry",
    "card expiration",
    "expiry date",
    "expiration date",
    "card pin",
    "upi pin",
    "pin",
    "otp",
    "one time code",
    "one time password",
    "password",
    "passcode",
    "passport",
    "passport number",
    "ssn",
    "social security",
    "aadhaar",
    "pan",
    "iban",
    "account number",
    "routing number",
];

/// Fewest digits a secret needs before its digits alone are masked too, so
/// a card number shown grouped differently is still caught while a
/// three-digit code is not hunted for in every price.
const MIN_MASKED_DIGITS: usize = 6;

/// A task's named values.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Facts {
    values: BTreeMap<String, String>,
    handles: BTreeMap<String, SecretFactHandle>,
    secret: BTreeSet<String>,
}

/// A reference to a secret whose value stays with its host.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretFactHandle {
    /// Opaque host identifier.
    pub handle: String,
    /// Safe display mask.
    pub mask: String,
    /// Host-defined kind of secret.
    pub kind: String,
}

impl std::fmt::Debug for SecretFactHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretFactHandle")
            .field("handle", &"[opaque]")
            .field("mask", &self.mask)
            .field("kind", &self.kind)
            .finish()
    }
}

impl std::fmt::Debug for Facts {
    /// Names only: a fact's value never reaches a log.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_set().entries(self.names()).finish()
    }
}

impl Facts {
    /// Facts from `(name, value)` pairs, each sensitive one secret.
    ///
    /// ```
    /// use tinycomputer_core::Facts;
    ///
    /// let facts = Facts::new([("email", "sam@example.com"), ("card number", "4111 1111 1111 1111")]);
    /// assert_eq!(facts.get("email"), Some("sam@example.com"));
    /// assert!(facts.is_secret("card number") && !facts.is_secret("email"));
    /// assert_eq!(facts.mask("paying with 4111111111111111"), "paying with ${card number}");
    /// ```
    #[must_use]
    pub fn new<N: Into<String>, V: Into<String>>(pairs: impl IntoIterator<Item = (N, V)>) -> Self {
        let values = pairs
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect::<BTreeMap<_, _>>();
        let secret = values
            .iter()
            .filter(|(name, value)| is_sensitive_name(name) || is_card_number(value))
            .map(|(name, _)| name.clone())
            .collect();
        Self {
            values,
            handles: BTreeMap::new(),
            secret,
        }
    }

    /// Facts from values and host-side secret handles. A name cannot be
    /// supplied in both maps, and every name in `secret` must exist in one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DuplicateFact`] when one name has both a value and a
    /// handle, [`Error::EmptyFactHandle`] for an empty handle, or
    /// [`Error::UnknownSecret`] when a requested secret name is missing.
    pub fn with_handles<N: Into<String>, V: Into<String>, S: Into<String>>(
        pairs: impl IntoIterator<Item = (N, V)>,
        handles: impl IntoIterator<Item = (N, SecretFactHandle)>,
        secret: impl IntoIterator<Item = S>,
    ) -> Result<Self> {
        let mut facts = Self::new(pairs);
        for (name, handle) in handles {
            let name = name.into();
            if handle.handle.trim().is_empty() {
                return Err(Error::EmptyFactHandle { name });
            }
            if facts.values.contains_key(&name) {
                return Err(Error::DuplicateFact { name });
            }
            facts.secret.insert(name.clone());
            facts.handles.insert(name, handle);
        }
        for name in secret {
            let name = name.into();
            if !facts.values.contains_key(&name) && !facts.handles.contains_key(&name) {
                return Err(Error::UnknownSecret { name });
            }
            facts.secret.insert(name);
        }
        Ok(facts)
    }

    /// [`Facts::new`], with every name in `secret` kept secret as well.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownSecret`] when a name in `secret` is not one of the
    /// facts: a misspelling there would leave the value shared.
    pub fn with_secrets<N: Into<String>, V: Into<String>, S: Into<String>>(
        pairs: impl IntoIterator<Item = (N, V)>,
        secret: impl IntoIterator<Item = S>,
    ) -> Result<Self> {
        let mut facts = Self::new(pairs);
        for name in secret {
            let name = name.into();
            if !facts.values.contains_key(&name) {
                return Err(Error::UnknownSecret { name });
            }
            facts.secret.insert(name);
        }
        Ok(facts)
    }

    /// These facts, with `other`'s added or replacing them. A name secret in
    /// either stays secret.
    #[must_use]
    pub fn merged(&self, other: &Self) -> Self {
        let mut merged = self.clone();
        for (name, value) in &other.values {
            if !merged.handles.contains_key(name) {
                merged.values.insert(name.clone(), value.clone());
            }
        }
        for (name, handle) in &other.handles {
            merged.values.remove(name);
            merged.handles.insert(name.clone(), handle.clone());
        }
        merged.secret.extend(other.secret.iter().cloned());
        merged
    }

    /// The value of the fact called `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// The host-side handle for a fact, if it has no engine-visible value.
    #[must_use]
    pub fn handle(&self, name: &str) -> Option<&SecretFactHandle> {
        self.handles.get(name)
    }

    /// Every fact's name.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.values
            .keys()
            .chain(self.handles.keys())
            .map(String::as_str)
            .collect()
    }

    /// Whether the fact called `name` is secret.
    #[must_use]
    pub fn is_secret(&self, name: &str) -> bool {
        self.secret.contains(name)
    }

    /// The names of the secret facts: all a model learns of them.
    #[must_use]
    pub fn secret_names(&self) -> Vec<&str> {
        self.secret.iter().map(String::as_str).collect()
    }

    /// The shared facts, name and value, for a model's brief.
    pub fn shared(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .filter(|(name, _)| !self.secret.contains(*name))
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// The names in `wanted` that no fact supplies, in order.
    #[must_use]
    pub fn missing<'a>(&self, wanted: &'a [String]) -> Vec<&'a str> {
        wanted
            .iter()
            .map(String::as_str)
            .filter(|name| !self.values.contains_key(*name) && !self.handles.contains_key(*name))
            .collect()
    }

    /// `text` with every fact value replaced by `‹name›`, longest values
    /// first so one value inside another is still replaced whole.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        replace_values(text, self.values.iter(), |name| format!("‹{name}›"))
    }

    /// `text` with every secret value replaced by `${name}` — the template a
    /// model knows it by. A secret of six digits or more is
    /// also found by its digits alone, so a card number the page shows in
    /// groups (`4111 1111 1111 1111`) is masked even when it was given as
    /// one run of digits.
    #[must_use]
    pub fn mask(&self, text: &str) -> String {
        let secrets = self
            .values
            .iter()
            .filter(|(name, _)| self.secret.contains(*name));
        let masked = replace_values(text, secrets.clone(), |name| format!("${{{name}}}"));
        secrets.fold(masked, |text, (name, value)| {
            let digits = digits_of(value);
            if digits.len() < MIN_MASKED_DIGITS {
                text
            } else {
                mask_digit_runs(&text, &digits, &format!("${{{name}}}"))
            }
        })
    }
}

/// `text` with each non-empty value replaced by `placeholder(name)`, longest
/// values first.
fn replace_values<'a>(
    text: &str,
    values: impl Iterator<Item = (&'a String, &'a String)>,
    placeholder: impl Fn(&str) -> String,
) -> String {
    let mut values = values
        .filter(|(_, value)| !value.trim().is_empty())
        .collect::<Vec<_>>();
    values.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
    values
        .into_iter()
        .fold(text.to_owned(), |text, (name, value)| {
            text.replace(value.as_str(), &placeholder(name))
        })
}

/// Whether a fact called `name` must be secret: its name labels a card, a
/// password, a one-time code, or an identity or account number.
///
/// ```
/// use tinycomputer_core::is_sensitive_name;
///
/// assert!(is_sensitive_name("Passport Number"));
/// assert!(is_sensitive_name("card_cvv"));
/// assert!(!is_sensitive_name("date of birth"));
/// ```
#[must_use]
pub fn is_sensitive_name(name: &str) -> bool {
    let words = format!(
        " {} ",
        name.to_lowercase()
            .replace(|character: char| !character.is_alphanumeric(), " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
    SENSITIVE_NAMES
        .iter()
        .any(|term| words.contains(&format!(" {term} ")))
}

fn digits_of(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
}

/// `text` with each run of digits, spaces, and dashes whose digits are
/// exactly `digits` replaced by `placeholder`.
fn mask_digit_runs(text: &str, digits: &str, placeholder: &str) -> String {
    let characters = text.char_indices().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < characters.len() {
        let (start, character) = characters[index];
        if !character.is_ascii_digit() {
            out.push(character);
            index += 1;
            continue;
        }
        let mut end = index;
        let mut last_digit = index;
        while end < characters.len()
            && (characters[end].1.is_ascii_digit() || matches!(characters[end].1, ' ' | '-'))
        {
            if characters[end].1.is_ascii_digit() {
                last_digit = end;
            }
            end += 1;
        }
        let stop = characters
            .get(last_digit + 1)
            .map_or(text.len(), |(offset, _)| *offset);
        let run = &text[start..stop];
        if digits_of(run) == digits {
            out.push_str(placeholder);
        } else {
            out.push_str(run);
        }
        index = last_digit + 1;
    }
    out
}

/// Whether `value` is 13 to 19 digits (spaces and dashes aside) that pass
/// the Luhn checksum every payment card number carries.
fn is_card_number(value: &str) -> bool {
    if value
        .chars()
        .any(|character| !(character.is_ascii_digit() || matches!(character, ' ' | '-')))
    {
        return false;
    }
    let digits = value
        .chars()
        .filter_map(|character| character.to_digit(10))
        .collect::<Vec<_>>();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(position, &digit)| {
            if position % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                digit
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod facts_tests;
