# Members, by family

The module serves 80 members on one interface,
`ai.tinyhumans.tinycomputer.Desktop`, in the order
`crates/tinycomputer_bus::names::METHODS` declares them, which is also the
order they are dispatched in
`crates/tinycomputer/src/tinybus_module/dispatch/mod.rs`. A test in
`tinybus_module_tests/manifest_tests.rs` asserts the two stay in lockstep,
so this list cannot silently drift from the code.

Every member here except the eight task members (their own section below)
takes at most one request payload and always answers with a `DesktopResponse`
whose `ok` flag picks between `data` and a structured `error`. That includes
the 13 browser members: those acting on an open session take one object, the
session beside the member's own fields (`BrowserListSessions` takes nothing),
and all answer in the same envelope, reusing a desktop error code
wherever the meaning is shared. The module never answers with a bare
`TinyBus` transport error for something the caller did; that is reserved for
the module failing to even start the command, which is rare enough that you
should treat it as a bug report if you see one. See
[calling-it.md](calling-it.md) for what that envelope actually looks like on
the wire.

## Observation (5)

`Snapshot`, `Find`, `Get`, `Is`, `Screenshot`

These read the current state of an application without changing anything.
`Snapshot` is the big one: it walks an application's accessibility tree and
hands back a compact description where every element carries a *ref*, a
qualified handle like `@s8f3k2p9:e1`. Every interaction member below takes a
ref from a snapshot, never raw coordinates and never a selector re-evaluated
at click time, so an action either reaches the exact element that was
described or fails closed with `STALE_REF` asking for a fresh snapshot. `Find`
is a filtered, narrower `Snapshot`. `Get` reads one property of one ref, `Is`
tests one boolean state of one ref, and `Screenshot` captures an application,
window, or display as an image.

## Interaction (16)

`Click`, `DoubleClick`, `TripleClick`, `RightClick`, `Type`, `SetValue`,
`Clear`, `Focus`, `Select`, `Toggle`, `Check`, `Uncheck`, `Expand`,
`Collapse`, `Scroll`, `ScrollTo`

Ref-addressed actions: every one of these takes the ref of an element a
snapshot already described. They go through the platform's accessibility
API, not synthesized input, which is what lets them run without stealing
focus, moving the visible mouse cursor, or touching the system clipboard as a
side effect, so a run can proceed on a machine someone else is actively
using. `Desktop::with_headed` relaxes that for interactions that specifically
need a real cursor to work, but that is the exception, and the input members
below are the other, deliberate exception.

## Input (10)

`Press`, `KeyDown`, `KeyUp`, `Hover`, `Drag`, `MouseMove`, `MouseClick`,
`MouseDown`, `MouseUp`, `MouseWheel`

Synthesized keyboard and mouse input, for the cases a ref-addressed action
cannot cover: a keyboard shortcut with no element to target, a drag between
two points, a hover that needs to actually move the cursor. Unlike the
interaction family, these do move the real cursor and can steal focus, on
purpose.

`KeyDown`, `KeyUp`, `MouseDown`, and `MouseUp` are a special case worth
knowing about: they validate their arguments and then always fail closed with
a structured reply naming the alternative. Holding a key or button down across
separate calls would be stateful, and this module keeps no state between
calls, so a success reply here would be silently wrong in a way nothing could
catch. They are served, rather than removed from the interface entirely, so
the refusal comes back as a normal structured error a caller can read and act
on, and so the names stay reserved against the day a stateful daemon variant
exists.

## Application and window management (12)

`Launch`, `CloseApp`, `ListApps`, `ListWindows`, `ListDisplays`,
`ListSurfaces`, `FocusWindow`, `ResizeWindow`, `MoveWindow`, `Minimize`,
`Maximize`, `Restore`

