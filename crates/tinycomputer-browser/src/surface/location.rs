//! Letting a page read where the person is, when the agent presses the
//! page's own "use my current location" button.

use serde_json::json;
use tinycomputer_core::surface::Candidate;

use super::BrowserSurface;

/// Words of a control that asks the page to find where the person is.
const ASKS_WHERE: &[&str] = &[
    "current location",
    "my location",
    "detect location",
    "detect my location",
    "locate me",
    "use location",
    "use my current",
];

/// Whether pressing `target` asks the page for the person's location.
pub(super) fn asks_where(target: &Candidate) -> bool {
    let said = format!(
        "{} {}",
        target.name.as_deref().unwrap_or_default(),
        target.description.as_deref().unwrap_or_default()
    )
    .to_lowercase()
    .replace('-', " ");
    ASKS_WHERE.iter().any(|words| said.contains(words))
}

impl BrowserSurface {
    /// Grants the session's pages the location permission before a press
    /// that asks for it (`asks_where`). The browser asks a person in a
    /// bubble outside the page, which the agent can neither see nor press,
    /// so a store's "use my current location" waited on it forever; the
    /// press stands in for that person's "Allow". Best effort: a browser
    /// that refuses is pressed as it is.
    ///
    /// Only in a browser the module launched, on a throwaway profile or the
    /// one a task keeps between runs (a `user_data_dir`): the grant covers
    /// every page of that browser while it runs, and is not written into
    /// the profile. A person's own running browser (an `endpoint`) keeps its
    /// own say, in the bubble they can answer.
    pub(super) fn allow_location(&self) {
        if self.options.endpoint.is_some() {
            return;
        }
        let Ok(id) = self.ensure_session() else {
            return;
        };
        let _granted = self.block(self.browser.command(
            &id,
            json!({"action": "permissions", "permissions": ["geolocation"]}),
        ));
    }
}
