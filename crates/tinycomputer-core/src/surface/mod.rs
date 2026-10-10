//! The surface abstraction every decision loop runs against.
//!
//! A [`Surface`] observes a [`Screen`] of [`Candidate`]s and acts on them with
//! closed operations. `tinycomputer-desktop` implements it over agent-desktop
//! and `tinycomputer-browser` over agent-browser; the engine's flow runtime is
//! generic over it. The helpers here — fingerprints, change notes, verified
//! text delivery — depend on nothing but the trait.

mod delivery;
mod digest;
mod groups;
mod screen;

use tinycomputer_bus::{DesktopResponse, JevOperation};

pub use delivery::{deliver_text, holds, tokenized};
pub use digest::{Digest, Region, RegionKind, Rendering, digest};
pub use groups::{Group, result_families, result_groups};
pub use screen::{
    Candidate, Depth, MAX_CANDIDATES, Screen, change_note, describe, difference, element_line,
    exact_named_match, fingerprint, label, signature, target_payload, untrusted_context,
};

/// One thing a task can observe and act on: a desktop application or a
/// browser tab.
///
/// The flow runtime is written against this trait alone, so its decision
/// loops run the same over agent-desktop and agent-browser, and a test drives
/// them with a scripted implementation.
pub trait Surface: Clone + Send + 'static {
    /// Reads the current surface of `app`, optionally rooted at a container.
    ///
    /// # Errors
    ///
    /// The engine's reply, boxed, when nothing readable could be observed:
    /// the application is not running, a permission is missing, or the root
    /// ref is stale.
    fn observe(
        &self,
        app: &str,
        root: Option<&str>,
        depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>>;

    /// Runs one closed operation. `TypeText` only sets the value; callers that
    /// need it verified go through [`deliver_text`].
    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse;

    /// Reads an element's current value, when the platform exposes one.
    fn read_value(&self, target: &Candidate) -> Option<String>;

    /// Focuses `target`, puts `text` into it through the pasteboard, and
    /// restores whatever the pasteboard held before.
    ///
    /// A field that supports set-value is replaced (select all, then paste).
    /// One that does not — a rich-text body such as a mail message — has the
    /// text pasted at the caret, so a reply keeps the message it quotes.
    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse;

    /// Presses a key combination at `app`.
    fn press(&self, app: &str, combo: &str) -> DesktopResponse;

    /// Launches `app`, or brings it forward when it is already running.
    fn launch(&self, app: &str) -> DesktopResponse;

    /// Gives the application a moment to finish reacting: after an action
    /// (one that fetches nothing settles briefly instead,
    /// [`Surface::settle_briefly`]), before the next look, and before a value
    /// is read back — a page that closes a banner a beat after the click, or
    /// a token field turning an address into a token.
    fn settle(&self) {}

    /// Settles after an action that fetches nothing — launching what is
    /// already open, Escape closing a layer — so only the application's
    /// own movement is waited out. A surface that cannot tell such an
    /// action apart settles as after any other.
    fn settle_briefly(&self) {
        self.settle();
    }

    /// Waits, for up to the given milliseconds, for the application to
    /// change by itself — a list of suggestions a box fetches for the text
    /// just typed, the rest of a page arriving — and says whether it did, so
    /// a caller watching for something to appear stops once nothing moves.
    ///
    /// A surface that cannot watch for a change pauses as a `Wait` does,
    /// however long it was given, and says it may have changed.
    fn await_change(&self, _ms: u64) -> bool {
        let _paused = self.execute(JevOperation::Wait, None, None);
        true
    }

    /// Loads `url`, for a surface that has addresses.
    ///
    /// A desktop application has none, so the default refuses with
    /// `ACTION_NOT_SUPPORTED`; a browser tab navigates.
    fn navigate(&self, url: &str) -> DesktopResponse {
        DesktopResponse::err(
            "navigate",
            tinycomputer_bus::DesktopError::new(
                "ACTION_NOT_SUPPORTED",
                format!("this surface has no addresses to load {url} into"),
            ),
        )
    }

    /// Goes back to the previous page, for a surface that keeps a history.
    ///
    /// A desktop application keeps none, so the default refuses with
    /// `ACTION_NOT_SUPPORTED`; a browser tab goes back, and its reply carries
    /// the address it landed on as `url`.
    fn back(&self, app: &str) -> DesktopResponse {
        DesktopResponse::err(
            "back",
            tinycomputer_bus::DesktopError::new(
                "ACTION_NOT_SUPPORTED",
                format!("{app} keeps no history to go back through"),
            ),
        )
    }

    /// Closes an empty layer lying over `target`, the backdrop a popup, a
    /// menu, or a box's list of suggestions leaves over a page, by pressing
    /// it where nothing pressable lies beneath, as a person clicks outside
    /// a popup to close it. The reply names what it pressed as `dismissed`,
    /// or `null` when nothing covers `target` any more.
    ///
    /// Only an empty layer is ever pressed: never a control, text, a
    /// picture, or a dialog. A surface that cannot tell what lies over an
    /// element, as a desktop application's cannot, refuses with
    /// `ACTION_NOT_SUPPORTED`.
    fn dismiss_cover(&self, _target: &Candidate) -> DesktopResponse {
        DesktopResponse::err(
            "dismiss-cover",
            tinycomputer_bus::DesktopError::new(
                "ACTION_NOT_SUPPORTED",
                "this surface cannot tell what lies over an element",
            ),
        )
    }
}

/// Whether a person would carry out `operation` with the pointer — click,
/// expand, collapse, check, uncheck — so a surface that draws the agent's
/// cursor glides it onto the target before the action lands.
#[must_use]
pub fn uses_pointer(operation: JevOperation) -> bool {
    matches!(
        operation,
        JevOperation::Click
            | JevOperation::Expand
            | JevOperation::Collapse
            | JevOperation::Check
            | JevOperation::Uncheck
    )
}

#[cfg(test)]
mod surface_tests;
