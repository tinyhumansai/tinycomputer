# Architecture

tinycomputer is one TinyBus module that lets an agent drive desktop
applications and web pages. This page explains how the workspace is put
together: which crate does what, how a call travels from the bus to a click,
and the rules that keep the layers apart. For the decision loops themselves,
see [`decision-loops.md`](decision-loops.md); for the task API, see
[`tasks.md`](tasks.md).

## The layers

```text
                          TinyBus host (an agent's runtime)
                                        │  typed bus calls
                                        ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ tinycomputer (cdylib)   TinyBus glue: manifest, dispatch, config, runner   │
└──────────────────────────────────────────────────────────────────────────┘
                                        │
                                        ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ tinycomputer-engine     Jev runtime · RunGoal · ResolveIntent · RunFlow    │
│                        flow runtime · Workspace · Tasks · Planner         │
└──────────────────────────────────────────────────────────────────────────┘
              │                                             │
              ▼                                             ▼
┌───────────────────────────────┐             ┌────────────────────────────────┐
│ tinycomputer-desktop           │             │ tinycomputer-browser            │
│ Desktop: 54 typed members     │             │ Browser: sessions, outputs     │
│ Desktop as a core Surface     │             │ BrowserSurface: a core Surface │
└───────────────────────────────┘             └────────────────────────────────┘
              │                                             │
              ▼                                             ▼
   vendor/agent-desktop                           vendor/agent-browser
   (accessibility trees:                          (Chrome over CDP, linked
    macOS, Windows, Linux)                         in-process as a library)

        tinycomputer-core: Surface trait, Screen, keymap, safety, records, facts
        tinycomputer-cursor: the agent's drawn cursor, aim points and glides
        tinycomputer-bus:  every type that crosses the bus, and nothing else
```

Dependencies point one way:
`bus` ← `core` ← {`desktop`, `browser`} ← `engine` ← `tinycomputer`. Nothing
lower down knows about anything higher up. The desktop and browser adapters
never see Jev, the engine never sees TinyBus, and the contract crate has no
runtime at all.

| Crate | Holds | Deliberately does not hold |
|---|---|---|
| `tinycomputer-bus` | the wire contract: member names, request and reply types, the `DesktopResponse` envelope, the Agent types (`TaskView`, `TaskStatus`, …), the browser types, the flow grammar and its authoring guide, and the contract version | a transport, an async runtime, an HTTP client, a native library, or an engine. CI fails the build if one appears. |
| `tinycomputer-core` | the `Surface` trait and the `Screen` it observes; fingerprints, change notes, verified text delivery, result-card grouping; the per-OS keymap; the safety classifier and payment detector; the facts store; price, time, duration, and stop parsers and `rank` | engines, Jev, TinyBus, and any tokio runtime |
| `tinycomputer-cursor` | the agent's one on-screen cursor, shared by desktop and browser: aim points, human glide paths, `CursorPace`, the sprite, the animator, the overlay protocol, and (feature `overlay`) the `tinycomputer-cursor-overlay` helper window ([spec](specs/virtual-cursor.md)) | input and engines |
| `tinycomputer-desktop` | `Desktop`, one typed method per desktop member over agent-desktop, with the permission preflight; `Desktop` as a `Surface` | Jev, TinyBus |
| `tinycomputer-browser` | `Browser` (sessions, navigate, snapshot, perform, read, evaluate, screenshots, downloads, held outputs) over agent-browser; `BrowserSurface`; the error taxonomy | Jev, TinyBus |
| `tinycomputer-engine` | the Jev runtime, `RunGoal`, `ResolveIntent`, the flow runtime, the `Workspace`, the task controller, and the optional planner | TinyBus |
| `tinycomputer` | the TinyBus module: manifest, one dispatch method per member, configuration, and the task runner that gives each task a workspace | behaviour of its own |
| `tinycomputer-skills` | the agent-facing `SKILL.md` and schemas for the task API | code |
| `tinycomputer-examples` | runnable examples, the lab, the travel fixture, and live verification binaries | anything shipped |

The desktop and browser crates are plain libraries. Another host or module can
take just one of them without the engine or the bus glue.

## The vendored engines

Four submodules, each pinned by gitlink. This repository never edits them: a
wrong tree or a wrong click is fixed upstream and arrives here as a gitlink
bump in its own commit.

