# Desktop Module Contract

## Purpose

Expose the vendored `agent-desktop` automation engine over TinyBus so a host can
offer desktop observation and interaction to an agent as typed tool calls,
without that host compiling the engine or any platform accessibility backend.

## Scope

This repository is an adapter. The engine's behavior — how a tree is walked, how
a ref is allocated and resolved, what counts as actionable — is upstream and
out of scope. In scope: the wire contract, the conversion between it and the
engine's argument types, the permission preflight, and the bus surface.

## Contract

### Members

- The interface `ai.tinyhumans.tinycomputer.Desktop` is served at
  `/ai/tinyhumans/tinycomputer/Desktop` with exactly eighty members,
  enumerated in dispatch order by `tinycomputer_bus::names::METHODS`, and
  catalogued — family, one-line summary, confidentiality — by
  `tinycomputer_bus::catalogue::MEMBERS`, which `Describe` serves as
  `Capabilities.catalogue` (2.6).
- Every member takes at most one request payload. Members taking no argument:
  `ListDisplays`, `ClipboardClear`, `FlowGuide`, `Version`, `Status`,
  `Describe`, `ListTasks`, `BrowserListSessions`.
- Every desktop and agentic member returns a `DesktopResponse`. The eight task
  members (`Describe`, `PlanTask`, `StartTask`, `AwaitTask`, `ContinueTask`,
  `CancelTask`, `TaskReport`, `ListTasks`, contract 1.7; briefs, votes,
  secrets, and the payment mode 1.8; 2.0 renamed the interface and object
  path from `tinydesktop` to `tinycomputer`; 2.1 added the flow `strategy`
  and the `survey` and `digest` loops, `specs/jev-wide-turns.md`; 2.2 added
  the `reflection` loop, `specs/flow-reflection.md`; 2.3 added the flow
  `deliberation` level and its eight loops, `specs/jev-deliberation.md`; 2.4
  added task rescues, `budget.max_rescues`, `TaskReport.rescues`, and
  `Capabilities.rescue_configured`, `specs/task-rescue.md`; 2.5 added task
  output shapes, `StartTask.output`, `done.result`, and
  `Capabilities.output_configured`, `specs/task-output.md`; 2.8 added the
  `open_jev` and `sage` decision providers, `JevConfig.fast`,
  `JevConfiguration.fast`, the planner's `LanguageModelProvider` routes, and
  `Capabilities.decision_model`, `planner_model`, `rescue_model`, and
  `output_model`; 2.9 added `RunFlowRequest.dialog_left_open` and
  `FlowRunResult.dialog_left_open`; 2.10 added the `StartTask` constraints
  `browser_executable` and `browser_profile`, and `*` (any public host) among
  the allowed origins, which the module checks on pages rather than on every
  request; all optional) return an
  `AgentResponse` instead — see [`unified-agent.md`](unified-agent.md). They
  share this interface because a TinyBus module exports one interface. A
  task hands `dialog_left_open` from one run's result to its next run's
  request: a run takes a dialog in front at its first look as the task's
  own stage only when the run before it left that dialog open.
- The thirteen browser members (2.6) close the list, each prefixed `Browser`
  (`tinycomputer_bus::browser::names`): sessions, navigate, snapshot,
  perform, read, evaluate, screenshot, held outputs, and downloads. A member
  that acts on an open session takes one object, `{"session": …}` beside the
  member's own fields; `BrowserOpenSession` takes `SessionOptions` (it makes
  the session), `BrowserListSessions` takes nothing, and `BrowserReadOutput`
  and `BrowserReleaseOutput` take `{"output": …}`. Every one returns a
  `DesktopResponse`. A failure's `code` is `browser::errors::code` of its
  wire name, which reuses the desktop's code where the meaning is the same
  (`STALE_REF`, `ELEMENT_NOT_FOUND`, `TIMEOUT`, `POLICY_DENIED`,
  `INVALID_ARGS`, `INTERNAL`); the full name is in `details.name`, the
  recovery hint is `browser::errors::recovery`'s — a timeout or page error,
  which may follow a click that landed, says to inspect the page before
  retrying — and only a failure decided before anything reaches the page (an
  unknown session or output, an unresolvable ref, a refused origin) is marked
  `not_delivered`; every other failure's delivery is `unknown`. The members share
  one `Browser` with the task runner, so `BrowserReadOutput` reads a
  screenshot in a task's `TaskReport.artifacts` (never in a task view) and
  `BrowserListSessions` shows a task's session. The `ai.tinyhumans.tinycomputer.Browser` interface and its
  unprefixed names, never served by any release, are retired; the error
  names keep that prefix because they are published values, not members.
- `TaskReport` takes a `TaskReportRequest` (2.7), `{"id", "trace"}` with
  `trace` always serialized, rather than a `TaskRef`. A TinyBus client
  refuses a confidential body holding an object whose only field is a string
  `id` — the shape of a stream handle — so the 2.6 request could not be sent
  at all. `trace: false` leaves the Jev exchanges out of a report.
- Members are named in `PascalCase`, matching the engine's command names where
  Rust allows it. `Type` is renamed explicitly because `type` is a keyword.
