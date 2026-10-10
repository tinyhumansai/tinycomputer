# Giving it a task

A task is the simplest way to use tinycomputer. You describe a job in plain
words, hand over the details it needs, and it works in the background until
it finishes or needs you.

## A task from start to finish

Say you want a flight booked up to the payment page. You send one request:

```json
{"member": "StartTask", "args": [{
  "task": "Find the cheapest one-way flight from Delhi to Srinagar on 14 October and fill in my details up to payment",
  "facts": {"first name": "Asha", "last name": "Raina", "email": "asha@example.com"},
  "constraints": {"surfaces": ["browser"]}
}]}
```

- `task` is the job, in plain words.
- `facts` are your details. They get typed into forms. How they are kept
  private is covered in [Safety and privacy](safety-and-privacy.md).
- `constraints` limit what the task may touch. Here, only the browser.

It answers right away with a task id. The work happens in the background.

From then on you ask "how is it going?" with `AwaitTask`. That call waits up
to a minute and comes back as soon as something changes. Each answer has:

- a one-sentence `summary` of where things stand;
- the current step and a progress number between 0 and 1;
- `next`, a list of the calls that make sense right now.

If you only ever do what `next` suggests, you can't get the protocol wrong.

A typical booking ends like this:

> Stopped: reached the payment step (Pay now); payment is always left to you.

The browser stays open on the payment page, so a person can finish paying.

## When it stops and asks you

A task pauses for a small number of reasons. Each one tells you what it
needs.

| It says | Because | You answer with |
|---|---|---|
| `needs_input` | the plan needs a detail you didn't give, like a date of birth | `ContinueTask` with the missing values |
| `needs_approval` | it found something that can't be undone, like "Send" or "Delete" | `ContinueTask` with `approve: true` or `false` |
| `needs_human` | there's a captcha, a one-time code, or a login wall | a person handles it, then `ContinueTask` |
| `checkpoint` | it reached the payment page and stopped there | a person pays; `CancelTask` releases the browser |
| `needs_plan` | you gave plain words but no planner is set up | start again with a written flow |
| `done` | every step finished | `TaskReport` for the full story |
| `failed` | a step failed and couldn't be rescued, or a budget ran out | `TaskReport`, then try again |
| `cancelled` | you cancelled it, or declined an approval | nothing |

When you approve something, the task presses that one control and carries on
with the rest of the plan. When you decline, it stops.

When it needs a person, it waits. Solve the captcha or type the code in the
open browser, then call `ContinueTask`. It retries the step that was blocked
and keeps going.

## Before it starts: the plan

With a planner configured, plain words are enough. The planner, a language
model, turns your task into a flow: a list of steps like "search for one-way
flights from Delhi to Srinagar on 14 October", "pick the cheapest result",
"enter the passenger details", and "stop before paying".

The planner follows a few rules:

- it writes your details only as placeholders like `${first name}`, never the
  values themselves;
- it never invents a detail you didn't give; it asks for it instead;
- it ends any purchase with a "stop before paying" step, and guards sending,
  deleting, and submitting the same way.

Every plan is checked before it runs. If the check finds a mistake, the plan
goes back to the planner with the errors, up to twice.

Want to see the plan first? `PlanTask` returns the plan and any questions
without running anything. You can read it, edit it, save it, and pass it to
`StartTask` as `flow`. Saved plans are covered in
[Memory and saving](memory-and-saving.md).

Without a planner, you write the flow yourself. See
[Writing flows](writing-flows.md).

## Limits you can set

### Budgets

A budget caps how much a task can do, across all of its runs:

- `max_actions`: clicks, key presses, and typing (default 120);
- `max_model_calls`: questions asked of Jev (default 6000);
- `max_elapsed_ms`: how long it may work. Time spent waiting for you doesn't
  count;
- `max_rescues`: how many times a failed step may be rescued, 0 to 5
  (default 5).

Pausing for your approval never refills the budget. The task picks up with
whatever is left.

Two more settings trade speed for care:

- `deliberation` sets how hard it thinks about each decision: `deep` (the
  default), `standard`, or `off`. See [How it decides](how-it-decides.md).
- `strategy` sets how questions are asked: `narrow` (the default) asks many
  small questions, `wide` asks one bigger question per turn over a summary of
  the screen.

### Where it can go

- `surfaces`: `browser`, `desktop`, or both. A task limited to the browser
  can't reach your desktop apps at all, even by mistake.
- `origins`: the websites it may open pages on, such as
  `https://.goindigo.in` for a site and its subdomains, or `*` for any public
  site. Pages are checked, not the pictures and scripts a page loads from
  elsewhere. This is a guard rail rather than a sandbox.
- `browser_endpoint`: use your own running Chrome instead of starting a new
  one. Booking sites often turn away a fresh automated browser but serve a
  person's own. When the task ends, it disconnects and leaves your browser
  open.
- `headed`: show the browser window so you can watch.
- `browser_executable`: the full path of the Chrome to start, when it isn't
  where the module looks.
- `browser_profile`: a folder, given as a full path, the browser keeps its
  profile in, so a site you signed into once stays signed in for the next
  task.

Your app sets these two from its own settings; they don't go with
`browser_endpoint`, which starts no browser.

### How it treats irreversible actions and payment

- By default it pauses for approval before anything that can't be undone,
  and it stops for good at payment.
- `payment: "fill_then_approve"` lets it fill in the payment form from secret
  details you gave it, then pause for your approval before pressing Pay. This
  needs `origins`, so card details are only ever typed on sites you named.
- `allow_destructive: true` lets it press irreversible controls, payment
  included, without asking. Use it with care.

## When something goes wrong

If a step fails and no person is needed, the task doesn't give up at once.
It asks the rescuer, a reasoning model, for another way through, up to five
times. Only when the rescuer gives up, or the rescues run out, does the task
report `failed`. See [Rescues](rescue.md).

## After it's done

`TaskReport` gives you the whole story:

- every step, how it ended, and a note on why;
- anything the task read, like the price of the flight it picked;
- every rescue, with what the rescuer suggested and whether it worked;
- what the task learned about the site, which can make the next run faster
  (see [Memory and saving](memory-and-saving.md)).

If you want the results as clean JSON rather than raw screen text, pass
`output` with `StartTask`: instructions in plain words and, optionally, a
JSON Schema. When the task finishes, the result comes back in that shape as
`done.result`. See [Memory and saving](memory-and-saving.md#getting-results-back-in-your-own-shape).

`ListTasks` shows every task the module holds. `CancelTask` stops one and
releases its browser.

## Where to find out more

- [`technical/tasks.md`](technical/tasks.md): the task controller in detail.
- [`crates/tinycomputer-skills/skills/tinycomputer/SKILL.md`](../crates/tinycomputer-skills/skills/tinycomputer/SKILL.md):
  the guide written for agents that call the task API.
