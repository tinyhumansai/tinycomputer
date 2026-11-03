//! Delivering slots: filling every pending slot a field or an option can
//! take, each text verified by reading it back.

use std::collections::BTreeSet;

use serde_json::Value;
use tinycomputer_bus::Slot;
use tinycomputer_core::reformat_date;

use crate::agentic::flow::{
    AgentBackend, FlowRun, Halt, StepLog,
    backend::deliver_text,
    memory::{learn, remember},
    steps::looks_like_date,
    view::{Candidate, Screen, element_kind, label},
};

use super::{BLIND_PICK_MISSES, editable, names};

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Picks each pending date of `untried` from the calendar showing,
    /// before any box is looked for, and says whether one arrived: the box
    /// looked for is the calendar's own button, and pressing it closes the
    /// calendar. Live, a check-out date's step found the calendar open from
    /// the check-in and pressed its button four times, and a departure's
    /// calendar opened by the step's own press was closed again the same way.
    async fn pick_shown_dates(
        &mut self,
        log: &mut StepLog,
        slots: &[Slot],
        private: &[bool],
        pending: &mut BTreeSet<usize>,
        untried: &mut BTreeSet<usize>,
    ) -> Result<bool, Halt> {
        let dates = pending
            .iter()
            .copied()
            .filter(|index| untried.contains(index) && looks_like_date(&slots[*index].text))
            .collect::<Vec<_>>();
        let mut picked = false;
        for index in dates {
            untried.remove(&index);
            let slot = &slots[index];
            match self
                .pick_option(log, &slot.slot, &slot.text, private[index], false)
                .await
            {
                Ok(_) => {
                    pending.remove(&index);
                    picked = true;
                }
                Err(Halt::Failed(_)) => {}
                Err(halt) => return Err(halt),
            }
        }
        Ok(picked)
    }

    /// Fills every slot in `pending` it can find a field or an option for,
    /// removing each one that arrives.
    pub(super) async fn fill_pending(
        &mut self,
        log: &mut StepLog,
        slots: &[Slot],
        private: &[bool],
        pending: &mut BTreeSet<usize>,
    ) -> Result<(), Halt> {
        let mut revealed = false;
        // The dates not yet picked from a calendar showing, each tried once.
        let mut dates = pending.clone();
        // The slots whose own opener was looked for, each once: a form that
        // draws its place boxes as buttons ("From DEL", "To BLR") opens one
        // box at a time. Live, the "to" box was never opened once "from"
        // had been, and the place was pressed in a link at the foot of the
        // page instead.
        let mut opened: BTreeSet<usize> = BTreeSet::new();
        // The fields this step already filled: one slot's box is never
        // another's. Live, a pickup box that had not yet become the place
        // chosen was the only box on screen in the next round, and the drop
        // was typed over the pickup.
        let mut filled_fields: BTreeSet<String> = BTreeSet::new();
        let mut filled_texts: Vec<String> = Vec::new();
        let mut saw_fields = false;
        // Fields that refused the text this step: a `div` a page labels a
        // combobox, or a field that would not hold what was typed. Offered
        // again, the same wrong field wins again — struck by `element_kind`
        // so the rows of a city list, each a same-kind box that differs only
        // by the city it holds, are struck together rather than one at a
        // time (`a_row_that_refused_the_text_is_never_pressed_while_revealing_a_field`).
        let mut struck: BTreeSet<String> = BTreeSet::new();
        // Each slot may take a round to open its box and one to fill it.
        for _ in 0..3.max(2 * pending.len() + 1) {
            if pending.is_empty() {
                break;
            }
            let mut screen = self.look().await?;
            if self.front.calendar
                && self
                    .pick_shown_dates(log, slots, private, pending, &mut dates)
                    .await?
            {
                continue;
            }
            if editable(&screen).len() < pending.len() && !screen.unexplored.is_empty() {
                self.explore(&mut screen).await;
            }
            let fields = unfilled(editable(&screen), &struck, &filled_fields, &filled_texts);
            saw_fields |= !fields.is_empty();
            let assignments = if fields.is_empty() {
                Vec::new()
            } else {
                self.assign(log, &screen, slots, pending, &fields).await?
            };
            if assignments.is_empty() {
                if self
                    .open_next_box(log, &screen, slots, pending, &mut opened)
                    .await?
                {
                    continue;
                }
                if revealed {
                    break;
                }
                revealed = true;
                self.reveal_fields(log, slots, pending, fields.is_empty())
                    .await?;
                continue;
            }
            for assignment in assignments {
                let slot = &slots[assignment.slot];
                let filled = self
                    .fill(
                        log,
                        slot,
                        &assignment.field,
                        &screen,
                        private[assignment.slot],
                    )
                    .await?;
                if !filled {
                    struck.insert(element_kind(&assignment.field));
                    self.refused.insert(element_kind(&assignment.field));
                    self.ledger.tried(format!(
                        "{} did not take the {}",
                        label(&assignment.field),
                        slot.slot
                    ));
                }
                if filled {
                    filled_fields.insert(assignment.field.ref_id.clone());
                    filled_texts.push(slot.text.split_whitespace().collect::<Vec<_>>().join(" "));
                    pending.remove(&assignment.slot);
                    learn(
                        &mut self.learned,
                        remember(&self.app, &slot.slot, &assignment.field),
                    );
                    log.confidence = Some(log.confidence.map_or(assignment.probability, |seen| {
                        seen.min(assignment.probability)
                    }));
                }
            }
        }
        self.pick_unboxed(log, slots, private, pending, saw_fields)
            .await
    }

    /// Picks each slot still `pending` instead of typing it, as a date from a
    /// calendar or a city from a list of suggestions, when no field took it.
    /// Only a "the value was not found" failure is safe to shrug off and move
    /// to the next slot; a budget stop or a backend error means acting
    /// further is unsafe or pointless, and must end the step instead of being
    /// read as "this slot has no picker".
    async fn pick_unboxed(
        &mut self,
        log: &mut StepLog,
        slots: &[Slot],
        private: &[bool],
        pending: &mut BTreeSet<usize>,
        saw_fields: bool,
    ) -> Result<(), Halt> {
        let mut missed = 0;
        for index in pending.clone() {
            if !saw_fields && missed >= BLIND_PICK_MISSES {
                self.history.push(format!(
                    "no field and no picker on this screen; not looking for: {}",
                    names(slots, pending)
                ));
                break;
            }
            let slot = &slots[index];
            match self
                .pick_option(log, &slot.slot, &slot.text, private[index], false)
                .await
            {
                Ok(_) => {
                    pending.remove(&index);
                }
                Err(Halt::Failed(_)) => missed += 1,
                Err(halt) => return Err(halt),
            }
        }
        Ok(())
    }

    /// Delivers one slot's text and reports whether it verifiably arrived.
    ///
    /// A date is typed in the layout the field or the page around it asks
    /// for ("DD-MM-YYYY"), so an input mask does not mangle it. Text that
    /// arrived and opened a list of suggestions has the matching one picked
    /// (`commit_suggestion`), since an autocomplete box keeps it only then; a
    /// `private` text is never offered there, as Jev would see it.
    async fn fill(
        &mut self,
        log: &mut StepLog,
        slot: &Slot,
        field: &Candidate,
        before: &Screen,
        private: bool,
    ) -> Result<bool, Halt> {
        let handle_names = crate::agentic::flow::validate::references(&slot.text)
            .into_iter()
            .filter(|name| self.fact_handles.contains_key(name))
            .collect::<Vec<_>>();
        if !handle_names.is_empty() {
            if handle_names.len() != 1 || slot.text != format!("${{{}}}", handle_names[0]) {
                return Err(Halt::Failed(
                    "a host secret must occupy the entire field value".to_owned(),
                ));
            }
            self.type_handle(log, &handle_names[0], field).await?;
            return Ok(true);
        }
        let app = self.app.clone();
        let target = field.clone();
        let hints = [field.name.as_deref(), field.description.as_deref()]
            .into_iter()
            .flatten()
            .chain(before.context.iter().map(String::as_str));
        let text = reformat_date(&slot.text, hints).unwrap_or_else(|| slot.text.clone());
        let typed = text.clone();
        let reply = self
            .act(
                log,
                &format!("fill {}", slot.slot),
                Some(field),
                move |backend| deliver_text(&backend, &app, &target, &text),
            )
            .await?;
        self.typed.insert(element_kind(field));
        let path = reply
            .data
            .as_ref()
            .and_then(|data| data.get("path"))
            .cloned()
            .unwrap_or(Value::Null);
        self.history.push(format!(
            "entered the {} into {} ok={} via {path}",
            slot.slot,
            label(field),
            reply.ok
        ));
        if reply.ok && !private {
            self.commit_suggestion(log, &slot.slot, &typed, field, before)
                .await?;
        }
        Ok(reply.ok)
    }
}

/// The `fields` a slot may still take: of no kind struck this step, and
/// neither filled this step (`filled`, by ref) nor holding a text this step
/// delivered (`delivered`).
fn unfilled(
    fields: Vec<Candidate>,
    struck: &BTreeSet<String>,
    filled: &BTreeSet<String>,
    delivered: &[String],
) -> Vec<Candidate> {
    fields
        .into_iter()
        .filter(|field| {
            !struck.contains(&element_kind(field))
                && !filled.contains(&field.ref_id)
                && !holds_one_of(field, delivered)
        })
        .collect()
}

/// Whether `field` holds one of the texts this step already delivered:
/// one slot's box, read again under a new ref.
fn holds_one_of(field: &Candidate, delivered: &[String]) -> bool {
    field
        .value
        .as_ref()
        .and_then(Value::as_str)
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .is_some_and(|value| !value.is_empty() && delivered.contains(&value))
}
