# tinycomputer-engine

tinycomputer is a decision-model harness for desktop and browser
automation, written in Rust: Jev makes every choice about what to press by
answering small closed questions, and the harness does everything else. This
crate, `tinycomputer-engine`, is that harness. It reads the screen, asks
Jev what to do, checks the answer, acts, verifies the action landed, and
enforces safety, and it keeps a task moving until it is done, blocked, or
needs a person. If `tinycomputer-desktop` is the hand that clicks and
types, this crate is the part that decides where the hand goes next. The
default `desktop` feature enables desktop entry points; embedding hosts can
disable it and use `BrowserRunner` with `tinycomputer-browser` alone.

It has no bus of its own. The `tinycomputer` crate wraps these functions as
TinyBus members; this crate only holds the logic.

## What lives here

| Area | Module | What it does |
|---|---|---|
| The decision model's front door | `src/agentic/runtime.rs` (`JevRuntime`) | Configures and calls Jev, and hosts the two native loops below. |
| Point at one thing, maybe act | `src/agentic/resolve.rs` (`resolve_intent`) | Grounds one described element on the live screen and optionally acts on it. |
| Run a bounded goal | `src/agentic/goal.rs` and `src/agentic/task/` (`run_goal`) | An observe-decide-act loop toward one visible end state, with a confirmation handle for anything hard to undo. |
| The high-level flow runtime | `src/agentic/flow/` | Runs a whole flow (the plain-language step language), one step at a time. Documented separately: see [flow/README.md](flow/README.md). |
| The debug journal | `src/agentic/journal/` | Every Jev exchange and how long each part of a run took, written to disk, opt-in and inert when off. |
| The task controller | `src/task/` | `Tasks` runs a flow in the background and reports its status; `BrowserRunner` supplies one in-process browser session per task without TinyBus. |
| The planner | `src/planner/` | Turns a plain-language task into a flow, using a language model that never touches the screen. |
| The rescuer | `src/rescue/` | Consulted only when a task's step fails; suggests replacement steps or gives up. |
| The shaper | `src/shape/` | Turns a finished task's saved records into the JSON its caller asked for, one reasoning-model pass. See [output.md](output.md). |
| The workspace | `src/workspace/` | Joins the desktop and the browser into one surface, so a flow can move between an application and a web page without the caller tracking which is which. |

See [`docs/technical/jev-harness.md`](../../technical/jev-harness.md) for the
Jev stack underneath all of it, described from the implementer's side.

## How the pieces fit

A caller, a model driving the module over TinyBus, has three ways to get
something done, in increasing order of how much it hands over:

1. **Point at one thing.** `ResolveIntent` finds the one element on screen
   that matches a short description ("the submit button") and, if asked,
   clicks or types into it. See [goals-and-intents.md](goals-and-intents.md).
2. **Run toward a visible end state.** `RunGoal` keeps observing, deciding,
   and acting until the goal looks satisfied, gets stuck, or hits something
   that needs a person's yes. Same page.
3. **Hand over a whole task.** `StartTask` runs a flow (or a plain-language
   task, if a planner is configured) in the background, across as many
   flow runs as it takes, pausing only for missing values, irreversible
   actions, and payment. See [tasks.md](tasks.md).

All three eventually ask Jev the same kind of small, closed question through
one door, [`JevRuntime`](jev-runtime.md): given this screen and this goal,
which operation and which element. `RunGoal` and `ResolveIntent` ask it
directly, one decision at a time; the flow runtime (in `flow/`, out of scope
for this page, see [flow/README.md](flow/README.md)) asks it once per flow
step, with its own grounding and voting; the task controller sits above the
flow runtime and does not talk to Jev itself.

```
StartTask / ContinueTask / AwaitTask      (task/)
        |
        v
   flow runtime                            (agentic/flow/, see its own README)
        |
        v
   JevRuntime::evaluate                     (agentic/runtime.rs)
        |
        v
   Jev (tinyinference_decisions)
```

A few things run alongside that column rather than inside it:

- the **planner** (`planner/`) turns a plain task into a flow *before* any of
  this starts, and never sees the screen;
- the **rescuer** (`rescue/`) is called by the task controller *after* a step
  fails, to suggest a fix, and also never sees the screen except as text with
  every fact value already redacted;
- the **shaper** (`shape/`) is called by the task controller once every
  step has finished, to turn what the flow saved into the JSON its caller
  asked for; it never acts and never sees a screen either. See
  [output.md](output.md);
- the **journal** (`agentic/journal/`) is a passive listener wired into
  `JevRuntime`, writing what happened for later reading;
- the **workspace** (`workspace/`) sits underneath the flow runtime, so a
  step that says `open Mail` or `browse https://…` reaches the right surface.

`JevRuntime` also has other decision models behind the same loops: OpenJEV
(`provider: "open_jev"`) and Levanto Sage (`provider: "sage"`, or
`JevRuntime::sage`), selected by the module's `jev` configuration. See
[jev-runtime.md](jev-runtime.md) and [sage.md](sage.md).

## Reading order

If you are new to this crate, read in this order:

1. [jev-runtime.md](jev-runtime.md), what Jev is, how it is configured, and
   the one door everything calls through, and, if you are curious whether
   another decision model could sit behind that same door, [sage.md](sage.md).
2. [goals-and-intents.md](goals-and-intents.md), the older, lower-level
   loops: `ResolveIntent` and `RunGoal`.
3. [tasks.md](tasks.md), the task controller: the thing most callers
   actually use.
4. [planner.md](planner.md), [rescue.md](rescue.md), and [output.md](output.md),
   the three places a language model helps without touching the screen:
   writing the flow, recovering from a failed step, and shaping the
   finished task's answer.
5. [workspace.md](workspace.md), how one flow reaches both a desktop
   application and a web page.
6. [journal.md](journal.md), how to see what a run actually did.

## Cross-cutting guides

These live one level up, outside this crate's docs, and cover the same ideas
from a less technical angle:

- [how-it-works.md](../../how-it-works.md)
- [giving-it-a-task.md](../../giving-it-a-task.md)
- [writing-flows.md](../../writing-flows.md)
- [how-it-decides.md](../../how-it-decides.md)
- [catching-mistakes.md](../../catching-mistakes.md)
- [rescue.md](../../rescue.md)
- [memory-and-saving.md](../../memory-and-saving.md)
- [seeing-the-screen.md](../../seeing-the-screen.md)
- [safety-and-privacy.md](../../safety-and-privacy.md)
- [watching-a-run.md](../../watching-a-run.md)
- [glossary.md](../../glossary.md)

## Technical references

- [`docs/technical/architecture.md`](../../technical/architecture.md), the
  whole system, how a call travels.
- [`docs/technical/jev-harness.md`](../../technical/jev-harness.md), the Jev
  stack end to end, with latency levers.
- [`docs/technical/tasks.md`](../../technical/tasks.md), the task API's
  formal contract.
- [`docs/technical/specs/task-rescue.md`](../../technical/specs/task-rescue.md),
  the rescuer's spec.
- [`docs/technical/specs/task-output.md`](../../technical/specs/task-output.md),
  the shaper and the run memory's spec.
- [`docs/technical/specs/unified-agent.md`](../../technical/specs/unified-agent.md),
  where the browser joins the desktop.
- [`docs/technical/jev-journal.md`](../../technical/jev-journal.md), reading
  and summarising a journal.
- [`docs/technical/evals/2026-09-29-sage.md`](../../technical/evals/2026-09-29-sage.md),
  Levanto Sage measured live against Jev behind the same loops.
