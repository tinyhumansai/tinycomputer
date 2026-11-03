//! Asking Jev: every request is briefed, masked, fitted to size, and voted
//! on through one door.

use std::{collections::BTreeMap, time::Instant};

use serde_json::{Value, json};
use tinycomputer_bus::{FlowLoop, FlowStopReason, JevExchange};
use tinycomputer_core::Facts;
use tinyinference_decisions::{Answer, EvaluationRequest, Question};

use super::{
    FlowRun, Halt, MAX_REQUEST_BYTES, StepLog, ask, backend::AgentBackend, brief::clip, hedge,
    quorum, vote,
};
use crate::agentic::{journal::millis, merge_metrics, provider_error};

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Asks Jev one request, charging it to the run and the step.
    ///
    /// The request is briefed and masked first, then asked in as many
    /// framings as the run votes with — concurrently, each one charged as an
    /// evaluation — and the answers are averaged: every framing's, or those
    /// in once all but two of seven or more agree plainly (`quorum.rs`). On
    /// a web page it also carries a page-kind question, whose answer briefs
    /// the next request.
    pub(in crate::agentic::flow) async fn ask(
        &mut self,
        log: &mut StepLog,
        request: EvaluationRequest,
    ) -> Result<BTreeMap<String, Answer>, Halt> {
        let mut answers = self.ask_batch(log, vec![request]).await?;
        answers
            .pop()
            .ok_or_else(|| Halt::Failed("no Jev evaluation completed".to_owned()))
    }

    /// Asks Jev several independent requests at once: one round trip, not
    /// one per request. Each is briefed, masked, fitted, and voted on
    /// exactly as [`FlowRun::ask`] would, every framing of every request is
    /// in flight together, and the answers come back in request order.
    ///
    /// The first request is the one the caller needs; the rest may be
    /// speculative. When the budget has no room for all of them at full
    /// votes, the batch is cut from the end — never below the first — so the
    /// reply may be shorter than `requests`.
    pub(in crate::agentic::flow) async fn ask_batch(
        &mut self,
        log: &mut StepLog,
        mut requests: Vec<EvaluationRequest>,
    ) -> Result<Vec<BTreeMap<String, Answer>>, Halt> {
        if self.metrics.calls >= self.max_calls {
            return Err(Halt::Stop(FlowStopReason::ModelBudget));
        }
        let room = self.max_calls - self.metrics.calls;
        let votes = if self.enabled(FlowLoop::Vote) {
            self.votes.max(1)
        } else {
            1
        };
        let affordable = usize::try_from(room / votes).unwrap_or(usize::MAX).max(1);
        requests.truncate(affordable);
        let votes = votes.min(room);
        if votes > 1 {
            log.used(FlowLoop::Vote);
        }
        let asked = self.send(log, requests, room, votes);
        let batched = asked.len();
        self.rounds = self.rounds.saturating_add(1);
        let asked_at = Instant::now();
        let mut replies = Vec::with_capacity(batched);
        for (parts, framings, handles) in asked {
            self.decisions = self.decisions.saturating_add(1);
            let mut answered = Vec::new();
            let mut failure = None;
            // A part none of whose framings answered leaves its questions
            // without an answer, which fails the decision as a whole.
            let mut unanswered = false;
            let mut left = 0_u32;
            let mut asked_in = BTreeMap::new();
            for ((part, framings), handles) in parts.iter().zip(framings).zip(handles) {
                let before = answered.len();
                for id in part.questions.keys() {
                    asked_in.insert(id.clone(), framings.len());
                }
                let size = quorum::size(framings.len());
                let gathered = quorum::gather(framings, handles, size).await;
                for (framing, evaluation) in gathered.answered {
                    merge_metrics(&mut self.metrics, &evaluation);
                    log.calls = log.calls.saturating_add(1);
                    answered.push((framing, evaluation.response.answers));
                }
                // Framings a quorum did not wait for still run, and are
                // charged as made: their tokens are not known yet.
                self.metrics.calls = self.metrics.calls.saturating_add(gathered.left);
                log.calls = log.calls.saturating_add(gathered.left);
                left = left.saturating_add(gathered.left);
                if let Some(error) = gathered.failure {
                    failure.get_or_insert(error);
                }
                unanswered |= answered.len() == before;
            }
            let request = whole(&parts);
            let answers = match (unanswered, failure) {
                (true, Some(failure)) => return Err(Halt::Error(provider_error(&failure))),
                (true, None) => {
                    return Err(Halt::Failed("no Jev evaluation completed".to_owned()));
                }
                _ => {
                    let ballots = vote::ballots(&answered);
                    let merged = vote::tally(&ballots);
                    self.ballots.extend(ballots);
                    self.asked.extend(asked_in);
                    merged
                }
            };
            // The decision's wall time: its framings run at once, and the
            // batch's requests with them, so this is the slowest framing so
            // far plus the merge — what the step waited for this answer.
            self.runtime.journal.record("decision", || {
                json!({
                    "step": self.step,
                    "questions": request.questions.keys().collect::<Vec<_>>(),
                    "framings": votes,
                    "answered": answered.len(),
                    "left": left,
                    "batched": batched,
                    "parts": parts.len(),
                    "request_bytes": largest(&parts),
                    "wall_ms": millis(asked_at.elapsed()),
                })
            });
            if self.tracing {
                let mut answers = serde_json::to_value(&answers).unwrap_or_default();
                super::secret::redact_value(&mut answers, &|text| self.redact_text(text));
                self.trace.push(JevExchange {
                    step: self.step.clone(),
                    state: request.state.clone(),
                    questions: serde_json::to_value(&request.questions).unwrap_or_default(),
                    answers,
                });
            }
            if let Some((kind, _)) = ask::chosen(&answers, PAGE_KIND) {
                self.page = Some(kind);
            }
            replies.push(answers);
        }
        Ok(replies)
    }

    /// Sends each of `requests` in its parts, every part in `votes`
    /// framings at once, within `room` calls. A request asked in parts costs
    /// a call per part and framing: the budget is charged for every one, and
    /// a speculative request that would run past it is left out, never the
    /// first, which asks fewer framings instead.
    fn send(
        &self,
        log: &mut StepLog,
        requests: Vec<EvaluationRequest>,
        room: u32,
        votes: u32,
    ) -> Vec<Sent> {
        let mut asked = Vec::with_capacity(requests.len());
        let mut spent = 0_u32;
        for (index, request) in requests.into_iter().enumerate() {
            let parts = self.outgoing(log, request);
            let count = u32::try_from(parts.len()).unwrap_or(u32::MAX).max(1);
            let votes = if index == 0 {
                votes.min((room / count).max(1))
            } else {
                votes
            };
            let cost = count.saturating_mul(votes);
            if index > 0 && spent.saturating_add(cost) > room {
                break;
            }
            spent = spent.saturating_add(cost);
            let framings = parts
                .iter()
                .map(|part| vote::framings(part, votes))
                .collect::<Vec<_>>();
            let handles = framings
                .iter()
                .map(|framings| self.spawn(framings))
                .collect::<Vec<_>>();
            asked.push((parts, framings, handles));
        }
        asked
    }

    /// `request` as it leaves for Jev: with the page-kind question on a web
    /// page, briefed, masked, and fitted to size, in parts when its
    /// questions outgrow one request ([`split`]).
    pub(super) fn outgoing(
        &self,
        log: &mut StepLog,
        mut request: EvaluationRequest,
    ) -> Vec<EvaluationRequest> {
        if self.enabled(FlowLoop::PageKind) && self.app == crate::workspace::BROWSER {
            log.used(FlowLoop::PageKind);
            request
                .questions
                .insert(PAGE_KIND.to_owned(), ask::page_kind());
        }
        self.brief_into(&mut request);
        self.mask(&mut request);
        clip_masked_state(&mut request.state);
        split(request, MAX_REQUEST_BYTES)
            .into_iter()
            .map(|mut part| {
                fit(&mut part, MAX_REQUEST_BYTES);
                part
            })
            .collect()
    }

    /// Sends every framing to Jev at once, each one [`hedged`](hedge::hedged).
    pub(super) fn spawn(
        &self,
        framings: &[vote::Framing],
    ) -> Vec<
        tokio::task::JoinHandle<
            Result<
                tinyinference_decisions::EvaluationResult,
                tinyinference_decisions::EvaluationFailure,
            >,
        >,
    > {
        framings
            .iter()
            .map(|framing| {
                let runtime = self.runtime.clone();
                let step = self.step.clone();
                let request = framing.request.clone();
                tokio::spawn(async move { hedge::hedged(&runtime, &step, &request).await })
            })
            .collect()
    }

    /// Masks every secret out of a request, wherever it appears.
    fn mask(&self, request: &mut EvaluationRequest) {
        if self.secrets.secret_names().is_empty() && self.secret_host.is_none() {
            return;
        }
        mask_value(&mut request.state, &self.secrets);
        for question in request.questions.values_mut() {
            let mut value = serde_json::to_value(&*question).unwrap_or_default();
            mask_value(&mut value, &self.secrets);
            super::secret::redact_value(&mut value, &|text| self.redact_text(text));
            if let Ok(masked) = serde_json::from_value(value) {
                *question = masked;
            }
        }
        super::secret::redact_value(&mut request.state, &|text| self.redact_text(text));
    }
}

