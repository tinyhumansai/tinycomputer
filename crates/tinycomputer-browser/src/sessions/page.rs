//! The calls on a session's page: navigation, snapshots, actions, reading,
//! evaluation, and screenshots.

use serde_json::Value;
use tinycomputer_bus::browser::{
    Action, ActionOutcome, EvaluateRequest, NavigateRequest, OutputRef, PageState, PageText,
    ReadFormat, ReadRequest, ScreenshotRequest, SessionId, Snapshot, SnapshotRequest, Target,
};

use super::{Browser, Session};
use crate::action_gate::{ActionRequest, ObservationKind};
use crate::convert;
use crate::deep;
use crate::error::{Error, Result};
use crate::outputs::within_cap;
use crate::reply;
use crate::{ActionObservation, ObservedAction, ObservedTarget};

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

fn raw_destination(command: &Value) -> Option<String> {
    URL_FIELDS
        .iter()
        .find_map(|field| command.get(*field).and_then(Value::as_str))
        .map(str::to_owned)
}

impl Browser {
    /// Navigates the session's active page.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] for an empty URL,
    /// [`Error::ActionRefused`] when the trusted host refuses it,
    /// [`Error::BlockedByPolicy`] outside the allowed origins (refused before
    /// the browser is asked, or after a redirect that leaves them), and
    /// whatever else the navigation reports.
    pub async fn navigate(&self, id: &SessionId, request: NavigateRequest) -> Result<PageState> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        let command = convert::navigate(&request)?;
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::raw(
            command.clone(),
            current_url.clone(),
            Some(request.url.clone()),
            &session.action_context,
            &session.refs,
        ))?;
        if !session.origins.admits(&request.url) {
            return Err(Error::BlockedByPolicy { url: request.url });
        }
        let data = session.run(command).await?;
        session.deep.follow("navigate");
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
        if let Some(before_url) = current_url {
            self.observe_action(ActionObservation {
                task_id: session.action_context.task_id.clone(),
                session_id: session.info.id.to_string(),
                action: ObservedAction::Navigate,
                before_url,
                after_url: page.url.clone(),
                target: None,
            });
        }
        Ok(page)
    }

    /// Captures the page's accessibility tree with element refs.
    ///
    /// The controls a page keeps inside open shadow roots that the tree
    /// gives no ref (a component's own clickable `span` or `div`, an
    /// editable region) are looked for after agent-browser replies, and
    /// listed with refs of their own, `d1`, `d2`, …: on the line that shows
    /// their words, or after the tree (`crate::deep`). A page with none
    /// comes out as agent-browser gave it.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], and whatever the snapshot reports.
    pub async fn snapshot(&self, id: &SessionId, request: SnapshotRequest) -> Result<Snapshot> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        // The URL lookup above is an internal engine command and is not sent
        // through the gate. Read the live address once more immediately before
        // content is observed, without recursively gating that URL lookup.
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::observation(
            ObservationKind::Snapshot,
            current_url,
            &session.action_context,
        ))?;
        let data = session.run(convert::snapshot(&request)).await?;
        let controls = session.find_deep(request.selector.as_deref()).await;
        let page = session.page().await?;
        let page = session.admit(page).await?;
        self.check_action(&ActionRequest::observation(
            ObservationKind::Snapshot,
            Some(page.url.clone()),
            &session.action_context,
        ))?;
        session.sequence += 1;
        let mut snapshot = reply::snapshot(
            &data,
            page.url,
            page.title,
            session.sequence,
            request.max_chars,
        );
        deep::place(
            &mut snapshot,
            &reply::text(&data, "snapshot"),
            &controls,
            request.max_chars,
        );
        session.refs.clone_from(&snapshot.refs);
        Ok(snapshot)
    }

    /// Performs one interaction and reports the page it left.
    ///
    /// Keys go only where the action points: an element focused, filled,
    /// typed into, or sent a key is given the focus first, and the action is
    /// refused unless the focus lands on it (see `sessions/focus.rs`). A
    /// locator's fill, which only the engine resolves, has the focus taken
    /// off whatever held it first, so its text lands in the field or nowhere.
    ///
    /// An action on a deep ref (`d1`, from [`Browser::snapshot`]) is the
    /// session's own, as agent-browser cannot reach the element: it is found
    /// by its mark, and pressed, hovered, typed into, or read where it
    /// shows, with the pointer's and keyboard's own input events
    /// (`sessions/deep.rs`). `Click`, `Hover`, `Focus`, `Type`, `Fill`,
    /// `Press`, `GetText`, and `IsVisible` take one.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::InvalidInput`] for an action the
    /// engine cannot express or a deep ref cannot take, [`Error::StaleRef`]
    /// for a ref from an earlier snapshot or a deep ref whose element is
    /// gone, [`Error::NotActionable`] for a deep ref's element covered or
    /// not shown, [`Error::FocusMissed`] when the focus does not land on the
    /// element keys were meant for, [`Error::BlockedByPolicy`] when the page
    /// it would act on, or the page it leads to, is outside the allowed
    /// origins, and whatever else the action reports.
    pub async fn perform(&self, id: &SessionId, action: Action) -> Result<ActionOutcome> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        if let Some(number) = deep::aimed_at(&action) {
            deep::supports(&action)?;
            // The page may have moved on its own since the last call: an
            // action is never sent to a refused one.
            session.check_if_readable(false).await?;
            let current_url = session.action_url().await;
            let request = ActionRequest::typed(
                &action,
                current_url.clone(),
                None,
                &session.action_context,
                &session.refs,
            );
            let target = observed_target(request.target.as_ref(), &session.refs);
            self.check_action(&request)?;
            let value = session.act_deep(number, &action).await?;
            let outcome = session.outcome(value, None).await?;
            if let Some(before_url) = current_url {
                self.observe_action(ActionObservation {
                    task_id: session.action_context.task_id.clone(),
                    session_id: session.info.id.to_string(),
                    action: observed_action(&action),
                    before_url,
                    after_url: outcome.page.url.clone(),
                    target,
                });
            }
            return Ok(outcome);
        }
        let command = convert::action(&action, session.options.default_timeout_ms)?;
        let keys = keys_go_to(&action)?;
        // The page may have moved on its own since the last call (a redirect,
        // a timer): an action is never sent to a refused one.
        session.check_if_readable(false).await?;
        let current_url = session.action_url().await;
        let request = ActionRequest::typed(
            &action,
            current_url.clone(),
            None,
            &session.action_context,
            &session.refs,
        );
        let target = observed_target(request.target.as_ref(), &session.refs);
        self.check_action(&request)?;
        match keys {
            Keys::Checked(selector) => session.focus_only(&selector).await?,
            Keys::Blurred => session.blur().await?,
            Keys::Anywhere => {}
        }
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
        let outcome = session.outcome(value, matched).await?;
        if let Some(before_url) = current_url {
            self.observe_action(ActionObservation {
                task_id: session.action_context.task_id.clone(),
                session_id: session.info.id.to_string(),
                action: observed_action(&action),
                before_url,
                after_url: outcome.page.url.clone(),
                target,
            });
        }
        Ok(outcome)
    }

    /// Checks the configured action gate for a host-owned secret fill without
    /// sending any text to the page.
    ///
    /// # Errors
    ///
    /// Returns the browser/session error or the action gate's refusal. The
    /// current URL must be readable before a host secret can be typed.
    pub async fn authorize_secret_type(
        &self,
        id: &SessionId,
        role: &str,
        name: Option<&str>,
        reference: &str,
    ) -> Result<String> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        let current_url = session
            .action_url()
            .await
            .ok_or_else(|| Error::invalid_input("the current page URL is unknown"))?;
        let reference = reference.strip_prefix('@').unwrap_or(reference);
        self.check_action(&ActionRequest {
            payload: serde_json::json!({
                "action": "fill",
                "selector": format!("@{reference}"),
                "value": "[host secret]",
            }),
            target: Some(crate::action_gate::ActionTarget {
                role: Some(role.to_owned()),
                name: name.map(str::to_owned),
                reference: Some(reference.to_owned()),
                unique: false,
            }),
            current_url: Some(current_url.clone()),
            destination_url: None,
            observation: None,
            context: session.action_context.clone(),
        })?;
        Ok(current_url)
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
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::observation(
            ObservationKind::ReadPage,
            current_url,
            &session.action_context,
        ))?;
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
        self.check_action(&ActionRequest::observation(
            ObservationKind::ReadPage,
            Some(page.url.clone()),
            &session.action_context,
        ))?;
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
    /// not cover. The trusted host's [`ActionGate`](crate::ActionGate) is
    /// consulted before the raw command is sent:
    /// a command that would open or fetch an address outside them is never
    /// sent, nor any command that acts on the page or reads what it shows
    /// while the session shows a refused page (only a wait for the page to
    /// load and a read of its address go ahead), and the page a command
    /// leaves the session on is checked before its result is returned. A
    /// A host that does not install a gate allows everything by default.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::ActionRefused`] when the trusted
    /// host refuses it, [`Error::InvalidInput`] when `command` names
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
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::raw(
            command.clone(),
            current_url,
            raw_destination(&command),
            &session.action_context,
            &session.refs,
        ))?;
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
        let action = action.to_owned();
        session.check_if_readable(false).await?;
        let data = session.run(command).await?;
        // A frame agent-browser now reads, or reads no more: the look for
        // deep controls follows it.
        session.deep.follow(&action);
        // What a command read or did on a page it then left for a refused
        // one is never returned.
        session.check_if_readable(true).await?;
        Ok(data)
    }

    /// Runs the browser surface's fixed-purpose stability watcher. This is
    /// crate-private so callers cannot label arbitrary evaluation as an
    /// observation or bypass the host action gate.
    pub(crate) async fn observe_page_stability(
        &self,
        id: &SessionId,
        script: String,
    ) -> Result<Value> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        let current_url = session
            .action_url()
            .await
            .ok_or_else(|| Error::invalid_input("the current page URL is unknown"))?;
        self.check_action(&ActionRequest::observation(
            ObservationKind::PageStability,
            Some(current_url),
            &session.action_context,
        ))?;
        let data = session
            .run(serde_json::json!({"action": "evaluate", "script": script}))
            .await?;
        let page = session.page().await?;
        let page = session.admit(page).await?;
        self.check_action(&ActionRequest::observation(
            ObservationKind::PageStability,
            Some(page.url.clone()),
            &session.action_context,
        ))?;
        session.info.url.clone_from(&page.url);
        Ok(data)
    }

    /// Runs the browser surface's fixed-purpose cover hit-test. This stays
    /// crate-private so arbitrary evaluations cannot claim observation status.
    pub(crate) async fn observe_target_check(
        &self,
        id: &SessionId,
        script: String,
    ) -> Result<Value> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        let current_url = session
            .action_url()
            .await
            .ok_or_else(|| Error::invalid_input("the current page URL is unknown"))?;
        self.check_action(&ActionRequest::observation(
            ObservationKind::TargetCheck,
            Some(current_url),
            &session.action_context,
        ))?;
        let data = session
            .run(serde_json::json!({"action": "evaluate", "script": script}))
            .await?;
        let page = session.page().await?;
        let page = session.admit(page).await?;
        self.check_action(&ActionRequest::observation(
            ObservationKind::TargetCheck,
            Some(page.url.clone()),
            &session.action_context,
        ))?;
        session.info.url.clone_from(&page.url);
        Ok(data)
    }

    /// Runs the browser surface's Sight snapshot script. This is crate-private
    /// so raw evaluations cannot claim Snapshot observation authority.
    pub(crate) async fn observe_sight(&self, id: &SessionId, script: String) -> Result<Value> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check_if_readable(false).await?;
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::observation(
            ObservationKind::Snapshot,
            current_url,
            &session.action_context,
        ))?;
        let data = session
            .run(serde_json::json!({"action": "evaluate", "script": script}))
            .await?;
        let page = session.page().await?;
        let page = session.admit(page).await?;
        self.check_action(&ActionRequest::observation(
            ObservationKind::Snapshot,
            Some(page.url.clone()),
            &session.action_context,
        ))?;
        session.info.url.clone_from(&page.url);
        Ok(data)
    }

    /// Evaluates JavaScript in the page and returns its value.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], [`Error::ActionRefused`] when the trusted
    /// host refuses it, [`Error::InvalidInput`] for an empty
    /// expression, [`Error::BlockedByPolicy`] when the page it would run in,
    /// or the page it leads to, is outside the allowed origins, and
    /// [`Error::PageError`] when the script throws.
    pub async fn evaluate(&self, id: &SessionId, request: EvaluateRequest) -> Result<Value> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        let command = convert::evaluate(&request)?;
        session.check_if_readable(false).await?;
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::raw(
            command.clone(),
            current_url,
            None,
            &session.action_context,
            &session.refs,
        ))?;
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
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::observation(
            ObservationKind::Screenshot,
            current_url,
            &session.action_context,
        ))?;
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
        let current_url = session.action_url().await;
        self.check_action(&ActionRequest::observation(
            ObservationKind::Screenshot,
            current_url,
            &session.action_context,
        ))?;
        within_cap(bytes.len().div_ceil(3) * 4)?;
        let (width, height) = reply::image_size(&bytes);
        self.lock_outputs()?
            .insert(bytes, request.format.media_type(), width, height)
    }
}

