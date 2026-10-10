//! `Describe`: everything a model needs to drive the module, in one reply —
//! what is available, the flow guide, each task member's input schema,
//! worked requests to adapt, and a catalogue of every other member.
//!
//! The schemas here are written by hand, so each field of a request type
//! must be added here when it is added to the contract, save the host's
//! own settings no model is offered (a task's browser binary and profile);
//! `task_tests/describe_tests.rs` checks the `StartTask` and `PlanTask`
//! schemas name every other field their types serialize.

use serde_json::{Value, json};
use tinycomputer_bus::agent::names::{CONFIDENTIAL, methods};
use tinycomputer_bus::agent::{Capabilities, Example, MemberDoc, SurfaceAvailability};
use tinycomputer_bus::browser::names::methods as browser;
use tinycomputer_bus::{CONTRACT_VERSION, FLOW_GUIDE, JevConfiguration, STEP_KINDS};

/// The capabilities reply for a module with these surfaces, the decision
/// model it asks when one is configured, and what `tasks` has: a planner, a
/// rescuer, a shaper, and the models behind them.
#[must_use]
pub fn capabilities(
    surfaces: Vec<SurfaceAvailability>,
    decision_model: Option<&JevConfiguration>,
    tasks: &super::Tasks,
) -> Capabilities {
    Capabilities {
        contract_version: CONTRACT_VERSION,
        surfaces,
        jev_configured: decision_model.is_some(),
        planner_configured: tasks.planner_configured(),
        rescue_configured: tasks.rescue_configured(),
        output_configured: tasks.output_configured(),
        decision_model: decision_model.cloned(),
        planner_model: tasks.planner_model().cloned(),
        rescue_model: tasks.rescue_model().cloned(),
        output_model: tasks.output_model().cloned(),
        step_kinds: STEP_KINDS.iter().map(|kind| (*kind).to_owned()).collect(),
        guide: FLOW_GUIDE.to_owned(),
        members: members(),
        examples: examples(),
        catalogue: tinycomputer_bus::catalogue::summaries(),
    }
}

fn member(name: &str, summary: &str, input: Value, output: &str) -> MemberDoc {
    MemberDoc {
        name: name.to_owned(),
        summary: summary.to_owned(),
        confidential: CONFIDENTIAL.contains(&name),
        input,
        output: json!({"description": output}),
    }
}

fn surfaces() -> Value {
    json!({"type": "array", "items": {"enum": ["desktop", "browser"]}})
}

fn task_id() -> Value {
    json!({"type": "object", "required": ["id"], "properties": {
        "id": {"type": "string", "description": "the task id StartTask returned"}
    }})
}

/// `StartTask`'s input schema: every field of `StartTaskRequest` a model may
/// set. `browser_executable` and `browser_profile` are the host's to set
/// from its own settings, so they are not offered.
fn start_task_input() -> Value {
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "required": required, "properties": properties});
    object(
        json!({
            "task": {"type": "string", "description": "the goal in plain language"},
            "flow": {"type": "object", "description": "a flow written from the guide"},
            "facts": {
                "type": "object",
                "additionalProperties": {"type": "string"},
                "description": "values the flow may type, by name; secret ones never reach a model"
            },
            "secret_facts": {
                "type": "array",
                "items": {"type": "string"},
                "description": "names among facts to keep secret beyond the ones recognised as sensitive"
            },
            "constraints": {"type": "object", "properties": {
                "payment": {
                    "enum": ["stop_at_payment", "fill_then_approve"],
                    "default": "stop_at_payment",
                    "description": "fill_then_approve needs origins that name the sites card details may be typed on, never *"
                },
                "surfaces": surfaces(),
                "origins": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "sites the browser may open pages on: https://example.com, https://.example.com with its subdomains, or * for any public site; only pages are checked, never the files a page loads"
                },
                "allow_destructive": {"type": "boolean"},
                "browser_endpoint": {"type": "string"},
                "headed": {"type": "boolean"}
            }},
            "budget": {"type": "object", "properties": {
                "max_actions": {"type": "integer"},
                "max_model_calls": {"type": "integer"},
                "votes": {"type": "integer"},
                "strategy": {"enum": ["narrow", "wide"], "default": "narrow"},
                "deliberation": {"enum": ["off", "standard", "deep"], "default": "deep"},
                "max_elapsed_ms": {"type": "integer"},
                "max_rescues": {
                    "type": "integer",
                    "maximum": 5,
                    "description": "how often a failed step may be rescued by the reasoning model; 0 turns rescues off"
                }
            }},
            "memory": {
                "type": "array",
                "items": {"type": "object"},
                "description": "TaskReport.learned from an earlier run, so this one reads less"
            },
            "output": {
                "type": "object",
                "required": ["instructions"],
                "description": "the shape to return the answer in, as done.result; needs output_configured",
                "properties": {
                    "instructions": {"type": "string"},
                    "schema": {
                        "type": "object",
                        "description": "a JSON Schema with an object at the top, using only type, properties, required, additionalProperties, items, enum, minItems, maxItems, description, title"
                    }
                }
            },
            "trace": {"type": "boolean"}
        }),
        &[],
    )
}