/// One request as sent: its parts, each part's framings, and the tasks
/// evaluating them.
type Sent = (
    Vec<EvaluationRequest>,
    Vec<Vec<vote::Framing>>,
    Vec<
        Vec<
            tokio::task::JoinHandle<
                Result<
                    tinyinference_decisions::EvaluationResult,
                    tinyinference_decisions::EvaluationFailure,
                >,
            >,
        >,
    >,
);

/// The id of the page-kind question a request on a web page carries.
pub(super) const PAGE_KIND: &str = "page_kind";

/// `request` cut by its questions into requests of at most `limit` bytes of
/// JSON, each carrying the whole state and as many of the questions, in
/// order, as fit beside it. Jev evaluates every question on its own against
/// the state, so the parts ask exactly what the whole would have, and their
/// answers merge back by question id; shrinking the request instead would
/// cut the screen and the brief. A request that fits, or holds one question,
/// stays whole, and a question too large to share a part goes alone, for
/// [`fit`] to shrink.
pub(in crate::agentic::flow) fn split(
    request: EvaluationRequest,
    limit: usize,
) -> Vec<EvaluationRequest> {
    if request.questions.len() < 2 || bytes(&request) <= limit {
        return vec![request];
    }
    let EvaluationRequest {
        state,
        model,
        questions,
    } = request;
    let mut empty = EvaluationRequest {
        state,
        model,
        questions: BTreeMap::new(),
    };
    // A screen that fills most of a part by itself would leave room for one
    // question each, and every question would cost its own call: the
    // screen's longest lists are cut to half the limit first.
    while bytes(&empty) > limit / 2 {
        let Some(longest) = longest_list(&mut empty.state) else {
            break;
        };
        let cut = (longest.len() / 4).max(1);
        longest.truncate(longest.len() - cut);
    }
    let base = bytes(&empty);
    let mut parts = Vec::new();
    let mut part = empty.clone();
    let mut used = base;
    for (id, question) in questions {
        // `"id":{...},` in the part's JSON.
        let size = id.len() + 4 + serde_json::to_vec(&question).map_or(0, |json| json.len());
        if !part.questions.is_empty() && used + size > limit {
            parts.push(std::mem::replace(&mut part, empty.clone()));
            used = base;
        }
        used += size;
        part.questions.insert(id, question);
    }
    parts.push(part);
    parts
}

