//! Runs a plain-language task end to end against real websites or desktop
//! applications, the way an outside agent would: it loads the built module
//! through the `TinyBus` loader and uses nothing but its bus members. The
//! module's planner writes the flow (`PlanTask`), the task runs with live
//! Jev (`StartTask`), and the run is followed until it stops (`AwaitTask`);
//! the report comes from `TaskReport` and the final screenshot from
//! `BrowserScreenshot` and `BrowserReadOutput`. Payment is never entered: a booking ends at the payment
//! checkpoint, and a task that only reads ends `done` with its records.
//!
//! Inputs, all read from the environment:
//!
//! - `TINYCOMPUTER_MODULE` — the attested module, as `scripts/build-module`
//!   prints it (default `$CARGO_TARGET_DIR/lab/`, or `target/lab/`).
//! - `OPENROUTER_API_KEY` — Jev and the planner.
//! - `TINYHUMANS_TOKEN` — optional, in place of `OPENROUTER_API_KEY`: a Tiny
//!   Humans bearer (a session token, or an API key with the `inference`
//!   scope) that sends Jev and the planner through Tiny Humans' routes, with
//!   `openrouter/deepseek/deepseek-v4-flash` planning, rescuing, and shaping
//!   unless the model variables below name another.
//! - `TASK_FILE` — the task in plain language.
//! - `FACTS_FILE` — a JSON object of facts for the task, by name. A value is
//!   a string, or `{"value": "...", "secret": true}` to keep it secret; a
//!   card number or passport number is secret anyway.
//! - `FLOW_FILE` — optional: run this flow instead of planning one.
//! - `TASK_PLAN` — optional: `in-task` hands the task to `StartTask` to plan,
//!   as `OpenHuman` does, rather than planning it first with `PlanTask`; the
//!   plan is printed and saved once the task stops.
//! - `TINYCOMPUTER_BROWSER_PRELAUNCH` — optional: `0` opens a browser-only
//!   task's browser at its first step rather than while the task plans
//!   itself (with `TASK_PLAN=in-task`), at the address the task names, as
//!   it does by default.
//! - `TASK_OUT` — optional: where the plan, report, and final screenshot go
//!   (default `target/task-live`).
//! - `OUTPUT_FILE` — optional: a JSON `TaskOutput` (`instructions` and a
//!   `schema`) asking for the answer in a fixed shape; the result is written
//!   to `result.json`.
//! - `TINYCOMPUTER_OUTPUT_MODEL` — optional: the model that shapes it
//!   (`openai/gpt-6-luna` by default on `OpenRouter`,
//!   `openrouter/deepseek/deepseek-v4-flash` on Tiny Humans).
//! - `TASK_SURFACE` — optional: `browser` (default) or `desktop`, the
//!   applications on this Mac through the accessibility tree. A desktop task
//!   runs on the host, in a shell that has the Accessibility permission.
//! - `TINYCOMPUTER_FLOW_STRATEGY` — optional: `narrow` (default) or `wide`.
//! - `TINYCOMPUTER_FLOW_DELIBERATION` — optional: `deep` (default),
//!   `standard`, or `off`.
//! - `TASK_MAX_MINUTES` — optional: cancel the task after this long (20).
//! - `TASK_RESCUES` — optional: how many failed steps the reasoning model
//!   may rescue (0 to 5, default 5; 0 turns rescues off).
//! - `TINYCOMPUTER_RESCUE_MODEL` — optional: the model that rescues them
//!   (`openai/gpt-6-luna` by default on `OpenRouter`,
//!   `openrouter/deepseek/deepseek-v4-flash` on Tiny Humans).
//! - `TINYCOMPUTER_DECISIONS` — optional: `sage` makes Levanto Sage take
//!   every decision in place of Jev, with `SAGE_API_KEY`, through the
//!   module's `jev` configuration; `SAGE_FAST=1` scores each choice in one
//!   pass. The planner and the rescuer keep their own route
//!   (`OPENROUTER_API_KEY`, or `TINYHUMANS_TOKEN`).
//! - `TINYCOMPUTER_BROWSER_EXECUTABLE`, `TINYCOMPUTER_BROWSER_USER_AGENT`, and
//!   `TINYCOMPUTER_BROWSER_ARGS` (space-separated) — how the browser
//!   launches, `TINYCOMPUTER_BROWSER_PERCEPTION` (`sight` or `tree`) how
//!   pages are read, and `TINYCOMPUTER_BROWSER_SETTLE` (`prompt`, the
//!   default, or `steady`) how a page settles after an action; all passed
//!   as the module's `browser` configuration.
//! - `TASK_CURSOR` — optional: the agent's on-screen cursor pace (`off`,
//!   `brisk`, `natural`, `calm`; default `natural`). It is drawn by the
//!   `tinycomputer-cursor-overlay` helper, which the module finds beside
//!   itself, over a browser window on this screen — an attached Chrome, or a
//!   headed one.
//! - `TASK_INTERACTIVE` — optional: `1` makes the run wait for the person
//!   at this terminal where only a person can go on, instead of ending it:
//!   approve or decline an irreversible action, get past a login or captcha
//!   in the browser window and press Enter, type a detail the task lacks,
//!   and finish on a payment page before the browser closes.
//! - `TASK_MEMORY` — optional: a JSON file of grounding hints. The task
//!   starts with the elements earlier runs learned there (`StartTask`'s
//!   `memory`), so a remembered one is confirmed with a yes or no instead of
//!   searched for, and what this run learns is saved back, newer hints
//!   replacing older ones for the same element.
//! - `TASK_HEADED` — optional: `1` shows the browser the task launches
//!   instead of running it headless. A headed browser needs a display, so
//!   such a run is on the host.
//! - `TINYCOMPUTER_BROWSER_ENDPOINT` — attach to a running Chrome instead
//!   (`http://127.0.0.1:9222`); booking sites turn away a fresh headless
//!   browser but serve a person's own. Closing the run only disconnects.
//!
//! Launching a headless browser runs in the Docker lab, never on the host:
//! `scripts/docker-lab -- crates/tinycomputer-examples/tasks/run kashmir`.
//! A headed launch (`TASK_HEADED=1`) and attaching to your own Chrome run on
//! the host, since that is where the display and the browser are.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tinycomputer_bus::Flow;
use tinycomputer_bus::agent::{
    PlanTaskRequest, StartTaskRequest, SurfaceKind, TaskBudget, TaskConstraints, TaskOutput,
};
use tinycomputer_examples::host::{Host, LabError, jev_config, module_path};
use tinycomputer_examples::task::{Person, Terminal, conclude, follow, passed};

