//! The optional planner: a language model that turns a plain-language task
//! into a flow, and never acts.
//!
//! It is given the task, the flow guide, the *names* of the facts the caller
//! supplied, and which surfaces are available — never a fact's value and
//! never the screen. Its answer is validated with the same checker `RunFlow`
//! uses; an invalid flow goes back to the model with the errors, up to
//! [`REPAIRS`] times. Values the flow needs and no fact supplies come back as
//! questions for the caller.
//!
//! The model is behind [`LanguageModel`], so this logic is tested with
//! scripted answers; the `planner` feature adds the hosted adapter, on
//! `OpenRouter` or Tiny Humans' OpenAI-compatible gateway (`config.rs`,
//! `hosted.rs`).

#[cfg(feature = "planner")]
mod config;
#[cfg(feature = "planner")]
mod hosted;

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::{ModelRole, UsageRecorder};
use serde_json::Value;
use tinycomputer_bus::agent::{InputField, LanguageModelConfiguration, SurfaceKind, TaskPlan};
use tinycomputer_bus::{FLOW_GUIDE, Flow};
use tinycomputer_core::is_sensitive_name;

#[cfg(feature = "planner")]
pub use config::{
    HostModelConfig, HostModels, ModelRoute, OPEN_ROUTER_BASE_URL, OUTPUT_MODEL, PLANNER_MODEL,
    PlannerConfig, RESCUE_MODEL, TINYHUMANS_BASE_URL,
};
#[cfg(feature = "planner")]
pub use hosted::{open_router, open_router_rescuer, open_router_shaper, openai_compatible_models};

/// Validation repairs a plan gets.
pub const REPAIRS: usize = 2;

/// Who said a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The instructions.
    System,
    /// The caller's side.
    User,
    /// The model's side.
    Assistant,
}

/// What one plan or rescue used of its model: the calls it made, the first
/// and each repair of a refused answer, and the bytes the first call sent.
/// The task journals it, so a run shows what its planning and rescues cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelUse {
    /// Calls made: the first, and one per repair. A hosted call tried again
    /// after a passing failure (`hosted.rs`) counts once: its tries, and
    /// the waits between them, are inside it.
    pub calls: u32,
    /// Bytes of text in the first call's turns.
    pub sent_bytes: usize,
}

/// Provider-reported measurements for one completed language-model call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportedUsage {
    /// Input tokens, when the provider reported them.
    pub input_tokens: Option<u64>,
    /// Cached input tokens, when the provider reported them.
    pub cached_input_tokens: Option<u64>,
    /// Output tokens, when the provider reported them.
    pub output_tokens: Option<u64>,
    /// USD charge in nanodollars, when the provider reported it.
    pub cost_usd_nanos: Option<i64>,
}

/// A language-model reply and the measurements its provider supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelReply {
    /// The assistant's text.
    pub text: String,
    /// Usage reported for this request, absent when the provider omitted it.
    pub usage: ReportedUsage,
    /// Provider requests used to obtain this reply, including retries.
    pub attempts: u32,
}

/// A language-model failure with the number of provider requests attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFailure {
    /// Human-readable failure details.
    pub message: String,
    /// Provider requests made before the failure was returned.
    pub attempts: u32,
}

impl ModelUse {
    /// The use of a conversation about to be sent for the first time.
    pub(crate) fn starting(turns: &[Turn]) -> Self {
        Self {
            calls: 0,
            sent_bytes: turns.iter().map(|turn| turn.text.len()).sum(),
        }
    }
}

/// One message in a planning conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Who said it.
    pub role: Role,
    /// What was said.
    pub text: String,
}

impl Turn {
    pub(crate) fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
        }
    }
}

/// The future a [`LanguageModel`] returns: its reply text, or why it failed.
pub type Completion = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// Future returned by [`LanguageModel::complete_reported`].
pub type ReportedCompletion =
    Pin<Box<dyn Future<Output = Result<ModelReply, ModelFailure>> + Send>>;

