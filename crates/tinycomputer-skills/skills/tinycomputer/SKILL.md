---
name: tinycomputer
description: Hand a task to the tinycomputer module and follow it to completion across web pages and desktop applications. Use when a person asks you to do something on their computer or on the web for them, such as finding and booking travel up to payment, filling a form, or drafting an email in Mail.
---

# tinycomputer

tinycomputer runs a task for you on the person's computer: in a browser, in desktop applications, or across both. You describe *what* should happen; it works out *how* on the live screen, and pauses only for what you or the person must decide.

## The loop

1. `Describe` once. It returns what is available (desktop, browser, planner), the flow guide, every task member's input schema, worked examples, and `catalogue`: every member the module serves, with its family and one sentence on what it is for.
2. `StartTask` with the task. It returns at once with a task id.
3. `AwaitTask` until the status is not `running`.
4. Act on the status (below), then go back to step 3 until the status is final.
5. `TaskReport` for the details: every step, what was read, and the extracted records.

Every reply is `{ok, data}` or `{ok: false, error: {code, message, hint, recoverable}}`. When a call fails, follow the `hint`. A `TaskView` always carries a one-sentence `summary` and `next`: the calls that make sense now.

## Starting a task

- **With a planner configured** (`Describe` says `planner_configured`): pass `task` in plain words.
- **Without one:** write a `flow` from the guide and pass that. A flow says what to accomplish, step by step, and never how: no button names, menus, or shortcuts.
- **Facts:** pass the person's details (names, date of birth, email, phone) as `facts`, and refer to them in a flow as `${name}`. They brief the module's decision model so it knows whom it is acting for, and are typed on the person's machine.
- **Secrets stay templates.** A card number, CVV, passport number, password, or one-time code is secret: the decision model only ever sees `${name}`, and a flow may use it only as an `enter` value. Name any other secret in `secret_facts`.
- **Never pay on the person's behalf.** By default the task stops at payment for the person to finish. With `constraints.payment` set to `fill_then_approve` (and `origins` listing the sites, never `*`) it fills the card form from secret facts, then waits for `ContinueTask.approve` before pressing pay — ask the person before approving.
- **Want the answer in a fixed shape?** Pass `output`: `instructions` in plain words and, optionally, a JSON `schema` (a subset: `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `minItems`, `maxItems`; an object at the top). The finished task returns `done.result` matching it, beside the raw `records`. Needs the planner (`Describe` says `output_configured`).
- **Constraints:** `constraints.surfaces` limits where the task may act; `constraints.origins` limits which sites it may open pages on (`*` for any public site; a page's own files load from wherever it takes them). `allow_destructive` lets it send, delete, or confirm without asking; leave it off unless the person said so. Which browser binary and profile a task uses is the host's setting, never yours.

## Statuses and what to do

| Status | Meaning | Do |
|---|---|---|
| `running` | working | `AwaitTask` again |
| `needs_input` | values it does not have | ask the person for `fields`, then `ContinueTask` with `inputs` |
| `needs_approval` | about to do something irreversible (`action`, `target`) | ask the person; `ContinueTask` with `approve: true` or `false` |
| `needs_human` | a captcha, a one-time code, or a login only a person can pass | tell the person `reason`; once they are done, `ContinueTask` with `answer: "done"` |
| `checkpoint` | stopped, usually at a payment page | tell the person the `summary` and where it stopped; they finish from there |
| `needs_plan` | plain-language task, no planner | write a flow with `guide` and start again with `flow` |
| `done` | finished | report `answer`, or `result` when you passed `output`; `records` holds anything read or extracted |
| `failed` | could not finish; a configured rescuer already tried up to five times | explain `reason`; if `recoverable`, try again changed as `hint` says |

## Looking closer, or driving the browser yourself

Tasks are the way in; the primitives are there when you need to look or act directly. They reply `{ok, command, data}` or `{ok: false, error: {code, message, suggestion, recovery}}`, the same on the desktop and in the browser: `STALE_REF` always means take a fresh snapshot and choose again, and a `recovery` with `retryable: false` means do not repeat the call as it was.

- `BrowserListSessions` shows every open browser session, a running task's included.
- `BrowserOpenSession`, then `BrowserNavigate` and `BrowserSnapshot` with `{"session": id, …}`; act on a ref with `BrowserPerform`, such as `{"session": "s-1", "action": "click", "target": {"kind": "ref", "value": "e3"}}`.
- `BrowserScreenshot` returns an output id, not an image: read it with `BrowserReadOutput` from `offset` 0 until `eof`, then `BrowserReleaseOutput`. The same works for a screenshot in `TaskReport.artifacts`: a task keeps one each time it stops, and a task view never carries one.
- `BrowserCloseSession` when you are done with a session you opened; leave a task's session to the task.

## Writing flows that work

- **One idea per step.** "search for flights", then "open the cheapest result", not both in one step.
- **`browse`** opens a web address; **`open`** switches to a desktop application.
- **`enter`** fills several fields at once, keyed by what each field is for: `{"enter": {"where from": "Delhi", "email": "${email}"}}`.
- **`pick`** chooses from a list and opens the choice. Prices, times, durations, and stops are compared exactly: `{"pick": {"from": "the flight results", "by": "lowest price", "into": "flight"}}`.
- **`extract`** captures a whole list.
- **`read`** captures one piece of text.
- **Always end a purchase or booking with `stop_before`** paying. End a message you draft with `stop_before` sending.

## Example

```json
{"member": "StartTask", "args": [{
  "flow": {"app": "browser", "steps": [
    {"browse": "https://www.google.com/travel/flights"},
    {"enter": {"where from": "${from}", "where to": "${to}", "departure date": "${date}"}},
    "search for flights",
    {"wait_for": "flight results are listed"},
    {"pick": {"from": "the flight results", "by": "lowest price", "into": "cheapest"}},
    "continue to booking",
    {"enter": {"first name": "${first name}", "last name": "${last name}", "email": "${email}", "phone": "${phone}"}},
    {"stop_before": "paying for the booking"}
  ]},
  "facts": {"from": "Delhi", "to": "Srinagar", "date": "14 October",
            "first name": "Asha", "last name": "Raina", "email": "asha@example.com"}
}]}
```

This pauses with `needs_input` for `phone`, runs to the payment page, and stops at a `checkpoint` naming the cheapest flight.
