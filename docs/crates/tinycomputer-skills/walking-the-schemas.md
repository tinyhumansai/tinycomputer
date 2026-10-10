# Walking the schemas

There is currently one schema file in this crate,
[`start_task.schema.json`](../../../crates/tinycomputer-skills/schemas/start_task.schema.json),
covering the request an agent sends to begin a task. This page walks it
section by section.

## `task` and `flow`: two ways to say what should happen

```json
"task": {"type": "string", "description": "The goal in plain language; needs a planner."},
"flow": {"type": "object", "description": "A flow written from the guide Describe returns."},
```

Exactly one of these is normally supplied. `task` is plain language, and
only works when the module has a planner configured (`Describe` reports
`planner_configured`); the planner turns it into a flow before anything
runs. `flow` is a flow written directly by the caller, following the guide
`Describe` returns, for when no planner is available or the caller wants
more control over exactly what happens. See
[Writing flows](../../writing-flows.md) for how to actually write one.

## `facts` and `secret_facts`: what the task is allowed to know

```json
"facts": {
  "type": "object",
  "additionalProperties": {"type": "string"},
  "description": "Values the task may type, by name. Shared ones brief the decision model; card, passport, password, and one-time-code values are secret and only ever shown to a model as ${name}."
},
"secret_facts": {
  "type": "array",
  "items": {"type": "string"},
  "description": "Names among facts to keep secret as well."
}
```

`facts` is a flat map of name to value: a first name, an email address, a
date of birth. Most facts are shown to the decision model as their actual
values, so it understands who it is acting for. A handful of kinds (card
numbers, passport numbers, passwords, one-time codes) are automatically
treated as secret and are only ever shown to the model as a placeholder like
`${card number}`; `secret_facts` lets the caller mark any other fact name as
secret the same way. This split, and what "secret" actually guarantees, is
covered in more depth in [Safety and privacy](../../safety-and-privacy.md).

## `constraints`: the fence around what a task may touch

```json
"constraints": {
  "type": "object",
  "properties": {
    "payment": {"enum": ["stop_at_payment", "fill_then_approve"], ...},
    "surfaces": {"type": "array", "items": {"enum": ["desktop", "browser"]}},
    "origins": {"type": "array", "items": {"type": "string"}},
    "allow_destructive": {"type": "boolean"},
    "browser_endpoint": {"type": "string"},
    "headed": {"type": "boolean"}
  }
}
```

- `payment` decides whether a task stops right before anything that pays
  (`stop_at_payment`, the default) or fills the payment form from secret
  facts and then waits for an explicit approval before actually pressing
  pay (`fill_then_approve`, which also needs `origins` set).
- `surfaces` limits the task to `desktop`, `browser`, or both, so for
  example a task that should only ever touch a website cannot reach a
  person's desktop applications even by mistake.
- `origins` is the list of sites (or origin patterns, or `*` for any public
  site) a task may open pages on; the files those pages load from elsewhere
  are not checked.
- `allow_destructive` lets the task send, delete, or confirm things without
  pausing for approval first; it defaults to off.
- `browser_endpoint` and `headed` control how the browser itself is run: an
  existing browser connection to reuse, or whether to show the browser
  window rather than running it invisibly.
- The browser binary and the profile folder a task launches with
  (`browser_executable`, `browser_profile` in the contract) are the host's
  settings, never a model's, so the skill's schema and `Describe` leave them
  out.

## `budget`: how much the task is allowed to do

```json
"budget": {
  "type": "object",
  "properties": {
    "max_actions": {"type": "integer"},
    "max_model_calls": {"type": "integer"},
    "max_elapsed_ms": {"type": "integer"},
    "votes": {"type": "integer", ...},
    "strategy": {"type": "string", "enum": ["narrow", "wide"], ...},
    "deliberation": {"type": "string", "enum": ["off", "standard", "deep"], ...},
    "max_rescues": {"type": "integer", "minimum": 0, "maximum": 5, ...}
  }
}
```

These are hard caps (`max_actions`, `max_model_calls`, `max_elapsed_ms`) plus
two settings that trade speed for care rather than capping anything:
`strategy` (how each decision is put to the decision model: many small
`narrow` requests, the default, or one bigger `wide` request per turn) and
`deliberation` (how hard a close call is double-checked before acting:
`deep`, the default, `standard`, or `off`). `max_rescues` caps how many
times a failed step may be handed to a reasoning model for another way
forward, from 0 (off) up to 5, and needs a planner configured to do
anything at all. These map directly onto the budget concepts explained for
a human reader in [Giving it a task](../../giving-it-a-task.md#limits-you-can-set),
and onto the deeper mechanics in
[`docs/technical/decision-loops.md`](../../technical/decision-loops.md) and
[`docs/technical/decision-thresholds.md`](../../technical/decision-thresholds.md).

## `trace`

```json
"trace": {"type": "boolean"}
```

A flag, not something most callers need. Turning it on asks the task to
return its full decision trace (every question asked of the decision model
and every merged answer) alongside the ordinary result, which is mainly
useful for debugging a run rather than for driving one. See
[Watching a run](../../watching-a-run.md).

## `members`

```json
"members": ["Describe", "PlanTask", "StartTask", "AwaitTask", "ContinueTask", "CancelTask", "TaskReport", "ListTasks"]
```

This is not part of the request body; it lists every task-related member
the module serves, so a caller reading the schema alongside the skill
document knows the full set of calls available, not just `StartTask`'s
shape. A test in this crate checks that this list is always exactly the
module's real list of task members (`tinycomputer_bus::agent::names::METHODS`),
so the schema cannot go stale if a member is ever added, renamed, or
removed.

## Keeping it honest

Two tests worth knowing about, both in
[`src/lib_tests.rs`](../../../crates/tinycomputer-skills/src/lib_tests.rs):

- one parses the schema and checks that every field on the real
  `StartTaskRequest` type (aside from an internal `memory` field) is
  documented in `properties`, so a new request field cannot ship without
  the schema mentioning it;
- another extracts the worked example from `SKILL.md` and actually
  deserializes it into `StartTaskRequest`, so the example in the prose
  document is a real, currently-valid request, not just plausible-looking
  text.
