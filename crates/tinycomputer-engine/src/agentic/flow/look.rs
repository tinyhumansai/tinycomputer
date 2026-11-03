//! Reading the screen: one budgeted look, and exploring the subtrees it cut
//! short.

use std::time::Instant;

use serde_json::json;

use super::{
    FlowRun, Halt, MAX_BLIND_LOOKS, MAX_EXPLORED,
    backend::{AgentBackend, observe_async},
    view::{Depth, Screen},
};
use crate::agentic::journal::millis;

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Reads the application's current surface.
    ///
    /// An application can be running with nothing readable on screen — a
    /// document app showing only its open panel, or one still starting. That
    /// is reported to Jev as a blank screen with a note, so a keyboard move can
    /// still make progress; only [`MAX_BLIND_LOOKS`] such looks in a row fail
    /// the step.
    pub(in crate::agentic::flow) async fn look(&mut self) -> Result<Screen, Halt> {
        let started = Instant::now();
        let observed =
            observe_async(self.backend.clone(), self.app.clone(), None, Depth::Full).await;
        self.runtime.journal.record("observe", || {
            json!({
                "step": self.step,
                "part": "screen",
                "wall_ms": millis(started.elapsed()),
                "ok": observed.is_ok(),
                "candidates": observed.as_ref().map_or(0, |screen| screen.candidates.len()),
                "unexplored": observed.as_ref().map_or(0, |screen| screen.unexplored.len()),
            })
        });
        match observed {
            Ok(mut screen) => {
                self.redact_screen(&mut screen);
                self.blind_looks = 0;
                if let Some(note) = self.front.look(&screen, self.location.as_deref()) {
                    self.history.push(note.to_owned());
                }
                Ok(screen)
            }
            Err(error) => {
                let reason = error
                    .error
                    .as_ref()
                    .map_or_else(|| "unknown error".to_owned(), |error| error.message.clone());
                self.blind_looks = self.blind_looks.saturating_add(1);
                if self.blind_looks >= MAX_BLIND_LOOKS {
                    return Err(Halt::Failed(format!(
                        "the screen could not be read: {reason}"
                    )));
                }
                Ok(Screen {
                    app: self.app.clone(),
                    window: None,
                    surface: "none".to_owned(),
                    candidates: Vec::new(),
                    context: vec![format!(
                        "No window of the application can be read right now ({reason}). A keyboard shortcut may still work."
                    )],
                    unexplored: Vec::new(),
                    text_nodes: Vec::new(),
                })
            }
        }
    }

    /// Reads the subtrees the engine cut short and adds what they hold.
    ///
    /// Called only when a step did not find what it needs in the budgeted
    /// view — a note editor below a long folder list — so the common case
    /// still reads one bounded snapshot.
    pub(in crate::agentic::flow) async fn explore(&self, screen: &mut Screen) {
        for root in std::mem::take(&mut screen.unexplored)
            .into_iter()
            .take(MAX_EXPLORED)
        {
            let started = Instant::now();
            let part = observe_async(
                self.backend.clone(),
                self.app.clone(),
                Some(root),
                Depth::Full,
            )
            .await;
            self.runtime.journal.record("observe", || {
                json!({
                    "step": self.step,
                    "part": "subtree",
                    "wall_ms": millis(started.elapsed()),
                    "ok": part.is_ok(),
                    "candidates": part.as_ref().map_or(0, |part| part.candidates.len()),
                })
            });
            if let Ok(mut part) = part {
                self.redact_screen(&mut part);
                screen.candidates.extend(part.candidates);
                screen.context.extend(part.context);
                screen.text_nodes.extend(part.text_nodes);
            }
        }
    }
}
