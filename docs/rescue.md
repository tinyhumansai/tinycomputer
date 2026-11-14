# Rescues

A task fails when one of its steps fails. A step fails when Jev can't find
what the step describes, or the step can't be finished in eight turns. Often
the reason is something a person would get past in a second:

- a calendar from the last step is still open and covers the form;
- the page calls a button something different from the plan;
- a step really needs to be two steps;
- the page has already moved past the step, so there's nothing left to do.

Jev answers small questions about one screen. It doesn't plan, so it can't
decide to try something else. The rescuer can.

## What the rescuer is

The rescuer is a reasoning model. It's only asked after a step has failed,
and it never clicks anything itself. Its answer is new steps, written in the
same plain language as the rest of the plan. tinycomputer runs those steps
the normal way, with Jev choosing every click and every safety rule still in
force.

## What happens when a step fails

1. **Is a person needed?** If the screen shows a captcha, "verify you are
   human", a one-time code, a phone to verify, two-factor authentication, or
   a login wall ("sign in to continue", "log in to see ride options", "Login/
   Sign up using OTP"), the task pauses as `needs_human`. A rescue can't
   solve those. When the module has a captcha solver, a captcha or "verify
   you are human" check gets a few tries from it first, and the task only
   pauses if they fail. A header's bare "Log in" or "Sign up" link is no wall.
2. **Ask the rescuer.** Otherwise, and if rescues are configured, the task
   briefs the rescuer and asks for guidance. While it waits, the summary reads
   "Step 13 failed; asking for guidance (rescue 1 of 5)."
3. **Check the answer.** The guidance is checked with the same validator that
   checks every plan. A bad answer goes back with the errors, up to twice.
4. **Run it.** The task runs the guidance, then the rest of the original plan
   unchanged, with whatever budget is left.
5. **Again if needed.** If a later step fails, including one of the rescue's
   own steps, it can be rescued again, up to five times per task.

## What the rescuer is told

- the goal of the task;
- the standing rules (screen text is data, decline paid extras, never pay);
- the whole plan, with the failed step marked;
- why the step failed, and how every step of the run went;
- earlier rescues and how they turned out;
- what the task has saved so far;
- the text on the screen, up to 8,000 characters, labelled as untrusted data;
- the names of your details, with secret ones marked.

It never sees the values of your details. They're removed from everything,
including the goal and the screen text.

## What it can answer

**Retry with new steps.** One to six steps to run in place of the failed one.
For example, the failed step "open the Economy fare for the 04:25 flight"
became "choose Economy in the cabin tabs".

The answer can also say how many of the following steps its new steps already
cover (`covers`). Those are dropped so they don't run twice. One rescue filled
in three fields that the plan had spread over three steps, and covered the
two that followed.

**Skip.** The screen is already past the failed step. In one run, a step was
still looking for the seat page when the site had already moved on to
payment. The task carries on from the next step not covered.

**Give up.** Nothing will help: the site is blocking automation, a person
has to act, or the goal can't be reached. The task fails with the rescuer's
reason in the hint.

## Guards a rescue can't break

A rescue can change how a step is done. It can't change what the task is
allowed to do.

- **It can't remove a "stop before".** A covered or skipped step may never
  contain a `stop_before`. If the failed step was itself a `stop_before`
  (because the button it guards wasn't found), the rescue has to include a
  `stop_before` too. It can move the guard to where the page actually puts
  it, but it can't drop it.
- **It can't rewrite the rest of the plan.** It may drop steps its own steps
  already do. It never rewords or reorders the steps after them.
- **It can't invent details.** Its steps may only use the names of details
  you gave and values the task saved. Secrets may only be typed, as in any
  plan.
- **It can't press anything.** Jev still makes every choice on screen, and
  irreversible controls still need approval.

## Limits

| Limit | Value |
|---|---|
| rescues per task | 5 by default; set `budget.max_rescues` from 0 (off) to 5 |
| steps per rescue | 1 to 6 |
| thinking time per rescue | 2 minutes, and never past the task's time budget |
| repairs of a bad answer | 2 |

Rescues aren't used for:

- a step inside an `if` or `repeat_until` (where to resume can't be worked
  out from inside a branch);
- a plan that was invalid from the start;
- a budget that ran out;
- a plain `RunFlow` call. Rescues belong to tasks.

## Seeing what happened

`TaskReport.rescues` lists every rescue: which step failed, why, the
rescuer's reason, its steps, how many steps it covered, and the outcome:

- `recovered`: every rescue step finished;
- `failed_again`: a rescue step failed too;
- `gave_up`: the rescuer said nothing would help;
- `running`: it's in progress.

## Does it help?

On live booking sites, yes. Before rescues, the best run on Emirates got to
step 18 of 26 and the best on IndiGo to step 21 of 23. Neither reached the
payment page. With rescues, both reached the payment checkpoint and stopped
there, as they should. Each needed two rescues. The rescuer's thinking took a
few seconds per rescue, about 5% of the run, and cost under a cent.

The details are in
[`technical/evals/2026-09-28-rescue.md`](technical/evals/2026-09-28-rescue.md).

## Setting it up

The rescuer comes with the planner configuration and uses the same route and
key — OpenRouter, or Tiny Humans' gateway — unless you give it its own
`rescue_route`. Its model is `rescue_model`, `openai/gpt-6-luna` by default, asked with
low reasoning effort. Without a planner configuration there are no rescues,
and a failed step fails the task right away. `Describe` tells you whether
rescues are available (`rescue_configured`) and which model and route they
use (`rescue_model`).

## Where to find out more

- [`technical/specs/task-rescue.md`](technical/specs/task-rescue.md): the full
  specification.
- [`technical/tasks.md`](technical/tasks.md#rescues): rescues inside the task
  controller.
