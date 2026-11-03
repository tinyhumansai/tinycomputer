//! The Jev runtime: its configured client, the journal it writes, and the
//! one door every Jev evaluation goes through.

#[cfg(feature = "desktop")]
use std::{collections::HashMap, sync::Mutex};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use tinycomputer_bus::{DesktopError, JevConfig, JevConfiguration, JevProvider};
use tinyinference_decisions::{
    Client, ClientConfig, Error as JevError, EvaluationFailure, EvaluationRequest,
    EvaluationResult, Noul, Question, RetryPolicy,
};

use super::flow::MAX_VOTES;
use super::journal::Journal;
#[cfg(feature = "desktop")]
use super::pending::PendingRun;
use super::sage;
use crate::{ModelRole, UsageRecorder};

/// How a Jev call retries a provider's server error or rate limit when the
/// configuration does not say: four more attempts, waiting 1, 2, 4, then 8
/// seconds, or what the provider asks for. Live, the client's own 100 ms and
/// 200 ms waits gave a gateway's 502s 2.5 s before they ended the run.
pub(super) const RETRY: RetryPolicy = RetryPolicy {
    max_retries: 4,
    initial_backoff: Duration::from_secs(1),
    max_backoff: Duration::from_secs(8),
};

/// How long one Jev attempt may take when the configuration does not say.
/// Live, the slowest answer took 8.6 s, and a request nothing came back for
/// waited the client's own 30 s before its retry answered in under 1 s.
pub(super) const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest [`JevRuntime::warm`] waits for its answers before it gives
/// its calls up.
pub(super) const WARM_TIMEOUT: Duration = Duration::from_secs(10);

/// Requests a step's first turn asks at once, each in every framing: its
/// judging, and grounding's opening for the move it almost always makes
/// (`flow::act::judge`).
pub(super) const FIRST_TURN: u32 = 2;

/// Configured Jev transport and non-secret policy metadata.
#[derive(Clone)]
pub struct JevRuntime {
    pub(super) client: Arc<dyn Evaluator>,
    pub(super) configuration: JevConfiguration,
    #[cfg(feature = "desktop")]
    pub(super) pending: Arc<Mutex<HashMap<String, PendingRun>>>,
    pub(super) journal: Journal,
    /// Hedged copies of framings in flight, shared by every flow run on
    /// this runtime.
    pub(super) copies: Arc<AtomicUsize>,
    /// Optional task-scoped collector, set by an embedding host's runner.
    pub(super) usage: Option<Arc<UsageRecorder>>,
}

impl std::fmt::Debug for JevRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("JevRuntime");
        debug
            .field("client", &"[configured]")
            .field("configuration", &self.configuration);
        #[cfg(feature = "desktop")]
        debug.field("pending", &"[redacted]");
        debug
            .field("journal", &self.journal)
            .field("copies", &self.copies)
            .field("usage", &self.usage.as_ref().map(|_| "[tracked]"))
            .finish()
    }
}

impl JevRuntime {
    /// Build a runtime from the module's private `jev` configuration.
    ///
    /// `request.provider` selects the decision model: Jev through one of its
    /// three routes, `OpenJEV`, or Levanto Sage (see [`JevProvider`]).
    ///
    /// # Errors
    ///
    /// Returns a `JEV_INVALID_CONFIG` [`DesktopError`] when the provider,
    /// endpoint, or credentials in `request` cannot form a trusted client.
    pub fn configure(request: &JevConfig) -> Result<Self, Box<DesktopError>> {
        Self::configure_inner(request, false)
    }

    /// Build a Jev runtime for an embedding host, allowing the host to route
    /// its configured provider to a private endpoint.
    ///
    /// The `TinyBus` module uses [`JevRuntime::configure`], which retains the
    /// module's provider allow-list. This constructor is for Rust hosts that
    /// own and trust their configuration.
    ///
    /// # Errors
    ///
    /// Returns `JEV_INVALID_CONFIG` when the provider or credentials cannot
    /// form a client.
    pub fn configure_for_host(request: &JevConfig) -> Result<Self, Box<DesktopError>> {
        Self::configure_inner(request, true)
    }