fn observed_target(
    target: Option<&crate::action_gate::ActionTarget>,
    refs: &[tinycomputer_bus::browser::ElementRef],
) -> Option<ObservedTarget> {
    let target = target?;
    let name = target.name.clone();
    let reference = target.reference.clone();
    let matches = refs
        .iter()
        .filter(|item| Some(item.role.as_str()) == target.role.as_deref())
        .filter(|item| Some(item.name.as_str()) == target.name.as_deref())
        .collect::<Vec<_>>();
    let unique = target
        .name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
        && matches.len() == 1
        && reference.as_deref() == Some(matches[0].id.as_str());
    Some(ObservedTarget {
        role: target.role.clone()?,
        name,
        reference,
        unique,
    })
}

fn observed_action(action: &Action) -> ObservedAction {
    match action {
        Action::Click { .. } => ObservedAction::Click,
        Action::Fill { .. } => ObservedAction::Fill,
        Action::Type { .. } => ObservedAction::Type,
        Action::Select { .. } => ObservedAction::Select,
        Action::Check { checked: true, .. } => ObservedAction::Check,
        Action::Check { checked: false, .. } => ObservedAction::Uncheck,
        Action::DoubleClick { .. } => ObservedAction::DoubleClick,
        Action::Hover { .. } => ObservedAction::Hover,
        Action::Focus { .. } => ObservedAction::Focus,
        Action::Scroll { .. } => ObservedAction::Scroll,
        Action::GetText { .. } | Action::GetAttribute { .. } | Action::IsVisible { .. } => {
            ObservedAction::Read
        }
        Action::WaitFor { .. } => ObservedAction::Wait,
        Action::Press { key, .. } if key.eq_ignore_ascii_case("escape") => {
            ObservedAction::PressEscape
        }
        Action::Press { .. } => ObservedAction::Press,
        Action::Back | Action::Forward | Action::Reload => ObservedAction::Other,
    }
}

