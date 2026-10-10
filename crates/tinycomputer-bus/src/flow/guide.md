# Writing a desktop flow

A flow is a short JSON script that says **what** to accomplish in one desktop
application. It never says **how**: no button names, no menu paths, no keyboard
shortcuts, no coordinates. The desktop module works out the how on the live
screen, one small decision at a time.

Write a flow the way you would brief a capable person who has never used the
app: a handful of plain steps, each describing a state to reach or a thing to
do.

## Shape

```json
{
  "app": "TextEdit",
  "vars": { "greeting": "Hello from a flow." },
  "steps": [
    { "open": "TextEdit" },
    "start a new blank document",
    { "enter": { "document text": "${greeting}" } },
    { "verify": "the document shows the greeting text" }
  ]
}
```

- `app` is the application's name as the operating system shows it, or
  `browser` for a flow that starts on the web.
- `vars` are optional named values; any step text may use `${name}`.
- `steps` run in order. A step is a plain string or an object with one key.

## Steps

| Step | Example | Meaning |
|---|---|---|
| string | `"open the Liked Songs list"` | Reach the described state. |
| `open` | `{"open": "Mail"}` | Launch the app or bring it forward. |
| `browse` | `{"browse": "https://www.google.com/travel/flights"}` | Open a web address in the browser; later steps act on the page until an `open` switches back to an app. |
| `do` | `{"do": "start a new note"}` | Same as a plain string. |
| `enter` | `{"enter": {"subject": "Hi"}}` | Put each text into the field its key describes; a box that suggests matches as you type (a location, a city) has the matching suggestion picked. |
| `choose` | `{"choose": {"what": "the font list", "option": "Helvetica"}}` | Pick an option in a list, menu, or popup, or in a group of option buttons (a size, a colour, a quantity, a day in a strip of dates). |
| `read` | `{"read": {"what": "the newest message's subject", "into": "subject"}}` | Store visible text in a variable. Read the price of one item before any step raises its count: after that, its line and the cart show the total for all of them. |
| `extract` | `{"extract": {"what": "the flight results", "into": "flights"}}` | Store every item of a list, as JSON rows of their text, in a variable. |
| `pick` | `{"pick": {"from": "the flight results", "by": "lowest price", "into": "flight"}}` | Choose the best of a list of results (cards or rows, each an item to open) and open it; `into` stores its text. Prices, times, durations, stops, and nearness to a number ("closest to 9", for a size when 9 may be sold out) are compared exactly. |
| `verify` | `{"verify": "the draft shows a recipient"}` | Fail the flow unless this holds. |
| `wait_for` | `{"wait_for": "the search results are showing"}` | Wait until this holds. |
| `stop_before` | `{"stop_before": "sending the email"}` | Find an irreversible action and stop in front of it. |
| `repeat_until` | see below | Repeat steps until a condition holds. |
| `if` | see below | Branch on a condition. |

```json
{
  "app": "Finder",
  "steps": [
    { "open": "Finder" },
    "show the Desktop folder",
    { "if": {
        "condition": "a folder named tinycomputer-lab is visible",
        "then": [ "open the tinycomputer-lab folder" ],
        "else": [ "create a new folder",
                  { "enter": { "folder name": "tinycomputer-lab" } } ]
    } }
  ]
}
```

```json
{
  "app": "Calculator",
  "steps": [
    { "open": "Calculator" },
    "clear the calculator",
    { "repeat_until": {
        "condition": "the display shows 4736",
        "steps": [ "calculate 128 times 37" ],
        "max": 2
    } }
  ]
}
```

## Writing good steps

1. **Describe outcomes, not clicks.** Write "start a new email message", not
   "click the compose button". The module finds the button, the menu item, or
   the shortcut.
2. **One idea per step.** "Open the Sent mailbox" and "open the newest message"
   are two steps.
3. **Name fields by purpose in `enter`.** Use `recipient`, `subject`,
   `message body`: what the text *is*, not where you guess it goes.
4. **Put all text in the flow.** The module chooses and acts; it never writes
   prose. Every word that should end up on screen belongs in an `enter` value.
5. **End with `verify`** for anything that matters, and **guard irreversible
   actions with `stop_before`** (sending, deleting, buying, submitting). The
   caller decides separately whether those may run. A `stop_before` comes
   where the flow would take that action, at its end; never one for
   logging in: a page's header offers its login button on every page, and
   a login wall pauses the task for a person by itself.
6. **Do not guess the interface.** If you are unsure whether a panel is open,
   say what you need ("show the formatting options"); do not script how to get
   there.
7. **Shared details may be named; secrets are only typed.** A `${name}` for a
   shared detail (a name, a date of birth, an email) may appear in any step:
   "choose ${title} in the title field" is fine, and helps the module choose.
   A `${name}` for a secret (a card number, a CVV, a passport number, a
   password, a one-time code) may appear only as an `enter` step's value.
   Putting a secret anywhere else — an `open` application name or `browse`
   address, a `do`, `verify`, `wait_for`, or `stop_before` text, a `choose`'s
   `what`/`option`, a `read`/`extract`'s `what`, a `pick`'s `from`/`by`, a
   condition, or an `enter` slot's own name — fails validation. The module
   reasons about all of that text, and it only ever sees a secret's name.