    fn configure_inner(
        request: &JevConfig,
        allow_custom_endpoint: bool,
    ) -> Result<Self, Box<DesktopError>> {
        if !allow_custom_endpoint
            && let Some(endpoint) = &request.endpoint_url
            && !trusted_endpoint(request.provider, endpoint)
        {
            return Err(Box::new(DesktopError::new(
                "JEV_INVALID_CONFIG",
                "endpoint is not an approved Jev provider route",
            )));
        }
        if request.provider == JevProvider::Sage {
            return Self::sage_at(
                request.api_key(),
                request.fast.unwrap_or(false),
                request.endpoint_url.as_deref(),
            );
        }
        let client = Client::new(client_config(request)).map_err(|error| config_error(&error))?;
        Ok(Self {
            client: Arc::new(client),
            configuration: JevConfiguration {
                provider: request.provider,
                model: request
                    .model
                    .clone()
                    .filter(|model| !model.trim().is_empty())
                    .unwrap_or_else(|| request.provider.default_model().to_owned()),
                endpoint_url: request.endpoint_url.clone(),
                fast: false,
            },
            #[cfg(feature = "desktop")]
            pending: Arc::new(Mutex::new(HashMap::new())),
            journal: Journal::from_env(),
            copies: Arc::default(),
            usage: None,
        })
    }

    /// A runtime whose decisions Levanto Sage makes in place of Jev, with
    /// `api_key`; `fast` scores each choice in one pass rather than one per
    /// option.
    ///
    /// The same runtime [`JevRuntime::configure`] builds for the `sage`
    /// provider (`agentic/sage/`); [`JevConfiguration`] names it by its
    /// model, `levanto-sage`.
    ///
    /// # Errors
    ///
    /// Returns a `JEV_INVALID_CONFIG` [`DesktopError`] when `api_key` is
    /// empty.
    pub fn sage(api_key: &str, fast: bool) -> Result<Self, Box<DesktopError>> {
        Self::sage_at(api_key, fast, None)
    }

    /// The Sage runtime, at `endpoint` when one was approved.
    fn sage_at(
        api_key: &str,
        fast: bool,
        endpoint: Option<&str>,
    ) -> Result<Self, Box<DesktopError>> {
        let client = match endpoint {
            Some(endpoint) => {
                tinyinference_decisions::sage::SageClient::with_base_url(api_key, endpoint)
            }
            None => tinyinference_decisions::sage::SageClient::new(api_key),
        }
        .map_err(|error| config_error(&error))?;
        Ok(Self {
            client: Arc::new(sage::SageEvaluator::new(client, fast)),
            configuration: JevConfiguration {
                provider: JevProvider::Sage,
                model: JevProvider::Sage.default_model().to_owned(),
                endpoint_url: endpoint.map(str::to_owned),
                fast,
            },
            #[cfg(feature = "desktop")]
            pending: Arc::new(Mutex::new(HashMap::new())),
            journal: Journal::from_env(),
            copies: Arc::default(),
            usage: None,
        })
    }

    /// The non-secret summary of this runtime's decision model: provider,
    /// model, endpoint override, and Sage's `fast` flag.
    #[must_use]
    pub fn configuration(&self) -> &JevConfiguration {
        &self.configuration
    }

    /// This runtime recording Jev usage into a task-scoped collector.
    #[must_use]
    pub fn with_usage_recorder(mut self, usage: Arc<UsageRecorder>) -> Self {
        self.usage = Some(usage);
        self
    }

    /// This runtime with the debug journal written under `dir`, whatever
    /// [`JOURNAL_ENV`](crate::JOURNAL_ENV) says.
    ///
    /// Every Jev exchange of every run, with its latency, and the time each
    /// flow spends observing, acting, and on each step, is appended to
    /// `<dir>/<run id>/journal.jsonl`. See `docs/technical/jev-journal.md`.
    #[must_use]
    pub fn with_journal(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.journal = Journal::at(dir);
        self
    }

    /// This runtime writing its journal to the run named `run_id`, so the
    /// several runs of one task — a flow and each continuation — share one
    /// journal file. Does nothing when the journal is off.
    #[must_use]
    pub fn journaled_as(&self, run_id: &str) -> Self {
        Self {
            journal: self.journal.named(run_id),
            ..self.clone()
        }
    }

