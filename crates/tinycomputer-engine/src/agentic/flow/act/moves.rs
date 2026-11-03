//! Making a `do` move: pressing a grounded control, a shortcut, a scroll,
//! or a wait, and clearing an obstacle out of the way.

use std::collections::BTreeSet;

use serde_json::json;
use tinycomputer_bus::{JevOperation, StepOutcome};

use crate::agentic::flow::{
    Ended, FlowRun, Halt, StepLog,
    ask::{self, Questions, chosen},
    backend::AgentBackend,
    memory::{learn, remember},
    view::{Candidate, Screen, element_kind, is_banned, is_destructive, label},
};

use super::{
    Expected, MAX_REPEAT_PRESSES, Move, activate_purpose, creates_new, dialog::in_dialog,
    judge::Judgement,
};

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Carries out the move Jev chose.
    pub(super) async fn make_move(
        &mut self,
        log: &mut StepLog,
        screen: &Screen,
        intent: &str,
        judged: &Judgement,
        banned: &BTreeSet<String>,
        branch: Option<Candidate>,
    ) -> Result<Move, Halt> {
        match judged.next.as_str() {
            "finished" if creates_new(intent) && log.actions.is_empty() => {
                // Jev sees an existing item and calls it done; make a new one
                // with the shortcut it would use, or by pressing a control.
                let next = Judgement {
                    next: if judged.shortcut.is_some() {
                        "shortcut".to_owned()
                    } else {
                        "activate".to_owned()
                    },
                    ..judged.clone()
                };
                Box::pin(self.make_move(log, screen, intent, &next, banned, branch)).await
            }
            "finished" => Ok(Move::Ended(Ended::new(
                StepOutcome::Done,
                "Jev chose finished",
            ))),
            "stuck" => Err(Halt::Failed(
                "no visible control or standard shortcut moves toward the step".to_owned(),
            )),
            "wait" => {
                self.act(log, "wait", None, |backend| {
                    backend.execute(JevOperation::Wait, None, None)
                })
                .await?;
                Ok(Move::Acted(None, None))
            }
            "shortcut" => {
                let Some((combo, name)) = judged.shortcut else {
                    self.history
                        .push("no standard shortcut fits; press a visible control".to_owned());
                    return Ok(Move::Skipped);
                };
                if banned.contains(&format!("key:{combo}")) {
                    self.history.push(format!(
                        "did not press {combo} again: it was pressed {MAX_REPEAT_PRESSES} times in this step; judge whether the step is done, or act on something else"
                    ));
                    return Ok(Move::Skipped);
                }
                // Return in a search box runs its search wherever the box
                // sits: live, a store's search opened as a full-window
                // sheet, and its step to press Enter in the box just typed
                // into was refused 154 times.
                let searching = self
                    .typed_last
                    .as_ref()
                    .is_some_and(|field| is_search_box(field) && still_shows(screen, field));
                if combo == "return" && screen.surface != "window" && !searching {
                    self.history.push(format!(
                        "refused return while a {} is showing: it would press its default button",
                        screen.surface
                    ));
                    return Ok(Move::Skipped);
                }
                // In a composer Return sends: a flow that stops before an
                // irreversible step must press a control, which is gated.
                if combo == "return" && !searching && !self.stop_before.is_empty() {
                    self.history.push("refused return: this flow stops before an irreversible step, and Return may perform it; press the control instead".to_owned());
                    return Ok(Move::Skipped);
                }
                let app = self.app.clone();
                let reply = self
                    .act(log, &format!("press {combo}"), None, move |backend| {
                        backend.press(&app, combo)
                    })
                    .await?;
                self.history
                    .push(format!("pressed {combo} ({name}), ok={}", reply.ok));
                Ok(Move::Acted(None, None))
            }
            operation @ ("activate" | "expand" | "scroll") => {
                let pressed = self
                    .activate(log, screen, intent, operation, (banned, branch), judged)
                    .await?;
                Ok(match pressed {
                    Some((target, expected)) => {
                        Move::Acted(Some(Box::new(target)), expected.map(Box::new))
                    }
                    None => Move::Acted(None, None),
                })
            }
            other => {
                // A malformed or prompt-injected answer must fail closed
                // rather than default to a click: only the moves above are
                // ever offered to Jev.
                self.history.push(format!(
                    "ignored an unrecognized move {other:?}; only activate, shortcut, expand, scroll, wait, finished, and stuck are valid"
                ));
                Ok(Move::Skipped)
            }
        }
    }

    /// The elements a move of `capability` may target for the step
    /// `intent`: not banned this step, not of a kind that refused text,
    /// and reachable with what is in front (`reachable`).
    pub(super) fn pool(
        &self,
        screen: &Screen,
        capability: &str,
        banned: &BTreeSet<String>,
        intent: &str,
    ) -> Vec<Candidate> {
        screen
            .candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .available_actions
                    .iter()
                    .any(|action| action == capability)
                    && !is_banned(banned, candidate)
                    && !self.refused.contains(&element_kind(candidate))
                    && self.reachable(candidate, intent)
            })
            .cloned()
            .collect()
    }

    /// Whether a move may target `candidate` with what is in front: not
    /// something a dialog or layer in front covers, which no press reaches
    /// (live, rescues kept pressing a language link behind a booking
    /// dialog), nor, while the dialog in front is the task's own, its close
    /// control, unless the step asks to close it: live, the format dialog
    /// a booking button opened was closed and opened again in a loop.
    pub(in crate::agentic::flow) fn reachable(&self, candidate: &Candidate, intent: &str) -> bool {
        let covered = candidate
            .states
            .iter()
            .any(|state| state.eq_ignore_ascii_case("covered"))
            // What the dialog's own bar covers in its list is the dialog's:
            // a press scrolls it out from under the bar (live, a seat table's
            // lower rows sat under its "Pay" bar).
            && !in_dialog(candidate);
        // A calendar the task has picked in is closed for such a press
        // (`press_uncovering`), so what it covers can be pressed.
        if covered && self.front.surface != "window" && !self.front.served_calendar() {
            return false;
        }
        !(self.front.opened_dialog() && closes(candidate) && !asks_to_close(intent))
    }

    /// Grounds and performs an `activate`, `expand`, or `scroll` move; the
    /// element pressed, and under deliberation what the press expects.
    ///
    /// A `branch` left by a backtrack is tried first, confirmed with one
    /// yes/no question, before anything is grounded afresh.
    async fn activate(
        &mut self,
        log: &mut StepLog,
        screen: &Screen,
        intent: &str,
        operation: &str,
        (banned, branch): (&BTreeSet<String>, Option<Candidate>),
        judged: &Judgement,
    ) -> Result<Option<(Candidate, Option<Expected>)>, Halt> {
        let (capability, jev_operation, verb) = match operation {
            "expand" => ("Expand", JevOperation::Expand, "expand"),
            "scroll" => ("Scroll", JevOperation::Scroll, "scroll"),
            _ => ("Click", JevOperation::Click, "click"),
        };
        let purpose = activate_purpose(verb, intent);
        let prepared = judged.prepared.get(operation);
        let speculated = judged.speculated.clone();
        let branched = match branch {
            Some(branch) if operation == "activate" => {
                self.try_branch(log, screen, &purpose, branch).await?
            }
            _ => None,
        };
        let grounded = match (branched, prepared, speculated) {
            (Some(branched), _, _) => Some(branched),
            (None, Some(prepared), _) => self.resolve(log, screen, &purpose, prepared).await?,
            (None, None, Some(speculated)) if operation == "activate" => {
                self.resume(log, screen, speculated.opening, Some(speculated.answers))
                    .await?
            }
            _ => {
                let pool = self.pool(screen, capability, banned, intent);
                self.ground(log, screen, &purpose, intent, pool).await?
            }
        };
        // Nothing serving the step itself while the task's own dialog is in
        // front: the dialog asks something first, and what answers it is
        // pressed instead, though not remembered as the step's control.
        let (grounded, answers_dialog) = match grounded {
            None if operation == "activate" && self.front.opened_dialog() => {
                (self.answer_dialog(log, screen, intent, banned).await?, true)
            }
            grounded => (grounded, false),
        };
        let Some(grounded) = grounded else {
            self.history.push(format!(
                "no element clearly serves {verb} for this step; consider a shortcut or another move"
            ));
            return Ok(None);
        };
        let target = grounded.candidate;
        log.confidence = Some(grounded.confidence);
        if jev_operation == JevOperation::Click
            && is_destructive(&target, screen, &self.stop_before)
        {
            // Never call the backend, and never report this target through
            // `Move::Acted`: nothing happened, so it must not be banned as a
            // no-op or stalled toward `note_change`'s three-turn failure. The
            // step fails on this turn, with a note that says why.
            return Err(Halt::Failed(format!(
                "refused to press {} inside an ordinary step: it looks irreversible; a flow must use stop_before for that",
                label(&target)
            )));
        }
        let expected = self.expect(log, operation, &target, screen);
        let reply = self
            .press_uncovering(log, verb, &target, jev_operation, intent)
            .await?;
        self.history
            .push(format!("{verb} {} ok={}", label(&target), reply.ok));
        if reply.ok && !answers_dialog {
            learn(&mut self.learned, remember(&self.app, intent, &target));
        }
        Ok(Some((target, expected)))
    }

    /// Dismisses whatever is blocking the step, choosing only safe controls.
    pub(super) async fn clear_obstacle(
        &mut self,
        log: &mut StepLog,
        screen: &Screen,
        intent: &str,
    ) -> Result<(), Halt> {
        let pool = screen
            .candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .available_actions
                    .iter()
                    .any(|action| action == "Click")
                    && !is_destructive(candidate, screen, &self.stop_before)
            })
            .take(ask::CAP)
            .cloned()
            .collect::<Vec<_>>();
        let keys = ask::numbered(pool.len());
        let mut options = keys
            .iter()
            .cloned()
            .zip(
                pool.iter()
                    .map(|node| crate::agentic::flow::view::describe(node, false)),
            )
            .collect::<Vec<_>>();
        options.push(("escape".to_owned(), json!("Press Escape to close it.")));
        let answers = self
            .ask(
                log,
                ask::request(
                    self.model(),
                    self.state(screen, intent),
                    Questions::default().with(
                        "dismiss",
                        ask::options(
                            json!({
                                "task": "Something unrelated to the step is in the way. Choose how to close it without losing work and without doing anything irreversible.",
                                "step": intent,
                            }),
                            options,
                        ),
                    ),
                ),
            )
            .await?;
        let choice = chosen(&answers, "dismiss").map(|(choice, _)| choice);
        let target = choice
            .as_deref()
            .and_then(|choice| keys.iter().position(|key| key == choice))
            .and_then(|index| pool.get(index).cloned());
        if let Some(target) = target {
            let chosen_target = target.clone();
            self.act(log, "click (dismiss)", Some(&target), move |backend| {
                backend.execute(JevOperation::Click, Some(chosen_target), None)
            })
            .await?;
            self.history
                .push(format!("dismissed an obstacle with {}", label(&target)));
        } else {
            let app = self.app.clone();
            self.act(log, "press escape (dismiss)", None, move |backend| {
                backend.press(&app, "escape")
            })
            .await?;
            self.history
                .push("pressed escape to dismiss an obstacle".to_owned());
        }
        Ok(())
    }
}

