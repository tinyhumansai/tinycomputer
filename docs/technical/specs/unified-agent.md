# Unified agent: browser and desktop behind one task API

**Status:** Accepted. **Owner:** tinycomputer maintainers.
**Plan:** [`../plans/unified-agent.md`](../plans/unified-agent.md).
**Builds on:** [`jev-intent-flows.md`](jev-intent-flows.md).
**Extended by:** [`jev-briefing.md`](jev-briefing.md) — briefs, secrets, votes,
and filling payment forms.

## Problem

Desktop automation lives here, browser automation lives in
`tinyhumansai/tinybrowser`, and each has its own Jev loop. A real task crosses
both. "Book the cheapest flight to Kashmir and fill in my details up to the
payment page" means searching and comparing in a browser, filling a traveller
form, and maybe writing a summary in Mail. No single caller-facing API can run
that today. The host has to learn two modules, two ref models, two error
vocabularies and two loops, and stitch them together itself.

## Goals

- One module, one agent-friendly task API: a caller hands over a task (plain
  language, or a high-level flow) and gets back structured progress, questions,
  checkpoints and results.
- Browser and desktop are two *surfaces* behind one runtime, so a single flow
  can move between them.
- The browser engine is `vercel-labs/agent-browser`, linked as a library and
  vendored like `agent-desktop`. It is not reimplemented.
- The layers are reusable crates: a host can take just the desktop or just the
  browser adapter.
- Nothing irreversible or financial happens without an explicit, visible
  checkpoint.

## Non-goals

- Paying on its own. Payment details are typed only from secret facts and only
  under `payment: fill_then_approve`; the control that pays is always a
  checkpoint or an approval ([`jev-briefing.md`](jev-briefing.md)).
- Solving captchas, or bypassing logins or two-factor prompts. They become
  `needs_human`.
- Vision models in the shipped module.

## Architecture

Dependencies point one way:
`bus` ← `core` ← {`desktop`, `browser`} ← `engine` ← `tinycomputer` (cdylib).

| Crate | Responsibility |
|---|---|
| `tinycomputer-bus` | Wire contract for all interfaces. No runtime. |
| `tinycomputer-core` | Shared domain: the `Surface` trait; `Screen`, `Candidate` and `Depth`; logical keys and the per-OS keymap; the safety classifier (irreversible labels, payment detection); facts; records and parsers; budgets. |
| `tinycomputer-desktop` | The agent-desktop adapter (`Desktop`, one method per member) and `DesktopSurface`. |
| `tinycomputer-browser` | The agent-browser adapter (sessions over `agent_browser::execute_command`, typed conversion, error mapping, origin policy, output handles) and `BrowserSurface`. |
| `tinycomputer-engine` | The Jev runtime, `RunGoal`, `ResolveIntent`, the flow runtime over `core::Surface`, the decision loops, the workspace, the task controller and the optional planner. End-to-end tested. |
| `tinycomputer` | The TinyBus cdylib. One interface impl per surface family, each delegating to the engine. No behavior. |

## Interfaces

A TinyBus module manifest declares one bus name and one object path, and
`module_export!` attaches its member list to the first interface only. Until
TinyBus serves several interfaces per module, the members below share
`ai.tinyhumans.tinycomputer.Desktop`; every browser member takes a `Browser`
prefix, so the family is obvious in a flat list and none collides with a
desktop member. The table is the logical split, and `Describe` serves it as
`Capabilities.catalogue`.

| Interface | For | Members |
|---|---|---|
| `ai.tinyhumans.tinycomputer.Agent` | external agents | `Describe`, `PlanTask`, `StartTask`, `AwaitTask`, `ContinueTask`, `CancelTask`, `TaskReport`, `ListTasks` |
| `Browser…` members | power users | served (2.6): sessions, navigate, snapshot, perform, read, evaluate, screenshot, outputs, downloads. Planned: tabs, cookies and storage state, upload, dialog, find, and a policy-checked `Command` |
| `ai.tinyhumans.tinycomputer.Desktop` | power users | the existing 59 members, unchanged |

### Agent API rules

- Every reply is a `TaskView`: `{id, status, summary, step, progress, needs?, result?, next}`.
  - `summary` is one sentence.
  - `next` names the calls that make sense now.
- `status` is one of:
  - `running`;
  - `needs_input{fields}`;
  - `needs_approval{action, target}`;
  - `checkpoint{reason, url, summary}`;
  - `needs_human{reason}`;
  - `needs_plan{guide}`;
  - `done{answer, records}`;
  - `failed{step, reason, hint}`;
  - `cancelled`.
- A status never carries a screenshot, although the wire form has an
  optional `screenshot` field on approvals, checkpoints, and human turns. A
  view also travels through `AwaitTask` and `ListTasks`, which are not
  confidential, and an output id is all `BrowserReadOutput` needs. The
  screenshot a stopped run takes lives in `TaskReport.artifacts` only.
- Calls return quickly. A task runs on the module's runtime, and `AwaitTask`
  long-polls it.
