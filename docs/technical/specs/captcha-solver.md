# Captcha solver

**Status:** Implemented (contract 3.1). **Owner:** tinycomputer engine.

## Problem

A task that meets a captcha or a "prove you are human" check stops as
`needs_human` and waits for a person, even when the check is a checkbox or an
image grid a decision model can now read. Jev reads images since 1.13 and
locates things in them since 1.14 (`jev-preview`), through TypeSafe's image
opt-in (`tinyinference-decisions`: `EvaluationRequest::with_image`,
`Question::Box`).

## Goals and non-goals

Goals:

- Try the checkbox widgets of reCAPTCHA v2, hCaptcha and Cloudflare
  Turnstile, and the 3 by 3 and 4 by 4 image-grid challenges behind them,
  before asking a person.
- Opt-in, bounded, journaled, and never in the way: anything the solver
  cannot answer goes to a person exactly as before, now saying what was tried.

Non-goals:

- Typed-text, audio, slider and puzzle challenges. Jev decides; it cannot
  write. Those are `Unsupported` without anything pressed.
- One-time codes, two-factor prompts and login walls. They are the person's
  own and never reach the solver (`tinycomputer_core::is_bot_check`).
- Hiding automation from a site. The solver presses what a person would.

## Behaviour

Configuration (`ComputerConfig.captcha`, `tinycomputer_bus::config::CaptchaConfig`):

```json
{"captcha": {"jev": {"api_key": "…", "model": "jev-preview"}, "max_attempts": 3}}
```

`jev` is a Jev route of its own (any `JevConfig`; `model` defaults to
`jev-preview`), so the main decision model may stay on a text-only route.
`max_attempts` (default 3, at most 10) bounds image challenges submitted per
try. Without the section there is no solver and nothing changes.

When a failed step leaves a page whose text is a bot check, `human_wall` calls
`FlowRunner::solve_captcha`. The module's runner (`WorkspaceRunner`) runs
`tinycomputer_engine::solve_captcha` on the task's browser session, when the
task is on the browser:

1. **Scan** (`agentic/captcha/scan.rs`). A fixed script in the top document
   lists the widget frames by address with their boxes, and reports whether a
   vendor response field (`g-recaptcha-response`, `h-captcha-response`,
   `cf-turnstile-response`) holds a token. The widget's own frames are
   cross-origin and are never read.
2. **Token present**: solved.
3. **Challenge showing**: cut it out of a viewport screenshot, scale it into
   280–1568 pixels a side (`pixels.rs`), and ask one request: a `layout`
   Choice (`grid_3x3`, `grid_4x4`, `typed`, `other`), a Box for the grid, and
   a Box for the verify button. Not a grid: `Unsupported`. Then cut the grid
   into tiles and ask one Noul per tile, each tile its own image beside the
   challenge. Press tiles at or over `TILE_THRESHOLD`, then verify.
4. **Checkbox showing**: a Box question finds it; under `BOX_CONFIDENCE`, its
   usual place (28 CSS pixels in, halfway down) stands in. Press it.
5. **Nothing showing**: solved if the solver acted, `Unsupported` if not.

Presses are trusted pointer events at a viewport point
(`Browser::press_point`), each one a raw command through the session's action
gate and allowed origins.

On `Solved` the failed step and the rest run again, as after a person's
`ContinueTask`. A drive gets past at most `MAX_SOLVED_WALLS` (3) walls on its
own; the next goes to a person. On `Unsolved` or `Unsupported` the task
pauses as before, its `reason` naming what was tried, and its report keeps the
screenshot every stop captures.

## Invariants

- Every Jev call goes through `JevRuntime::evaluate` and is journaled as step
  `captcha`; the journal records an image by its length, never its bytes.
- The runner journals one `captcha` event per try with its outcome.
- Only the widget's box and its tiles are sent to Jev, never the whole page.
- Screen text in an image is data: every question says so.

## Acceptance

- Unit tests: `agentic/captcha/captcha_tests/` (scan, pixel mapping, every
  loop outcome against a scripted page and Jev), `task_tests/human_tests.rs`
  (retry on solve, hand-off with what was tried, codes never sent, the loop
  bound), and the module's config and runner tests.
- Live: a browser task on Google's reCAPTCHA v2 demo page with a TypeSafe key
  ends past the captcha or at `needs_human` naming the tries.

## Thresholds

See [`../decision-thresholds.md`](../decision-thresholds.md), "Captcha solver".
