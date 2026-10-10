# tinycomputer-bus

Part of [tinycomputer](../../README.md), a decision model (Jev) based harness for
desktop and browser automation, written in Rust. This crate is its wire contract: what a host sends and gets back. Its user guide is
[`docs/crates/tinycomputer-bus/`](../../docs/crates/tinycomputer-bus/README.md).

Every type that crosses the tinycomputer module's `TinyBus` boundary, and the
names of the members that carry them.

tinycomputer ships as a loadable module so a host does not compile the
implementation: `crates/tinycomputer` is built as a `cdylib` and exports one
object. A host can load that binary but cannot `use` anything out of it, so the
payload vocabulary has to be published as an ordinary library. This is it.

| module          | what it holds                                                      |
| --------------- | ------------------------------------------------------------------ |
| `names`         | interface name, object path, one constant per member — 88 of them   |
| `catalogue`     | every member's family, one-line summary, and confidentiality        |
| `envelope`      | `DesktopResponse` and the structured `DesktopError` it carries      |
| `vocabulary`    | surfaces, modifiers, buttons, element properties                    |
| `observation`   | `Snapshot`, `Find`, `Get`, `Is`, `Screenshot` payloads              |
| `interaction`   | the ref-addressed action payloads                                   |
| `input`         | synthesized keyboard and mouse payloads                             |
| `apps`          | application, window, and display payloads                           |
| `clipboard`     | pasteboard payloads                                                 |
| `notifications` | notification-centre payloads                                        |
| `waiting`       | the `Wait` payload                                                  |
| `system`        | the `Permissions` payload                                           |
| `agent`         | the task members' requests, views, reports, and `Capabilities`      |
| `flow`          | the flow grammar `RunFlow` and `StartTask` carry                    |
| `browser`       | the `Browser…` members' payloads, names, and error codes            |
| `version`       | `CONTRACT_VERSION` and the bind rule a host applies to it           |

Two dependencies, both pure Rust: `serde` and `serde_json`.

## This crate sits underneath `tinycomputer`

`tinycomputer` **depends on this crate and re-exports all of it**. That direction
matters, and it is the opposite of the obvious one.

A *host* needs the payload types and needs nothing else: it loads the module and
makes calls, so it names `SnapshotRequest` and `DesktopResponse` but implements
no behavior and links no transport. Making it depend on the module crate — and
through it on `tinybus`, `tokio`, the module SDK, and a platform accessibility
backend — to spell a payload type would be the wrong shape.

The alternative, a parallel set of payload types for hosts, is worse: a
`SnapshotRequest` defined twice is two distinct types, with a conversion at every
call site that nothing checks. One definition, here, at the bottom.

Because the re-export is by module as well as by item,
`tinycomputer::SnapshotRequest`, `tinycomputer::names::OBJECT_PATH`, and
`tinycomputer_bus::observation::SnapshotRequest` all resolve to the same items,
not twins.

So: a module author depends on `tinycomputer` and gets behavior and vocabulary. A
host depends on `tinycomputer-bus` and gets vocabulary alone.

## What is deliberately absent

**No behavior, and no engine.** The implementation lives in
`crates/tinycomputer`, which wraps the vendored `agent-desktop` engine. A payload
type describes what a frame carries, not what the module does with it — and
pulling the engine in here would drag a platform accessibility backend into
every host that only wanted to name a member.

**No transport.** This crate does not depend on `tinybus` and holds no
connection, client, or codec. A host already owns its connection — its reconnect
policy, its timeouts, its tracing — and the useful part is the vocabulary.

That is also structural, not just preference: `tinybus` is vendored as a
submodule whose manifest inherits fields from its own nested
`[workspace.package]`. Keeping the contract crate transport-free is what keeps
it down to two dependencies and what lets anything in the workspace — or outside
it — depend on it freely. CI asserts the dependency tree stays that way.

## Making a call

Arguments travel as a positional JSON array — `#[tinybus::interface]` decodes
them into a tuple — and the member name comes from `names`:

```rust,ignore
use tinycomputer_bus::{names, DesktopResponse, RefRequest, SnapshotRequest};

let proxy = connection.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

let tree: DesktopResponse = proxy
    .call(names::methods::SNAPSHOT, (SnapshotRequest {
        app: Some("Safari".to_owned()),
        skeleton: true,
        ..Default::default()
    },))
    .await?;

// Pick a ref out of `tree.data`, then spend it.
let clicked: DesktopResponse = proxy
    .call(names::methods::CLICK, (RefRequest::new("@s8f3k2p9:e1"),))
    .await?;
```

Nothing above is a string literal at a call site. Renaming the interface, the
path, or a member is therefore a compile error in every consumer rather than an
`UnknownMethod` discovered at runtime.

## One envelope, both outcomes

Every member returns a `DesktopResponse`, on success and on failure alike. A
desktop command fails in ways a caller has to *act* on — a ref went stale and
the caller must re-snapshot, a permission is missing and the caller must prompt
for it — and folding those into a `TinyBus` error would reduce each one to a
string. The envelope carries the code, the suggestion, the recovery hint, and
whether a retry is safe.

A `TinyBus` error stays reserved for a genuine transport or dispatch failure: an
unknown member, an undecodable frame. Different kind of problem, different
channel.

Its wire form is `agent-desktop`'s own, byte for byte, so a host that already
parses that CLI's stdout needs no second parser.

## Staying in step with the module

`RunGoal` stops before a consequential action and returns a one-use
`confirmation_id`. A host resumes through `RunGoalRequest.continuation` with
that ID and an explicit approval or refusal. The module reobserves the target
before an approved action; stale or ambiguous targets are refused.
For a host-managed task whose desktop approvals are disabled, set
`require_confirmations: false` and provide `success` predicates. The module
then returns `verified` and compact `final_observation` evidence from the
last accessibility snapshot. The default remains confirmation for legacy
callers. `RunGoalRequest.window_id` can bind every observation to one ID from
`ListWindows`, while `window` checks the reported title.
`success` also accepts `{"kind":"name_contains","fragment":"...","within":"..."}`:
the fragment must occur in one node's accessible name beneath an ancestor
whose accessible name exactly matches `within`. This is useful when a visible
result adds timestamps or delivery status to a stable text fragment.

`names::METHODS` lists every member in dispatch order. `crates/tinycomputer`
asserts both its generated dispatch table and its embedded module manifest
against that list, so a method added to the interface without an entry here
fails that crate's tests rather than surfacing in a host.

## Versioning

`CONTRACT_VERSION` describes *this vocabulary*, not the package. Bump its major
component when a payload's wire form changes incompatibly or a member is removed
or renamed, and its minor component when a member or an optional field is added.
It is deliberately independent of the package version the release workflow owns,
which tracks the shipped artifact.

The payload tests pin the serde representation, because that representation is
the wire form: a host and a module that disagree about a field name fail at
runtime with a decode error, so the shape is asserted rather than assumed.

Contract 2.11 adds the eight native accessibility members. Their serialized
vocabulary and opaque Globe listener handles live in `accessibility`; native
execution remains inside the compiled module. Focus, target validation and paste
require confidential calls. Hosts must pin a released 2.11-compatible artifact
and verify its digest before adopting this surface.