/// The request `parts` were cut from: the first part's state with every
/// part's questions, as the journal and the trace record a decision.
pub(in crate::agentic::flow) fn whole(parts: &[EvaluationRequest]) -> EvaluationRequest {
    let mut whole = parts.first().cloned().unwrap_or_else(|| EvaluationRequest {
        state: Value::Null,
        model: String::new(),
        questions: BTreeMap::new(),
    });
    for part in parts.iter().skip(1) {
        whole.questions.extend(part.questions.clone());
    }
    whole
}

/// The size of the largest of `parts`, in bytes of JSON.
pub(in crate::agentic::flow) fn largest(parts: &[EvaluationRequest]) -> usize {
    parts.iter().map(bytes).max().unwrap_or_default()
}

/// The size of `request`, in bytes of JSON.
pub(super) fn bytes(request: &EvaluationRequest) -> usize {
    serde_json::to_vec(request).map_or(0, |json| json.len())
}

/// Shrinks `request` until its JSON is at most `limit` bytes: first the
/// brief is kept on the first briefed question only, then the longest lists
/// of screen text and elements in the shared state lose their last entries.
/// What remains is the part of the screen read first.
pub(in crate::agentic::flow) fn fit(request: &mut EvaluationRequest, limit: usize) {
    let size =
        |request: &EvaluationRequest| serde_json::to_vec(request).map_or(0, |json| json.len());
    if size(request) <= limit {
        return;
    }
    let mut kept = false;
    for question in request.questions.values_mut() {
        let instructions = match question {
            Question::Choice(choice) => &mut choice.instructions,
            Question::Noul(noul) => &mut noul.instructions,
            Question::Score(score) => &mut score.instructions,
        };
        if let Value::Object(fields) = instructions
            && fields.contains_key("brief")
        {
            if kept {
                fields.remove("brief");
            }
            kept = true;
        }
    }
    while size(request) > limit {
        let Some(longest) = longest_list(&mut request.state) else {
            return;
        };
        let cut = (longest.len() / 4).max(1);
        longest.truncate(longest.len() - cut);
    }
}

