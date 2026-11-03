//! [`Browser`]: sessions, the calls on them, and the outputs they produce.
//!
//! Each session owns one [`Engine`] and a private scratch directory where
//! agent-browser writes screenshots and downloads for this crate to collect.
//! Calls on one session are serialized; different sessions run independently.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tinycomputer_bus::browser::ElementRef;
use tinycomputer_bus::browser::{DownloadInfo, PageState, SessionId, SessionInfo, SessionOptions};

use crate::action_gate::{ActionContext, ActionGate, default_gate};
use crate::action_gate::{ActionRequest, ActionVerdict};
use crate::convert;
use crate::engine::{Engine, Launcher};
use crate::error::{Error, Result};
use crate::origins::Origins;
use crate::outputs::OutputStore;
use crate::reply;
use crate::{ActionObservation, ActionObserver};

mod artifacts;
mod deep;
mod focus;
mod page;

/// How many sessions may be open at once.
pub const MAX_SESSIONS: usize = 8;

/// How long a page check waits before reading the page's address a second
/// time: a page committing a navigation has none for a moment.
const ADDRESS_RETRY: std::time::Duration = std::time::Duration::from_millis(100);

/// What the engine says when a page is committing a navigation and its
/// address cannot be read for a moment, in lower case.
const NAVIGATING: &[&str] = &[
    "execution context was destroyed",
    "cannot find default execution context",
    "cannot find context with specified id",
    "inspected target navigated or closed",
];

/// Whether `error` only says the page was committing a navigation.
fn navigating(error: &Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    NAVIGATING.iter().any(|said| message.contains(said))
}

/// Browser automation over agent-browser, one engine per session.
pub struct Browser {
    launcher: Arc<dyn Launcher>,
    sessions: Mutex<HashMap<SessionId, Arc<tokio::sync::Mutex<Session>>>>,
    outputs: Mutex<OutputStore>,
    scratch: PathBuf,
    counter: AtomicU64,
    /// Sessions being launched: they count against [`MAX_SESSIONS`] from
    /// the moment the limit is checked, so two concurrent opens cannot both
    /// see room for the last slot.
    opening: AtomicUsize,
    action_gate: Arc<dyn ActionGate>,
    action_observer: Arc<RwLock<Option<Arc<dyn ActionObserver>>>>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Browser")
            .field("scratch", &self.scratch)
            .field(
                "session_count",
                &self.sessions.lock().map(|sessions| sessions.len()).ok(),
            )
            .finish_non_exhaustive()
    }
}

