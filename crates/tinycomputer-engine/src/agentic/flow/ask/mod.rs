//! Jev question builders and answer readers shared by every decision loop.
//!
//! Each loop asks small questions: one Noul, one Score, or one Choice over at
//! most [`CAP`] options. Independent questions about the same screen share one
//! request, because they share one `state`.
//!
//! `screen_state` builds the shared state, `questions` the questions, and
//! `answers` reads what comes back.

mod answers;
mod questions;
mod screen_state;

pub(super) use answers::{calibrated, chosen, combined, deferred, level, probability, top_level};
pub(super) use questions::{
    asks_for, belongs, completion, condition, corroborate, coverage, elements, field_error, helped,
    intended, negated, obstacle, only_near, options, page_kind, progress, reflects, strays,
    unfinished, unintended, viewed,
};
pub(super) use screen_state::{ordered_nodes, rich_text, state};

use std::collections::BTreeMap;

use serde_json::Value;
use tinyinference_decisions::{EvaluationRequest, Question};

/// Most options one Choice offers before narrowing takes over.
pub(super) const CAP: usize = 20;
/// Most pieces of text a `read` step chooses among.
pub(super) const MAX_READ_SOURCES: usize = 60;
/// Most element labels described in the shared state.
const MAX_STATE_ELEMENTS: usize = 120;
/// Recent history lines shared with Jev.
const MAX_HISTORY: usize = 20;
/// Most fields shown in `field_contents`.
const MAX_FIELDS: usize = 12;

pub(super) fn request(model: &str, state: Value, questions: Questions) -> EvaluationRequest {
    EvaluationRequest {
        state,
        model: model.to_owned(),
        questions: questions.0,
        assets: BTreeMap::new(),
    }
}

/// Named questions for one request.
#[derive(Debug, Default)]
pub(super) struct Questions(pub(super) BTreeMap<String, Question>);

impl Questions {
    pub(super) fn with(mut self, id: &str, question: Question) -> Self {
        self.0.insert(id.to_owned(), question);
        self
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `1`..=`n`: the keys a first Choice uses.
pub(super) fn numbered(count: usize) -> Vec<String> {
    (1..=count).map(|index| index.to_string()).collect()
}

/// `A`, `B`, …, `AA`: distinct keys for a relabelled re-ask.
pub(super) fn lettered(count: usize) -> Vec<String> {
    (0..count)
        .map(|mut index| {
            let mut key = String::new();
            loop {
                key.insert(0, char::from(b'A' + u8::try_from(index % 26).unwrap_or(0)));
                if index < 26 {
                    break key;
                }
                index = index / 26 - 1;
            }
        })
        .collect()
}