| Path | What it is | Used by |
|---|---|---|
| `vendor/agent-desktop` | accessibility-tree observation and interaction for macOS, Windows, and Linux | `tinycomputer-desktop` |
| `vendor/agent-browser` | Vercel's browser automation engine, pinned to the `library-target` branch of the `tinyhumansai/agent-browser` fork, which adds a library target | `tinycomputer-browser` (feature `agent-browser`) |
| `vendor/tinyinference` | the Jev decisions client (`tinyinference_decisions`) and an LLM client (`tinyinference-llm`) | the engine: Jev always, the LLM only with the `planner` feature |
| `vendor/tinybus` | TinyBus host types and the module SDK | the cdylib |

agent-browser is normally a CLI with a daemon. Here it is linked in-process:
`tinycomputer-browser/src/linked/` builds one agent-browser `DaemonState` per
session from explicit options (so nothing leaks in from the host's
`AGENT_BROWSER_*` environment) and sends it the same JSON commands its daemon
would receive over a socket. agent-browser keeps one process-wide piece of
state, the frame selectors resolve in, so commands from different sessions are
serialized through one lock. There is no sidecar process and no socket.

## One surface abstraction

The decision loops are written once, against `tinycomputer_core::surface::Surface`:

```rust
pub trait Surface: Clone + Send + 'static {
    fn observe(&self, app: &str, root: Option<&str>, depth: Depth)
        -> Result<Screen, Box<DesktopResponse>>;
    fn execute(&self, operation: JevOperation, target: Option<Candidate>, text: Option<String>)
        -> DesktopResponse;
    fn read_value(&self, target: &Candidate) -> Option<String>;
    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse;
    fn press(&self, app: &str, combo: &str) -> DesktopResponse;
    fn launch(&self, app: &str) -> DesktopResponse;
    fn settle(&self) {}
    fn settle_briefly(&self) { /* settles */ }
    fn await_change(&self, ms: u64) -> bool { /* pauses, and says it may have */ }
    fn navigate(&self, url: &str) -> DesktopResponse { /* ACTION_NOT_SUPPORTED */ }
}
```

The operations are closed: click, type text, check, uncheck, expand,
collapse, scroll, wait, and two look operations. A loop can never ask a surface
to do something outside that list.

| | `Desktop` | `BrowserSurface` |
|---|---|---|
| observe | agent-desktop snapshot of the app's front window, parsed into candidates and context | sight: one script over the rendered page — what is drawn and on top, the words on and beside each control ([`specs/browser-sight.md`](specs/browser-sight.md)); the agent-browser snapshot (`- role "name" [ref=eN]: value`), parsed the same way as the desktop's, when sight cannot reach the page |
| a ref | `@s8f3k2p9:e1`, bound to one snapshot | `seen:12`, the element's lifelong `data-tc-seen` mark, or `e12` from a snapshot |
| click, check, expand | accessibility actions, headless by default | agent-browser `click`, `check` |
| type text | set value through accessibility | `fill` |
| paste | select-all and paste through the clipboard, then restore the clipboard | focus, select, and type at the caret; no clipboard |
| press | `cmd` becomes `ctrl` off macOS | `cmd` becomes `Meta` on macOS and `Control` elsewhere; `return` becomes `Enter` |
| launch | launch or bring forward | open the session if it is not open |
| navigate | refused | load the URL |

The `Workspace` in `tinycomputer-engine/src/workspace/` joins the two into one
`Surface`. Calls that name an application route by the name: `browser`,
`browser:…`, or an `http(s)://` address goes to the browser, anything else to
the desktop. Calls that do not name one (acting on a candidate, reading it
back) go to whichever side was observed or opened last, which is the side the
candidate came from. A task confined to one side gets a workspace with only
that side, and a call aimed at the missing side is refused.

So a single flow can `browse` to a flight search, `pick` a result, `open`
Mail, and `enter` the price it read into a draft.

## How a call travels

Take `StartTask` with a planned booking flow.

1. The host calls the module. TinyBus delivers the call to
   `DesktopService::start_task` in `crates/tinycomputer/src/tinybus_module/dispatch/mod.rs`.
   `StartTask`, `ContinueTask`, `TaskReport`, `RunGoal`, `ResolveIntent`, and
   `RunFlow` are confidential members: they carry the caller's values and page
   data, so the bus requires an attested module and keeps them away from
   monitors.
