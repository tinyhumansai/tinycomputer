//! The calls on a session's page: navigation, snapshots, actions, reading,
//! evaluation, and screenshots.

use serde_json::Value;
use tinycomputer_bus::browser::{
    Action, ActionOutcome, EvaluateRequest, NavigateRequest, OutputRef, PageState, PageText,
    ReadFormat, ReadRequest, ScreenshotRequest, SessionId, Snapshot, SnapshotRequest, Target,
};

use super::Browser;
use crate::convert;
use crate::error::{Error, Result};
use crate::outputs::within_cap;
use crate::reply;

/// The fields of a raw command that can name a page to open or fetch.
const URL_FIELDS: &[&str] = &["url", "url1", "url2"];

/// The raw commands whose `url` names no page to open: a pattern to match
/// (`route`, `waitforurl`), a same-origin history entry (`pushstate`), or a
/// place to file credentials under. Every other raw command that names one
/// ([`URL_FIELDS`]) opens, fetches, or runs it (`navigate`, `tab_new`,
/// `a11y`, `vitals`, `auth_login`, `recording_start`, `diff_url`, and
/// `addscript`/`addstyle`, which put a caller's chosen file into the page),
/// so its address is checked before it is sent; an action the engine adds
/// later is checked too until it is listed here.
const URL_IS_NO_PAGE: &[&str] = &[
    "auth_save",
    "credentials_set",
    "frame",
    "pushstate",
    "responsebody",
    "route",
    "unroute",
    "wait",
    "waitforurl",
];

/// The raw commands that neither act on the page nor read what it shows: a
/// wait for it to load, and its address, which a refusal names anyway. No
/// check of the page goes around them, so a wait while a navigation commits
/// runs. A box's place on screen is checked: a selector can test whether a
/// text shows. So is a permission grant, which changes what pages may do.
const NEITHER_ACTS_NOR_READS: &[&str] = &["url", "waitforloadstate"];

