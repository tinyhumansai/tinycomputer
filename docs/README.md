# tinycomputer documentation

tinycomputer is a decision model (Jev) based harness for desktop and browser
automation, written in Rust. Jev makes every choice about what to press; the
Rust harness around it reads the screen, asks, checks, acts, verifies, and
keeps it safe.

Pick a starting point by what you want to do.

## New here

1. [How it works](how-it-works.md): the big picture in ten minutes.
2. [Giving it a task](giving-it-a-task.md): hand over a whole job and get
   told when it needs you.
3. [Glossary](glossary.md): every term on one page.

## Guides

| Guide | Covers |
|---|---|
| [How it works](how-it-works.md) | the three ways to use it, the planner, Jev, the runtime, one step start to finish |
| [Giving it a task](giving-it-a-task.md) | starting a task, statuses, approvals, budgets, where it can go |
| [Writing flows](writing-flows.md) | every step kind, and how to write steps that work |
| [How it decides](how-it-decides.md) | Jev's questions, voting, the brief, narrowing, attention, deliberation |
| [Catching mistakes](catching-mistakes.md) | noticing a bad click, verified undo, backtracking, reflection, typing that checks itself |
| [Rescues](rescue.md) | what happens when a step fails, and what a rescue may and may not do |
| [Memory and saving](memory-and-saving.md) | saved values, output shapes, grounding memory, checkpoints, saved plans, resuming |
| [How it sees the screen](seeing-the-screen.md) | the accessibility tree, sight on the web, surfaces, the on-screen cursor |
| [Safety and privacy](safety-and-privacy.md) | irreversible actions, payment, shared and secret details, untrusted screen text |
| [Watching a run](watching-a-run.md) | step reports, traces, the journal, the lab, finding the fault |

## The code, folder by folder

One friendly guide per crate and per top-level folder. Start at
[`project/README.md`](project/README.md) for a map of the whole repository.

| Crate | What it is |
|---|---|
| [`tinycomputer`](crates/tinycomputer/README.md) | the installable module a host loads |
| [`tinycomputer-engine`](crates/tinycomputer-engine/README.md) | tasks, the planner, rescues, the Jev runtime, and the [flow runtime](crates/tinycomputer-engine/flow/README.md) |
| [`tinycomputer-bus`](crates/tinycomputer-bus/README.md) | the wire contract: every type that crosses the bus |
| [`tinycomputer-core`](crates/tinycomputer-core/README.md) | the shared screen model, safety rules, facts, and parsers |
| [`tinycomputer-desktop`](crates/tinycomputer-desktop/README.md) | desktop apps through the accessibility tree |
| [`tinycomputer-browser`](crates/tinycomputer-browser/README.md) | web pages through Chrome, and sight |
| [`tinycomputer-cursor`](crates/tinycomputer-cursor/README.md) | the cursor you can watch |
| [`tinycomputer-accessibility`](crates/tinycomputer-accessibility/README.md) | in-process focus, permission and Globe-key answers for a host |
| [`tinycomputer-skills`](crates/tinycomputer-skills/README.md) | the guide and schemas for agents that call tasks |
| [`tinycomputer-examples`](crates/tinycomputer-examples/README.md) | examples, the lab, saved plans, the journal reader |

| Folder | What it is |
|---|---|
| [`scripts/`](project/scripts/README.md) | the lab, the Docker lab, the journal viewer |
| [`docker/`](project/docker/README.md) | the container for browser runs |
| [`vendor/`](project/vendor/README.md) | the pinned engines and clients |
| [CI and tooling](project/ci-and-tooling/README.md) | workflows, releases, lints, supply-chain checks |

## Technical reference

The engineering docs live in [`technical/`](technical/README.md): the
architecture, every decision loop and threshold, every Jev question, the task
controller, the debug journal, specifications, implementation plans, decision
records, and recorded evaluations. Read those when you're changing the code.

[Globe lease replay and joined native shutdown](technical/specs/globe-replay.md) specifies the reliable macOS input composition contract.

## Conventions

- Keep every Markdown file at 500 lines or fewer. Split a topic that outgrows
  that, and link the parts from the nearest `README.md`.
- Update docs in the same commit as the behaviour they describe.
- One fact lives in one place. Link rather than copy.
- The guides here explain; `technical/` specifies. When they disagree, the
  code is right, and both docs get fixed.
