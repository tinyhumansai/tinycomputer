//! What a flow sees, and the judgements it makes about it.
//!
//! The screen model and its neutral helpers live in `tinycomputer-core`, the
//! desktop's snapshot parsing in `tinycomputer-desktop`; this module re-exports
//! them for the flow runtime and keeps the flow's own policy: when a choice is
//! confident enough to act on, and which controls a flow must not press.

pub(in crate::agentic) use tinycomputer_core::surface::{
    Candidate, Depth, Digest, MAX_CANDIDATES, Region, RegionKind, Rendering, Screen, change_note,
    describe, difference, digest, element_line, exact_named_match, fingerprint, label, signature,
    target_payload, untrusted_context,
};

/// Least probability a target choice needs to be used without re-asking.
pub(in crate::agentic) const ACT: f64 = 0.70;

/// A pressed control's identity across the states pressing it flips: its
/// label and where it sits, without the value and states a toggle changes
/// (`signature` keeps them, so a toggle's open and closed looks are two
/// signatures).
pub(in crate::agentic) fn press_key(candidate: &Candidate) -> String {
    format!("press:{}:{}", label(candidate), candidate.path.join(">"))
}

/// Whether a step has struck `candidate` off: by its signature, after a
/// press that changed nothing, or by its press key, after it was pressed
/// too often or its copy on another item was pressed.
pub(in crate::agentic) fn is_banned(
    banned: &std::collections::BTreeSet<String>,
    candidate: &Candidate,
) -> bool {
    banned.contains(&signature(candidate)) || banned.contains(&press_key(candidate))
}

/// Words that commit to something wherever they sit in a label ("Post",
/// "Transfer $500", "Log out"). Matched as whole words, so "Postcode" and
/// "Design" stay pressable; "sign" counts unless it is signing in.
const COMMITTING_WORDS: &[&str] = &[
    "post",
    "transfer",
    "checkout",
    "check out",
    "log out",
    "logout",
    "sign",
];

/// Whether a lower-cased label names an action that is hard to undo. A
/// counter's minus button ("remove adult") is not: it only lowers a number.
pub(in crate::agentic) fn destructive_label(evidence: &str) -> bool {
    let words = format!(
        " {} ",
        evidence
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    )
    .replace(" sign in ", " ");
    !tinycomputer_core::adjusts_a_count(evidence)
        && (COMMITTING_WORDS
            .iter()
            .any(|word| words.contains(&format!(" {word} ")))
            || [
                "delete",
                "remove",
                "send",
                "purchase",
                "buy",
                "pay",
                "submit",
                "confirm",
                "overwrite",
                "quit without saving",
                "empty trash",
                "sign out",
            ]
            .iter()
            .any(|term| evidence.contains(term)))
}

/// Whether `label` is named by a `stop_before` phrase the flow itself
/// declares elsewhere.
///
/// A flow that already plans to `stop_before: "sending the email"` has told
/// us, in its own words, that whatever performs that action is irreversible —
/// even when the generic English denylist above does not happen to cover the
/// word it uses. A label under three characters is never checked: it is too
/// short for containment to mean anything ("ok", "go") and would otherwise
/// match almost any phrase. The label must start a word of the phrase, so an
/// inflection still counts ("Send" in "sending the email") but a label that
/// only sits inside another word does not ("Rent" in "the current bill").
pub(in crate::agentic) fn named_in_stop_before(label: &str, stop_before: &[String]) -> bool {
    let label = label.trim().to_ascii_lowercase();
    label.chars().count() >= 3
        && stop_before
            .iter()
            .any(|phrase| starts_a_word_in(&phrase.to_ascii_lowercase(), &label))
}

/// Whether `needle` occurs in `haystack` at the start of a word: at the very
/// start, or right after a character that is neither a letter nor a digit.
fn starts_a_word_in(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(at, _)| {
        haystack[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric())
    })
}

/// Whether pressing `candidate` on `screen` must be treated as irreversible:
/// its own label names a hard-to-undo action, the flow's own `stop_before`
/// steps already name it, it is an unnamed control offered inside a
/// confirmation sheet — the shape of "Delete"/"Cancel" dialogs whose default
/// button carries no accessible name on some platforms, so the denylist can
/// never see the word that would otherwise gate it — or the screen itself
/// shows payment evidence, so a control worded only "Continue" on a card form
/// is caught even though its own label says nothing about money. A form
/// control on that page — a card field, an expiry month, a saved-card radio —
/// is not: filling a payment form commits to nothing until its button is
/// pressed, and that button stays gated.
///
/// A tab is navigation: pressing it shows another panel of the same page and
/// commits to nothing. A `stop_before` phrase therefore never names one, and
/// on a payment page choosing one is filling the form, like a saved-card
/// radio (`IndiGo` keeps its flight search behind a tab labelled "Book",
/// which a flow stopping before "paying for the booking" named and refused,
/// so no step could open the search form; tinycomputer#62). Its own label
/// still counts: the role is only the page's claim, so a tab labelled like an
/// irreversible control ("Pay ₹7,346") stays gated.
pub(in crate::agentic) fn is_destructive(
    candidate: &Candidate,
    screen: &Screen,
    stop_before: &[String],
) -> bool {
    let name = candidate
        .name
        .as_deref()
        .or(candidate.description.as_deref())
        .unwrap_or_default();
    let navigation = is_navigation(candidate);
    destructive_label(&label(candidate).to_ascii_lowercase())
        || (!navigation && named_in_stop_before(name, stop_before))
        || (screen.surface == "sheet" && candidate.name.is_none())
        || (!is_form_control(candidate)
            && !navigation
            && tinycomputer_core::screen_payment_evidence(screen).is_some())
}