8. **Name a variable as `${name}`, and verify what the screen shows.** A step
   that reads a variable writes `${cheapest_flight}`, never `cheapest_flight`,
   so its value is shown; a bare identifier fails validation. A `verify` or
   `wait_for` must be checkable on the current screen alone — never "matches
   what the other site showed": `pick` already ranks. Nor a change: "the grid
   has updated after sorting" cannot be seen on one screen, and is judged
   false over it; name what shows once it has happened, such as "the sort
   shows Price: Low to High". Never name a `pick`
   variable in a `verify`, `wait_for`, `repeat_until`, or `if` condition: it
   holds the whole item's text, which the opened item seldom shows again, so
   the check fails a pick that worked; a pick already fails when nothing
   fits, and validation rejects it. A `choose` option is
   the label the page shows ("Saver"), not a description ("the cheapest
   fare"); choosing among results by a criterion is what `pick` is for. A
   size, colour, or quantity is a `choose` of the shortest label the page
   is likely to show (`"option": "9"` for "UK size 9"), never a `pick`:
   option buttons are not results, and a `pick` over them fails. A filter
   on which result to take ("skip Sponsored items", "rated 4 stars or
   more") belongs in that `pick`'s `from` or `by`, never in a step of its
   own: there is nothing on screen to do for it, so such a step fails.
   For the same reason, a pop-up that may or may not show (a login,
   cookie, or offer pop-up) is closed in an `if` on it being visible,
   never in a plain step.
   Keep the task's own words in a `pick`: its `from` names the item the
   task asks for ("the results for <the item named>", not just "the search
   results"), and its `by` is the task's criterion, "first" when the task
   says the first one, never a stand-in such as "lowest price": a list
   often holds other items beside the one asked for. Keep every condition
   the task puts on the kind of item in that `from`: "the cheapest car"
   picks `from` "the car options" `by` "lowest fare", since a ranking by a
   measure reads only that measure, and the list may hold other kinds.
   A button that starts a booking or a purchase often opens a dialog that
   asks a question first (a format, a language, a quantity) before what
   comes next is offered: when a step finds such a dialog in front, answer
   its question and press its own continue button before going on. A
   picker drawn as a picture (a map, a chart) lists no controls to press;
   when the page offers an accessible alternative (a list or an
   accessibility view of the same choice), open it and choose from it. A plain
   step (`do`) presses, scrolls, and waits; it never types. Text to type
   goes in an `enter` step first: to search, `enter` the query into the
   search box, then press search or Enter in a step of its own (on a page
   that lists results as the query is typed, that step finds its work
   done). A
   quantity shown as a number between − and + buttons is no list to
   `choose` from: set it with a plain step ("increase the quantity to 2"),
   which presses + until the count reads it. Most stores show those
   buttons only once the item is in the cart, so to buy more than one,
   add the item first and raise its count in the next step; a − count +
   stepper where the add button was means the item is in the cart with
   that count. A cart button in a page's header shows only how many items
   the cart holds, never which: to check an item went in, look for its
   stepper, never wait for the cart button to show it. To report the
   price of one, `read` it before raising the
   count: after that, the item's line and the cart show the total for all
   of them, and a cart often shows no price for one. A
   `pick` opens a whole result card; to press one of several buttons inside
   the cards (a time or a slot listed under each place), use a plain step
   that names it ("press the earliest time listed"). A dialog's headings
   group its buttons and are not answers: answer with one of its buttons
   (a format such as "2D", not the language heading above it). A choice
   made by picking a suggestion or an option is set once picked: add no
   step to confirm or save it, unless the task or page names a confirm
   button, and never repeat the choice. A store that delivers to an
   address may list its products only once a delivery place is set, and
   until then often shows just a location button in its header. When the
   task names a place to deliver to, set it right after opening the store,
   in plain steps of their own rather than an `if` on a prompt showing:
   open that button and `enter` the place into the location box. When the
   task only says to use the current location if asked, do it where the
   page asks, with the page's own "use my current location" control, and
   go on without a place when there is no such control: never wait for a
   place to be set. A film's, show's, or stay's page often offers its
   dates, times, and seats only once its booking button is pressed: plan
   that press as a step of its own before choosing a date. Seats are chosen on the seat map with a
   plain step that names the section and the count ("choose 2 adjacent
   available seats in the cheapest section"), never a `pick`: a seat map's
   price list names sections, and has nothing to press. A day in a strip
   of dates is a `choose` of that day once the strip shows, never taken as
   chosen because it is on screen: a strip opens on today.

## A full example

```json
{
  "app": "Mail",
  "vars": { "to": "sam@example.com" },
  "steps": [
    { "open": "Mail" },
    "start a new email message",
    { "enter": {
        "recipient": "${to}",
        "subject": "Moving Thursday's sync",
        "message body": "Hi Sam,\n\nCould we move Thursday's sync to Friday at 3pm?\n\nThanks,\nAlex"
    } },
    { "verify": "the draft shows the recipient, the subject, and the message body" },
    { "stop_before": "sending the email" }
  ]
}
```

## Across the web and an application

A flow may move between surfaces: `browse` switches to the browser, `open`
switches back to an application. Values captured with `read` on one surface
can be typed on the other.

```json
{
  "app": "browser",
  "vars": { "from": "Delhi", "to": "Srinagar", "date": "14 October" },
  "steps": [
    { "browse": "https://www.google.com/travel/flights" },
    { "enter": { "where from": "${from}", "where to": "${to}", "departure date": "${date}" } },
    "search for flights",
    { "wait_for": "flight results are listed" },
    { "pick": { "from": "the flight results", "by": "lowest price", "into": "cheapest" } },
    { "open": "Mail" },
    "start a new email message",
    { "enter": { "subject": "Flight to ${to}", "message body": "Cheapest option: ${cheapest}" } },
    { "stop_before": "sending the email" }
  ]
}
```