- `KeyDown`, `KeyUp`, `MouseDown`, and `MouseUp` are served, validate their
  arguments, and then fail closed. Holding a button across calls is stateful and
  this module is not, so a success would be unobservably wrong. They are served
  rather than omitted so the refusal is a structured reply naming the
  alternative, and so the names are already reserved for a stateful daemon.
- Session lifecycle, trace read and export, and the engine's bundled skills
  loader are out of contract version 1.0. Adding a member is a minor bump, which
  the bind rule in `tinycomputer_bus::version` permits.

### Jev control

- `ResolveIntent`, `RunGoal`, and `RunFlow` require TinyBus confidential
  delivery; `ValidateFlow` and `FlowGuide` touch neither the desktop nor Jev.
  Flows are specified in [`jev-intent-flows.md`](jev-intent-flows.md). Jev
  configuration arrives through sensitive module initialization or
  reinitialization; the module never returns, logs, or traces its API key.
- Jev chooses only from module-supplied operations and compatible refs. Text is
  caller-supplied, ordinary field values are withheld by default, and a
  destructive result stops for confirmation by default. A host can explicitly
  disable confirmations for a bounded, scoped `RunGoal` task. Local label/goal checks
  also force confirmation for delete, send, purchase, payment, submission,
  overwrite, unsafe quit, trash, and sign-out actions.
- Exact endpoint overrides are limited to the selected provider's published
  route. Accessibility content is labeled as untrusted data, and observation
  visits at most 4,096 nodes and 64 levels before returning a bounded view.
- Execution gates on the selected option's probability, not Jev's distribution
  concentration. Exact accessible names and explicitly requested first/topmost
  rows may add deterministic identity evidence but never bypass risk checks.
- A scoped goal binds an exact app and optional window ID/title, allowed operations and
  exact target labels, prepared named text, and all-of visible success predicates.
  A supplied window ID is included in every snapshot, and a missing or different
  window stops execution without falling back to a newly focused window.
  The module reobserves immediately before every mutation, checks operation and
  target probabilities separately, and verifies completion from the accessibility
  tree rather than accepting Jev's `DONE` alone. A task stops at 40 actions,
  80 evaluations, three unchanged turns, or its five-minute-capped elapsed budget.
  Uncertain mutations are never replayed. Absence is not a valid success predicate:
  the bounded snapshot cannot prove it.

### The envelope

- Both outcomes travel in `DesktopResponse`: `ok` selects between `data` and a
  structured `error`. A command failure is never a `TinyBus` error.
- A `TinyBus` error means a transport or dispatch failure — an unknown member,
  an undecodable frame — and nothing else.
- `DesktopError` carries the engine's own code, message, suggestion, recovery
  hint, platform detail, structured details, and delivery disposition. None of
  those may be flattened into the message.
- The envelope's wire form is byte-identical to the `agent-desktop` CLI's stdout
  envelope, version `2.5`, so a host needs one parser rather than two.

### The contract crate

- `tinycomputer-bus` depends on `serde` and `serde_json` and nothing else. It may
  not depend on a transport, an async runtime, an HTTP client, a native library,
  or the engine. CI asserts the resolved dependency tree.
- It mirrors the engine's enumerations by wire form rather than importing them,
  and those mirrors carry no `#[non_exhaustive]`, so the module crate's
  conversions are exhaustive and a variant added upstream fails the build.
- `tinycomputer` depends on it and re-exports all of it, so the two crates name
  the same types rather than structural twins.

### Permissions

- Each member declares what it needs: nothing, accessibility, screen recording,
  or both. `Screenshot` needs both only when it targets a named application or
  window, because that target is resolved through the tree.
- The need is checked against the platform's report before the command runs. An
  unpermitted accessibility call typically returns an empty tree rather than an
  error, which would otherwise surface as "the element is not there".
- A member needing nothing does not fetch a report.
- `Permissions` prompts only when its `request` field is set; `Status` and
  `Permissions` report a denied permission rather than refusing to run.

### Configuration and concurrency

- The module's configuration is a JSON object with optional `session_id` and
  `trace_path` strings, `trace_strict` and `headed` booleans, and a `jev`
  object containing provider configuration and its API key. `null` and `{}`
  yield desktop defaults with Jev disabled; invalid recognized fields fail
  before the served object is replaced.
- TinyBus sensitive initialization and reinitialization carry the `jev` object.
  Reinitialization constructs the complete replacement service before
  `serve_at`, so a rejected key, endpoint, or desktop field leaves the existing
  service intact.
- Commands run on a blocking thread pool, not on the connection's dispatch task,
  because a dense snapshot or a thirty-second wait would otherwise stall every
  other caller.

## Verification

- Every test passes on a machine with no display server, no granted permission,
  and nothing running.
- Payload types pin their serde representation; the conversions assert every
  enumeration variant by the engine's own spelling.
- The served interface is exercised over TinyBus's in-memory transport,
  including a member with a payload, a member without one, a member that fails
  closed, and an unknown member.
- `crates/tinycomputer-examples/src/bin/verify_module.rs` loads the compiled `cdylib` through the real
  dynamic loader and calls `Version` before a release archive is accepted.
- The generated dispatch table and the embedded module manifest are both
  asserted against `tinycomputer_bus::names::METHODS`.
