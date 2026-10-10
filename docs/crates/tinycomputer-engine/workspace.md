# The workspace

A flow can open a desktop application, then browse to a web page, then
switch back to the application: a booking confirmation emailed and then
checked in a Mail app, say. `Workspace<D, W>`, in
`crates/tinycomputer-engine/src/workspace/mod.rs`, is what makes that
possible without every caller of the flow runtime having to track which
surface a given step belongs to. It joins a desktop surface and a browser
surface into one thing that implements `tinycomputer_core::surface::Surface`,
the same trait either side implements alone, so the flow runtime, and
everything above it, can just call one `Surface` and let the workspace route
each call correctly.

## Routing by name

A call that names an application routes by that name (`Workspace::side_for`):

- `browser`, anything starting with `browser:`, or an address starting with
  `http://` or `https://` goes to the browser;
- anything else goes to the desktop.

A call that does not name an application (acting on a candidate returned
by an earlier observation, or reading its value back) goes to whichever
side is *active*: the side that was last successfully observed or opened.
That is what lets a flow step like "click Continue" work without repeating
which surface it is on: the candidate it is acting on came from that side's
last observation, so that is where the click has to go too.

```rust
pub struct Workspace<D, W> {
    desktop: Option<D>,
    browser: Option<W>,
    active: Arc<Mutex<Side>>,
    last_app: Arc<Mutex<String>>,
}
```

`active` flips only after a call on that side actually succeeds. A failed
`navigate` or `launch` never makes the browser active, so a call that names
no application is never accidentally routed to a side that just failed to
open.

## Why both sides are optional

`Workspace::new(desktop: Option<D>, browser: Option<W>)` takes each side as
an `Option`, not a bare value, and that is deliberate: a task confined by
its `TaskConstraints.surfaces` to just the browser must genuinely be unable
to reach the desktop, and vice versa. There is no unconditional fallback:
a call that names the desktop on a browser-only workspace fails outright
with `DESKTOP_NOT_AVAILABLE`, rather than silently trying the browser
instead:

```rust
fn no_desktop(command: &str) -> DesktopResponse {
    DesktopResponse::err(
        command,
        DesktopError::new(
            "DESKTOP_NOT_AVAILABLE",
            "this task has no desktop; allow the desktop surface to use applications",
        ),
    )
}
```

The initial active side defaults to whichever one is actually present,
desktop first, if both are available, so a browser-only workspace never
starts pointed at a side it was never given.

## What routes where

Every method on `Surface` is handled, but not all the same way:

| Method | Routing |
|---|---|
| `observe` | By the named application; whichever side answers becomes active and is remembered as `last_app`. |
| `execute` | By the *active* side: it never takes an application name, since it acts on a candidate the active side's own observation produced. |
| `read_value` | Same as `execute`: whichever side is active. |
| `paste`, `press` | By the named application, like `observe`. |
| `launch` | By the named application; success makes that side active. |
| `navigate` | Always the browser (there is no desktop equivalent); failure never activates it. |
| `back` | The active side if it is the browser, else the desktop; there is no browser fallback if the desktop is active and has no browser. |
| `dismiss_cover` | Same as `back`: the active side if it is the browser, else the desktop, which refuses it, since only a page can say what lies over a control. |
| `settle`, `settle_briefly`, `await_change` | The active side, or the desktop if the browser is not active; with neither, nothing changes. |

## Reading what is on screen without acting

`Workspace::visible_text()` returns the visible text of whatever was last
observed or opened: its context lines, every control's label, and what
each field currently holds, rendered as `label = "value"` and cut to 80
characters per field. This is what the task controller hands the rescuer
(see [rescue.md](rescue.md)) when a step fails, and what `human_wall`
checks for the signs of a captcha or login wall (see
[tasks.md](tasks.md#how-a-needshuman-pause-is-detected)). If nothing has
ever been observed or opened, or the last one can no longer be read, it
returns an empty list rather than failing.

## Source

- `crates/tinycomputer-engine/src/workspace/mod.rs`, `Workspace`, `Side`,
  `side_for`, `visible_text`.
- `crates/tinycomputer-core/src/surface/`, the `Surface` trait both sides
  implement.
- `crates/tinycomputer-engine/src/task/store.rs`, where a task keeps its
  `TaskConstraints.surfaces`, and
  `crates/tinycomputer/src/tinybus_module/runner.rs`, which builds the
  task's workspace with that side (or sides).
- [`docs/technical/specs/unified-agent.md`](../../technical/specs/unified-agent.md),
  the spec for joining the desktop and the browser.