    /// Writes one event of `kind` with `fields` to the debug journal: into
    /// this runtime's open run (see [`JevRuntime::journaled_as`]), or, with
    /// none open, into a new run named for the time and `kind`. It is for
    /// time a task spends outside its flows, such as planning, a rescue, or
    /// waiting on a person, so the task's journal accounts for all of its
    /// time. `fields` is only built when recording: nothing is, and no run
    /// is opened, when the journal is off.
    pub fn journal_event(&self, kind: &str, fields: impl FnOnce() -> serde_json::Value) {
        if !self.journaling() {
            return;
        }
        let journal = if self.journal.is_open() {
            self.journal.clone()
        } else {
            self.journal.fresh(kind)
        };
        journal.record(kind, fields);
    }

    /// Whether this runtime's debug journal is on.
    #[must_use]
    pub fn journaling(&self) -> bool {
        self.journal.is_on()
    }

    /// The directory this runtime's current run journal is written to, if
    /// the journal is on and a run has begun.
    #[must_use]
    pub fn journal_dir(&self) -> Option<std::path::PathBuf> {
        self.journal.run_dir()
    }

    /// This runtime writing to `journal`.
    #[cfg(feature = "desktop")]
    pub(super) fn within(self, journal: Journal) -> Self {
        Self { journal, ..self }
    }

    /// This runtime with a run of `kind` begun in its journal.
    pub(super) fn begin_run(&self, kind: &str, label: &str) -> Self {
        Self {
            journal: self.journal.begin(kind, label, &self.configuration.model),
            ..self.clone()
        }
    }

    /// Opens connections to Jev for the decisions to come, while a task's
    /// plan is drafted: one for each call of a step's first turn, which asks
    /// its judging and grounding's opening together (`FIRST_TURN`), each in
    /// `votes` framings (at most 9, `MAX_VOTES`). Each connection is opened
    /// with a one-question evaluation, all at once, journaled as `warm-up`;
    /// its answer is dropped, and those still out after 10 s
    /// (`WARM_TIMEOUT`) are given up on. A decision asks its framings all
    /// at once, each on a connection of its own; live, a task's first
    /// decision took 330 ms more a call than its later ones, opening them.
    /// Sage, whose calls take seconds, is not warmed.
    pub async fn warm(&self, votes: u32) {
        if self.configuration.provider == JevProvider::Sage {
            return;
        }
        let request = Arc::new(warm_up(&self.configuration.model));
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..votes.clamp(1, MAX_VOTES).saturating_mul(FIRST_TURN) {
            let (runtime, request) = (self.clone(), Arc::clone(&request));
            calls.spawn(async move {
                let _answer = runtime.evaluate(Some("warm-up"), &request).await;
            });
        }
        // Calls still out when the time is up are dropped with the set.
        let _warmed = tokio::time::timeout(WARM_TIMEOUT, calls.join_all()).await;
    }

    /// Asks Jev one request, journaling the exchange against `step`.
    pub(super) async fn evaluate(
        &self,
        step: Option<&str>,
        request: &EvaluationRequest,
    ) -> std::result::Result<EvaluationResult, EvaluationFailure> {
        if let Some(usage) = &self.usage {
            usage.record_started(ModelRole::Jev);
        }
        let outcome = self.client.evaluate(request).await;
        match (&self.usage, &outcome) {
            (Some(usage), Ok(result)) => {
                usage.record_additional_calls(
                    ModelRole::Jev,
                    u64::from(result.attempts.saturating_sub(1)),
                );
                usage.record_measurements(
                    ModelRole::Jev,
                    result.response.usage.input_tokens,
                    result.response.usage.cached_input_tokens,
                    result.response.usage.output_tokens,
                    result.response.usage.cost_usd_nanos,
                );
            }
            (Some(usage), Err(failure)) => usage.record_additional_calls(
                ModelRole::Jev,
                u64::from(failure.attempts.saturating_sub(1)),
            ),
            _ => {}
        }
        self.journal.exchange(step, request, outcome.as_ref());
        outcome
    }
}