/// Labels of a control that closes what it sits in. A dialog's "Cancel"
/// is an answer, not a close: live, it was the way back from a show that
/// had already started.
const CLOSE_LABELS: &[&str] = &["close", "×", "x", "✕", "✖"];

/// Whether `candidate` closes the dialog it sits in.
fn closes(candidate: &Candidate) -> bool {
    let said = candidate
        .name
        .as_deref()
        .or(candidate.description.as_deref())
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    CLOSE_LABELS.contains(&said.as_str()) || said.starts_with("close ")
}

/// Whether the step `intent` asks for something to be closed or left.
fn asks_to_close(intent: &str) -> bool {
    intent
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "close" | "dismiss" | "cancel" | "exit" | "leave" | "back"
            )
        })
}

/// Whether `field` is a search box: a `searchbox`, or a box that takes text
/// and names itself for searching ("Search Lenskart", "Search for atta dal
/// and more"). Return there runs the search, never a dialog's default
/// button.
pub(in crate::agentic::flow) fn is_search_box(field: &Candidate) -> bool {
    let takes_text = field
        .available_actions
        .iter()
        .any(|action| action == "SetValue" || action == "TypeText");
    let named = [field.name.as_deref(), field.description.as_deref()]
        .into_iter()
        .flatten()
        .any(|text| text.to_lowercase().contains("search"));
    field.role.eq_ignore_ascii_case("searchbox") || (takes_text && named)
}

/// Whether `field` still shows on `screen` with nothing covering it: a
/// dialog that has come up over it since it was typed into takes the keys
/// itself.
pub(in crate::agentic::flow) fn still_shows(screen: &Screen, field: &Candidate) -> bool {
    screen.candidates.iter().any(|candidate| {
        candidate.role == field.role
            && candidate.name == field.name
            && !candidate
                .states
                .iter()
                .any(|state| state.eq_ignore_ascii_case("covered"))
    })
}