/// `pool` with every element Jev could not tell apart from an earlier one
/// left out: of candidates whose descriptions match, bounds aside, the first
/// in page order is kept.
///
/// Offered side by side, lookalikes split the vote: measured on a booking
/// widget with nine unnamed search boxes in one dropdown, each framing of a
/// voted slot question picked a different one, and the merged answer fell
/// under the slot floor although every framing had found the right box.
pub(in crate::agentic) fn distinct(pool: Vec<Candidate>, include_values: bool) -> Vec<Candidate> {
    let mut seen = std::collections::BTreeSet::new();
    pool.into_iter()
        .filter(|candidate| {
            let mut key = describe(candidate, include_values);
            if let Some(fields) = key
                .get_mut("untrusted_accessibility_data")
                .and_then(serde_json::Value::as_object_mut)
            {
                fields.remove("bounds");
            }
            seen.insert(key.to_string())
        })
        .collect()
}

/// What kind of element `field` is, and where: its role, its accessible
/// name, and its ancestors — without the value or states that tell one list
/// row from the next, and without `description`, which an unnamed row's
/// content can fill in per row (`describe_by_content`), making it just as
/// row-specific as a value. A field that refused text strikes every element
/// of its kind: the rows of a city list each hold their city as a value or a
/// content-derived description, and trying them one by one only spends the
/// step — or, pressed while revealing a field, chooses a city nobody asked
/// for.
pub(in crate::agentic) fn element_kind(field: &Candidate) -> String {
    format!(
        "{}:{}:{}",
        field.role,
        field.name.as_deref().unwrap_or_default(),
        field.path.join(">")
    )
}

/// Words a purpose is phrased with that say nothing about which element
/// serves it.
const PURPOSE_FILLER: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "into",
    "from",
    "that",
    "this",
    "click",
    "press",
    "expand",
    "scroll",
    "perform",
    "accomplish",
    "step",
    "choose",
    "type",
    "use",
];

/// Reorders `pool` so the elements whose label shares a word stem with
/// `purpose` come first, keeping the order within each group.
///
/// Jev leans toward the first options it is shown: measured on a payment
/// page, "perform: paying for the booking" picked `button "Pay ₹6,840"` at
/// 0.44 when it came first and 0.01 when it came fifth. Putting the
/// elements the purpose names first spends that lean where it helps.
pub(in crate::agentic) fn named_first(purpose: &str, pool: &mut [Candidate]) {
    let wanted = stems(purpose)
        .into_iter()
        .filter(|word| !PURPOSE_FILLER.contains(&word.as_str()))
        .collect::<Vec<_>>();
    if wanted.is_empty() {
        return;
    }
    pool.sort_by_key(|candidate| {
        let named = stems(&label(candidate))
            .iter()
            .any(|word| wanted.iter().any(|want| same_stem(word, want)));
        !named
    });
}

/// The lower-cased words of `text` at least three characters long.
fn stems(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// Whether two words share a stem: the shorter is a prefix of the longer,
/// or they share their first four characters ("pay" and "paying", "book"
/// and "booking").
fn same_stem(left: &str, right: &str) -> bool {
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let head = short.chars().take(4).collect::<String>();
    long.starts_with(short) || (head.chars().count() == 4 && long.starts_with(&head))
}

/// Roles that hold or choose a value rather than submit anything.
const FORM_ROLES: &[&str] = &[
    "textbox",
    "textfield",
    "text field",
    "searchbox",
    "combobox",
    "listbox",
    "option",
    "radio",
    "radiobutton",
    "checkbox",
    "spinbutton",
    "menuitemradio",
    "popupbutton",
];

/// Whether `candidate` holds or chooses a value: a field, a list, an option.
fn is_form_control(candidate: &Candidate) -> bool {
    FORM_ROLES
        .iter()
        .any(|role| candidate.role.eq_ignore_ascii_case(role))
}

/// Roles that move between panels of the same page rather than act on it.
const NAVIGATION_ROLES: &[&str] = &["tab"];

/// Whether `candidate` only shows another panel when pressed: a tab.
fn is_navigation(candidate: &Candidate) -> bool {
    NAVIGATION_ROLES
        .iter()
        .any(|role| candidate.role.eq_ignore_ascii_case(role))
}

#[cfg(test)]
mod view_tests;