/// A chat model that answers a conversation with text.
pub trait LanguageModel: Send + Sync + 'static {
    /// The model's reply to `turns`, which it is asked to give as one JSON
    /// object.
    fn complete(&self, turns: &[Turn]) -> Completion;

    /// A reply with provider-reported usage. Existing scripted models may
    /// implement only [`LanguageModel::complete`]; their measurements remain
    /// unknown.
    fn complete_reported(&self, turns: &[Turn]) -> ReportedCompletion {
        let completion = self.complete(turns);
        Box::pin(async move {
            completion.await.map_or_else(
                |message| {
                    Err(ModelFailure {
                        message,
                        attempts: 1,
                    })
                },
                |text| {
                    Ok(ModelReply {
                        text,
                        usage: ReportedUsage::default(),
                        attempts: 1,
                    })
                },
            )
        })
    }

    /// Completes a task-role call while recording attempts and provider data.
    fn complete_for_task(
        &self,
        turns: &[Turn],
        usage: Option<Arc<UsageRecorder>>,
        role: ModelRole,
    ) -> ReportedCompletion {
        let completion = self.complete_reported(turns);
        Box::pin(async move {
            if let Some(usage) = &usage {
                usage.record_started(role);
            }
            let result = completion.await;
            if let Some(usage) = &usage {
                match &result {
                    Ok(reply) => {
                        usage.record_additional_calls(
                            role,
                            u64::from(reply.attempts.saturating_sub(1)),
                        );
                        usage.record_measurements(
                            role,
                            reply.usage.input_tokens,
                            reply.usage.cached_input_tokens,
                            reply.usage.output_tokens,
                            reply.usage.cost_usd_nanos,
                        );
                    }
                    Err(failure) => usage.record_additional_calls(
                        role,
                        u64::from(failure.attempts.saturating_sub(1)),
                    ),
                }
            }
            result
        })
    }
}

const PROTOCOL: &str = "You plan tasks for a module that drives web pages and desktop \
applications for a person. You cannot see the screen and you do not know any site's or \
application's interface: you describe what should happen, in plain steps, following the \
guide below. Reply with exactly one JSON object and nothing else: a flow \
({\"app\": ..., \"steps\": [...]}). \
Use `browse` for anything on the web and `open` for a desktop application. \
Refer to the person's details only as ${name} variables: use the fact names you are given, \
and invent a clear name for any other detail the task needs, so the person can be asked for \
it. Never invent personal details. A shared fact may appear in any step's text, so write \
steps the way a person would (\"choose ${title} in the title field\"). A secret fact — a \
card number, a passport number, a password — may appear only as an `enter` step's value: \
it is never shown to the model that runs the steps, so it may not appear in an `open` \
application name, a `browse` address, a `do`, `verify`, `wait_for`, `stop_before`, `choose`, \
`read`, `extract`, `pick`, `repeat_until`, or `if` text, or as an `enter` slot's own name. \
Enter payment details only from secret facts you are given, and end any purchase or \
booking with a stop_before step for paying. Write a variable a step reads as ${name}, \
never bare, so its value is shown. A `verify` or `wait_for` must be checkable on the \
current screen alone: never compare with another site or an earlier page, since `pick` \
already ranks, and never name a `pick` variable in a `verify`, `wait_for`, `repeat_until`, \
or `if` condition: it holds the whole item's text, and a pick already fails when nothing \
fits. A `choose` option is the label the page shows for it (\"Saver\"), never \
a description (\"the cheapest fare\"): choosing by a criterion is `pick`. Guard \
sending, deleting, publishing, or submitting with a stop_before step.";

/// Turns tasks into flows with a [`LanguageModel`].
#[derive(Clone)]
pub struct Planner {
    model: Arc<dyn LanguageModel>,
    configuration: Option<LanguageModelConfiguration>,
    usage: Option<Arc<UsageRecorder>>,
}

impl std::fmt::Debug for Planner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Planner").finish_non_exhaustive()
    }
}

impl Planner {
    /// A planner asking `model`.
    #[must_use]
    pub fn new(model: Arc<dyn LanguageModel>) -> Self {
        Self {
            model,
            configuration: None,
            usage: None,
        }
    }

    /// This planner, reporting `configuration` as its route and model in
    /// `Describe`.
    #[must_use]
    pub fn with_configuration(mut self, configuration: LanguageModelConfiguration) -> Self {
        self.configuration = Some(configuration);
        self
    }

    /// This planner recording calls into a task-scoped collector.
    #[must_use]
    pub fn with_usage_recorder(mut self, usage: Arc<UsageRecorder>) -> Self {
        self.usage = Some(usage);
        self
    }

    /// The route and model this planner was configured with, when known.
    #[must_use]
    pub fn configuration(&self) -> Option<&LanguageModelConfiguration> {
        self.configuration.as_ref()
    }

    /// Drafts a flow for `task`.
    ///
    /// # Errors
    ///
    /// Why no valid flow came back: the model failed, or its answer stayed
    /// invalid after [`REPAIRS`] repairs.
    pub async fn plan(
        &self,
        task: &str,
        fact_names: &[String],
        secret_names: &[String],
        surfaces: &[SurfaceKind],
    ) -> Result<TaskPlan, String> {
        self.plan_measured(task, fact_names, secret_names, surfaces)
            .await
            .0
    }

