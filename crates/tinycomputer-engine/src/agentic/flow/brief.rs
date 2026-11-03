//! The run's brief: the goal, the plan, and what has been chosen so far,
//! added to every question that chooses.

use serde_json::{Value, json};
use tinyinference_decisions::{EvaluationRequest, Question};

use super::{
    FlowRun, MAX_GOAL, MAX_PLAN_LINE, MAX_SO_FAR, MAX_SO_FAR_NOTE, backend::AgentBackend,
    decide::PAGE_KIND,
};

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Adds the run's brief — the goal, whom it is for, the plan with this
    /// step marked, what has been chosen so far, and the kind of page showing
    /// — to the questions that choose: which element, option, move, field,
    /// or record, and whether an element is the right one.
    ///
    /// A yes/no judgement of the screen (is the step done, does a condition
    /// hold, is something in the way) is left without it. Measured on a live
    /// results page, the brief pulled Jev's "is the search done?" from 0.75
    /// down to 0.39: it judged the step against the whole task.
    pub(super) fn brief_into(&self, request: &mut EvaluationRequest) {
        let Some(brief) = self.brief() else {
            return;
        };
        for (id, question) in &mut request.questions {
            let instructions = match question {
                Question::Choice(choice) if id != PAGE_KIND => &mut choice.instructions,
                Question::Noul(noul)
                    if BRIEFED_NOULS.contains(&id.as_str())
                        || id.starts_with("is_")
                        || id.starts_with("only_near_") =>
                {
                    &mut noul.instructions
                }
                _ => continue,
            };
            if let Value::Object(fields) = instructions {
                fields.insert("brief".to_owned(), brief.clone());
            }
        }
    }

    /// The brief as Jev reads it, or `None` when there is nothing to say.
    fn brief(&self) -> Option<Value> {
        let current = self
            .step
            .split('.')
            .next()
            .and_then(|top| top.parse::<usize>().ok())
            .unwrap_or(0);
        let plan = self
            .outline
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let mark = match (index + 1).cmp(&current) {
                    std::cmp::Ordering::Less => "done",
                    std::cmp::Ordering::Equal => "now",
                    std::cmp::Ordering::Greater => "next",
                };
                clip(&format!("{}. [{mark}] {step}", index + 1), MAX_PLAN_LINE)
            })
            .collect::<Vec<_>>();
        let mut brief = serde_json::Map::new();
        if !self.brief.goal.is_empty() {
            brief.insert("goal".to_owned(), json!(clip(&self.brief.goal, MAX_GOAL)));
        }
        if !self.brief.details.is_empty() {
            brief.insert("for".to_owned(), json!(self.brief.details));
        }
        if !self.brief.secrets.is_empty() {
            brief.insert(
                "secrets".to_owned(),
                json!({
                    "note": "Held locally and typed by the module; you only ever see them as these names.",
                    "names": self.brief.secrets.iter().map(|name| format!("${{{name}}}")).collect::<Vec<_>>(),
                }),
            );
        }
        if !self.brief.rules.is_empty() {
            brief.insert("rules".to_owned(), json!(self.brief.rules));
        }
        if plan.len() > 1 {
            brief.insert("plan".to_owned(), json!(plan));
        }
        if !self.so_far.is_empty() {
            brief.insert(
                "so_far".to_owned(),
                json!(
                    self.so_far
                        .iter()
                        .map(|note| clip(note, MAX_SO_FAR_NOTE))
                        .collect::<Vec<_>>()
                ),
            );
        }
        if let Some(page) = &self.page {
            brief.insert("page".to_owned(), json!(page));
        }
        (!brief.is_empty()).then_some(Value::Object(brief))
    }

    /// Notes something the run chose or entered, for the brief's `so_far`.
    pub(in crate::agentic::flow) fn remember_choice(&mut self, note: &str) {
        self.so_far.push(self.redact_text(note));
        if self.so_far.len() > MAX_SO_FAR {
            self.so_far.remove(0);
        }
    }
}

/// The yes/no questions that are about choosing, not judging the screen:
/// whether an element is the right one for a purpose.
const BRIEFED_NOULS: &[&str] = &["confirm"];

/// `text` cut to `limit` characters, marked with `…` when it was cut.
pub(super) fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut clipped = text.chars().take(limit).collect::<String>();
    clipped.push('…');
    clipped
}