/// One reserved launch slot, given back when the launch ends — inserted,
/// failed, or dropped mid-launch by a cancelled caller.
struct Reservation<'a>(&'a AtomicUsize);

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Session {
    engine: Box<dyn Engine>,
    info: SessionInfo,
    options: SessionOptions,
    /// The pages `options.allowed_origins` admits.
    origins: Origins,
    sequence: u64,
    /// The deep refs the latest snapshot handed out (`crate::deep`).
    deep: crate::deep::Minted,
    downloads: Vec<DownloadInfo>,
    dir: PathBuf,
    action_context: ActionContext,
    refs: Vec<ElementRef>,
    closing: bool,
    closed: bool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("info", &self.info)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

impl Session {
    async fn run(&mut self, command: Value) -> Result<Value> {
        if self.closing || self.closed {
            return Err(Error::NoSuchSession {
                id: self.info.id.to_string(),
            });
        }
        reply::data(&self.engine.execute(command).await)
    }

    async fn close(&mut self) -> Result<()> {
        self.closing = true;
        reply::data(&self.engine.execute(json!({"action": "close"})).await).map(|_| ())
    }

    /// The current address for gate context, or `None` while the page is
    /// briefly between documents. Never substitute a cached, potentially
    /// stale address for the location of the page about to be acted on.
    async fn action_url(&mut self) -> Option<String> {
        match self.run(json!({"action": "url"})).await {
            Ok(data) => Some(reply::text(&data, "url")),
            Err(_) => None,
        }
    }

    async fn page(&mut self) -> Result<PageState> {
        let url = reply::text(&self.run(json!({"action": "url"})).await?, "url");
        let title = reply::text(&self.run(json!({"action": "title"})).await?, "title");
        Ok(PageState {
            url,
            title,
            status: None,
        })
    }

    /// `page`, the page a call left the session on, unless the allowed
    /// origins refuse it: then the session leaves it, and the call reports
    /// [`Error::LeftRefusedPage`], since it ran. A click, a key, a redirect,
    /// or the page's own script can take a session anywhere; only what it
    /// then shows is checked, never the files it loads.
    async fn admit(&mut self, page: PageState) -> Result<PageState> {
        if self.origins.admits(&page.url) {
            return Ok(page);
        }
        self.leave().await;
        Err(Error::LeftRefusedPage { url: page.url })
    }

    /// The address of the page the session shows, asked twice before its
    /// failure is reported.
    async fn address(&mut self) -> Result<String> {
        if let Ok(data) = self.run(json!({"action": "url"})).await {
            return Ok(reply::text(&data, "url"));
        }
        tokio::time::sleep(ADDRESS_RETRY).await;
        Ok(reply::text(
            &self.run(json!({"action": "url"})).await?,
            "url",
        ))
    }

    /// Checks the page the session shows against its allowed origins, and
    /// leaves a refused one; free when the session has no list. A page found
    /// `after` the call's own work is reported as [`Error::LeftRefusedPage`],
    /// one found before it as [`Error::BlockedByPolicy`].
    ///
    /// # Errors
    ///
    /// The refusal, and whatever reading the page's address reports.
    async fn check(&mut self, after: bool) -> Result<()> {
        if !self.origins.restricts() {
            return Ok(());
        }
        let url = self.address().await?;
        if self.origins.admits(&url) {
            return Ok(());
        }
        self.leave().await;
        Err(if after {
            Error::LeftRefusedPage { url }
        } else {
            Error::BlockedByPolicy { url }
        })
    }

    /// [`Session::check`] around a call's own work, which goes ahead when
    /// the page's address cannot be read because the page is committing a
    /// navigation ([`NAVIGATING`]): it has none for a moment, and a wait for
    /// it to load must not fail then. What it reaches is checked after it,
    /// and before the next observation. Any other failure to read the
    /// address fails the call.
    ///
    /// # Errors
    ///
    /// The refusal, and a failure to read the address other than a
    /// navigation's.
    async fn check_if_readable(&mut self, after: bool) -> Result<()> {
        match self.check(after).await {
            Err(error) if navigating(&error) => Ok(()),
            other => other,
        }
    }

    /// Leaves a refused page: back, or to `about:blank` when going back does
    /// not reach an admitted page. Best effort: a page that cannot be left is
    /// still never worked on, as every checked call refuses it.
    async fn leave(&mut self) {
        let _back = self.run(json!({"action": "back"})).await;
        let back_on = self
            .run(json!({"action": "url"}))
            .await
            .map(|data| reply::text(&data, "url"))
            .unwrap_or_default();
        if !self.origins.admits(&back_on) {
            let _blank = self
                .run(json!({"action": "navigate", "url": "about:blank"}))
                .await;
        }
        if let Ok(page) = self.page().await {
            self.info.url = page.url;
            self.info.title = page.title;
        }
    }

    fn scratch_file(&self, stem: &str, sequence: u64, extension: &str) -> Result<String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| Error::failed(format!("cannot create scratch space: {error}")))?;
        Ok(self
            .dir
            .join(format!("{stem}-{sequence}.{extension}"))
            .to_string_lossy()
            .into_owned())
    }
}

impl Browser {
    /// A browser whose sessions are opened by `launcher`.
    ///
    /// Screenshots and downloads are staged under the system temporary
    /// directory, one private folder per session, and removed as they are
    /// collected or the session closes.
    #[must_use]
    pub fn new(launcher: Arc<dyn Launcher>) -> Self {
        Self::with_scratch(
            launcher,
            std::env::temp_dir().join(format!("tinycomputer-browser-{}", std::process::id())),
        )
    }

