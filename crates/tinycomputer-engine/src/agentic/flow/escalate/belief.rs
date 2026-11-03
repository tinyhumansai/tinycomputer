//! The ladder for a yes/no judgement: more framings, and at the deep level
//! other views of the screen, until the belief settles.

use std::collections::BTreeMap;
use std::time::Instant;

use serde_json::json;
use tinycomputer_bus::{FlowLoop, JevExchange};
use tinyinference_decisions::{Answer, EvaluationRequest};

use crate::agentic::flow::{
    AgentBackend, FlowRun, Halt, StepLog, decide,
    evidence::{self, Verdict},
    vote,
};

use super::Belief;

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Asks `request` again in the framings it was not asked in yet, up to
    /// [`vote::MAX_VOTES`] and within the budget, and returns every one of
    /// its questions re-tallied over the whole ballot. `None` when there is
    /// no framing left to ask or no budget to ask it with.
    pub(in crate::agentic::flow) async fn widen(
        &mut self,
        log: &mut StepLog,
        request: &EvaluationRequest,
    ) -> Result<Option<BTreeMap<String, Answer>>, Halt> {
        let asked = request
            .questions
            .keys()
            .map(|id| self.asked_in(id))
            .max()
            .unwrap_or_default();
        let from = u32::try_from(asked).unwrap_or(u32::MAX);
        let to = vote::MAX_VOTES.min(from.saturating_add(self.room()));
        if from >= to || !self.enabled(FlowLoop::Vote) {
            return Ok(None);
        }
        let parts = self.outgoing(log, request.clone());
        let framings = parts
            .iter()
            .flat_map(|part| vote::framings_between(part, from, to))
            .collect::<Vec<_>>();
        let votes = u32::try_from(framings.len() / parts.len().max(1)).unwrap_or(u32::MAX);
        let handles = self.spawn(&framings);
        self.rounds = self.rounds.saturating_add(1);
        self.decisions = self.decisions.saturating_add(1);
        let asked_at = Instant::now();
        let mut answered = Vec::new();
        for (framing, handle) in framings.into_iter().zip(handles) {
            if let Ok(Ok(evaluation)) = handle.await {
                crate::agentic::flow::merge_metrics(&mut self.metrics, &evaluation);
                log.calls = log.calls.saturating_add(1);
                answered.push((framing, evaluation.response.answers));
            }
        }
        if answered.is_empty() {
            return Ok(None);
        }
        // Widening is its own decision — an extra rung asked outside
        // `FlowRun::ask_batch` — so it journals and traces exactly as that
        // shared path does: a `decision` event with this round's framings,
        // and a `JevExchange` with this round's own (not the accumulated)
        // answers, when tracing.
        let prepared = decide::whole(&parts);
        let fresh = vote::ballots(&answered);
        for (id, ballot) in fresh.clone() {
            self.ballots.entry(id).or_default().extend(ballot);
        }
        let widened_to = usize::try_from(to).unwrap_or(usize::MAX);
        for id in parts.iter().flat_map(|part| part.questions.keys()) {
            self.asked.insert(id.clone(), widened_to);
        }
        self.runtime.journal.record("decision", || {
            json!({
                "step": self.step,
                "questions": prepared.questions.keys().collect::<Vec<_>>(),
                "framings": votes,
                "answered": answered.len(),
                "batched": 1,
                "parts": parts.len(),
                "request_bytes": decide::largest(&parts),
                "wall_ms": crate::agentic::journal::millis(asked_at.elapsed()),
            })
        });
        if self.tracing {
            let mut answers = serde_json::to_value(vote::tally(&fresh)).unwrap_or_default();
            super::super::secret::redact_value(&mut answers, &|text| self.redact_text(text));
            self.trace.push(JevExchange {
                step: self.step.clone(),
                state: prepared.state,
                questions: serde_json::to_value(&prepared.questions).unwrap_or_default(),
                answers,
            });
        }
        let ballots = request
            .questions
            .keys()
            .filter_map(|id| Some((id.clone(), self.ballots.get(id)?.clone())))
            .collect::<BTreeMap<_, _>>();
        Ok(Some(vote::tally(&ballots)))
    }

    /// Settles a yes/no judgement `answers` hold for `request`: taken as it
    /// reads when its framings agree clearly, otherwise widened and, at the
    /// deep level, asked again over `views` — requests over other
    /// renderings of the screen asking `belief.yes` and `belief.no`.
    ///
    /// Returns the settled belief; `answers` is updated with whatever the
    /// widening re-tallied, so the rest of the request (a move, a progress
    /// level) is read from the larger ballot too.
    pub(in crate::agentic::flow) async fn settle_belief(
        &mut self,
        log: &mut StepLog,
        belief: Belief<'_>,
        request: &EvaluationRequest,
        answers: &mut BTreeMap<String, Answer>,
        views: Vec<EvaluationRequest>,
    ) -> Result<Option<f64>, Halt> {
        let Some(mut held) = belief.read(answers) else {
            return Ok(None);
        };
        if !self.deliberates(FlowLoop::Evidence) {
            return Ok(Some(held));
        }
        let mut framed = self.framed(&belief);
        if framed.is_empty() {
            framed.push(held);
        }
        let weighed = evidence::of_beliefs(&framed, belief.threshold);
        let verdict = evidence::belief_verdict(&weighed, belief.threshold);
        self.weighed(log, belief.site, &weighed, verdict);
        if verdict == Verdict::Accept || !self.enabled(FlowLoop::Escalation) {
            return Ok(Some(held));
        }
        if let Some(widened) = self.widen(log, request).await? {
            answers.extend(widened);
            held = belief.read(answers).unwrap_or(held);
            let weighed = evidence::of_beliefs(&self.framed(&belief), belief.threshold);
            let verdict = evidence::belief_verdict(&weighed, belief.threshold);
            self.climbed(log, belief.site, "framings", Some(verdict));
            self.weighed(log, belief.site, &weighed, verdict);
            if verdict == Verdict::Accept {
                return Ok(Some(held));
            }
        }
        // Views guard a pass: a judgement that would not clear its bar has
        // nothing for them to veto, and pulling it lower only overrules what
        // else rests on it — Jev's own "finished" at `LEANS_DONE`.
        if !self.deep() || views.is_empty() || self.room() == 0 || held < belief.threshold {
            return Ok(Some(held));
        }
        let seen = self.ask_batch(log, views).await?;
        let mut readings = vec![held];
        readings.extend(seen.iter().filter_map(|answers| belief.read_view(answers)));
        let settled = evidence::median(&readings);
        self.climbed(
            log,
            belief.site,
            "views",
            Some(if settled >= belief.threshold {
                Verdict::Accept
            } else {
                Verdict::Deliberate
            }),
        );
        self.runtime.journal.record("views", || {
            json!({
                "step": self.step,
                "site": belief.site,
                "readings": readings,
                "settled": settled,
            })
        });
        Ok(Some(settled))
    }
}
