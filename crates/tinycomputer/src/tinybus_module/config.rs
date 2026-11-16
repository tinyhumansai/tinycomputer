//! Reading the module configuration's `browser` and `cursor` keys.
//!
//! `browser` sets how the module launches every browser it opens, a task's or
//! a `BrowserOpenSession` caller's. Booking sites turn away a browser that
//! announces itself as headless, so a host running tasks in a container sets
//! a desktop `user_agent` and launch `args` once here rather than on every
//! request.

use tinycomputer_browser::{
    CursorPace, Perception, ScreenCursor, SessionOptions, Settle, UpstreamProxy,
};
use tinycomputer_bus::config::{BrowserConfig, BrowserPerception, BrowserSettle};

use crate::Result;

/// The browser settings the module applies to every session it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserDefaults {
    /// The Chrome or Chromium binary to launch, where the platform's own
    /// discovery would not find one.
    pub(crate) executable: Option<String>,
    /// The `User-Agent` every launched browser sends.
    pub(crate) user_agent: Option<String>,
    /// Extra command-line arguments for every launched browser.
    pub(crate) args: Vec<String>,
    /// How a task reads a page: by sight (the default) or the tree alone.
    pub(crate) perception: Perception,
    /// How a task lets a page settle after an action: `prompt` (the
    /// default) or `steady`.
    pub(crate) settle: Settle,
    /// Whether a browser-only task's browser opens while its plan is
    /// drafted (the default), at the one web address the task's text names
    /// if it names one, rather than at its first step.
    pub(crate) prelaunch: bool,
    /// Immutable `TinySecurity` scope captured from host-private configuration.
    pub(crate) egress: Option<BrowserEgressConfig>,
}

/// Host-selected policy identity and generation; never an agent argument.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BrowserEgressConfig {
    pub(crate) policy_id: String,
    pub(crate) generation: u64,
    /// Destination domains the browser reaches through an upstream HTTP
    /// proxy, still under the policy above. Host-private: the URLs carry
    /// credentials.
    pub(crate) upstream_proxies: Vec<UpstreamProxy>,
}

/// The most upstream proxies one egress scope may name.
const MAX_UPSTREAM_PROXIES: usize = 16;

impl std::fmt::Debug for BrowserEgressConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserEgressConfig([REDACTED])")
    }
}

impl Default for BrowserDefaults {
    fn default() -> Self {
        Self {
            executable: None,
            user_agent: None,
            args: Vec::new(),
            perception: Perception::default(),
            settle: Settle::default(),
            prelaunch: true,
            egress: None,
        }
    }
}

impl BrowserDefaults {
    /// Applies the browser defaults decoded from the public module config.
    pub(crate) fn from_config(
        config: Option<&BrowserConfig>,
        egress: Option<BrowserEgressConfig>,
    ) -> Self {
        let Some(config) = config else {
            return Self {
                egress,
                ..Self::default()
            };
        };
        Self {
            executable: config.executable.clone(),
            user_agent: config.user_agent.clone(),
            args: config.args.clone(),
            perception: match config.perception {
                BrowserPerception::Sight => Perception::Sight,
                BrowserPerception::Tree => Perception::Tree,
            },
            settle: match config.settle {
                BrowserSettle::Prompt => Settle::Prompt,
                BrowserSettle::Steady => Settle::Steady,
            },
            prelaunch: config.prelaunch,
            egress,
        }
    }