mod saved;

use saved::{read_memory, record_plan, remember};

#[tokio::main]
async fn main() -> Result<(), LabError> {
    let task = std::fs::read_to_string(env("TASK_FILE")?)?;
    let (facts, secret_facts) = read_facts(&std::fs::read_to_string(env("FACTS_FILE")?)?)?;
    let out =
        PathBuf::from(std::env::var("TASK_OUT").unwrap_or_else(|_| "target/task-live".into()));
    std::fs::create_dir_all(&out)?;
    let output: Option<TaskOutput> = match std::env::var("OUTPUT_FILE") {
        Ok(path) => Some(serde_json::from_str(&std::fs::read_to_string(path)?)?),
        Err(_) => None,
    };
    let kind = surface_kind()?;

    // `TASK_PLAN=in-task` leaves planning to `StartTask`, as `OpenHuman` does,
    // so the module can open the browser while it plans.
    let in_task = in_task(std::env::var("TASK_PLAN").ok().as_deref())?;
    let host = Host::load(&module_path(), module_config()?).await?;
    let flow = match std::env::var("FLOW_FILE") {
        Ok(path) => Some(serde_json::from_str(&std::fs::read_to_string(path)?)?),
        Err(_) if in_task => None,
        Err(_) => Some(plan(&host, &task, &facts, &secret_facts, kind, &out).await?),
    };
    // A given flow is no plan the task drafted.
    let drafted_in_task = flow.is_none();
    // The task travels with the flow, so every Jev question is briefed on it.
    // Sessions open before the task are not its own, and `conclude` leaves
    // them alone.
    let before = host.browser_sessions().await?;
    let memory_file = std::env::var("TASK_MEMORY").ok().map(PathBuf::from);
    let memory = match &memory_file {
        Some(path) => read_memory(path)?,
        None => Vec::new(),
    };
    let view = host
        .start_task(&StartTaskRequest {
            task: Some(task.clone()),
            flow,
            handoff_to_planner: false,
            facts,
            fact_handles: BTreeMap::new(),
            secret_facts,
            constraints: TaskConstraints {
                surfaces: vec![kind],
                browser_endpoint: std::env::var("TINYCOMPUTER_BROWSER_ENDPOINT").ok(),
                headed: std::env::var("TASK_HEADED").is_ok_and(|value| value == "1"),
                ..TaskConstraints::default()
            },
            budget: TaskBudget {
                max_actions: Some(200),
                max_model_calls: Some(5000),
                strategy: tinycomputer_examples::flow_strategy_from_env(),
                deliberation: tinycomputer_examples::flow_deliberation_from_env(),
                max_rescues: std::env::var("TASK_RESCUES")
                    .ok()
                    .and_then(|value| value.trim().parse().ok()),
                ..TaskBudget::default()
            },
            trace: true,
            output,
            memory,
            site: None,
        })
        .await?;
    let limit = Duration::from_secs(
        60 * std::env::var("TASK_MAX_MINUTES")
            .ok()
            .and_then(|minutes| minutes.parse().ok())
            .unwrap_or(20),
    );
    // A person at the terminal answers the pauses only a person can.
    let person = std::env::var("TASK_INTERACTIVE")
        .is_ok_and(|value| value == "1")
        .then_some(&Terminal as &dyn Person);
    let view = follow(&host, view, &BTreeMap::new(), limit, person).await?;
    conclude(&host, &view, &before, &out).await?;
    if drafted_in_task {
        record_plan(&out)?;
    }
    if let Some(path) = &memory_file {
        remember(&out, path)?;
    }
    host.shutdown();
    if passed(&view.status) {
        println!(
            "PASS finished or stopped at payment; artifacts in {}",
            out.display()
        );
        Ok(())
    } else {
        Err("FAIL the task neither finished nor reached the payment checkpoint".into())
    }
}