/// The smallest request `model` answers: one yes/no question about nothing
/// on any screen.
fn warm_up(model: &str) -> EvaluationRequest {
    EvaluationRequest {
        state: serde_json::json!("A connection check."),
        model: model.to_owned(),
        questions: [(
            "ready".to_owned(),
            Question::Noul(Noul {
                instructions: serde_json::json!("The state is a connection check."),
                criteria: None,
            }),
        )]
        .into(),
    }
}

pub(super) trait Evaluator: Send + Sync {
    fn evaluate<'a>(
        &'a self,
        request: &'a EvaluationRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<EvaluationResult, EvaluationFailure>>
                + Send
                + 'a,
        >,
    >;
}

impl Evaluator for Client {
    fn evaluate<'a>(
        &'a self,
        request: &'a EvaluationRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<EvaluationResult, EvaluationFailure>>
                + Send
                + 'a,
        >,
    > {
        // The decisions client reports failures as its crate error; the loops
        // want the failure record with its attempts and latency.
        Box::pin(async move {
            Client::evaluate(self, request)
                .await
                .map_err(|error| match error {
                    JevError::EvaluationFailure(failure) => failure,
                    other => EvaluationFailure {
                        error: Box::new(other),
                        attempts: 0,
                        latency: Duration::ZERO,
                    },
                })
        })
    }
}

/// The one endpoint each decision provider may be configured at: its own
/// published route. `OpenJEV` and Sage have no `TinyHumans` proxy route in
/// `tinyinference-decisions`, so none is approved for them.
#[must_use]
pub(super) const fn approved_endpoint(provider: JevProvider) -> &'static str {
    match provider {
        JevProvider::TypeSafe => "https://api.typesafe.ai/v1/systemone",
        JevProvider::OpenRouter => "https://openrouter.ai/api/alpha/decisions",
        JevProvider::TinyHumansOpenRouter => {
            "https://api.tinyhumans.ai/agent-integrations/openrouter/systemone"
        }
        JevProvider::OpenJev => "https://api.openjev.sh/v1/systemone",
        JevProvider::Sage => "https://sage.levanto.ai/",
    }
}

pub(super) fn trusted_endpoint(provider: JevProvider, endpoint: &str) -> bool {
    let approved = approved_endpoint(provider);
    // Sage's endpoint is its API root, so its trailing slash is optional.
    if endpoint == approved
        || (provider == JevProvider::Sage && endpoint == approved.trim_end_matches('/'))
    {
        return true;
    }
    #[cfg(test)]
    return endpoint.starts_with("http://127.0.0.1:");
    #[cfg(not(test))]
    false
}

/// The HTTP client configuration for a Jev `request`: its provider's
/// route, endpoint, timeout ([`ATTEMPT_TIMEOUT`] unless the request sets
/// one), and attribution, retrying as [`RETRY`] unless the request sets its
/// own number of retries.
pub(super) fn client_config(request: &JevConfig) -> ClientConfig {
    let mut config = match request.provider {
        JevProvider::TypeSafe => ClientConfig::new(request.api_key()),
        JevProvider::OpenRouter => ClientConfig::openrouter(request.api_key()),
        JevProvider::TinyHumansOpenRouter => ClientConfig::tinyhumans_openrouter(request.api_key()),
        // Sage has a client of its own; `configure` never asks for it here.
        JevProvider::OpenJev | JevProvider::Sage => ClientConfig::openjev(request.api_key()),
    };
    if let Some(endpoint) = &request.endpoint_url {
        config = config.with_endpoint_url(endpoint);
    }
    config.timeout = request
        .timeout_ms
        .map_or(ATTEMPT_TIMEOUT, Duration::from_millis);
    config.retry = RETRY;
    if let Some(max_retries) = request.max_retries {
        config.retry.max_retries = max_retries;
    }
    if request.provider == JevProvider::TinyHumansOpenRouter
        && let Some(sdk_name) = request.sdk_name.as_deref()
    {
        config = config.with_sdk_name(sdk_name);
    }
    config
}

pub(super) fn config_error(error: &JevError) -> Box<DesktopError> {
    Box::new(DesktopError::new("JEV_INVALID_CONFIG", error.to_string()))
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