- No refs or selectors at this level. `Describe` serves JSON Schemas and worked
  examples.
- `StartTask`, `ContinueTask` and `TaskReport` are confidential, because they
  carry facts and page data.

### Flow grammar additions

These are additive to the grammar in [`jev-intent-flows.md`](jev-intent-flows.md):

- `browse "<url or site>"`
- `in "<surface>" {steps}`
- `extract {what, fields, into}`
- `pick {from, by, into}` (implemented): chooses the best result card and
  opens it; prices, times, durations, stops, and nearness to a number
  ("closest to 9") are ranked exactly
- `ask [slots]`
- `checkpoint "<why>"`

## Components

### Deterministic

- **Browser engine:** agent-browser. It provides sessions, tabs, refs,
  locators, state save and load, CDP attach, downloads and uploads, and
  dialogs. Its domain filter is not used (see Invariants).
- **Surfaces and keymap.** Logical keys map to `cmd` on macOS and `ctrl`
  elsewhere.
- **Workspace:** named surfaces and handoff between them.
- **Extraction:** records and typed parsers for price, time, duration, stops
  and dates, with deterministic ranking for parsed criteria.
- **Form model:** `autocomplete`, `type` and `required`, with obvious slots
  pre-matched.
- **Facts store:** shared facts brief Jev by value; secret facts (cards,
  passports, passwords, one-time codes) reach Jev and the planner only as
  `${name}` and are masked out of every request
  ([`jev-briefing.md`](jev-briefing.md)).
- **Safety gate:** irreversible labels, plus a payment detector that looks for
  `cc-*` fields, card-number or CVV inputs, checkout URLs and pay labels.
- **Task store:** state machine, TTL, cancellation and budgets.
- **Obstacle heuristics:** cookie banners; captcha, login and 2FA detection.
- **Settle and wait:** network idle and DOM stable.
- **Artifacts:** checkpoint screenshots, traces and records.

### Jev decision loops

- **Reused from intent flows:**
  - completion and negation;
  - progress;
  - moves (a per-surface catalogue);
  - narrowing and knockout;
  - consistency and corroboration;
  - slots;
  - obstacles, undo and memory (keyed by origin in the browser).
- **New:**
  - **page kind:** search, results, details, login, traveller form, seats,
    extras, review, payment, error, captcha;
  - **pick and rank** for criteria that are not parsed;
  - **combobox and autocomplete** suggestion choice;
  - **date picker**;
  - **upsell decline**;
  - **missing detail:** Noul per slot;
  - **validation error:** Noul.

### Optional planner

- It is compiled in, and inert until the host sends a confidential `planner`
  config.
- It writes the flow from the task, the guide, the facts' names and the
  available surfaces, and validates it with up to two repair turns.
- It replans from step reports and summarizes the result from records.
- Without it, a plain-language task returns `needs_plan`.

## Invariants

- An LLM runs only when the host configures `planner`. It never sees fact
  values, typed text, or payment pages.
- Jev only ever sees a fact's name, never its value. A flow may substitute a
  fact's value only into an `enter` step's typed text; a `${fact}` reference
  anywhere else in a flow — including an `open` application name, a `browse`
  address, or an `enter` slot's own name — fails validation, and `StartTask`
  fails fast rather than run it. `open` and `browse` count too because the
  launched application or address becomes visible state (`screen.app`, the
  run's history) on every step after it, not just the one destination. The
  flow runtime also never expands a fact into that text at run time, as a
  backstop behind validation.
- A payment page ends at a `checkpoint`, and no payment data is typed, ever.
- Irreversible actions need `allow_destructive` or an explicit approval via
  `ContinueTask`.
- The origin allow-list is enforced per task on pages, by the browser session
  (`tinycomputer-browser` `origins/`), reading addresses by the WHATWG URL
  rules the browser does: a navigation outside it is refused before it is
  sent, no call acts on or reads a page outside it, and a page a task is
  taken to outside it is left before it is read or acted on. A name is never
  resolved, so `*` refuses only what is local by how it is written.
  agent-browser's domain filter is not used: it
  refuses every request outside the list, so a page's own CDN and APIs fail,
  it stalls or closes the browser when it cannot install itself on a new
  target, and it refuses a profile beside a list. It is a guard rail, not a
  sandbox.
- Browser tests that launch Chromium run in the Docker lab
  ([`../docker-lab.md`](../docker-lab.md)) or behind `TINYCOMPUTER_LIVE_BROWSER=1`.

## Acceptance criteria

- The four contract commands pass, with 90% line coverage per file, clean
  `cargo deny`, and builds on all six release targets.
- A fixture travel site runs end to end in the Docker lab with a mock Jev:
  search, extract, pick the cheapest, fill the traveller form, decline extras,
  and stop at a payment checkpoint.
- The live `kashmir-booking` scenario reaches the payment checkpoint with a
  screenshot, and `cross-surface` leaves a Mail draft stopped before Send.
