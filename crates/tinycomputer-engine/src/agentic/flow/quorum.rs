//! Ending a decision on a quorum: once all but its last two framings have
//! answered, and agree so plainly that the last two almost never change
//! what the loops or the evidence gate make of the answers, the decision is
//! merged without waiting for them.
//!
//! A decision waits for its slowest framing. Replayed over 13,669 decisions
//! asked seven ways, from 245 live runs, a quorum of five ended 12% of them,
//! a median 0.14 s before the seventh answer and about 2 s a run, and every
//! one read the same on every threshold, choice floor, and evidence gate as
//! all seven framings did, but for two whose mean moved by under 0.002 across
//! the edge of the evidence band. A quorum of four agreed with all seven only
//! 99.2% of the time: its stragglers dissented.
//!
//! The framings left are not cancelled: each runs to its end, its exchange
//! journaled, so its connection goes back to the pool for the next decision.

use std::collections::BTreeMap;

use tinyinference_decisions::{Answer, EvaluationFailure, EvaluationResult};
use tokio::{sync::mpsc, task::JoinHandle};

use super::{decide::PAGE_KIND, vote};

/// Framings a decision ends without, at most, once the rest agree.
pub(super) const QUORUM_LEFT: usize = 2;
/// Fewest framings a decision must be asked in to end on a quorum.
pub(super) const QUORUM_VOTES: usize = 7;
/// Least probability every framing gives the option a Choice or Score
/// ranks first, the same option in each.
pub(super) const QUORUM_TOP: f64 = 0.9;
/// A yes/no every framing answers at or above this, or every one at or
/// below [`SURE_NO`], lies the evidence band's 0.12 beyond every threshold
/// the loops read one yes/no against (0.20 to 0.85). A belief that pairs it
/// with its negation or a coverage can still fall within a band, and is
/// widened then as it would be after all the framings.
pub(super) const SURE_YES: f64 = 0.97;
/// See [`SURE_YES`].
pub(super) const SURE_NO: f64 = 0.08;

/// One framing's evaluation, as its task ends.
type Evaluated = Result<EvaluationResult, EvaluationFailure>;

/// A framing's index and how its task ended.
type Finished = (usize, Result<Evaluated, tokio::task::JoinError>);

/// What one part's framings came back with.
pub(super) struct Gathered {
    /// Each framing that answered, with its evaluation, in framing order.
    pub(super) answered: Vec<(vote::Framing, EvaluationResult)>,
    /// The first failure heard, if any framing failed.
    pub(super) failure: Option<EvaluationFailure>,
    /// Framings a quorum ended the wait without.
    pub(super) left: u32,
}

/// The framings heard from so far.
#[derive(Default)]
struct Heard {
    /// Those that answered, by index, in the order they finished.
    answered: Vec<(usize, EvaluationResult)>,
    failure: Option<EvaluationFailure>,
    count: usize,
}

impl Heard {
    fn take(&mut self, (index, outcome): Finished) {
        self.count += 1;
        match outcome {
            Ok(Ok(evaluation)) => self.answered.push((index, evaluation)),
            Ok(Err(error)) => {
                self.failure.get_or_insert(error);
            }
            Err(_) => {}
        }
    }
}

/// How many answers can end a decision asked in `framings` framings: all
/// but [`QUORUM_LEFT`] of them, when it is asked in [`QUORUM_VOTES`] or more;
/// `None` when it waits for every framing.
pub(super) fn size(framings: usize) -> Option<usize> {
    (framings >= QUORUM_VOTES).then(|| framings - QUORUM_LEFT)
}