    /// Like [`Browser::new`], staging files under `scratch`.
    #[must_use]
    pub fn with_scratch(launcher: Arc<dyn Launcher>, scratch: PathBuf) -> Self {
        Self {
            launcher,
            sessions: Mutex::new(HashMap::new()),
            outputs: Mutex::new(OutputStore::default()),
            scratch,
            counter: AtomicU64::new(0),
            opening: AtomicUsize::new(0),
            action_gate: default_gate(),
            action_observer: Arc::default(),
        }
    }

    /// Sets the trusted host policy consulted before browser actions.
    #[must_use]
    pub fn with_action_gate(mut self, gate: Arc<dyn ActionGate>) -> Self {
        self.action_gate = gate;
        self
    }

    /// Installs a trusted host observer for successful browser actions.
    pub fn set_action_observer(&self, observer: Option<Arc<dyn ActionObserver>>) {
        if let Ok(mut current) = self.action_observer.write() {
            *current = observer;
        }
    }

    pub(crate) fn observe_action(&self, observation: ActionObservation) {
        if let Ok(observer) = self.action_observer.read()
            && let Some(observer) = observer.as_ref()
        {
            observer.observed(observation);
        }
    }

    pub(crate) fn check_action(&self, request: &ActionRequest) -> Result<()> {
        match self.action_gate.check(request) {
            ActionVerdict::Allow => Ok(()),
            ActionVerdict::Refuse { reason } => Err(Error::ActionRefused { reason }),
        }
    }

    /// Launches (or attaches to) a browser and returns the session owning it.
    ///
    /// # Errors
    ///
    /// [`Error::LimitExceeded`] when [`MAX_SESSIONS`] are open,
    /// [`Error::BlockedByPolicy`] when the browser shows a page the allowed
    /// origins refuse and it cannot be left, [`Error::BrowserUnavailable`]
    /// saying what to set when a browser to launch is not found or will not
    /// start, and whatever the engine reports when one cannot be reached.
    pub async fn open_session(&self, options: SessionOptions) -> Result<SessionInfo> {
        self.open_session_with_context(options, ActionContext::default())
            .await
    }

    /// Opens a session with host-supplied task context for its action gate.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Browser::open_session`].
    pub async fn open_session_with_context(
        &self,
        options: SessionOptions,
        action_context: ActionContext,
    ) -> Result<SessionInfo> {
        // Checked and reserved under the table's lock, so the check and the
        // claim are one step for every concurrent caller.
        let reservation = {
            let sessions = self.lock_sessions()?;
            if sessions.len() + self.opening.load(Ordering::SeqCst) >= MAX_SESSIONS {
                return Err(Error::LimitExceeded {
                    message: format!("at most {MAX_SESSIONS} browser sessions may be open"),
                });
            }
            self.opening.fetch_add(1, Ordering::SeqCst);
            Reservation(&self.opening)
        };
        let id = SessionId::new(format!("s-{}", self.next()));
        let mut action_context = action_context;
        action_context.session_id = Some(id.to_string());
        let mut session = Session {
            engine: self.launcher.open(id.as_str()),
            info: SessionInfo {
                id: id.clone(),
                endpoint: options.endpoint.clone().unwrap_or_default(),
                launched: options.endpoint.is_none(),
                headless: options.headless,
                viewport: options.viewport,
                url: String::new(),
                title: String::new(),
            },
            origins: Origins::new(&options.allowed_origins),
            options,
            sequence: 0,
            deep: crate::deep::Minted::default(),
            downloads: Vec::new(),
            dir: self.scratch.join(id.as_str()),
            action_context,
            refs: Vec::new(),
            closing: false,
            closed: false,
        };
        // A browser this module launches says what to set when it cannot
        // start; one attached to keeps the engine's own words.
        let (launched, named) = (session.info.launched, session.options.executable.is_some());
        session
            .run(convert::launch(&session.options))
            .await
            .map_err(|error| {
                if launched {
                    error.launching(named)
                } else {
                    error
                }
            })?;
        session.run(convert::viewport(&session.options)).await?;
        let page = session.page().await?;
        let page = if session.origins.admits(&page.url) {
            page
        } else {
            if session.info.launched {
                // A profile that restores its last pages can open on a page
                // the list refuses: it is left before anything reads it.
                session.leave().await;
            } else {
                // A browser attached to is someone's own: their tab stays as
                // it is, and the session works in a blank tab of its own.
                session.run(json!({"action": "tab_new"})).await?;
            }
            let left = session.page().await?;
            if !session.origins.admits(&left.url) {
                // A session is never handed out on a page the list refuses.
                let _closed = session.run(json!({"action": "close"})).await;
                return Err(Error::BlockedByPolicy { url: left.url });
            }
            left
        };
        session.info.url = page.url;
        session.info.title = page.title;
        let info = session.info.clone();
        // The slot moves from the reservation to the table in one step under
        // the lock, so no concurrent check ever counts this launch twice.
        let mut sessions = self.lock_sessions()?;
        sessions.insert(id, Arc::new(tokio::sync::Mutex::new(session)));
        drop(reservation);
        drop(sessions);
        Ok(info)
    }