Starting and stopping applications, enumerating what is running and what
displays exist, and moving windows around. `ListSurfaces` is the one that
answers "what can I even automate right now", surfacing the same
availability information `Describe` reports for tasks (see
[configuration.md](configuration.md#what-happens-without-jev) for what
"available" means when a permission is missing).

## Clipboard (3)

`ClipboardGet`, `ClipboardSet`, `ClipboardClear`

Read, write, and clear the system clipboard. On macOS, `ClipboardSet` and
`ClipboardClear` depend on the `agent-desktop-macos-helper` binary shipping
beside the loaded module in the release archive; see
[installing-and-loading.md](installing-and-loading.md).

## Notifications (4)

`ListNotifications`, `NotificationAction`, `DismissNotification`,
`DismissAllNotifications`

Reading and acting on system notifications: listing what is currently shown,
invoking an action button on one, dismissing one, or clearing every
notification, optionally scoped to one application.

## Waits and status (4)

`Wait`, `Version`, `Status`, `Permissions`

`Wait` blocks until a condition holds or a timeout passes, up to thirty
seconds. `Version` reports the engine version and target and needs no
permission at all, which is why the release pipeline uses exactly this call
to prove a freshly built module actually answers (see
[releases.md](releases.md)). `Status` reports permissions, the active
session, and the latest snapshot in one call. `Permissions` reports the
platform's current accessibility and screen-recording state and, only when
its `request` field is explicitly set, prompts the user for the missing one;
otherwise it only reports, it never nags.

## Jev-driven (5)

`ResolveIntent`, `RunGoal`, `RunFlow`, `ValidateFlow`, `FlowGuide`

The members that hand a decision to Jev rather than taking one deterministic
action. `ResolveIntent` resolves one natural-language intent against the
current screen. `RunGoal` runs a bounded observe-decide-act loop toward a
single goal. `RunFlow` runs a whole authored flow, a sequence of steps with
loops and grounding, as described in
[`../../technical/specs/jev-intent-flows.md`](../../technical/specs/jev-intent-flows.md).
`ValidateFlow` checks a flow's shape without touching the desktop or Jev at
all, and `FlowGuide` just returns the flow-authoring guide as prompt text, so
neither of those last two needs Jev configured to work. All three of the
actually Jev-driven members (`ResolveIntent`, `RunGoal`, `RunFlow`) reply with
`JEV_NOT_CONFIGURED` rather than acting when no `jev` configuration was
supplied; see [configuration.md](configuration.md#what-happens-without-jev).

## Task (8)

`Describe`, `PlanTask`, `StartTask`, `AwaitTask`, `ContinueTask`,
`CancelTask`, `TaskReport`, `ListTasks`

The members for an outside agent that wants to hand over a whole job rather
than drive it call by call. A task can span both the desktop and a browser,
pauses for missing details or approvals rather than guessing, and always
stops in front of anything that pays. These eight answer with an
`AgentResponse<T>` rather than a `DesktopResponse`, a different envelope
shape with the same idea: `ok`, then either `data` or a structured `error`
with a code, message, hint, and whether retrying could help.

`Describe` is the self-describing entry point: call it first and it returns
`Capabilities`, everything a model needs to drive the rest of this family,
including whether Jev and the planner are configured, which surfaces are
available right now, the flow step kinds, the full authoring guide, every
member's JSON Schema, and worked examples. `PlanTask` drafts a flow from
plain language without running anything. `StartTask` actually starts one and
returns immediately with its first view. `AwaitTask` blocks until it needs
something or finishes. `ContinueTask` answers whatever it is paused on.
`CancelTask` stops it. `TaskReport` gives the full history of what it did.
`ListTasks` lists every task the module currently holds.

[`../../giving-it-a-task.md`](../../giving-it-a-task.md) walks through this
family from a caller's point of view, including facts, secrets, budgets, and
payment handling; [calling-it.md](calling-it.md) here shows the raw request
and response shapes.

## Browser (13)

`BrowserOpenSession`, `BrowserCloseSession`, `BrowserListSessions`,
`BrowserNavigate`, `BrowserSnapshot`, `BrowserPerform`, `BrowserReadPage`,
`BrowserEvaluate`, `BrowserScreenshot`, `BrowserReadOutput`,
`BrowserReleaseOutput`, `BrowserListDownloads`, `BrowserWaitDownload`

The primitives for driving a Chrome session directly, on the same interface
and object path as the desktop members, each carrying a `Browser` prefix so
none collides with a desktop member of a different shape (`Snapshot` and
`Screenshot` exist on both sides). `BrowserOpenSession` launches or attaches
to a browser and returns a `SessionId`; every other member except
`BrowserListSessions` takes that session beside its own fields.
`BrowserNavigate`, `BrowserSnapshot`, and `BrowserReadPage` observe the active
page; `BrowserPerform` acts on a ref, selector, or semantic locator the same
way the desktop's ref-addressed members act on an accessibility ref, and
fails loudly rather than reporting success when the target is covered.
`BrowserScreenshot` hands back an output id rather than an inline image;
`BrowserReadOutput` pulls it in chunks and `BrowserReleaseOutput` frees it
early. `BrowserListDownloads` and `BrowserWaitDownload` track files a page
downloads.

The module holds one `Browser`, shared with the task runner, so a task's own
browser session shows up in `BrowserListSessions` and a screenshot in its
`TaskReport.artifacts` can be read back with `BrowserReadOutput`, without having to go
through the task API to see it. See
[docs/crates/tinycomputer-bus/browser.md](../tinycomputer-bus/browser.md) for
the full payload shapes and error codes, and
[docs/crates/tinycomputer-browser/README.md](../tinycomputer-browser/README.md)
for the engine underneath.

## Confidential members

These members, among others, are marked `#[tinybus(confidential)]` in
`crates/tinycomputer/src/tinybus_module/dispatch/mod.rs`:

- `ResolveIntent`
- `RunGoal`
- `RunFlow`
- `StartTask`
- `ContinueTask`
- `TaskReport`
- `BrowserReadOutput`

TinyBus treats a confidential call as sensitive host-control traffic rather
than ordinary bus chatter, the same category the module's own configuration
travels in. They are marked that way because of what they can
carry or reveal:

- `ResolveIntent`, `RunGoal`, and `RunFlow` are the members that actually
  drive Jev, and the accessibility content they read and the goals or flows
  they are handed can include anything visible on screen or typed by a
  caller.
- `StartTask` and `ContinueTask` are how facts, including secret ones like
  card numbers or passwords, enter a task. Those secret values are never
  shown to Jev or the planner as anything but a `${name}` placeholder, and
  keeping the calls that carry them out of ordinary bus traffic is one more
  layer against them leaking through a monitor.
- `TaskReport` is confidential because it is the member that hands back
  everything a task did, potentially including the full Jev exchange trace
  when the task requested one.
- `BrowserReadOutput` returns the bytes of a held screenshot, and a
  screenshot can show what a task typed into a form, card details included.
  `BrowserScreenshot` and `BrowserReleaseOutput` stay ordinary: they carry
  only the output's id.

The other two task members that touch a running task, `AwaitTask` and
`CancelTask`, are not confidential: they carry or return only a task id and
its current status, nothing that needs the extra protection. `PlanTask` and
`ListTasks` are ordinary calls for the same reason, and `Describe` is
explicitly meant to be readable by anyone deciding whether to start a task at
all.

## Next

[calling-it.md](calling-it.md) has worked examples for both an ordinary
desktop member and a full task, and
[`../../technical/specs/desktop-module-contract.md`](../../technical/specs/desktop-module-contract.md)
is the frozen, versioned source of truth if this page and the code ever
disagree.
