//! Showing a slot's box when the screen has none for it: pressing the
//! control a slot's own name names ("To BLR" for the slot "to"), one slot at
//! a time, or else a short `do` loop that reveals the fields.

use std::collections::BTreeSet;

use tinycomputer_bus::{JevOperation, Slot};

use crate::agentic::flow::{
    AgentBackend, FlowRun, Halt, StepLog,
    ask::{self, Questions, corroborate, probability},
    view::{Candidate, Screen, is_destructive, label},
};

use super::{OPENER_FLOOR, REVEAL_TURNS, editable, names};

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Presses the control on `screen` whose label holds a pending slot's
    /// own word (`named_opener`); `true` when it did. A control named by
    /// the slot, a search link for the "search box", is how the field
    /// shows: live, the turns that look for a way in hesitated over it, and
    /// the search was never typed.
    async fn open_by_name(
        &mut self,
        log: &mut StepLog,
        screen: &Screen,
        slots: &[Slot],
        pending: &BTreeSet<usize>,
    ) -> Result<bool, Halt> {
        let Some(opener) = named_opener(screen, slots, pending)
            .filter(|opener| !is_destructive(opener, screen, &self.stop_before))
        else {
            return Ok(false);
        };
        // A shared word is a hint, not a reason to press: a link named
        // "Email us" shares the slot "email", and pressing it left the form.
        let purpose = format!("click to show the box for: {}", names(slots, pending));
        let answers = self
            .ask(
                log,
                ask::request(
                    self.model(),
                    self.state(screen, &purpose),
                    Questions::default().with(
                        "confirm",
                        corroborate(&purpose, &opener, self.include_values),
                    ),
                ),
            )
            .await?;
        if probability(&answers, "confirm").is_none_or(|yes| yes < OPENER_FLOOR) {
            return Ok(false);
        }
        let pressed = opener.clone();
        let reply = self
            .act(
                log,
                "click (show the field)",
                Some(&opener),
                move |backend| backend.execute(JevOperation::Click, Some(pressed), None),
            )
            .await?;
        if reply.ok {
            self.history.push(format!(
                "pressed {} to show the field for {}",
                label(&opener),
                names(slots, pending)
            ));
        }
        Ok(reply.ok)
    }

    /// Runs a short `do` loop that shows the fields for the `pending` slots,
    /// any field at all when `no_fields` showed. A field that cannot be revealed
    /// is looked for another way, or found not to be asked for; it is not a
    /// failure.
    pub(super) async fn reveal_fields(
        &mut self,
        log: &mut StepLog,
        slots: &[Slot],
        pending: &BTreeSet<usize>,
        no_fields: bool,
    ) -> Result<(), Halt> {
        let reveal = if no_fields {
            format!("show the editable fields for: {}", names(slots, pending))
        } else {
            format!("show the fields for: {}", names(slots, pending))
        };
        match self.accomplish(log, &reveal, REVEAL_TURNS).await {
            Err(Halt::Failed(note)) => self
                .history
                .push(format!("could not reveal the fields ({note})")),
            other => {
                other?;
            }
        }
        Ok(())
    }

    /// Presses the opener of the first slot of `pending` whose own opener was
    /// not looked for yet (`opened`), noting each slot it looks for; `true`
    /// when it pressed one.
    pub(super) async fn open_next_box(
        &mut self,
        log: &mut StepLog,
        screen: &Screen,
        slots: &[Slot],
        pending: &BTreeSet<usize>,
        opened: &mut BTreeSet<usize>,
    ) -> Result<bool, Halt> {
        for index in pending.difference(opened).copied().collect::<Vec<_>>() {
            opened.insert(index);
            if self
                .open_by_name(log, screen, slots, &BTreeSet::from([index]))
                .await?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Words of a slot's name that say only that it is a box, or name a kind
/// of control rather than what the slot is.
const BOX_WORDS: &[&str] = &[
    "field", "box", "input", "bar", "the", "your", "text", "here", "link", "button", "menu", "icon",
];

/// Short words of a place slot ("to") that name its box only as the first
/// word of a control's label ("To BLR, Bengaluru"): anywhere else in a
/// label, "to" is any sentence's ("Tap to add a return date").
const PLACE_LEADS: &[&str] = &["to", "via"];

/// A control on `screen` that is no field itself and whose label holds a
/// word of a pending slot's name (four letters or more, not a box word)
/// among its first three words, or begins with its short place word
/// ([`PLACE_LEADS`]): the link or button that shows the slot's field, such
/// as a store's search link for the slot "search box", or a flight form's
/// "To BLR" for the slot "to". Further into a label, the slot's word is a
/// sentence's ("Read our tips to search faster"), and a press there leaves
/// the form.
///
/// Of several, the first, in document order, after ranking: one on screen
/// before one scrolled away, then the one holding more of the slot's words
/// ("Check-in …" over "Check availability" for "check-in date"), then the
/// one holding the slot's first word (its most particular: "Departure …"
/// over "Date Change" for "departure date", "From DEL" over "Multi City"
/// for "from city"), then one that wraps no other match (a button wrapping
/// the whole form names every box inside it). Live, for "from city" the
/// "Multi City" tab came first and was refused; the `do` loop that followed
/// pressed the button wrapping the whole form ("From DEL … To BLR …
/// Departure … Return …") at its centre, twice, and picked a return date
/// that made a one-way search a round trip.
pub(in crate::agentic::flow) fn named_opener(
    screen: &Screen,
    slots: &[Slot],
    pending: &BTreeSet<usize>,
) -> Option<Candidate> {
    let named = pending
        .iter()
        .filter_map(|index| slots.get(*index))
        .flat_map(|slot| words(&slot.slot))
        .collect::<Vec<_>>();
    let wanted = named
        .iter()
        .filter(|word| word.chars().count() > 3 && !BOX_WORDS.contains(&word.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    let leads = named
        .iter()
        .filter(|word| PLACE_LEADS.contains(&word.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    if wanted.is_empty() && leads.is_empty() {
        return None;
    }
    let fields = editable(screen);
    let matches = screen
        .candidates
        .iter()
        .filter(|candidate| {
            ["link", "button"]
                .iter()
                .any(|role| candidate.role.eq_ignore_ascii_case(role))
                && candidate
                    .available_actions
                    .iter()
                    .any(|action| action == "Click")
                && !fields.iter().any(|field| field.ref_id == candidate.ref_id)
                && !candidate
                    .states
                    .iter()
                    .any(|state| state.eq_ignore_ascii_case("covered"))
        })
        .filter_map(|candidate| {
            let label = words(candidate.name.as_deref().unwrap_or_default());
            let opens = |word: &String| {
                (wanted.contains(word) && label.iter().take(3).any(|said| said == word))
                    || (leads.contains(word) && label.first() == Some(word))
            };
            let first = named.iter().position(opens)?;
            let held = named
                .iter()
                .filter(|word| !BOX_WORDS.contains(&word.as_str()))
                .filter(|word| label.iter().take(3).any(|said| said == *word))
                .count();
            Some((candidate, label, first, held))
        })
        .collect::<Vec<_>>();
    let wraps = |label: &[String]| {
        let outer = format!(" {} ", label.join(" "));
        matches.iter().any(|(_, inner, _, _)| {
            inner.len() >= 2
                && inner.len() < label.len()
                && outer.contains(&format!(" {} ", inner.join(" ")))
        })
    };
    matches
        .iter()
        .min_by_key(|(candidate, label, first, held)| {
            let away = candidate
                .states
                .iter()
                .any(|state| state.eq_ignore_ascii_case("offscreen"));
            (away, std::cmp::Reverse(*held), *first, wraps(label))
        })
        .map(|(candidate, ..)| (*candidate).clone())
}

/// The lower-case words of `text`.
fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}
