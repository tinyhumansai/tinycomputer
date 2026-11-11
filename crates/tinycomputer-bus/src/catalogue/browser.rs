//! The catalogue entries for the browser primitives.

use crate::browser::names::methods as browser;

use super::{Family, Member, entry};

/// The browser members, in [`crate::names::METHODS`] order.
pub(super) const BROWSER: &[Member] = &[
    entry(
        browser::OPEN_SESSION,
        Family::Browser,
        "Launches or attaches a browser and returns its session.",
        false,
    ),
    entry(
        browser::CLOSE_SESSION,
        Family::Browser,
        "Closes a session and everything it owns; closing a gone one succeeds.",
        false,
    ),
    entry(
        browser::LIST_SESSIONS,
        Family::Browser,
        "Lists the browser sessions this module holds, a task's included.",
        false,
    ),
    entry(
        browser::NAVIGATE,
        Family::Browser,
        "Navigates a session's active page and reports where it settled.",
        false,
    ),
    entry(
        browser::SNAPSHOT,
        Family::Browser,
        "Captures the active page's accessibility tree with element refs.",
        false,
    ),
    entry(
        browser::PERFORM,
        Family::Browser,
        "Performs one interaction — click, fill, press, scroll, wait — on the active page.",
        false,
    ),
    entry(
        browser::READ_PAGE,
        Family::Browser,
        "Extracts the active page as text or markdown.",
        false,
    ),
    entry(
        browser::EVALUATE,
        Family::Browser,
        "Evaluates JavaScript in the active page and returns its value.",
        false,
    ),
    entry(
        browser::SCREENSHOT,
        Family::Browser,
        "Captures the page and holds the image for BrowserReadOutput.",
        false,
    ),
    entry(
        browser::READ_OUTPUT,
        Family::Browser,
        "Reads one base64 chunk of a held screenshot or PDF.",
        true,
    ),
    entry(
        browser::RELEASE_OUTPUT,
        Family::Browser,
        "Releases a held output before it expires.",
        false,
    ),
    entry(
        browser::LIST_DOWNLOADS,
        Family::Browser,
        "Lists a session's retained downloads.",
        false,
    ),
    entry(
        browser::WAIT_DOWNLOAD,
        Family::Browser,
        "Waits for a session's next download to finish.",
        false,
    ),
    entry(
        browser::CLAIM_PROFILE,
        Family::Browser,
        "Claims a persistent browser profile folder, waiting for a closing Chrome.",
        false,
    ),
    entry(
        browser::RELEASE_PROFILE,
        Family::Browser,
        "Gives up a browser profile claim.",
        false,
    ),
];