impl Session {
    /// What an action read, with the page it left the session on: refused,
    /// and left, when the allowed origins refuse it.
    async fn outcome(&mut self, value: Value, matched: Option<String>) -> Result<ActionOutcome> {
        let page = self.page().await?;
        let page = self.admit(page).await?;
        self.info.url.clone_from(&page.url);
        self.info.title.clone_from(&page.title);
        Ok(ActionOutcome {
            value,
            page,
            matched,
        })
    }
}

/// How an action's keys, or focus, are kept on its target.
enum Keys {
    /// The target's selector: focus it and check the focus landed there.
    Checked(String),
    /// A target only the engine resolves: take the focus off whatever held it.
    Blurred,
    /// Nothing to keep on a target: no keys, or keys for whatever has focus.
    Anywhere,
}

/// Where `action` sends keys or focus.
///
/// # Errors
///
/// [`Error::InvalidInput`] for a press on a locator, which has no selector to
/// check the focus against.
fn keys_go_to(action: &Action) -> Result<Keys> {
    Ok(match action {
        Action::Focus { target } | Action::Fill { target, .. } => match target {
            Target::Locator { .. } => Keys::Blurred,
            other => Keys::Checked(convert::selector(other, "focus")?),
        },
        Action::Type {
            target: Some(target),
            ..
        }
        | Action::Press {
            target: Some(target),
            ..
        } => Keys::Checked(convert::selector(target, "press")?),
        _ => Keys::Anywhere,
    })
}
