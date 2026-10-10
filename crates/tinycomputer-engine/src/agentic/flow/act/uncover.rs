//! Pressing a target the page refuses as covered: closing what lies over
//! it, then pressing the same target once more.

use tinycomputer_bus::{DesktopResponse, JevOperation};

use crate::agentic::flow::{
    FlowRun, Halt, StepLog,
    attention::front_closer,
    backend::AgentBackend,
    view::{Candidate, label, signature},
};

use super::covered;

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// Performs `operation` on an already-vetted `target`. When the click
    /// is refused because something covers it — a drawer, a menu, a consent
    /// banner, or a result card's own click layer — closes what covers it
    /// once and tries the same target again: with the least committal
    /// control of a layer in front ([`front_closer`]; never a layer the
    /// step's `intent` names, or the one `target` sits in), or else with
    /// Escape. When that leaves it covered, an empty layer over it (the
    /// backdrop a popup or a box's suggestions leave over the page) is
    /// pressed where nothing lies beneath, as a person clicks outside a
    /// popup, and the target tried once more. None of these chooses a new
    /// target. The history names what covered it, as the surface said.
    pub(in crate::agentic::flow) async fn press_uncovering(
        &mut self,
        log: &mut StepLog,
        verb: &str,
        target: &Candidate,
        operation: JevOperation,
        intent: &str,
    ) -> Result<DesktopResponse, Halt> {
        let reply = self.press_once(log, verb, target, operation).await?;
        if !covered(&reply) {
            return Ok(reply);
        }
        // A press the page refused counts as none in the dialog in front.
        let served = self.front.served_calendar();
        let cover = cover_of(&reply);
        // A dialog in front is the page's question (a format, a quantity),
        // not a popover in the way: Escape would close it, and pressing what
        // lies behind it leaves the flow it began (live, a movie's language
        // link behind its booking dialog led to a listing of other films).
        // A layer drawn over the window is such a question only when the
        // task's own press opened it; a calendar left open is in the way,
        // and so is one the task opened and has pressed in since: live, a
        // calendar stayed in front of the guests and Search buttons once
        // both dates were picked, and every press behind it was refused.
        if (self.front.opened_dialog()
            || !matches!(self.front.surface.as_str(), "window" | "layer"))
            && !served
        {
            self.history.push(format!(
                "{} lies behind the dialog in front; act within the dialog instead",
                label(target)
            ));
            return Ok(reply);
        }
        // A layer in front that closes with a control of its own (a consent
        // banner's "Allow Selection") is closed with it: Escape leaves such a
        // banner where it is.
        let screen = self.look().await?;
        if let Some(closer) = front_closer(
            &screen,
            target,
            intent,
            &self.stop_before,
            &self.step_cleared,
        ) {
            self.step_cleared.insert(signature(&closer));
            let pressed = closer.clone();
            self.act(log, "click (uncover)", Some(&closer), move |backend| {
                backend.execute(JevOperation::Click, Some(pressed), None)
            })
            .await?;
            self.history.push(format!(
                "{} was covered by {cover}; pressed {} to close it",
                label(target),
                label(&closer)
            ));
        } else {
            let app = self.app.clone();
            self.act(log, "press escape (uncover)", None, move |backend| {
                backend.press(&app, "escape")
            })
            .await?;
            self.history.push(format!(
                "{} was covered by {cover}; pressed escape to close it",
                label(target)
            ));
        }
        self.look_again().await?;
        let reply = self.press_once(log, verb, target, operation).await?;
        if !covered(&reply) {
            return Ok(reply);
        }
        if !empty_layer(&reply) {
            return Ok(self.stays_covered(target, reply));
        }
        // Live, a store's search box left its list of suggestions open over
        // the page, every control outside it read as covered, and Escape
        // left it there: its basket button was refused until rescues re-added
        // items.
        let cover = cover_of(&reply);
        let lying = target.clone();
        let dismissed = self
            .act(log, "press the cover (uncover)", None, move |backend| {
                backend.dismiss_cover(&lying)
            })
            .await?;
        if !dismissed.ok {
            return Ok(self.stays_covered(target, reply));
        }
        let pressed = dismissed
            .data
            .as_ref()
            .and_then(|data| data.get("dismissed"))
            .is_some_and(serde_json::Value::is_string);
        if pressed {
            self.history.push(format!(
                "{} was still covered by {cover}; pressed an empty part of it, as a person clicks outside a popup to close it",
                label(target)
            ));
        }
        self.look_again().await?;
        let reply = self.press_once(log, verb, target, operation).await?;
        if covered(&reply) {
            return Ok(self.stays_covered(target, reply));
        }
        Ok(reply)
    }

    /// Looks again once a cover is closed, before the target is pressed
    /// once more: what the cover hid is counted as uncovered, so a layer the
    /// next press opens is counted as the task's, rather than measured
    /// against everything the cover covered.
    async fn look_again(&mut self) -> Result<(), Halt> {
        self.look().await.map(|_| ())
    }

    /// Performs `operation` on `target` once, and forgets a press of this
    /// step a cover refused once one lands.
    async fn press_once(
        &mut self,
        log: &mut StepLog,
        verb: &str,
        target: &Candidate,
        operation: JevOperation,
    ) -> Result<DesktopResponse, Halt> {
        let chosen = target.clone();
        let reply = self
            .act(log, verb, Some(target), move |backend| {
                backend.execute(operation, Some(chosen), None)
            })
            .await?;
        if reply.ok {
            self.step_covered = None;
        }
        Ok(reply)
    }

    /// `reply`, the last press of `target`, still refused as covered after
    /// everything that could close the cover: the history and the step's
    /// failure say what covers it, so the next move, or a rescue, deals with
    /// that rather than press `target` again or redo the steps before it.
    fn stays_covered(&mut self, target: &Candidate, reply: DesktopResponse) -> DesktopResponse {
        let cover = cover_of(&reply);
        self.history.push(format!(
            "{} is still covered by {cover}: deal with that first, or reach what {} does another way",
            label(target),
            label(target)
        ));
        self.step_covered = Some(format!(
            "its press of {} was refused because {cover} lies over it",
            label(target)
        ));
        reply
    }
}

/// Whether what covers the target of a press refused as covered is an
/// empty layer, as the surface said (`details.empty_layer`): one a press
/// outside a popup closes. A surface that cannot tell never says so.
fn empty_layer(reply: &DesktopResponse) -> bool {
    reply
        .error
        .as_ref()
        .and_then(|error| error.details.as_ref())
        .and_then(|details| details.get("empty_layer"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// What covers the target of a press refused as covered, as the surface
/// said it (`details.cover`: `button "Select Location"`, `an empty
/// layer`), or "something" when it could not say.
fn cover_of(reply: &DesktopResponse) -> String {
    reply
        .error
        .as_ref()
        .and_then(|error| error.details.as_ref())
        .and_then(|details| details.get("cover"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("something")
        .to_owned()
}
