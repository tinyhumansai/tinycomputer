//! One function per step kind; `run` dispatches between them.
//!
//! - `launch`: `open` and `browse`.
//! - `condition`: `verify`, `wait_for`, `repeat_until`, and `if`, all built
//!   on judging one condition (`holds`).
//! - `choose` and `reveal`: `choose`, and the ways it makes a missing option
//!   show.
//! - `read`, `list`, and `stop`: `read`; `pick` and `extract`; `stop_before`.
//! - `date` and `matching`: the pure rules for reading a date option and
//!   matching an option to the controls on screen.

mod choose;
mod condition;
mod date;
mod launch;
mod list;
mod matching;
mod read;
mod reveal;
mod stop;
mod suggestion;
mod typing;

pub(super) use date::{looks_like_date, names_a_month};
pub(super) use matching::left_unchosen;
#[cfg(test)]
pub(super) use {
    date::{date_words, heads_month_of, shows_date},
    launch::failure,
    list::{first_meeting, leaning},
    matching::{
        already_chosen, already_holds, closest, date_shown_in, in_region, lists_more_than,
        redacted, search_text,
    },
    read::chosen_together,
    read::readable,
    reveal::heads_its_month,
    stop::only_signs_in,
    suggestion::{fresh_rows, same_search, searches, shares_most_words, suggests},
    typing::typing,
};

use tinycomputer_bus::FlowAction;

use super::{Ended, FlowRun, Halt, StepLog, backend::AgentBackend};

/// Turns a `do` step may spend.
const DO_TURNS: u32 = 8;
/// Turns spent opening the thing a `choose` step picks from.
pub(super) const REVEAL_TURNS: u32 = 3;
/// Times `open` checks for a readable window, waiting between checks.
pub(super) const WINDOW_CHECKS: u32 = 10;
/// Times a `wait_for` checks its condition, waiting between checks.
pub(super) const WAIT_CHECKS: u32 = 10;
/// Checks in a row, a wait apart, on which a page says it found nothing
/// before a `wait_for` stops waiting for what it searched for.
pub(super) const EMPTY_CHECKS: u32 = 2;
/// Belief a condition must keep, on a screen that no longer changes, for a
/// `wait_for` to take it as held after [`STEADY_CHECKS`] checks.
pub(super) const STEADY_HOLD: f64 = 0.65;
/// Checks in a row of one unchanged screen, each judged at [`STEADY_HOLD`]
/// or more, after which a `wait_for` takes its condition as held.
pub(super) const STEADY_CHECKS: u32 = 3;
/// Most characters of a picked item's text kept in its variable.
pub(super) const MAX_PICK_SUMMARY: usize = 400;
/// Items an exact ranking puts first that a `pick` asks Jev about, at once,
/// for the first that belongs to the list it picks from.
pub(super) const RANKED_CHECKS: usize = 8;
/// Least belief a deep run needs that a control is the one a `stop_before`
/// names before it presses it irreversibly.
pub(in crate::agentic::flow) const IRREVERSIBLE_FLOOR: f64 = 0.85;
/// Least probability a `read` or `stop_before` target needs.
pub(super) const LOCATE_FLOOR: f64 = 0.5;
/// How many lists an `extract` offers Jev when several show.
pub(super) const MAX_LISTS: usize = 6;
/// How many of a list's first items an `extract` shows Jev to tell it apart.
pub(super) const LIST_PREVIEW: usize = 3;
/// Least probability a list not clearly chosen needs to be taken as the one
/// Jev leaned to, rather than the longest.
pub(super) const LIST_LEAN: f64 = 0.3;
/// How many times the next list's probability the list Jev leaned to needs.
pub(super) const LIST_LEAD: f64 = 3.0;

/// Runs one step.
pub(super) async fn run<B: AgentBackend + Sync>(
    run: &mut FlowRun<'_, B>,
    log: &mut StepLog,
    action: &FlowAction,
    text: &str,
    path: &str,
) -> Result<Ended, Halt> {
    match action {
        FlowAction::Open(app) => run.open(log, app).await,
        FlowAction::Browse(url) => run.browse(log, url).await,
        // A plain step that types is an `enter`: a `do` cannot type. It is
        // read from the step as written, never from `text`, which has
        // already been through substitution: `enter` substitutes once more,
        // and a value read off a page that says `${card_number}` would
        // then be typed as the caller's card number.
        FlowAction::Do(intent) => match typing::typing(intent) {
            Some(slot) => run.enter(log, &[slot]).await,
            None => run.accomplish(log, text, DO_TURNS).await,
        },
        FlowAction::Enter(slots) => run.enter(log, &slots.0).await,
        FlowAction::Choose(choose) => run.choose(log, choose).await,
        FlowAction::Read(read) => run.read(log, read).await,
        FlowAction::Pick(pick) => run.pick(log, pick).await,
        FlowAction::Extract(read) => run.extract(log, read).await,
        FlowAction::Verify(_) => run.verify(log, text).await,
        FlowAction::WaitFor(_) => run.wait_for(log, text).await,
        FlowAction::StopBefore(_) => run.stop_before(log, text).await,
        FlowAction::RepeatUntil(repeat) => run.repeat(log, repeat, path).await,
        FlowAction::If(branch) => run.branch(log, branch, path).await,
    }
}
