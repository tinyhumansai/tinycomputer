# The do loop

Most of a flow is plain strings: "start a new email message", "search for
flights", "calculate 128 times 37". Each one runs the same loop, for up to
eight turns (`DO_TURNS`), in `act/`. This is the loop people usually mean
when they talk about "the loop."

## One turn

```text
look ──► note what the last action changed ──► judge ──► done? ──yes──► end step
                                                  │
                                                  no
                                                  ▼
                                   blocked? ──yes──► clear the obstacle ──► next turn
                                   regressed? ─yes─► press escape, ban it ─► next turn
                                                  │
                                                  no
                                                  ▼
                                            make the chosen move ──► next turn
```

### 1. Note what changed

Before judging anything, the runtime compares a fingerprint of the screen
before and after the last action. The fingerprint deliberately leaves out
refs, the ids each snapshot mints fresh: a fingerprint that included them
would look different on every single turn, and stall detection would never
fire. If nothing changed, the element that was just pressed is banned for
the rest of the step, so the loop does not click the same dead button
twice. After three turns in a row with no change (`STALL_TURNS`), one
question (unless the run turned the completion loop off) asks whether the
screen already shows the result the step is meant to bring about (a search box that listed results as it was typed in
leaves "press search" nothing to do). If it clearly does (`DONE`), the step
ends `AlreadyDone`; otherwise it fails with the note "the last three actions
changed nothing on screen". Live, 25 of a day's 189 rescues found such a
step's work already done, each ~18 s later.

### 2. Judge

One request asks up to seven questions about the same screen:

| Id | Shape | Asks |
|---|---|---|
| `done` | yes/no | has this step been fully accomplished? |
| `not_done` | yes/no | is this step still NOT fully accomplished? |
| `progress` | five-level score | how far along is the screen, from "nothing relates to the step" to "fully accomplished"? |
| `blocked` | yes/no | is a dialog, alert, sheet, popup, or prompt that is not part of this step in the way? |
| `move` | choice | which kind of move advances the step? |
| `shortcut` | choice | if a standard keyboard shortcut would advance it, which one? |
| `helped` | yes/no | did the last action move toward the step? (only asked after an action has happened) |

Asking a question and its negation and averaging the two is a cheap way to
calibrate a model that says yes to everything: it says yes to `done` and
yes to `not_done`, and the average of the two lands near 0.5 instead of
near 1. The completion estimate is `mean(P(done), 1 minus P(not_done))`,
then averaged again with the probability `progress` puts on its top level.

### 3. Decide whether the step is over

- **Before anything has been done** (turn 0), the step is treated as
  already done only at 0.85 or above (`ALREADY_DONE`). The bar is set high
  on purpose: skipping a step that was not really done is worse than doing
  a step twice.
- **After acting**, 0.75 (`DONE`) is enough.
- **A step whose text contains "new" or "create" can never be complete
  before it acts.** An existing draft or folder already on screen belongs
  to someone else, and treating it as the fresh one the step asked for is
  how a flow ends up typing into a stranger's unsent draft. If Jev answers
  `finished` on such a step anyway, the runtime turns that answer into a
  `shortcut` or `activate` move instead of ending the step.
- **`finished` is one vote among several, not the final word.** On turn 0
  it needs the completion estimate at 0.85 to be trusted; after acting it
  stands unless the estimate is under 0.5 (`LEANS_DONE`). When it is
  overruled, the move becomes `activate` instead.

### 4. Recover, before making a move

Two checks run before the loop decides what to press next.

**Obstacles.** If `blocked` is at least 0.7 (`BLOCKED`), the runtime asks a
separate Choice: which of the visible, non-destructive, clickable elements
closes the obstacle, or press Escape. At most two obstacles get dismissed
per step (`MAX_OBSTACLES`).

**Regressions.** If progress dropped by a quarter of the scale or more
since the last action (`REGRESSION`), or `helped` came back under 0.2
(`UNHELPFUL`), the runtime presses Escape, bans the element that caused it,
and writes "that made things worse; undid it" into the run's history. At
most two undos per step (`MAX_UNDOS`), shared between the two triggers.

**Idle waits.** After two `wait` moves in a row that changed nothing
(`MAX_IDLE_WAITS`), Jev is not let choose `wait` again for the rest of the
step.

**A dialog the task opened.** When the run's own press or typing puts a
dialog (or, on the same page, a layer covering `LAYER_COVERS` more
controls) in front, that dialog is the task's next stage, not an obstacle
(`front.rs`): attention, obstacle clearing, and undo leave it open, a
covered control behind it is not offered as a move (one its own bar covers
inside it is), and its close control is offered only to a step that asks
to close something. A step that then presses inside it has served it: at
the next step it is an ordinary overlay again, so a calendar left open
after its day is cleared out of the way. A scroll, the run's own
housekeeping (a distraction cleared, a dismissal, an undo), or a press the
page refused (`NOT_ACTIONABLE`, which never reached it) opens no dialog of
the task's and answers none, and opening an address forgets it. On a browser task,
when nothing serves the step while such a dialog is in front, grounding
asks once which of the dialog's own controls answers it the way the task
wants (a format a booking button asks for before its dates), never one
that commits (yes, OK, confirm, pay, book, buy, send, delete), and presses
that without remembering it as the step's control.

