# TinyBus Adapter

This module is the boundary between the engines and TinyBus module ABI v1.
`DesktopService` exposes each of `Desktop`'s methods, the task controller, and
each of `Browser`'s methods as a typed bus member, and `setup` registers its
object and claims the well-known interface name. Neither the name, the object path, nor the payload types are spelled here:
they come from `tinycomputer-bus`, so a rename is a compile error in every
consumer instead of an `UnknownMethod` at runtime.

## Why the members are written out

`dispatch/mod.rs` is eighty near-identical `async fn`s, in one `impl` block
(building the service lives in `dispatch/service.rs`, the browser members'
shared helpers in `dispatch/browser.rs`). They cannot be generated
by a `macro_rules!` inside the `impl` block: `#[tinybus::interface]` reads that
block's items to build its dispatch table, and a macro invocation there is still
unexpanded when the attribute runs. Writing them out is what lets the macro see
them.

The order they appear in is the order of `tinycomputer_bus::names::METHODS`, and
`tinybus_module_tests/manifest_tests.rs` asserts the generated dispatch table
against that list.

## Why every member blocks elsewhere

Accessibility APIs are synchronous, and some calls are slow on purpose — a dense
snapshot walks thousands of elements, a wait blocks for up to thirty seconds.
Running one on the connection's dispatch task would stall every other caller for
that whole time, so each member hands its work to `tokio::task::spawn_blocking`.

That is affordable because `Desktop` is four small fields and constructs its
platform adapter per call: nothing platform-specific has to cross a thread
boundary or survive an `await`. The module asks for two worker threads for the
same reason — one to run a blocking command on, one to keep answering on.

## The browser members

The thirteen `Browser…` members await the module's one `Browser` directly —
its calls are async, and serialized per session inside it — except the two
held-output calls, which base64-encode up to four mebibytes and so run on the
blocking pool. The same `Browser` is handed to `WorkspaceRunner`, so a task's
session appears in `BrowserListSessions` and a screenshot in a task's
`TaskReport.artifacts` is readable with `BrowserReadOutput`. `browser_reply` wraps each result in
the `DesktopResponse` envelope, the error converted by
`tinycomputer_browser::Error::envelope`. `BrowserOpenSession` takes the
configured `browser` settings (`browser_defaults.rs`) wherever the request
left them unset; an attached session takes no executable or launch
arguments.

`tinybus_module_tests/browser_tests.rs` drives them over the in-memory bus
against a scripted engine, so no browser is launched.

## Configuration

The host-visible loader fields are decoded as
`tinycomputer_bus::config::ComputerConfig`: optional `jev`, `planner`,
`browser`, and `trace_path`. The planner's route types are the same public
types used by the engine. The module adapter converts `trace_path` into
`Desktop` settings and applies the typed browser defaults: `browser` sets how
every browser launches — `executable`, the kept `profile`, `user_agent`, `args`, a task's page
`perception`, how a page `settle`s after an action, and whether to `prelaunch`
a planned browser task's browser.

Setting `trace_sink` to an object with the unique `destination` supplied by
`ModuleHost::prepare_host_callbacks` opts into the host's confidential
`TraceSinkHost` interface for ordered desktop trace events, trace artifacts,
and requested task reports; selected sink mode never writes local trace files.
Artifact bytes use base64 and are capped at 8 MiB decoded so the callback stays
below TinyBus's frame limit. Leaving it absent or `false` retains the existing
`trace_path` and session behavior. `true` without a callback destination is
rejected.

Module-only legacy options remain outside this host contract: `session_id`,
`trace_strict`, and `headed` still configure `Desktop`, while `cursor` sets the
agent's on-screen cursor (a pace of `off`, `brisk`, `natural`, or `calm`, or
`{pace, overlay}`). The module keeps accepting these fields from existing
loader configurations. An unreadable option fails before replacing the served
object rather than falling back to defaults. TinyBus treats initial and
replacement module configuration as sensitive host-control traffic.

Setting `consequence_host` to an object with the unique `destination` supplied
by `ModuleHost::prepare_host_callbacks` enables confidential consequence
reviews. When `ContinueTask` approves a pending action for host review, the
module declines it if the configured callback is absent or cannot answer. A
declined approval, an unanswered pause, or a continuation with no pending
action does not fail solely because this callback is absent. Without host
context, the legacy well-known host interface remains available.

## The manifest

`tinybus_module::module_export_optional_static!` emits (C exports by default; `linked_module()` under `static-link`) the descriptor, embedded manifest, and
initialization symbols consumed by the dynamic loader. The manifest method list
must stay aligned with the interface macro's dispatch table and with
`tinycomputer_bus::names::METHODS`; the unit tests check both relationships. The
manifest side is checked by reading the literals `module_export!` was handed out
of this module's own source, because reading them back out of the exported
`extern "C"` function would need `unsafe`, which this workspace forbids.

Integration tests use TinyBus's in-memory transport, and
`crates/tinycomputer-examples/src/bin/verify_module.rs` loads a compiled `cdylib` through
the real dynamic loader before a release archive is accepted. Its trace-sink
fixture invokes the no-permission `Version` member, checks captured native trace
records and confirms sink mode leaves the configured legacy trace path absent;
it also verifies successful and failed confidential task-report writes.

## Native accessibility lifecycle

Ten native members delegate to the accessibility implementation on blocking
threads, with module-owned listener leases. No device or helper behavior runs
in the host. See the [contract](../../../tinycomputer-bus/src/accessibility/README.md).

GlobeRead retains a bounded typed batch until acknowledgment. Native overflow or
legacy destructive polling marks continuity loss, requiring activation reset.
GlobeShutdown rejects new listener acquisition and awaits native child/pipe
cleanup before returning. Stop failures retain ownership for retry; terminal
shutdown joins native listeners and the trace sink worker before generic ABI
unload.
