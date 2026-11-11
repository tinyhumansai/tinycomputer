//! The browser members' bus identity: where they are served, and one constant
//! per member, in dispatch order.
//!
//! A `TinyBus` module exports one interface at one object path, so the
//! browser members are served on the module's interface beside the desktop
//! and task members, exactly as [`crate::agent::names`] are. Each carries a
//! `Browser` prefix: several (`Snapshot`, `Screenshot`) would otherwise
//! collide with a desktop member of a different shape, and a prefix on every
//! one of them keeps the family obvious to a model reading a member list.
//!
//! Nothing here is a string literal at a call site. A host names a member
//! through [`methods`], so a rename is a compile error in every consumer
//! rather than a runtime "unknown method". [`METHODS`] also appears, in the
//! same order, at the end of [`crate::names::METHODS`], and `crates/tinycomputer`
//! asserts its dispatch table against that list.
//!
//! Every member takes one JSON object and returns a [`crate::DesktopResponse`],
//! the same envelope the desktop members use; a failure carries one of the
//! codes in [`crate::browser::errors`].

/// The interface the browser members are served on.
pub const INTERFACE: &str = crate::names::INTERFACE;

/// The object path the browser members are served at.
pub const OBJECT_PATH: &str = crate::names::OBJECT_PATH;

/// One constant per browser member of [`INTERFACE`].
pub mod methods {
    /// Launches or attaches a browser and returns the session that owns it.
    ///
    /// Takes a [`crate::browser::SessionOptions`]; its `data` is a
    /// [`crate::browser::SessionInfo`].
    pub const OPEN_SESSION: &str = "BrowserOpenSession";

    /// Closes a session and everything it owns.
    ///
    /// Takes a [`crate::browser::SessionRef`]; its `data` is
    /// `{"session": id, "closed": true}`. Closing a
    /// session that is already gone succeeds: a host retrying a close must
    /// not have to distinguish "never existed" from "already cleaned up".
    pub const CLOSE_SESSION: &str = "BrowserCloseSession";

    /// Lists the sessions this module is holding open, a task's included.
    ///
    /// Takes nothing; its `data` is an array of
    /// [`crate::browser::SessionInfo`].
    pub const LIST_SESSIONS: &str = "BrowserListSessions";

    /// Navigates the session's active page.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::NavigateRequest`]; its `data` is the
    /// [`crate::browser::PageState`] the navigation settled on.
    pub const NAVIGATE: &str = "BrowserNavigate";

    /// Captures the accessibility tree of the active page, with element refs.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::SnapshotRequest`]; its `data` is a
    /// [`crate::browser::Snapshot`]. The refs it hands back are what
    /// [`PERFORM`] resolves as [`crate::browser::Target::Ref`].
    pub const SNAPSHOT: &str = "BrowserSnapshot";

    /// Performs one interaction against the active page.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::Action`]; its `data` is an
    /// [`crate::browser::ActionOutcome`].
    pub const PERFORM: &str = "BrowserPerform";

    /// Extracts the active page as agent-readable text.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::ReadRequest`]; its `data` is a
    /// [`crate::browser::PageText`].
    pub const READ_PAGE: &str = "BrowserReadPage";

    /// Evaluates JavaScript in the active page and returns its value.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::EvaluateRequest`]; its `data` is the resolved value
    /// as arbitrary JSON.
    pub const EVALUATE: &str = "BrowserEvaluate";

    /// Captures a screenshot and holds it for collection.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::ScreenshotRequest`]; its `data` is an
    /// [`crate::browser::OutputRef`] naming the held image. The image itself
    /// is pulled with [`READ_OUTPUT`] — see [`crate::browser::output`] for why
    /// it is not returned inline.
    pub const SCREENSHOT: &str = "BrowserScreenshot";

    /// Reads one chunk of a held output: a screenshot this interface took,
    /// or one a task's `TaskReport.artifacts` names (task views never carry
    /// one).
    ///
    /// Takes a [`crate::browser::ReadOutputRequest`]; its `data` is an
    /// [`crate::browser::OutputChunk`]. Requires confidential delivery: a
    /// screenshot can show what a task typed, card details included.
    pub const READ_OUTPUT: &str = "BrowserReadOutput";

    /// Releases a held output before it expires.
    ///
    /// Takes an [`crate::browser::OutputRequest`]; its `data` is
    /// `{"output": id, "released": true}`.
    /// Releasing an output that is already gone succeeds, for the same reason
    /// [`CLOSE_SESSION`] does.
    pub const RELEASE_OUTPUT: &str = "BrowserReleaseOutput";

    /// Lists retained downloads for one browser session.
    ///
    /// Takes a [`crate::browser::SessionRef`]; its `data` is an array of
    /// [`crate::browser::DownloadInfo`].
    pub const LIST_DOWNLOADS: &str = "BrowserListDownloads";

    /// Waits for the next terminal download not returned by an earlier wait.
    ///
    /// Takes a [`crate::browser::SessionRequest`] of
    /// [`crate::browser::DownloadWaitRequest`]; its `data` is a
    /// [`crate::browser::DownloadInfo`].
    pub const WAIT_DOWNLOAD: &str = "BrowserWaitDownload";

    /// Claims a persistent profile folder for a browser about to start,
    /// waiting for a closing Chrome that holds it.
    ///
    /// Takes a [`crate::browser::ClaimProfileRequest`]; its `data` is a
    /// [`crate::browser::ClaimProfileResponse`].
    pub const CLAIM_PROFILE: &str = "BrowserClaimProfile";

    /// Gives up a profile claim. Releasing a folder that is not claimed
    /// succeeds.
    ///
    /// Takes a [`crate::browser::ReleaseProfileRequest`]; its `data` is
    /// `{"path": path, "released": true}`.
    pub const RELEASE_PROFILE: &str = "BrowserReleaseProfile";
}

/// Every browser member of [`INTERFACE`], in dispatch order.
pub const METHODS: &[&str] = &[
    methods::OPEN_SESSION,
    methods::CLOSE_SESSION,
    methods::LIST_SESSIONS,
    methods::NAVIGATE,
    methods::SNAPSHOT,
    methods::PERFORM,
    methods::READ_PAGE,
    methods::EVALUATE,
    methods::SCREENSHOT,
    methods::READ_OUTPUT,
    methods::RELEASE_OUTPUT,
    methods::LIST_DOWNLOADS,
    methods::WAIT_DOWNLOAD,
    methods::CLAIM_PROFILE,
    methods::RELEASE_PROFILE,
];

#[cfg(test)]
mod names_tests;