fn members() -> Vec<MemberDoc> {
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "required": required, "properties": properties});
    vec![
        member(
            methods::DESCRIBE,
            "How to use these members: surfaces, the flow guide, schemas, and examples.",
            json!({"type": "null"}),
            "Capabilities",
        ),
        member(
            methods::PLAN_TASK,
            "Drafts a flow for a plain-language task without acting; needs a planner.",
            object(
                json!({
                    "task": {"type": "string"},
                    "fact_names": {"type": "array", "items": {"type": "string"}},
                    "secret_facts": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "names among fact_names the plan may only type"
                    },
                    "surfaces": surfaces()
                }),
                &["task"],
            ),
            "TaskPlan: the flow, and the values to collect first",
        ),
        member(
            methods::START_TASK,
            "Starts a task from a flow (or a plain-language task, with a planner) and returns at once.",
            start_task_input(),
            "TaskView",
        ),
        member(
            methods::AWAIT_TASK,
            "Waits until the task needs something or finishes, up to timeout_ms.",
            object(
                json!({
                    "id": {"type": "string"},
                    "timeout_ms": {"type": "integer", "default": 30_000, "maximum": 60_000}
                }),
                &["id"],
            ),
            "TaskView",
        ),
        member(
            methods::CONTINUE_TASK,
            "Answers a paused task: inputs for needs_input, approve for needs_approval.",
            object(
                json!({
                    "id": {"type": "string"},
                    "inputs": {"type": "object", "additionalProperties": {"type": "string"}},
                    "approve": {"type": "boolean"},
                    "answer": {"type": "string"}
                }),
                &["id"],
            ),
            "TaskView",
        ),
        member(methods::CANCEL_TASK, "Stops a task.", task_id(), "TaskView"),
        member(
            methods::TASK_REPORT,
            "Everything a task did: steps, records, rescues, artifacts, and learned hints.",
            object(
                json!({
                    "id": {"type": "string"},
                    "trace": {
                        "type": "boolean",
                        "description": "include every Jev exchange StartTask.trace recorded; false keeps the report small. Required: a confidential body of only {\"id\"} is refused by the TinyBus client as a stream handle"
                    }
                }),
                &["id", "trace"],
            ),
            "TaskReport",
        ),
        member(
            methods::LIST_TASKS,
            "The tasks this module holds, newest first.",
            json!({"type": "null"}),
            "array of TaskView",
        ),
    ]
}

fn examples() -> Vec<Example> {
    vec![
        Example {
            title: "Find the cheapest flight and fill traveller details up to payment".to_owned(),
            member: methods::START_TASK.to_owned(),
            request: json!({
                "flow": {
                    "app": "browser",
                    "steps": [
                        {"browse": "https://www.google.com/travel/flights"},
                        {"enter": {"where from": "${from}", "where to": "${to}", "departure date": "${date}"}},
                        "search for flights",
                        {"wait_for": "flight results are listed"},
                        {"pick": {"from": "the flight results", "by": "lowest price", "into": "cheapest"}},
                        "continue to booking",
                        {"enter": {"first name": "${first name}", "last name": "${last name}", "email": "${email}", "phone": "${phone}"}},
                        {"stop_before": "paying for the booking"}
                    ]
                },
                "facts": {
                    "from": "Delhi",
                    "to": "Srinagar",
                    "date": "14 October",
                    "first name": "Asha",
                    "last name": "Raina",
                    "email": "asha@example.com"
                },
                "constraints": {"surfaces": ["browser"]}
            }),
        },
        Example {
            title: "Wait for the task to need something".to_owned(),
            member: methods::AWAIT_TASK.to_owned(),
            request: json!({"id": "t-1", "timeout_ms": 30_000}),
        },
        Example {
            title: "Supply a value the task asked for".to_owned(),
            member: methods::CONTINUE_TASK.to_owned(),
            request: json!({"id": "t-1", "inputs": {"phone": "+91 98765 43210"}}),
        },
        Example {
            title: "Approve the irreversible action the task stopped before".to_owned(),
            member: methods::CONTINUE_TASK.to_owned(),
            request: json!({"id": "t-1", "approve": true}),
        },
        Example {
            title: "Collect a task's findings as JSON in a fixed shape".to_owned(),
            member: methods::START_TASK.to_owned(),
            request: json!({
                "flow": {
                    "app": "browser",
                    "steps": [
                        {"browse": "https://news.ycombinator.com"},
                        {"extract": {"what": "the stories on the front page", "fields": ["title", "points"], "into": "stories"}}
                    ]
                },
                "output": {
                    "instructions": "the five stories with the most points, highest first",
                    "schema": {
                        "type": "object",
                        "required": ["stories"],
                        "properties": {"stories": {
                            "type": "array",
                            "maxItems": 5,
                            "items": {"type": "object", "required": ["title", "points"], "properties": {
                                "title": {"type": "string"},
                                "points": {"type": "string"}
                            }}
                        }}
                    }
                },
                "constraints": {"surfaces": ["browser"]}
            }),
        },
        Example {
            title: "Drive the browser yourself: open a session".to_owned(),
            member: browser::OPEN_SESSION.to_owned(),
            request: json!({"headless": true}),
        },
        Example {
            title: "Navigate the session, then snapshot it for refs".to_owned(),
            member: browser::NAVIGATE.to_owned(),
            request: json!({"session": "s-1", "url": "https://example.com"}),
        },
        Example {
            title: "Click a ref from the latest BrowserSnapshot".to_owned(),
            member: browser::PERFORM.to_owned(),
            request: json!({"session": "s-1", "action": "click", "target": {"kind": "ref", "value": "e3"}}),
        },
        Example {
            title: "Read a held screenshot, from offset 0 until eof".to_owned(),
            member: browser::READ_OUTPUT.to_owned(),
            request: json!({"output": "o-1", "offset": 0}),
        },
    ]
}
