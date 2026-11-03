//! Keeping what a press opens in the tab the agent reads, and taking a
//! page that drew before it finished loading as open.

use serde_json::{Value, json};
use tinycomputer_bus::browser::SessionId;

use super::{BrowserSurface, READ_TIMEOUT, sight};

/// Where the page is, what it is called, and whether it has drawn words.
const DRAWN_JS: &str = r"(() => ({
  url: location.href,
  title: document.title,
  drawn: document.readyState !== 'loading' && !!document.body
    && document.body.innerText.trim().length > 0,
}))()";

/// `url` as host, path, and query, without its scheme, a leading `www.`,
/// its fragment, or the path's trailing slash, the host and path
/// lower-cased: two addresses of one page. The query stays: a search for
/// "boots" on screen is no page of a search for "shoes".
pub(super) fn place(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split('#').next().unwrap_or_default();
    let (path, query) = rest
        .split_once('?')
        .map_or((rest, ""), |(path, query)| (path, query));
    let path = path.trim_end_matches('/').to_ascii_lowercase();
    let path = path
        .strip_prefix("www.")
        .map_or(path.clone(), str::to_owned);
    if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    }
}

/// Points the pressed element's own link or form, when it would open a new
/// tab, at the page's own tab, and, for two seconds, sends a script's
/// `window.open(url)` of an address on the same site there too. A result
/// card's link that opens its product in a new tab left the agent reading
/// the results page live: the new tab never became the session's page, so
/// the next read found no product. Only the pressed element's link or form
/// changes, never the rest of the page's; an address on another site (an
/// advert a page opens on the first click) still opens its own window, as
/// does an `open` with no address, which a page fills in later; a link
/// aimed at a named frame is left alone; and the patched `open` returns no
/// window, so a page closing "its" window never closes the agent's tab.
/// Called as `(element) => true`.
const SAME_TAB_JS: &str = r"(element => {
  const blank = (aimed) => aimed === '_blank' || aimed === '_new';
  const base = (document.querySelector('base[target]')?.getAttribute('target') || '').toLowerCase();
  const link = element && element.closest('a[href], area[href]');
  const form = element && (element.form || element.closest('form'));
  for (const node of [link, form]) {
    if (!node) continue;
    const aimed = (node.getAttribute('target') ?? base).toLowerCase();
    if (blank(aimed)) node.setAttribute('target', '_self');
  }
  // A page that begins to leave says so before its address changes, so
  // a slow link is not followed a second time while it loads.
  if (!window.__tcWatching) {
    window.__tcWatching = true;
    addEventListener('beforeunload', () => { window.__tcLeaving = true; });
  }
  if (window.__tcOpen) return true;
  const open = window.open;
  window.__tcOpen = open;
  window.open = function (url, name) {
    const aimed = String(name || '').toLowerCase();
    let here = false;
    try { here = new URL(String(url), location.href).origin === location.origin; } catch (error) { here = false; }
    if (url && String(url) !== 'about:blank' && here && (!aimed || blank(aimed))) {
      location.assign(url);
      return null;
    }
    return open.apply(window, arguments);
  };
  setTimeout(() => { window.open = open; delete window.__tcOpen; }, 2000);
  return true;
})";

impl BrowserSurface {
    /// The page's address and title when session `id` shows `url` drawn
    /// with words, though its navigation timed out waiting for `load`: a
    /// heavy page keeps fetching long after it can be read (live, a store's
    /// results page). `None` when it shows another page, or nothing yet.
    pub(super) fn drawn_page(&self, id: &SessionId, url: &str) -> Option<(String, String)> {
        self.shown_page(id)
            .filter(|(shown, _)| place(shown) == place(url))
    }

    /// The address and title of the page session `id` shows, once it has
    /// drawn words; `None` while it shows nothing yet.
    pub(super) fn shown_page(&self, id: &SessionId) -> Option<(String, String)> {
        let reading = self
            .browser
            .command(id, json!({"action": "evaluate", "script": DRAWN_JS}));
        // Within a deadline: a call sent while a page is being replaced (a
        // redirect, a challenge's reload) can wait out the browser's own 30 s.
        let data = self
            .block(async { tokio::time::timeout(READ_TIMEOUT, reading).await })
            .ok()?
            .ok()?;
        let page = data.get("result")?;
        let shown = page.get("url").and_then(Value::as_str)?;
        let drawn = page.get("drawn").and_then(Value::as_bool).unwrap_or(false);
        drawn.then(|| {
            (
                shown.to_owned(),
                page.get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
    }

    /// Keeps the press of `reference` about to happen in this tab
    /// (`SAME_TAB_JS`). Best effort: a page that refuses the script, or an
    /// element it cannot find, is pressed as it is.
    pub(super) fn keep_in_tab(&self, reference: &str) {
        // Sight refs are CSS marks. A tree ref such as `e3` addresses the
        // browser's own snapshot and cannot be resolved by document.querySelector.
        if !sight::is_seen(reference) {
            return;
        }
        let selector = Value::String(sight::selector(reference));
        let script = format!("{SAME_TAB_JS}(document.querySelector({selector}))");
        if let Ok(id) = self.ensure_session() {
            let _kept = self.block(
                self.browser
                    .command(&id, json!({"action": "evaluate", "script": script})),
            );
        }
    }
}
