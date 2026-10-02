# tinycomputer TinyBus Module

tinycomputer is a decision model (Jev) based harness for desktop and browser
automation, written in Rust. This package contains the native `tinycomputer` module for TinyBus module ABI
v1. Install only the archive matching the host operating system and
architecture.

The module claims `ai.tinyhumans.tinycomputer.Desktop`, serves the object at
`/ai/tinyhumans/tinycomputer/Desktop`, and provides eighty members:

- 54 desktop primitives: accessibility-tree observation, ref-addressed
  interaction, synthesized keyboard and mouse input, application and window
  management, the clipboard, notifications, waits, and status;
- 5 Jev-driven members: `ResolveIntent`, `RunGoal`, `RunFlow`, `ValidateFlow`,
  and `FlowGuide`;
- 8 task members for outside agents: `Describe`, `PlanTask`, `StartTask`,
  `AwaitTask`, `ContinueTask`, `CancelTask`, `TaskReport`, and `ListTasks`.
  Tasks run across desktop applications and a Chrome browser, pause for
  missing details and approvals, and always stop at payment;
- 13 browser primitives, each prefixed `Browser`: `BrowserOpenSession`,
  `BrowserNavigate`, `BrowserSnapshot`, `BrowserPerform`, `BrowserReadPage`,
  `BrowserEvaluate`, `BrowserScreenshot`, `BrowserReadOutput`, and the
  session, output, and download members around them. They share one browser
  with the tasks, so a task's session and screenshots are reachable too.

Every member takes one request payload, or none, and returns a structured
reply carrying either the data or an error with its code, suggestion, and
recovery hint. Desktop and browser failures share one code vocabulary
(`STALE_REF`, `ELEMENT_NOT_FOUND`, `TIMEOUT`, `POLICY_DENIED`, …), so one
handler serves both. `Describe` returns a catalogue of every member with its
family and a one-line summary, and names the configured decision model
(`decision_model`) and the planner's, rescuer's, and shaper's routes and
models (`planner_model`, `rescue_model`, `output_model`). All payload types, the interface name, the object path, and the
member names are published as the `tinycomputer-bus` crate, so a host names
them from a library rather than by string literal.

The module reads its configuration from the loader as a JSON object. Every key
is optional:

- `session_id` and `trace_path` (strings), `trace_strict` and `headed`
  (booleans) for the desktop engine;
- `jev`: the decision model and its API key, needed by the Jev-driven and
  task members. `provider` picks it: Jev through `type_safe` (the default),
  `open_router`, or `tiny_humans_open_router`; `open_jev` (OpenJEV's
  Jev-compatible API, model `openjev`); `sage` (Levanto Sage in place of
  Jev, with an optional `fast`); or `self_hosted` (an operator-declared
  Jev-compatible decisions endpoint, where both `endpoint_url` and `model`
  are required). `endpoint_url` may only repeat the
  provider's own approved route, except for `self_hosted`, where it names it;
- `cursor`: the agent's on-screen cursor, shared by the desktop and the
  browser — a pace (`off`, `brisk`, `natural` (the default), `calm`), or an
  object with an optional `pace` and an optional `overlay` path to the
  `tinycomputer-cursor-overlay` helper shipped beside the module;
- `planner`: an `api_key` and optional `model`, which lets `StartTask`
  accept a plain-language task, and an optional `rescue_model` (default
  `openai/gpt-6-luna`) that a task's failed step is handed to for guidance,
  up to five times, before the task fails, and an optional `output_model`
  (default `openai/gpt-6-luna`) that shapes a finished task's answer when
  `StartTask` asks for an `output`. `provider` routes all three through
  `open_router` (the default, an OpenRouter key) or `tiny_humans` (Tiny
  Humans' OpenAI-compatible gateway, the host's TinyHumans bearer, with an
  optional `sdk_name`); `endpoint_url` may only repeat that route's approved
  base URL; and `rescue_route` (`api_key`, `provider`, `endpoint_url`,
  `sdk_name`) gives the rescuer a route and key of its own;
- `browser`: how every browser launches — `executable` (the Chrome or
  Chromium binary), `user_agent`, `args` (an array of launch arguments), and
  `perception` (`sight`, the default, or `tree`: how a task reads a page).

Configuration is delivered as sensitive host-control traffic and is never
shown to monitors.

## Permissions

Desktop automation needs permissions a person grants: accessibility access for
anything that reads or drives another application's tree, and screen recording
for captures. The module checks before it acts, so a missing permission is a
`PERM_DENIED` naming the setting to change rather than an empty tree that looks
like an application with no buttons. Call `Permissions` to read the current
state; it prompts only when explicitly asked to.

macOS and Windows have full accessibility backends. Linux loads and answers but
implements no desktop surfaces yet: observation there fails with
`PLATFORM_NOT_SUPPORTED`. Browser tasks need Chrome or Chromium on the machine,
or a running Chrome to attach to.

## Installing

The archive contains one `.so`, `.dylib`, or `.dll` plus `modules.toml`. Keep
those files together when copying them into a TinyBus module directory. The
allowlist binds the native library filename to its SHA-256 digest so TinyBus can
reject a missing, renamed, or modified artifact before initialization.

The GitHub release also publishes `checksum.toml` as a separate asset. TinyBus
checks that manifest before downloading and extracting the selected platform
archive. Install directly from a tagged release with:

```sh
tinybus modules load-github \
  https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.2.1 \
  tinycomputer-0.2.1-ubuntu-24.04-x86_64.tar.gz \
  <archive-sha256>
```

TinyBus modules are trusted in-process code, and this one can read any window on
the machine and drive any application on it. Install release artifacts only from
a trusted source and restart the host after replacing a loaded module.
