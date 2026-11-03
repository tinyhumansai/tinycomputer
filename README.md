# tinycomputer

**A decision model (Jev) based harness for desktop and browser automation,
written in Rust.**

tinycomputer drives desktop apps like Mail and Notes, and websites in a real
Chrome browser. Every choice about what to press comes from Jev, a decision
model that answers small, closed questions about the screen. Everything else
is the harness: plain Rust code that reads the screen, asks the questions,
checks the answers, acts, verifies what happened, and enforces the safety
rules.

Give it a job like "find the cheapest flight from Delhi to Srinagar on 14
October and fill in my details up to payment". It works out the clicks
itself, asks you when it needs something, and always stops before money
moves. It doesn't use screenshots or pixel matching, and nobody has to tell
it where the buttons are. It reads the screen the way a screen reader does,
as a list of named buttons and fields.

tinycomputer ships as a TinyBus module: one library that a host program
loads, which an agent then calls.

## What it can do

- **Run a whole task in the background.** Hand over a job in plain words. A
  planner turns it into steps, and tinycomputer carries them out across the
  browser and your desktop apps.
- **Pause when it needs you.** It stops for a missing detail, for your
  approval before anything irreversible (send, delete, submit), and for a
  person to solve a captcha or type a one-time code.
- **Stop at payment.** Reaching a Pay button ends the task there, with the
  page left open for you to finish.
- **Recover from mistakes.** It checks every click, undoes the ones that made
  things worse, and tries the next best option.
- **Get past a stuck step.** When a step fails, a reasoning model looks at the
  screen and suggests another way through, up to five times.
- **Remember.** It keeps track of what it has read during a task, can return
  results in the JSON shape you ask for, and learns which buttons worked so
  the next run is faster.
- **Keep your details private.** Secret details like passport or card numbers
  are typed into fields and never shown to any model.

## A task, from the caller's side

```json
{"member": "StartTask", "args": [{
  "task": "Find the cheapest one-way flight from Delhi to Srinagar on 14 October and fill in my details up to payment",
  "facts": {"first name": "Asha", "last name": "Raina", "email": "asha@example.com"},
  "constraints": {"surfaces": ["browser"]}
}]}
```

`StartTask` returns right away with a task id. You then call `AwaitTask`,
which waits until something changes and tells you where things stand:

| Status | Means | You do |
|---|---|---|
| `running` | still working | wait again |
| `needs_input` | it needs a detail you didn't give | answer with `ContinueTask` |
| `needs_approval` | it found something irreversible | approve or decline |
| `needs_human` | captcha, one-time code, or login | a person handles it, then `ContinueTask` |
| `checkpoint` | it reached payment and stopped | a person pays |
| `done` | finished | read the result |
| `failed` | a step failed and couldn't be rescued | read the report |

Every answer carries a one-sentence summary and a list of the calls that make
sense next. [Giving it a task](docs/giving-it-a-task.md) walks through a full
session.

## How it works, briefly

```text
 look at the screen ─► ask Jev small questions ─► act ─► check what changed
        ▲                                                       │
        └───────────── undo it and try again if it got worse ◄──┘
```

Jev never writes text and never plans. It answers yes/no questions, scale
questions, and "which of these options" questions, each with a probability.
Every question is asked several ways at once (options shuffled, wording
varied) and the answers are averaged, so a bias toward the first option or
toward "yes" cancels out. When the answers don't clearly agree, tinycomputer
asks more, compares the top candidates head to head, and keeps the
runners-up in case the first pick turns out wrong.

The safety rules are plain code, not a model's judgement. Text on a web page
is always treated as data, never as instructions.

[How it works](docs/how-it-works.md) tells the whole story.

## Three ways to use it

| Level | You give it | It works out |
|---|---|---|
| Tasks | a job in plain words | the plan, every click, when to stop and ask |
| Flows | a short list of plain steps ("start a new email", "stop before sending") | which buttons and fields make each step happen |
| Primitives | exact commands on the desktop (`Snapshot`, `Click`, `SetValue`, ...) or in a browser session (`BrowserOpenSession`, `BrowserNavigate`, `BrowserPerform`, ...) | nothing: you decide everything |