/// The longest non-empty array anywhere in `value`.
fn longest_list(value: &mut Value) -> Option<&mut Vec<Value>> {
    let mut best: Option<&mut Vec<Value>> = None;
    let candidates: Vec<&mut Vec<Value>> = match value {
        Value::Array(items) => {
            if items
                .iter()
                .all(|item| !item.is_array() && !item.is_object())
            {
                return (!items.is_empty()).then_some(items);
            }
            items.iter_mut().filter_map(longest_list).collect()
        }
        Value::Object(fields) => fields.values_mut().filter_map(longest_list).collect(),
        _ => Vec::new(),
    };
    for candidate in candidates {
        if best
            .as_ref()
            .is_none_or(|best| candidate.len() > best.len())
        {
            best = Some(candidate);
        }
    }
    best
}

/// Every string inside `value` with the secrets masked.
fn mask_value(value: &mut Value, secrets: &Facts) {
    match value {
        Value::String(text) => *text = secrets.mask(text),
        Value::Array(items) => items.iter_mut().for_each(|item| mask_value(item, secrets)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| mask_value(field, secrets)),
        _ => {}
    }
}

/// Longest an `elements` line may be, once masked.
const MAX_ELEMENT_CHARS: usize = 96;
/// Longest a `field_contents` entry's `holds` may be, once masked.
const MAX_HELD_CHARS: usize = 400;

/// Clips the two places a held value can make `state` long — the `elements`
/// lines built by `element_line`, and `field_contents`'s `holds` — down to a
/// readable length.
///
/// Called only after [`FlowRun::mask`], never before: `Facts::mask` finds a
/// secret by its exact, whole value, and a value already cut short would
/// leave its unmasked prefix in the request instead of `${name}`.
fn clip_masked_state(state: &mut Value) {
    if let Some(elements) = untrusted_array_mut(state, "elements") {
        for element in elements {
            if let Value::String(text) = element {
                *text = clip(text, MAX_ELEMENT_CHARS);
            }
        }
    }
    if let Some(fields) = untrusted_array_mut(state, "field_contents") {
        for field in fields {
            if let Some(Value::String(held)) = field.get_mut("holds") {
                *held = clip(held, MAX_HELD_CHARS);
            }
        }
    }
}

/// `state[family]["untrusted_accessibility_data"]`, when it is an array.
fn untrusted_array_mut<'a>(state: &'a mut Value, family: &str) -> Option<&'a mut Vec<Value>> {
    state
        .get_mut(family)?
        .get_mut("untrusted_accessibility_data")?
        .as_array_mut()
}
