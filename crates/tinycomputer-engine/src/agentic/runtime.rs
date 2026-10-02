//! The Jev runtime: its configured client, the journal it writes, and the
//! one door every Jev evaluation goes through.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use tinycomputer_bus::{DesktopError, JevConfig, JevConfiguration, JevProvider};
use tinyinference_decisions::{
    Client, ClientConfig, Error as JevError, EvaluationFailure, EvaluationRequest, EvaluationResult,
};

use super::journal::Journal;
use super::pending::PendingRun;
use super::sage;

/// Configured Jev transport and non-secret policy metadata.
#[derive(Clone)]
pub struct JevRuntime {
    pub(super) client: Arc<dyn Evaluator>,
    pub(super) configuration: JevConfiguration,
    pub(super) pending: Arc<Mutex<HashMap<String, PendingRun>>>,
    pub(super) journal: Journal,
}

impl std::fmt::Debug for JevRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JevRuntime")
            .field("client", &"[configured]")
            .field("configuration", &self.configuration)
            .field("pending", &"[redacted]")
            .field("journal", &self.journal)
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
        if request.provider == JevProvider::SelfHosted {
            // A self-hosted model has no conventional route and no default
            // model: the operator names both, and the key is sent only to
            // the declared endpoint. `Client::new` below is the syntactic
            // gate: an absolute HTTP(S) URL without embedded credentials,
            // query, or fragment, plain HTTP only on a literal loopback.
            if request
                .endpoint_url
                .as_deref()
                .is_none_or(|endpoint| endpoint.trim().is_empty())
            {
                return Err(Box::new(DesktopError::new(
                    "JEV_INVALID_CONFIG",
                    "self_hosted decision model requires an endpoint_url",
                )));
            }
            if request
                .model
                .as_deref()
                .is_none_or(|model| model.trim().is_empty())
            {
                return Err(Box::new(DesktopError::new(
                    "JEV_INVALID_CONFIG",
                    "self_hosted decision model requires a model",
                )));
            }
        } else if let Some(endpoint) = &request.endpoint_url
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
        let mut config = match request.provider {
            // A self-hosted model is configured exactly like first-party
            // TypeSafe Jev, at the operator's declared endpoint: the client
            // validates the URL and applies the first-party response check,
            // which a self-hosted model satisfies by echoing the requested
            // model id.
            JevProvider::TypeSafe | JevProvider::SelfHosted => ClientConfig::new(request.api_key()),
            JevProvider::OpenRouter => ClientConfig::openrouter(request.api_key()),
            JevProvider::TinyHumansOpenRouter => {
                ClientConfig::tinyhumans_openrouter(request.api_key())
            }
            // Sage returned above; it has a client of its own.
            JevProvider::OpenJev | JevProvider::Sage => ClientConfig::openjev(request.api_key()),
        };
        if let Some(endpoint) = &request.endpoint_url {
            config = config.with_endpoint_url(endpoint);
        }
        if let Some(timeout_ms) = request.timeout_ms {
            config.timeout = Duration::from_millis(timeout_ms);
        }
        if let Some(max_retries) = request.max_retries {
            config.retry.max_retries = max_retries;
        }
        if request.provider == JevProvider::TinyHumansOpenRouter
            && let Some(sdk_name) = request.sdk_name.as_deref()
        {
            config = config.with_sdk_name(sdk_name);
        }
        let client = Client::new(config).map_err(|error| config_error(&error))?;
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
            pending: Arc::new(Mutex::new(HashMap::new())),
            journal: Journal::from_env(),
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
            pending: Arc::new(Mutex::new(HashMap::new())),
            journal: Journal::from_env(),
        })
    }

    /// The non-secret summary of this runtime's decision model: provider,
    /// model, endpoint override, and Sage's `fast` flag.
    #[must_use]
    pub fn configuration(&self) -> &JevConfiguration {
        &self.configuration
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

    /// The directory this runtime's current run journal is written to, if
    /// the journal is on and a run has begun.
    #[must_use]
    pub fn journal_dir(&self) -> Option<std::path::PathBuf> {
        self.journal.run_dir()
    }

    /// This runtime writing to `journal`.
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

    /// Asks Jev one request, journaling the exchange against `step`.
    pub(super) async fn evaluate(
        &self,
        step: Option<&str>,
        request: &EvaluationRequest,
    ) -> std::result::Result<EvaluationResult, EvaluationFailure> {
        let outcome = self.client.evaluate(request).await;
        self.journal.exchange(step, request, outcome.as_ref());
        outcome
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

/// The one endpoint each hosted decision provider may be configured at: its
/// own published route. `OpenJEV` and Sage have no `TinyHumans` proxy route in
/// `tinyinference-decisions`, so none is approved for them. A self-hosted
/// model has no published route — the operator's declared endpoint is the
/// route — so it yields `None` and [`trusted_endpoint`] treats a declared
/// endpoint as trusted.
#[must_use]
pub(super) const fn approved_endpoint(provider: JevProvider) -> Option<&'static str> {
    match provider {
        JevProvider::TypeSafe => Some("https://api.typesafe.ai/v1/systemone"),
        JevProvider::OpenRouter => Some("https://openrouter.ai/api/alpha/decisions"),
        JevProvider::TinyHumansOpenRouter => {
            Some("https://api.tinyhumans.ai/agent-integrations/openrouter/systemone")
        }
        JevProvider::OpenJev => Some("https://api.openjev.sh/v1/systemone"),
        JevProvider::Sage => Some("https://sage.levanto.ai/"),
        JevProvider::SelfHosted => None,
    }
}

pub(super) fn trusted_endpoint(provider: JevProvider, endpoint: &str) -> bool {
    let Some(approved) = approved_endpoint(provider) else {
        // A self-hosted model is trusted at whatever non-empty endpoint the
        // operator declared: there is no published route to compare against,
        // and the API key travels only to that declared endpoint.
        // `Client::new` remains the syntactic gate on the URL's shape.
        return !endpoint.trim().is_empty();
    };
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

pub(super) fn config_error(error: &JevError) -> Box<DesktopError> {
    Box::new(DesktopError::new("JEV_INVALID_CONFIG", error.to_string()))
}