    pub(crate) fn egress_from_raw(
        config: &serde_json::Value,
    ) -> Result<Option<BrowserEgressConfig>> {
        let Some(browser) = config.as_object().and_then(|object| object.get("browser")) else {
            return Ok(None);
        };
        let Some(egress) = browser.as_object().and_then(|object| object.get("egress")) else {
            return Ok(None);
        };
        let invalid = || crate::Error::ConfigFieldType {
            field: "browser",
            expected: "an object with a valid private egress policy",
        };
        let object = egress.as_object().ok_or_else(invalid)?;
        let policy_id = object
            .get("policy_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 64)
            .ok_or_else(invalid)?;
        let generation = object
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .filter(|generation| *generation > 0)
            .ok_or_else(invalid)?;
        let upstream_proxies = match object.get("upstream_proxies") {
            None => Vec::new(),
            Some(value) => upstream_proxies(value)?,
        };
        let expected_keys = if object.contains_key("upstream_proxies") {
            3
        } else {
            2
        };
        // The host's own `executable` stays allowed: the enforced launcher
        // runs it with the proxy and resolver flags. Extra `args` could undo
        // them, so they are refused.
        if object.len() != expected_keys
            || browser
                .as_object()
                .is_some_and(|browser| browser.contains_key("args"))
        {
            return Err(invalid());
        }
        Ok(Some(BrowserEgressConfig {
            policy_id: policy_id.to_owned(),
            generation,
            upstream_proxies,
        }))
    }

    /// `options` with these defaults filled in where the caller left them
    /// unset. An attached session launches nothing, so it takes no
    /// executable and no launch arguments.
    pub(crate) fn apply(&self, mut options: SessionOptions) -> SessionOptions {
        if options.endpoint.is_none() {
            if options.executable.is_none() {
                options.executable.clone_from(&self.executable);
            }
            if options.args.is_empty() {
                options.args.clone_from(&self.args);
            }
        }
        if options.user_agent.is_none() {
            options.user_agent.clone_from(&self.user_agent);
        }
        options
    }
}

/// Reads `browser.egress.upstream_proxies`: a list of
/// `{"url": "http://user:pass@host:port", "domains": ["example.com"]}`. A
/// refusal names the field, never the URL, which carries credentials.
fn upstream_proxies(value: &serde_json::Value) -> Result<Vec<UpstreamProxy>> {
    let invalid = || crate::Error::ConfigFieldType {
        field: "browser.egress.upstream_proxies",
        expected: "a list of objects with an http:// proxy `url` and its `domains`",
    };
    let entries = value.as_array().ok_or_else(invalid)?;
    if entries.len() > MAX_UPSTREAM_PROXIES {
        return Err(invalid());
    }
    entries
        .iter()
        .map(|entry| {
            let entry = entry
                .as_object()
                .filter(|entry| entry.len() == 2)
                .ok_or_else(invalid)?;
            let url = entry
                .get("url")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid)?;
            let domains = entry
                .get("domains")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(invalid)?
                .iter()
                .map(|domain| domain.as_str().map(str::to_owned).ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?;
            UpstreamProxy::new(url, &domains).map_err(|error| crate::Error::ConfigFieldType {
                field: "browser.egress.upstream_proxies",
                expected: error.reason(),
            })
        })
        .collect()
}

/// Whether the host explicitly selected the trace/report callback.
pub(crate) fn trace_sink_destination(
    config: &serde_json::Value,
) -> Result<Option<tinybus::BusName>> {
    match config
        .as_object()
        .and_then(|object| object.get("trace_sink"))
    {
        None | Some(serde_json::Value::Bool(false)) => Ok(None),
        Some(serde_json::Value::Bool(true)) => Err(crate::Error::ConfigFieldType {
            field: "trace_sink",
            expected: "an object with the trusted host callback `destination`",
        }),
        Some(serde_json::Value::Object(options)) => {
            if options.len() != 1 {
                return Err(crate::Error::ConfigFieldType {
                    field: "trace_sink",
                    expected: "an object containing only the trusted host callback `destination`",
                });
            }
            let destination = options
                .get("destination")
                .and_then(serde_json::Value::as_str)
                .ok_or(crate::Error::ConfigFieldType {
                    field: "trace_sink.destination",
                    expected: "the unique name of the host callback prepared for this module",
                })?;
            tinybus::BusName::new(destination.to_owned())
                .map_err(|_| crate::Error::ConfigFieldType {
                    field: "trace_sink.destination",
                    expected: "the unique name of the host callback prepared for this module",
                })
                .and_then(|name| {
                    name.is_unique()
                        .then_some(Some(name))
                        .ok_or(crate::Error::ConfigFieldType {
                            field: "trace_sink.destination",
                            expected: "the unique name of the host callback prepared for this module",
                        })
                })
        }
        Some(_) => Err(crate::Error::ConfigFieldType {
            field: "trace_sink",
            expected: "false or an object with the trusted host callback `destination`",
        }),
    }
}

/// Whether the host prepared a confidential consequence-review callback.
pub(crate) fn consequence_host_destination(
    config: &serde_json::Value,
) -> Result<Option<tinybus::BusName>> {
    match config
        .as_object()
        .and_then(|object| object.get("consequence_host"))
    {
        None | Some(serde_json::Value::Bool(false)) => Ok(None),
        Some(serde_json::Value::Object(options)) if options.len() == 1 => {
            let destination = options
                .get("destination")
                .and_then(serde_json::Value::as_str)
                .ok_or(crate::Error::ConfigFieldType {
                    field: "consequence_host.destination",
                    expected: "the unique name of the host callback prepared for this module",
                })?;
            tinybus::BusName::new(destination.to_owned())
                .map_err(|_| crate::Error::ConfigFieldType {
                    field: "consequence_host.destination",
                    expected: "the unique name of the host callback prepared for this module",
                })
                .and_then(|name| {
                    name.is_unique()
                        .then_some(Some(name))
                        .ok_or(crate::Error::ConfigFieldType {
                            field: "consequence_host.destination",
                            expected: "the unique name of the host callback prepared for this module",
                        })
                })
        }
        Some(_) => Err(crate::Error::ConfigFieldType {
            field: "consequence_host",
            expected: "false or an object containing only the host callback `destination`",
        }),
    }
}

/// The `cursor` configuration: the agent's one on-screen cursor, shared by
/// the desktop and every task's browser. Either a pace name, or an object
/// with an optional `pace` and an optional `overlay` path to the
/// `tinycomputer-cursor-overlay` helper. Absent, the cursor glides at the
/// natural pace with the helper found where [`ProcessOverlay::locate`]
/// looks.
///
/// [`ProcessOverlay::locate`]: tinycomputer_browser::ProcessOverlay::locate
pub(super) fn cursor_config(config: &serde_json::Value) -> Result<ScreenCursor> {
    let invalid = || crate::Error::ConfigFieldType {
        field: "cursor",
        expected: "off, brisk, natural, or calm, or an object with an optional `pace` of those \
                   and an optional `overlay` path",
    };
    let pace = |value: Option<&serde_json::Value>| match value {
        None => Ok(CursorPace::default()),
        Some(serde_json::Value::String(name)) => name.parse().map_err(|_| invalid()),
        Some(_) => Err(invalid()),
    };
    let (pace, overlay) = match config.as_object().and_then(|object| object.get("cursor")) {
        None => (CursorPace::default(), None),
        Some(name @ serde_json::Value::String(_)) => (pace(Some(name))?, None),
        Some(serde_json::Value::Object(cursor)) => {
            let overlay = match cursor.get("overlay") {
                None => None,
                Some(serde_json::Value::String(path)) => Some(std::path::PathBuf::from(path)),
                Some(_) => return Err(invalid()),
            };
            (pace(cursor.get("pace"))?, overlay)
        }
        Some(_) => return Err(invalid()),
    };
    Ok(if pace.is_off() {
        ScreenCursor::off()
    } else {
        ScreenCursor::new(pace, overlay)
    })
}