**Presses.** A control pressed `MAX_REPEAT_PRESSES` times in a step is not
pressed again in it, nor a key; a scroll is no press. A named control's
copies on the other items of its list ("Add" on every product card) are
struck off once the next look shows the press changed the screen, unless
the step asks for every item or chooses several ("choose 2 adjacent
seats"); undoing that press lifts them again.

### 5. Make the move

The available moves describe things every application offers, on purpose,
so a flow never has to name a UI:

| Move | What the runtime does |
|---|---|
| `activate` | grounds one clickable element for the step, and clicks it |
| `expand` | grounds one expandable element, and expands it |
| `scroll` | grounds one scrollable element, and scrolls it |
| `shortcut` | presses the chosen standard shortcut, if Jev's probability for it is at least 0.5 |
| `wait` | waits briefly, on the theory that the application is still loading |
| `finished` | ends the step as `Done` |
| `stuck` | fails the step: nothing on screen and no shortcut can help |

Any other answer is ignored and logged. The runtime never falls back to a
click on an answer it does not recognise, because a malformed or injected
answer must fail closed rather than guess.

After any action the backend reports as successful, the runtime waits for
the surface to settle (on the browser, until the requests that change the
page end and it goes still, or only until it is still after a launch or
Escape, which fetch nothing; a short pause on the desktop) before looking
again, so the next turn's screen reflects what the action actually did
rather than the moment right before it took effect.

### Shortcuts

The shortcut list (`SHORTCUTS` in `act/mod.rs`) is deliberately short and safe:
new item, new folder, find, reply, settings, back, next field, confirm
(Return), and dismiss (Escape). None of them sends, deletes, or quits.
They are written in macOS spelling (`cmd+n`) and each surface translates
them: the desktop sends `ctrl+n` on Windows and Linux, the browser sends
`Meta+n` or `Control+n`. Return is refused while a sheet or alert is
showing, because there it presses that dialog's default button, which
might not be the one the step wants, unless the run's last action typed
into a search box (`is_search_box`): Return there runs the search. Live, a
store's search opened as a full-window sheet, and pressing Enter in it
was refused 154 times.

### The destructive check

Before any click, `is_destructive` runs (in `view/`). It refuses the click
and fails the step when any of these hold:

- the label contains a hard-to-undo word (delete, remove, send, purchase,
  buy, pay, submit, confirm, overwrite, sign out, and a few more);
- the flow's own `stop_before` phrases name the control ("sending the
  email" covers a button called "Send"); the label must start a word of the
  phrase, so "Rent" is not named by "the current bill";
- the element is an unnamed control inside a sheet, which is what the
  default button of a "Delete?" dialog looks like on some platforms;
- the screen shows payment evidence (a card number, CVV, or expiry field),
  so a button that only says "Continue" on a card form is caught too.

A tab is navigation: pressing it only shows another panel of the same page,
so a `stop_before` phrase never names it, and on a payment page choosing a
tab (say, "UPI") is filling the form, like a saved-card radio. IndiGo keeps
its flight search behind a tab labelled "Book"; a flow that stopped before
"paying for the booking" named that tab and refused it, so no step could
open the search form. A tab's own label still counts, though: the role is
only what the page claims, so a tab labelled "Pay ₹7,346" stays gated.

When a click is refused because the target is *covered* by something else
rather than because it is destructive, two things can happen. On the
browser, a result card sometimes lays a click layer, or its own text, over
its own link, and in that specific case the browser surface clicks through
at the link's position, but only when the exact target sits in the same
card as the cover and no dialog is involved. Anything else that comes back
covered gets a layer's own closer or Escape pressed once, then the runtime
looks again and retries the very same already-vetted target, whether that
is a `do` step's click or a `pick`'s click. When the browser says what
still covers it is an empty layer (the backdrop a popup or a box's list of
suggestions leaves over a page), that layer is pressed once where nothing
pressable lies beneath it (`Surface::dismiss_cover`, as a person clicks
outside a popup), and the target is retried once more. None of these
chooses a new element: nothing that dismissing the cover exposes gets
pressed without going through grounding and the destructive check again,
on a later turn. The history names the cover as the browser says it
(`button "Select Location"`, `an empty layer`), and a press still refused
ends the step's failure note with "its press of … was refused because …
lies over it", so a rescue deals with the cover rather than redo the steps
before it.

### Ending a dismissal cleanly

A dismissal the completion judge would otherwise never get to see ends the
step immediately, in one specific case: when the last action pressed a
control whose own words the step's intent names ("Accept Essential Only"
for a step about accepting cookies), or the intent asks to dismiss, close,
accept, decline, reject, or skip a banner, dialog, popup, cookie notice,
modal, overlay, or prompt, and the screen has returned to the application's
own window. In that case the step ends as `Done`, because a closed overlay
leaves nothing on screen afterward for a judge to read. The same check runs
once more after the very last turn, so a dismissal that lands on the last
permitted turn is not reported as a failure just for want of one more
look.

### Running out of turns

After eight turns with no end, the runtime looks one last time. If the
completion estimate reaches 0.75, the step is `Done` anyway; otherwise it
fails with "not accomplished after 8 turns." A long key sequence, like
several calculator presses in a row, is the clearest way to hit this: split
it into more than one step rather than relying on eight turns to cover it.

## Deliberation changes the thresholds above

Everything on this page describes the single-number bars. When
`deliberation` is on (the default), most of these bars are replaced by an
evidence gate that reads a decision's whole ballot rather than one number,
and a close call gets more chances before the runtime commits to it. See
[Deliberation](deliberation.md) and
[Undo and backtracking](undo-and-backtracking.md) for what changes and why.

See [`docs/technical/decision-thresholds.md`](../../../technical/decision-thresholds.md)
for the exact values and file locations of every constant named above.