/// The module's private configuration: Jev, the planner (which also brings
/// the rescuer and the output shaper), the cursor, and how browsers launch.
fn module_config() -> Result<Value, LabError> {
    let optional = |name: &str| std::env::var(name).ok();
    let mut browser = serde_json::Map::new();
    for (field, variable) in [
        ("executable", "TINYCOMPUTER_BROWSER_EXECUTABLE"),
        ("user_agent", "TINYCOMPUTER_BROWSER_USER_AGENT"),
    ] {
        if let Some(value) = optional(variable).filter(|value| !value.trim().is_empty()) {
            browser.insert(field.to_owned(), json!(value.trim()));
        }
    }
    if let Some(prelaunch) = prelaunch(optional("TINYCOMPUTER_BROWSER_PRELAUNCH").as_deref())? {
        browser.insert("prelaunch".to_owned(), json!(prelaunch));
    }
    for (variable, field) in [
        ("TINYCOMPUTER_BROWSER_PERCEPTION", "perception"),
        ("TINYCOMPUTER_BROWSER_SETTLE", "settle"),
    ] {
        if let Some(value) = optional(variable) {
            let value = value.trim().to_ascii_lowercase();
            if !value.is_empty() {
                browser.insert(field.to_owned(), json!(value));
            }
        }
    }
    if let Some(args) = optional("TINYCOMPUTER_BROWSER_ARGS") {
        browser.insert(
            "args".to_owned(),
            json!(args.split_whitespace().collect::<Vec<_>>()),
        );
    }
    let (jev, planner) = routes(&|name| std::env::var(name).ok())?;
    Ok(json!({
        "jev": jev,
        "planner": planner,
        "cursor": optional("TASK_CURSOR").unwrap_or_else(|| "natural".to_owned()),
        "browser": browser,
    }))
}

/// The model the Tiny Humans gateway plans, rescues, and shapes with when
/// none is named: the managed default `OpenHuman` runs its own hosted work
/// on, an `openrouter/`-prefixed id from the gateway's catalog. The gateway
/// refuses the engine's own defaults, bare vendor ids such as
/// `anthropic/claude-sonnet-5`, and its `agentic-v1` tier reasons for 25 to
/// 45 seconds over a small rescue, long enough on a real one to pass the
/// module's 120-second rescue limit; this model answers the same rescue in
/// 7 to 13 seconds.
const TINY_HUMANS_MODEL: &str = "openrouter/deepseek/deepseek-v4-flash";

/// What this runner calls itself to the Tiny Humans routes.
const SDK_NAME: &str = "tinycomputer-task-live";

/// The `jev` and `planner` configurations, from the variables `var` reads:
/// Tiny Humans' routes when `TINYHUMANS_TOKEN` holds a Tiny Humans bearer (a
/// session token, or an API key with the `inference` scope), else
/// `OpenRouter` with `OPENROUTER_API_KEY`.
fn routes(var: &dyn Fn(&str) -> Option<String>) -> Result<(Value, Value), LabError> {
    let bearer = var("TINYHUMANS_TOKEN")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(bearer) = bearer {
        let model = |name: &str| {
            var(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| TINY_HUMANS_MODEL.to_owned())
        };
        let jev = json!({
            "api_key": bearer,
            "provider": "tiny_humans_open_router",
            "sdk_name": SDK_NAME,
        });
        let planner = json!({
            "api_key": bearer,
            "provider": "tiny_humans",
            "sdk_name": SDK_NAME,
            "model": model("TINYCOMPUTER_PLANNER_MODEL"),
            "rescue_model": model("TINYCOMPUTER_RESCUE_MODEL"),
            "output_model": model("TINYCOMPUTER_OUTPUT_MODEL"),
        });
        return Ok((decisions(var, jev)?, planner));
    }
    let key = var("OPENROUTER_API_KEY").ok_or_else(|| {
        std::io::Error::other("neither OPENROUTER_API_KEY nor TINYHUMANS_TOKEN is exported")
    })?;
    let planner = json!({
        "api_key": key,
        "model": var("TINYCOMPUTER_PLANNER_MODEL"),
        "rescue_model": var("TINYCOMPUTER_RESCUE_MODEL"),
        "output_model": var("TINYCOMPUTER_OUTPUT_MODEL"),
    });
    Ok((decisions(var, jev_config(key, None)?)?, planner))
}