/// Waits for the framings in `handles`, asked as `framings`, in the order
/// they finish: for all of them, or, with a quorum `size`, until that many
/// have answered and settle every question ([`settled`]). An answer already
/// in when the quorum is reached is used all the same; only the waiting is
/// cut short.
pub(super) async fn gather(
    framings: Vec<vote::Framing>,
    handles: Vec<JoinHandle<Evaluated>>,
    size: Option<usize>,
) -> Gathered {
    let count = handles.len();
    let mut finished = in_order_of_finish(handles);
    let mut heard = Heard::default();
    while let Some(next) = finished.recv().await {
        heard.take(next);
        while let Ok(next) = finished.try_recv() {
            heard.take(next);
        }
        let answers = heard
            .answered
            .iter()
            .map(|(index, evaluation)| (*index, &evaluation.response.answers));
        if size
            .is_some_and(|size| heard.answered.len() >= size && settled(&framings, answers, size))
        {
            break;
        }
    }
    let Heard {
        mut answered,
        failure,
        count: heard,
    } = heard;
    answered.sort_unstable_by_key(|(index, _)| *index);
    let mut framings = framings.into_iter().map(Some).collect::<Vec<_>>();
    Gathered {
        answered: answered
            .into_iter()
            .filter_map(|(index, evaluation)| Some((framings.get_mut(index)?.take()?, evaluation)))
            .collect(),
        failure,
        left: u32::try_from(count - heard).unwrap_or(u32::MAX),
    }
}

/// Each of `handles`' outcomes, with its index, as its task ends. Every
/// task is awaited to its end, whether or not anything still listens.
fn in_order_of_finish(handles: Vec<JoinHandle<Evaluated>>) -> mpsc::UnboundedReceiver<Finished> {
    let (sender, finished) = mpsc::unbounded_channel();
    for (index, handle) in handles.into_iter().enumerate() {
        let sender = sender.clone();
        tokio::spawn(async move {
            let _heard = sender.send((index, handle.await));
        });
    }
    finished
}

/// Whether the `answered` framings, by their index in `framings`, settle
/// every question the framings asked: each answered by at least `size` of
/// them, every yes/no [sure](sure) and every Choice and Score ranking the
/// same option first at [`QUORUM_TOP`] or more. The page-kind question,
/// which only briefs the next request, needs only the same first option.
pub(super) fn settled<'a>(
    framings: &'a [vote::Framing],
    answered: impl Iterator<Item = (usize, &'a BTreeMap<String, Answer>)> + Clone,
    size: usize,
) -> bool {
    let ballots = vote::ballots_at(framings, answered);
    framings.first().is_some_and(|framing| {
        framing.request.questions.keys().all(|id| {
            ballots.get(id).is_some_and(|ballot| {
                ballot.len() >= size
                    && match ballot.first() {
                        _ if id == PAGE_KIND => same_top(ballot, 0.0),
                        Some(Answer::Noul(_)) => sure(ballot),
                        Some(_) => same_top(ballot, QUORUM_TOP),
                        None => false,
                    }
            })
        })
    })
}

/// Whether every answer in `ballot` is a yes/no at or above [`SURE_YES`],
/// or every one at or below [`SURE_NO`].
fn sure(ballot: &[Answer]) -> bool {
    let nouls = ballot
        .iter()
        .map(|answer| match answer {
            Answer::Noul(noul) => Some(noul.noul),
            _ => None,
        })
        .collect::<Option<Vec<_>>>();
    nouls.is_some_and(|nouls| {
        nouls.iter().all(|noul| *noul >= SURE_YES) || nouls.iter().all(|noul| *noul <= SURE_NO)
    })
}

/// Whether every answer in `ballot` ranks the same option first, each with
/// at least `floor`.
fn same_top(ballot: &[Answer], floor: f64) -> bool {
    let mut tops = ballot.iter().map(|answer| {
        let probabilities = match answer {
            Answer::Choice(choice) => &choice.probabilities,
            Answer::Score(score) => &score.probabilities,
            Answer::Noul(_) | Answer::Box(_) => return None,
        };
        probabilities
            .iter()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .filter(|(_, probability)| **probability >= floor)
            .map(|(option, _)| option)
    });
    let Some(Some(first)) = tops.next() else {
        return false;
    };
    tops.all(|top| top == Some(first))
}