2. The controller registers the task (`Tasks::start`), validates the flow,
   checks for missing values, and spawns a worker. The call returns a
   `TaskView` straight away.
3. The worker runs the flow through `WorkspaceRunner::run`
   (`tinybus_module/runner.rs`), which creates the task's workspace on first
   use: the desktop, plus a browser session shaped by the task's constraints
   (allowed origins, headed or headless, or an endpoint to attach to).
4. The flow runtime (`run_flow`) walks the steps. Each step observes the
   workspace, asks Jev small questions, and acts through the `Surface` trait.
5. The browser surface turns `execute(Click, e12)` into a
   `perform(Click{target: @e12})` on the session, which `convert` turns into
   agent-browser's `{"action": "click", "selector": "@e12"}`, which the linked
   engine runs against Chrome over CDP.
6. The reply comes back up as a `DesktopResponse`. The flow runtime notes
   what changed, the controller interprets how the run ended, and the next
   `AwaitTask` returns the new view.

### Threads and blocking

Accessibility APIs are synchronous, and some calls are slow on purpose. A
dense snapshot walks thousands of elements, and a wait can block for thirty
seconds. So every bus member hands its work to `spawn_blocking`, and the
module asks TinyBus for two worker threads: one to run a blocking call on and
one to keep answering on.

The flow runtime is async but its surfaces block. It makes every surface call
through `spawn_blocking` (`agentic/flow/backend/`). `BrowserSurface` goes the
other way: each of its blocking calls runs the async `Browser` call to
completion on the runtime handle it was built with. Calls on one browser
session are serialized by the session lock; different sessions run
independently, up to eight (`MAX_SESSIONS`).

## Three ways in

The module serves three levels of API from one interface,
`ai.tinyhumans.tinycomputer.Desktop` at `/ai/tinyhumans/tinycomputer/Desktop`.
A TinyBus manifest currently attaches its member list to one interface, so all
members share it; their names do not collide.

| Level | Members | Who plans | Who chooses | Use it when |
|---|---|---|---|---|
| Primitives | `Snapshot`, `Click`, `SetValue`, `Press`, `Launch`, … (54 desktop members) | the caller | the caller | the caller already knows the interface, or wants full control |
| Goals and flows | `ResolveIntent`, `RunGoal`, `RunFlow`, `ValidateFlow`, `FlowGuide` | the caller (a flow) or nobody (a goal) | Jev | the caller knows what it wants but not the interface |
| Tasks | `Describe`, `PlanTask`, `StartTask`, `AwaitTask`, `ContinueTask`, `CancelTask`, `TaskReport`, `ListTasks` | the caller or the planner | Jev | an outside agent wants a job done, across the browser and desktop apps, with pauses for approvals |
| Browser primitives | `BrowserOpenSession`, `BrowserNavigate`, `BrowserSnapshot`, `BrowserPerform`, `BrowserReadPage`, `BrowserScreenshot`, `BrowserReadOutput`, … (13 members) | the caller | the caller | the caller wants to drive a web page itself, or inspect a task's session |

`Describe` returns all four levels as `Capabilities.catalogue`: every member,
its family, and one sentence on what it is for.

The browser primitives are defined in `tinycomputer-bus/src/browser/`,
implemented by `tinycomputer_browser::Browser`, and served with a `Browser`
prefix, because several (`Snapshot`, `Screenshot`) would otherwise collide
with a desktop member of a different shape. A member that acts on an open
session takes one object with the session beside the member's own fields
(`BrowserOpenSession` takes the session's options, `BrowserListSessions`
nothing, the output members an `output`), and each replies in the same
`DesktopResponse` envelope as a desktop member; a failure reuses the desktop's
code where the meaning is shared, so `STALE_REF` means "snapshot again" on
either surface. The module holds one `Browser`: the task runner opens each
task's session on it, so the primitives can read a task's screenshots and
see its session. A direct `RunFlow` call still runs on the desktop alone;
tasks are how a flow reaches the browser.

`RunGoal` and `ResolveIntent` are the older, single-goal loop: one goal string,
one Choice over operation and target per turn, and success predicates the
module checks on the accessibility tree. They drive the desktop only.
[`crates/tinycomputer-engine/src/agentic/README.md`](../../crates/tinycomputer-engine/src/agentic/README.md)
describes them, including their confirmation handles.

