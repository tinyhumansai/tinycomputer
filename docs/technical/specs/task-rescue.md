# Task rescue: a reasoning model for failed steps

Status: implemented. Plan: [`../plans/task-rescue.md`](../plans/task-rescue.md).

## Why

A task fails when one step fails. A step fails when Jev can't find what it
describes on the screen. Live runs
([`../evals/2026-09-28-deliberation.md`](../evals/2026-09-28-deliberation.md))
failed steps for reasons a moment of reasoning could get past:

- a calendar left open over the form;
- a control the page labels differently from the step;
- a step that must be split in two.

Jev answers small, framed questions about the screen. It doesn't plan. A
person reading the failure note and the screen would try something
different. The rescuer is that person: a reasoning model, asked only after a
step has failed, whose answer is new steps rather than an action.

## What

When a task's top-level step fails and the failure is recoverable, the task
controller first checks for a wall only a person can pass. If there is none
and a rescuer is configured, it asks the rescuer for guidance before it
fails the task.

- **Briefing.** The goal, the standing rules Jev is briefed with, the running flow with the failed step marked, the
  failure note, the step reports of the run, and earlier rescues with their
  outcomes. Also the screen's visible text, cut to 8,000 characters and
  wrapped as `untrusted_accessibility_data`, and the variable names, secret
  ones marked.
  - Every fact value is redacted from all of it, including the goal and the
    screen.
  - The rescuer sees names only, like the planner.
- **Answer.** One JSON object:
  - `{"action": "retry", "reason", "steps", "covers"}` gives 1 to 6 flow
    steps (`MAX_RESCUE_STEPS`) to run in place of the failed one. `covers`
    (default 0) is how many of the steps right after the failed one they
    also do. Those steps are dropped rather than run twice, as when one
    rescue fills fields the plan filled in three steps. A covered step may
    never hold a `stop_before`, at any depth, and `covers` may not run past
    the end of the flow; either answer goes back as invalid. Guidance whose
    last step runs the failed step again (the same action, or the same
    `do` intent in other case or spacing) covers only the following steps
    its earlier steps do (the same step, or an `enter` filling one of their
    fields), counted from the failed step on, whatever its `covers` says.
  - When the failed step is itself a `stop_before` (the control it guards
    was not found), the guidance must hold a `stop_before` too, so a rescue
    can move the guard to where the page puts it but never remove it.
  - `{"action": "skip", "reason", "covers"}` runs nothing in place of the
    failed step: the screen is already past it, as when a run reached the
    payment page while a step still looked for the seat page. The flow goes
    on from the next step not covered. The same guards hold: a failed or
    covered `stop_before` is never skipped, and a skip that leaves nothing to
    run goes back as invalid.
  - `{"action": "give_up", "reason"}` is for when no step can help: the site
    blocks, a person must act, or the goal is out of reach.
- **Validation.** The guidance, followed by the steps after the failed one,
  is checked with the flow validator `RunFlow` uses, against the task's
  known and secret names. That means:
  - no new variables;
  - secrets only as `enter` values;
  - every rule the planner's flows meet.

  An invalid answer goes back with its errors, up to `REPAIRS` (2) times.
- **Resume.** The task runs a new flow: the guidance, then every step after
  the failed one and the covered ones, unchanged. It runs on the same app, variables, and
  `allow_destructive`, from what the budget has left. `stop_before` guards
  are kept, and the flow runtime gates irreversible presses as always. A
  later failure, including one in the guidance, can be rescued again.
- **Limits.**
  - At most `MAX_RESCUES` (5) rescues per task; 3 until live runs spent all three on one form. `budget.max_rescues` may
    lower this, and `0` turns rescues off.
  - One rescue may think for at most `RESCUE_TIMEOUT_MS` (2 minutes), and
    never past `max_elapsed_ms`. Its time counts toward that budget.
- **Failure.** A rescue that gives up, errs, times out, or never gives valid
  guidance leaves the task `failed` with its original reason. The hint then
  reads "the rescuer gave up: …". Once the rescues are spent, the task fails
  exactly as it did before rescues existed.
- **Record.** `TaskReport.rescues` lists every rescue: the step, the failure,
  the reason, the steps, how many following steps they covered, and the
  outcome. Outcomes are:
  - `running`;
  - `recovered`, when every guidance step finished or reached its approval;
  - `failed_again`;
  - `gave_up`.

  While a rescue is under way the task stays `running`, with summaries
  "Step n failed; asking for guidance (rescue k of 5)." and then "Rescue k
  of 5: \<reason\>".

## What it does not do

- **Act.** Jev still decides every action on the screen.
- **Rescue walls, invalid flows, or exhausted budgets.** A person comes
  first, and a budget or validity failure is not the page's fault.
- **Rescue a browser that cannot start.** A step whose last action was
  refused `BROWSER_UNAVAILABLE` fails the task at once, not recoverable, with
  a hint to give the path of Chrome or Chromium (`browser_executable`): no
  step a rescue writes starts one. Live, four rescues of it spent minutes
  before the rescuer gave up.
- **Run under `RunFlow` alone.** Rescues belong to the task controller, so
  the flow runtime keeps one door, to Jev.
- **Rewrite later steps.** It may drop the steps its own guidance already
  does, but never reword or reorder the caller's plan after them, and never
  drop a guard.
- **Rescue nested steps.** A failure inside an `if` or `repeat_until` stops
  the task as before, because the resume point of a nested step cannot be
  recovered from its path.

## Configuration

The planner's private configuration brings the rescuer, on the same route
and key (OpenRouter, or Tiny Humans' gateway with `provider: "tiny_humans"`)
unless `rescue_route` gives it a complete route of its own (contract 2.8). Its model is `rescue_model`, with default `openai/gpt-6-luna`
(`RESCUE_MODEL`). It is asked with `reasoning.effort = low`, no temperature,
JSON-object replies, and at most 8,000 output tokens. `Describe` reports
`rescue_configured`, and since 2.8 `rescue_model`: the rescuer's provider and
model.

## Contract

Contract 2.4 (minor) adds:

- `TaskBudget.max_rescues`;
- `TaskReport.rescues` (`Rescue`, `RescueOutcome`), omitted when empty;
- `Capabilities.rescue_configured`, false when absent.