    /// Updates the host action-gate context for an existing browser session.
    /// A retained payment checkout can therefore adopt its verified payment
    /// mode without opening or claiming another browser session.
    ///
    /// # Errors
    ///
    /// Returns an error when the session has gone away, is closing, or its
    /// state could not be read safely.
    pub async fn update_action_context(
        &self,
        id: &SessionId,
        action_context: ActionContext,
    ) -> Result<()> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check(false).await?;
        let mut action_context = action_context;
        action_context.session_id = Some(id.to_string());
        session.action_context = action_context;
        Ok(())
    }

    /// Closes a session and everything it owns. Closing one that is already
    /// gone succeeds.
    ///
    /// # Errors
    ///
    /// When the session table cannot be read or the browser does not confirm
    /// that it closed. A failed close stays in the table so a later caller
    /// can retry and no profile owner can mistake the attempt for completion.
    pub async fn close_session(&self, id: &SessionId) -> Result<()> {
        let session = self.lock_sessions()?.get(id).cloned();
        let Some(session) = session else {
            return Ok(());
        };
        let mut session = session.lock().await;
        if session.closed {
            return Ok(());
        }
        // A browser that is already gone has nothing left to close. A failure
        // leaves `closing` set so normal operations stay refused while a later
        // explicit close may retry.
        session.close().await?;
        let _removed = std::fs::remove_dir_all(&session.dir);
        session.closed = true;
        drop(session);
        self.lock_sessions()?.remove(id);
        Ok(())
    }

    /// The sessions currently open, with the page each was last seen on.
    ///
    /// # Errors
    ///
    /// Only when the session table itself is unusable.
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let sessions = self.lock_sessions()?.values().cloned().collect::<Vec<_>>();
        let mut infos = Vec::with_capacity(sessions.len());
        for session in sessions {
            infos.push(session.lock().await.info.clone());
        }
        infos.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(infos)
    }

    /// Checks the page session `id` shows against its allowed origins,
    /// leaving a refused one. Free when the session has no list. Every
    /// observation a task makes passes here, so a page a task was taken to
    /// by any means is caught before it is read or acted on.
    ///
    /// # Errors
    ///
    /// [`Error::BlockedByPolicy`] for a refused page, [`Error::NoSuchSession`],
    /// and whatever reading the page's address reports.
    pub(crate) async fn check_page(&self, id: &SessionId) -> Result<()> {
        let session = self.session(id)?;
        let mut session = session.lock().await;
        session.check(false).await
    }

    fn session(&self, id: &SessionId) -> Result<Arc<tokio::sync::Mutex<Session>>> {
        self.lock_sessions()?
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NoSuchSession { id: id.to_string() })
    }

    fn lock_sessions(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<SessionId, Arc<tokio::sync::Mutex<Session>>>>>
    {
        self.sessions
            .lock()
            .map_err(|_| Error::failed("the session table was poisoned by a panic"))
    }

    fn lock_outputs(&self) -> Result<std::sync::MutexGuard<'_, OutputStore>> {
        self.outputs
            .lock()
            .map_err(|_| Error::failed("the output store was poisoned by a panic"))
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[cfg(test)]
mod sessions_tests;