    /// [`Planner::plan`], with what it used of its model, whether or not a
    /// plan came back.
    pub async fn plan_measured(
        &self,
        task: &str,
        fact_names: &[String],
        secret_names: &[String],
        surfaces: &[SurfaceKind],
    ) -> (Result<TaskPlan, String>, ModelUse) {
        self.plan_measured_with_budget(task, fact_names, secret_names, surfaces, None)
            .await
    }

    /// Drafts a plan while reserving each provider call against `budget`
    /// before the call starts.
    pub async fn plan_measured_with_budget(
        &self,
        task: &str,
        fact_names: &[String],
        secret_names: &[String],
        surfaces: &[SurfaceKind],
        reserve_call: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> (Result<TaskPlan, String>, ModelUse) {
        let surfaces = if surfaces.is_empty() {
            "the web browser and desktop applications".to_owned()
        } else {
            surfaces
                .iter()
                .map(|surface| match surface {
                    SurfaceKind::Browser => "the web browser",
                    SurfaceKind::Desktop => "desktop applications",
                })
                .collect::<Vec<_>>()
                .join(" and ")
        };
        let known = fact_names.iter().cloned().collect::<BTreeSet<_>>();
        let secrets = known
            .iter()
            .filter(|name| secret_names.contains(name) || is_sensitive_name(name))
            .cloned()
            .collect::<BTreeSet<_>>();
        let listed = |names: Vec<&String>| {
            if names.is_empty() {
                "none".to_owned()
            } else {
                names
                    .iter()
                    .map(|name| format!("${{{name}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let facts = format!(
            "{}.\nSecret facts, only ever an `enter` value: {}",
            listed(
                known
                    .iter()
                    .filter(|name| !secrets.contains(*name))
                    .collect()
            ),
            listed(secrets.iter().collect()),
        );
        let mut turns = vec![
            Turn::new(Role::System, format!("{PROTOCOL}\n\n{FLOW_GUIDE}")),
            Turn::new(
                Role::User,
                format!(
                    "Task: {task}\n\nAvailable: {surfaces}.\nShared facts you may use: {facts}."
                ),
            ),
        ];
        let mut used = ModelUse::starting(&turns);
        let mut last = String::new();
        for _ in 0..=REPAIRS {
            if reserve_call.is_some_and(|reserve| !reserve()) {
                return (
                    Err("the task model-call budget is exhausted".to_owned()),
                    used,
                );
            }
            used.calls += 1;
            let reply = match self
                .model
                .complete_for_task(&turns, self.usage.clone(), ModelRole::Planner)
                .await
            {
                Ok(reply) => reply.text,
                Err(error) => return (Err(error.message), used),
            };
            turns.push(Turn::new(Role::Assistant, reply.clone()));
            let problem = match parse(&reply) {
                Ok(flow) => {
                    let errors = crate::agentic::check_flow(&flow, &known, &secrets)
                        .errors
                        .into_iter()
                        .filter(|error| !error.contains("` is not defined in `vars`"))
                        .collect::<Vec<_>>();
                    if errors.is_empty() {
                        return (Ok(plan_for(flow, &known, &secrets)), used);
                    }
                    format!("That flow is invalid:\n- {}", errors.join("\n- "))
                }
                Err(error) => format!("That was not a flow ({error})."),
            };
            last.clone_from(&problem);
            turns.push(Turn::new(
                Role::User,
                format!("{problem}\nReply with the corrected flow only, as one JSON object."),
            ));
        }
        (
            Err(format!("the planner did not produce a valid flow: {last}")),
            used,
        )
    }
}

fn plan_for(flow: Flow, known: &BTreeSet<String>, secrets: &BTreeSet<String>) -> TaskPlan {
    let questions = crate::agentic::missing_inputs(&flow, known, secrets)
        .into_iter()
        .map(|name| InputField {
            why: format!("the plan uses ${{{name}}}"),
            kind: crate::task::input_kind(&name),
            name,
            options: Vec::new(),
        })
        .collect();
    let notes = if flow
        .steps
        .iter()
        .any(|step| matches!(step.action(), tinycomputer_bus::FlowAction::StopBefore(_)))
    {
        vec!["The plan stops before any irreversible or paid action.".to_owned()]
    } else {
        Vec::new()
    };
    TaskPlan {
        flow,
        questions,
        notes,
    }
}

/// The flow in a model's reply, tolerating code fences and prose around it.
fn parse(text: &str) -> Result<Flow, String> {
    serde_json::from_value(json_object(text)?).map_err(|error| error.to_string())
}

/// The one JSON object in a model's reply, tolerating code fences and prose
/// around it.
pub(crate) fn json_object(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    let start = trimmed.find('{').ok_or("no JSON object")?;
    let end = trimmed.rfind('}').ok_or("no JSON object")? + 1;
    serde_json::from_str(&trimmed[start..end.max(start)]).map_err(|error| error.to_string())
}

#[cfg(test)]
mod planner_tests;