/// Who takes the flow's decisions, as the module's `jev` configuration:
/// Levanto Sage when `TINYCOMPUTER_DECISIONS` is `sage`, else `jev`.
fn decisions(var: &dyn Fn(&str) -> Option<String>, jev: Value) -> Result<Value, LabError> {
    if var("TINYCOMPUTER_DECISIONS").as_deref() == Some("sage") {
        let sage = var("SAGE_API_KEY").ok_or_else(|| {
            std::io::Error::other("TINYCOMPUTER_DECISIONS=sage needs SAGE_API_KEY")
        })?;
        let fast = var("SAGE_FAST").is_some_and(|value| value == "1");
        return Ok(json!({"api_key": sage, "provider": "sage", "fast": fast}));
    }
    Ok(jev)
}

/// The surface the task runs on, from `TASK_SURFACE`.
fn surface_kind() -> Result<SurfaceKind, LabError> {
    match std::env::var("TASK_SURFACE").as_deref() {
        Err(_) | Ok("browser") => Ok(SurfaceKind::Browser),
        Ok("desktop") => Ok(SurfaceKind::Desktop),
        Ok(other) => {
            Err(format!("TASK_SURFACE must be `browser` or `desktop`, not `{other}`").into())
        }
    }
}

/// The facts file's values by name, and the names it marks secret.
fn read_facts(text: &str) -> Result<(BTreeMap<String, String>, Vec<String>), LabError> {
    let raw: BTreeMap<String, serde_json::Value> = serde_json::from_str(text)?;
    let mut facts = BTreeMap::new();
    let mut secret = Vec::new();
    for (name, value) in raw {
        let text = match &value {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Object(fields) => {
                if fields.get("secret").and_then(serde_json::Value::as_bool) == Some(true) {
                    secret.push(name.clone());
                }
                fields
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("fact `{name}` has no string `value`"))?
                    .to_owned()
            }
            _ => return Err(format!("fact `{name}` must be a string or an object").into()),
        };
        facts.insert(name, text);
    }
    Ok((facts, secret))
}

/// Whether `TASK_PLAN`'s `value` leaves planning to the task: `in-task`
/// does, and no value does not.
fn in_task(value: Option<&str>) -> Result<bool, LabError> {
    match value.map(str::trim) {
        None | Some("") => Ok(false),
        Some("in-task") => Ok(true),
        Some(other) => Err(format!("TASK_PLAN must be `in-task`, not `{other}`").into()),
    }
}

/// The module's `prelaunch` from `TINYCOMPUTER_BROWSER_PRELAUNCH`'s `value`:
/// `0` or `1`, or the module's own default when unset.
fn prelaunch(value: Option<&str>) -> Result<Option<bool>, LabError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some("0") => Ok(Some(false)),
        Some("1") => Ok(Some(true)),
        Some(other) => {
            Err(format!("TINYCOMPUTER_BROWSER_PRELAUNCH must be `0` or `1`, not `{other}`").into())
        }
    }
}

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

/// Asks the planner for a flow over the bus, prints it, and keeps a copy
/// beside the report.
async fn plan(
    host: &Host,
    task: &str,
    facts: &BTreeMap<String, String>,
    secret_facts: &[String],
    kind: SurfaceKind,
    out: &std::path::Path,
) -> Result<Flow, LabError> {
    let plan = host
        .plan_task(&PlanTaskRequest {
            task: task.to_owned(),
            fact_names: facts.keys().cloned().collect(),
            secret_facts: secret_facts.to_vec(),
            surfaces: vec![kind],
        })
        .await?;
    let text = serde_json::to_string_pretty(&plan.flow)?;
    println!("plan:\n{text}");
    for note in &plan.notes {
        println!("  note: {note}");
    }
    for question in &plan.questions {
        println!("  question: {} ({})", question.name, question.why);
    }
    std::fs::write(out.join("plan.json"), &text)?;
    Ok(plan.flow)
}

#[cfg(test)]
mod main_tests;
