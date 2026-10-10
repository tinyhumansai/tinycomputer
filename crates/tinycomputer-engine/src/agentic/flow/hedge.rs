//! Hedging a framing: one still unanswered past its hedge delay is sent
//! again, and the first answer of the two counts.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;
use tinycomputer_bus::JevProvider;
use tinyinference_decisions::{EvaluationFailure, EvaluationRequest, EvaluationResult};

use super::decide::bytes;
use crate::agentic::{JevRuntime, journal::millis};

/// How long a framing runs before a copy of it is sent and the first answer
/// of the two taken. Live, one framing of a burst stalled 12–32 s (the
/// gateway gave up after ~10 s, or nothing came back before the client's
/// timeout) as its siblings answered in under 1 s. Calls on a slow evening
/// took up to 3.4 s and still answered: a copy sent at 2.5 s lost the race
/// 15 times in 16, so copies wait for 4 s, past what a slow answer takes.
const HEDGE_AFTER: Duration = Duration::from_secs(4);

/// [`HEDGE_AFTER`] for a request of [`HEDGE_LARGE_BYTES`] or more, whose
/// p99.9 was 3.9 s live.
const HEDGE_AFTER_LARGE: Duration = Duration::from_secs(5);

/// Size from which a request waits [`HEDGE_AFTER_LARGE`] for its first
/// answer.
const HEDGE_LARGE_BYTES: usize = 32 * 1024;

/// Most copies a runtime has in flight at once. A stall is rare (21 calls of
/// 36,459 live), so many framings outliving their delay together means a
/// slow or failing gateway, which a copy of each would only load more (the
/// client may be waiting out its retry delay): past this many, a framing
/// waits for its own answer.
const HEDGE_COPIES: usize = 2;

/// Asks `request` once, and once more when no answer has come within its
/// hedge delay ([`HEDGE_AFTER`], or [`HEDGE_AFTER_LARGE`] for a large
/// request), taking whichever copy answers first. A copy that fails gives
/// way to the other; when both fail, the first failure is returned. The
/// answer that counts carries the extra attempt, and the journal records a
/// `hedge` event. Sage gets no copy, and no copy is sent while
/// [`HEDGE_COPIES`] are in flight.
pub(in crate::agentic::flow) async fn hedged(
    runtime: &JevRuntime,
    step: &str,
    request: &EvaluationRequest,
) -> Result<EvaluationResult, EvaluationFailure> {
    let first = runtime.evaluate(Some(step), request);
    // Sage's calls take 5–6 s as a rule (`docs/technical/evals/
    // 2026-09-29-sage.md`): a copy would double its calls and its cost, and
    // rarely answer first.
    if runtime.configuration.provider == JevProvider::Sage {
        return first.await;
    }
    let delay = if bytes(request) >= HEDGE_LARGE_BYTES {
        HEDGE_AFTER_LARGE
    } else {
        HEDGE_AFTER
    };
    tokio::pin!(first);
    if let Ok(outcome) = tokio::time::timeout(delay, &mut first).await {
        return outcome;
    }
    let Some(_held) = Held::claim(&runtime.copies) else {
        return first.await;
    };
    let sent = Instant::now();
    let copy = runtime.evaluate(Some(step), request);
    tokio::pin!(copy);
    let (outcome, copy_first) = tokio::select! {
        outcome = &mut first => (outcome, false),
        outcome = &mut copy => (outcome, true),
    };
    // Which answer counts: `Some(true)` the copy's, `None` neither.
    let (outcome, counted) = match outcome {
        Ok(evaluation) => (Ok(evaluation), Some(copy_first)),
        Err(failure) => match if copy_first { first.await } else { copy.await } {
            Ok(evaluation) => (Ok(evaluation), Some(!copy_first)),
            Err(_) => (Err(failure), None),
        },
    };
    runtime.journal.record("hedge", || {
        json!({
            "step": step,
            "after_ms": millis(delay),
            "won": match counted {
                Some(true) => "copy",
                Some(false) => "first",
                None => "neither",
            },
            "ok": outcome.is_ok(),
            "wall_ms": millis(delay + sent.elapsed()),
        })
    });
    outcome.map(|mut evaluation| {
        evaluation.attempts = evaluation.attempts.saturating_add(1);
        evaluation
    })
}

/// A copy in flight, counted against [`HEDGE_COPIES`] until it is dropped.
struct Held<'a>(&'a AtomicUsize);

impl<'a> Held<'a> {
    /// A place for one more copy, unless [`HEDGE_COPIES`] are in flight.
    fn claim(copies: &'a AtomicUsize) -> Option<Self> {
        let held = Self(copies);
        // Counted first, so two claims at once cannot both slip under the
        // limit; one over it gives its place back as it is dropped.
        (copies.fetch_add(1, Ordering::AcqRel) < HEDGE_COPIES).then_some(held)
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