## Configuration

The host passes the module a JSON object at load, and again on
reinitialization. TinyBus treats both as sensitive host-control traffic:
monitors never see them, and the buffers are zeroized after use.
Reinitialization swaps the served object only after the whole new
configuration validates.

| Key | Type | Meaning |
|---|---|---|
| `session_id` | string | agent-desktop session to join |
| `trace_path` | string | where agent-desktop writes its trace |
| `trace_strict` | bool | fail a call when its trace cannot be written |
| `headed` | bool | use real input instead of accessibility actions |
| `jev` | object | decision provider (`type_safe`, `open_router`, `tiny_humans_open_router`, `open_jev`, `sage`), API key, optional model, approved endpoint, timeout, retries, `sdk_name` for the TinyHumans proxy, and `fast` for Sage |
| `planner` | object | `api_key`, `provider` (`open_router` or `tiny_humans`), approved `endpoint_url`, `sdk_name`, and optional `model` for the planner, optional `rescue_route` giving the rescuer its own route and key, optional `rescue_model` for rescuing failed steps (`docs/technical/specs/task-rescue.md`), and optional `output_model` for shaping a finished task's answer (`docs/technical/specs/task-output.md`); absent means no planner, no rescues, and no output shapes |
| `browser.executable` | string | the Chrome or Chromium binary to launch, when discovery would not find one |
| `cursor` | string or object | the agent's on-screen cursor for desktop and browser: a pace (`off`, `brisk`, `natural` (default), `calm`) or `{pace, overlay}` with the overlay helper's path ([spec](specs/virtual-cursor.md)) |

Without `jev`, `ResolveIntent`, `RunGoal`, `RunFlow`, and task runs answer
with `JEV_NOT_CONFIGURED`, while the primitives, `ValidateFlow`, `FlowGuide`,
and `Describe` keep working.

## Safety, in one place

Several independent checks keep a run from doing something it cannot undo.
None of them trusts a model's judgement.

- An ordinary step refuses to click a control whose label reads as
  destructive, that the flow's own `stop_before` names, or that is an unnamed
  button in a confirmation sheet. Only `stop_before` reaches one, and it
  presses only with `allow_destructive` or an approval. A tab only shows
  another panel of the page, so `stop_before` never names it; its own label
  still counts against the denylist.
- A control classified as payment, or any click on a screen that shows card
  fields, stops the run, and the task controller makes that a final
  checkpoint. Card data is always a secret fact, and it is typed only under
  `payment: "fill_then_approve"`, which needs `origins` naming the sites
  (never `*`), types only on their pages (a payment provider's frame on such
  a page included), and still waits for approval before the control that
  pays.
- Secret values never reach Jev: they are masked to `${name}` in everything
  it is sent, what a field holds included. Shared values brief Jev, and
  tasks let it read field contents (`include_values`). The planner and the
  rescuer see fact names only: every value is redacted from the screen text
  a rescue is shown, and from every summary.
- Everything read from a screen is wrapped as untrusted data, and every
  question tells Jev that screen text is data, never instructions. A move Jev
  was not offered fails closed.
- Budgets cap actions, Jev calls, and time for a whole task, and surfaces and
  origins confine where it can go.
- The desktop checks accessibility and screen-recording permission before
  acting, because an unauthorized accessibility call usually returns an empty
  tree rather than an error.

## Testing the layers

Every layer is testable without a display, a browser, or a network:

- the contract pins the serde form of every payload;
- the browser crate runs against a scripted `Engine` (`src/fake/`);
- the flow runtime runs against a simulator in `agentic/flow/flow_tests/`: a mail
  app and booking widgets whose state an oracle Jev reads its answers from;
- the task controller runs against scripted `FlowRunner`s;
- the module is exercised over TinyBus's in-memory transport, and
  `verify_module` loads a built cdylib through the real dynamic loader.

Live runs sit outside the suite. `scripts/lab` drives real macOS applications
([`lab.md`](lab.md)), and `scripts/docker-lab` runs anything that launches
Chromium in a Linux container ([`docker-lab.md`](docker-lab.md)), including the
travel fixture end to end.