The module serves 80 members: 54 desktop primitives, 13 browser primitives,
5 Jev-driven members for goals and flows, and 8 task members. `Describe`
lists every one with a sentence on what it's for. See
[Writing flows](docs/writing-flows.md) for flows, and the
[module docs](docs/crates/tinycomputer/members.md) for the full list.

## Documentation

Start at [`docs/README.md`](docs/README.md). The guides:

- [How it works](docs/how-it-works.md)
- [Giving it a task](docs/giving-it-a-task.md)
- [Writing flows](docs/writing-flows.md)
- [How it decides](docs/how-it-decides.md)
- [Catching mistakes](docs/catching-mistakes.md)
- [Rescues](docs/rescue.md)
- [Memory and saving](docs/memory-and-saving.md)
- [How it sees the screen](docs/seeing-the-screen.md)
- [Safety and privacy](docs/safety-and-privacy.md)
- [Watching a run](docs/watching-a-run.md)
- [Glossary](docs/glossary.md)

Each crate and top-level folder has its own guide under
[`docs/crates/`](docs/README.md#the-code-folder-by-folder) and
[`docs/project/`](docs/project/README.md). The engineering reference
(architecture, every loop and threshold, specs, plans, and recorded live
runs) is in [`docs/technical/`](docs/technical/README.md).

## Setting it up

The host passes the module a configuration when it loads it. The parts that
matter most:

| Key | What it's for |
|---|---|
| `jev` | the decision model and its key: Jev (`type_safe`, `open_router`, `tiny_humans_open_router`), OpenJEV (`open_jev`), or Levanto Sage (`sage`). Without it, only the primitives work |
| `planner` | a key for the planner, which also brings rescues and output shapes, on OpenRouter (default) or Tiny Humans' gateway (`provider: "tiny_humans"`); the rescuer may take its own `rescue_route`. Without it, you write flows yourself |
| `browser` | how browsers launch: `executable`, `user_agent`, `args`, and page `perception` |
| `cursor` | the on-screen cursor you can watch: `off`, `brisk`, `natural`, or `calm` |

Every key is described in
[the module's configuration guide](docs/crates/tinycomputer/configuration.md).

Desktop control needs Accessibility permission (and Screen Recording for
screenshots). tinycomputer checks first and tells you what's missing. macOS
is fully supported; on Windows and Linux the desktop side isn't available
yet, but the browser works anywhere Chrome runs.

## For developers

The repository is a Rust workspace. Clone it with its submodules:

```sh
git submodule update --init --recursive
```

The four checks CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

The tests need no display, no permissions, no browser, and no network. Live
runs against real apps and sites use the lab:

```sh
scripts/lab run mail-compose
scripts/docker-lab -- crates/tinycomputer-examples/fixtures/run task_fixture
```

| Crate | Holds |
|---|---|
| `tinycomputer` | the loadable module |
| `tinycomputer-engine` | tasks, the planner, rescues, the Jev runtime, the flow runtime |
| `tinycomputer-bus` | the wire contract |
| `tinycomputer-client` | typed, configuration-safe host calls |
| `tinycomputer-core` | the shared screen model, safety rules, facts |
| `tinycomputer-desktop` | desktop apps, through agent-desktop |
| `tinycomputer-browser` | web pages, through agent-browser |
| `tinycomputer-cursor` | the cursor you can watch |
| `tinycomputer-accessibility` | in-process focus, permission and Globe-key answers for a host |
| `tinycomputer-skills` | the guide and schemas for calling agents |
| `tinycomputer-examples` | examples, the lab, saved plans |

[`AGENTS.md`](AGENTS.md) has the full contributor guidelines
(`CLAUDE.md` links to it), and
[`docs/technical/architecture.md`](docs/technical/architecture.md) explains
how the layers fit. Releases are covered in
[the release guide](docs/crates/tinycomputer/releases.md). See also
[`CONTRIBUTING.md`](CONTRIBUTING.md) and [`SECURITY.md`](SECURITY.md).

## License

GPL-3.0-only. See [LICENSE](LICENSE). The vendored `agent-desktop` and
`agent-browser` engines are Apache-2.0; TinyBus and TinyInference are vendored
under their own licenses.