impl Browser {
    /// Navigates the session's active page.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] for an empty URL,
    /// [`Error::BlockedByPolicy`] outside the allowed origins (refused before
    /// the browser is asked, or after a redirect that leaves them), and
    /// whatever else the navigation reports.
    pub async fn navigate(&self, id: &SessionId, request: NavigateRequest) -> Result<PageState> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        let command = convert::navigate(&request)?;
        if !session.origins.admits(&request.url) {
            return Err(Error::BlockedByPolicy { url: request.url });
        }
        let data = session.run(command).await?;
        // Under a list the page is read afresh: the engine reports the
        // address it was asked for when the one it reached cannot be read.
        let page = if session.origins.restricts() {
            session.page().await?
        } else {
            PageState {
                url: reply::text(&data, "url"),
                title: reply::text(&data, "title"),
                status: None,
            }
        };
        let page = session.admit(page).await?;
        session.info.url.clone_from(&page.url);
        session.info.title.clone_from(&page.title);
        Ok(page)
    }

    /// Captures the page's accessibility tree with element refs.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], and whatever the snapshot reports.
    pub async fn snapshot(&self, id: &SessionId, request: SnapshotRequest) -> Result<Snapshot> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        let data = session.run(convert::snapshot(&request)).await?;
        let page = session.page().await?;
        let page = session.admit(page).await?;
        session.sequence += 1;
        Ok(reply::snapshot(
            &data,
            page.url,
            page.title,
            session.sequence,
            request.max_chars,
        ))
    }

    /// Performs one interaction and reports the page it left.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] for an action the
    /// engine cannot express, [`Error::StaleRef`] for a ref from an earlier
    /// snapshot, [`Error::BlockedByPolicy`] when the page it would act on, or
    /// the page it leads to, is outside the allowed origins, and whatever
    /// else the action reports.
    pub async fn perform(&self, id: &SessionId, action: Action) -> Result<ActionOutcome> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        let command = convert::action(&action, session.options.default_timeout_ms)?;
        // The page may have moved on its own since the last call (a redirect,
        // a timer): an action is never sent to a refused one.
        session.check_if_readable(false).await?;
        let data = session.run(command).await?;
        let value = match &action {
            Action::GetText { .. } => data.get("text").cloned().unwrap_or(Value::Null),
            Action::GetAttribute { .. } => data.get("value").cloned().unwrap_or(Value::Null),
            Action::IsVisible { .. } => data.get("visible").cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let matched = match &action {
            Action::Click {
                target: Target::Locator { value },
                ..
            }
            | Action::Fill {
                target: Target::Locator { value },
                ..
            } => Some(format!("{:?} {:?}", value.by, value.value)),
            _ => None,
        };
        let page = session.page().await?;
        let page = session.admit(page).await?;
        session.info.url.clone_from(&page.url);
        session.info.title.clone_from(&page.title);
        Ok(ActionOutcome {
            value,
            page,
            matched,
        })
    }

    /// Extracts the active page as text, markdown, or HTML.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], and whatever the extraction reports.
    pub async fn read_page(&self, id: &SessionId, request: ReadRequest) -> Result<PageText> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        let data = session.run(convert::read(&request)).await?;
        let content = ["content", "text", "html"]
            .into_iter()
            .find_map(|key| data.get(key).and_then(Value::as_str))
            .unwrap_or_default();
        let truncated = content.chars().count() > request.max_chars
            || data.get("truncated").and_then(Value::as_bool) == Some(true);
        let content = content.chars().take(request.max_chars).collect();
        let page = session.page().await?;
        let page = session.admit(page).await?;
        Ok(PageText {
            url: page.url,
            title: page.title,
            format: if request.format == ReadFormat::Markdown && request.selector.is_some() {
                ReadFormat::Text
            } else {
                request.format
            },
            content,
            truncated,
        })
    }

    /// Runs one raw agent-browser command, such as
    /// `{"action": "inputvalue", "selector": "@e3"}`, and returns its `data`.
    ///
    /// This is the escape hatch for engine capabilities the typed calls do
    /// not cover. It carries no policy of its own beyond the allowed origins:
    /// a command that would open or fetch an address outside them is never
    /// sent, nor any command that acts on the page or reads what it shows
    /// while the session shows a refused page (only a wait for the page to
    /// load and a read of its address go ahead), and the page a command
    /// leaves the session on is checked before its result is returned. A
    /// caller exposing it to a model must still decide which actions to
    /// allow.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] when `command` names
    /// no `action`, [`Error::BlockedByPolicy`] for a page the allowed origins
    /// refuse, and whatever the engine reports.
    pub async fn command(&self, id: &SessionId, command: Value) -> Result<Value> {
        let Some(action) = command
            .get("action")
            .and_then(Value::as_str)
            .filter(|action| !action.is_empty())
        else {
            return Err(Error::invalid_input("a command needs an action"));
        };
        let session = self.session(id)?;
        let mut session = session.lock().await;
        if !URL_IS_NO_PAGE.contains(&action) {
            for field in URL_FIELDS {
                if let Some(url) = command.get(*field).and_then(Value::as_str)
                    && !session.origins.admits(url)
                {
                    return Err(Error::BlockedByPolicy {
                        url: url.to_owned(),
                    });
                }
            }
        }
        // `read` with an address fetches it from this process and follows
        // its redirects, where no page is ever checked.
        if action == "read"
            && session.origins.restricts()
            && let Some(url) = command.get("url").and_then(Value::as_str)
        {
            return Err(Error::BlockedByPolicy {
                url: url.to_owned(),
            });
        }
        if NEITHER_ACTS_NOR_READS.contains(&action) {
            return session.run(command).await;
        }
        session.check_if_readable(false).await?;
        let data = session.run(command).await?;
        // What a command read or did on a page it then left for a refused
        // one is never returned.
        session.check_if_readable(true).await?;
        Ok(data)
    }

    /// Evaluates JavaScript in the page and returns its value.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] for an empty
    /// expression, [`Error::BlockedByPolicy`] when the page it would run in,
    /// or the page it leads to, is outside the allowed origins, and
    /// [`Error::PageError`] when the script throws.
    pub async fn evaluate(&self, id: &SessionId, request: EvaluateRequest) -> Result<Value> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        let command = convert::evaluate(&request)?;
        session.check_if_readable(false).await?;
        let data = session.run(command).await?;
        session.check_if_readable(true).await?;
        Ok(data.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Captures a screenshot and holds it for collection with
    /// [`Browser::read_output`].
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] for a bad quality or
    /// a locator target, [`Error::LimitExceeded`] for an oversized image,
    /// [`Error::BlockedByPolicy`] when the page shown is outside the allowed
    /// origins, and whatever the capture reports.
    pub async fn screenshot(
        &self,
        id: &SessionId,
        request: ScreenshotRequest,
    ) -> Result<OutputRef> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        let extension = match request.format {
            tinycomputer_bus::browser::ImageFormat::Png => "png",
            tinycomputer_bus::browser::ImageFormat::Jpeg => "jpeg",
            tinycomputer_bus::browser::ImageFormat::Webp => "webp",
        };
        let path = session.scratch_file("shot", self.next(), extension)?;
        let command = convert::screenshot(&request, &path)?;
        session.check_if_readable(false).await?;
        let data = session.run(command).await?;
        let written = data
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(|| path.clone(), str::to_owned);
        let bytes = std::fs::read(&written)
            .map_err(|error| Error::failed(format!("screenshot was not written: {error}")))?;
        let _removed = std::fs::remove_file(&written);
        // A picture of a page that turned out refused is never handed over.
        session.check_if_readable(true).await?;
        within_cap(bytes.len().div_ceil(3) * 4)?;
        let (width, height) = reply::image_size(&bytes);
        self.lock_outputs()?
            .insert(bytes, request.format.media_type(), width, height)
    }
}
