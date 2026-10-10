# Sight: reading a page like a person

Code: `crates/tinycomputer-browser/src/surface/sight/` (`mod.rs`, `sight.js`).
Full specification:
[`../../technical/specs/browser-sight.md`](../../technical/specs/browser-sight.md).

## The problem sight solves

Before sight existed, the browser surface built its picture of a page from
agent-browser's accessibility snapshot: the roles and names the page's own
markup *declares* through ARIA and label attributes. That sounds reasonable
until you look at what real sites actually declare.

IndiGo's destination picker is nine `<div role="combobox">` rows whose
`aria-labelledby` points at nothing. Read through the accessibility tree,
that is nine unnamed text boxes that do not take text, and a flow that
trusted the tree tried to type a city name into one of them and pressed
enter, landing on the wrong destination. A field labelled only by the words
printed above it, in plain text with no `<label>` and no `aria-label`, has no
name at all as far as the tree is concerned. An icon button with no
`aria-label` is the same story. And the tree lists elements that are off
screen, or hidden behind a dialog, exactly as if they could be clicked right
now.

A person looking at the same page never reads the markup. They see what is
drawn and what is on top of what. They read the words on a control, or
beside it. They know a box takes text because it has a blinking caret, not
because some attribute says `role="textbox"`.

Sight is `BrowserSurface`'s attempt to do the same thing: build the page's
`Screen` (the same structure a decision loop reads from a desktop
application) from what is actually rendered, independent of what the page's
markup claims about itself.

## How it runs

Sight is one script, `sight.js`, evaluated in the page in a single
`evaluate` call (`sight::script` in `mod.rs` builds the call; see
`BrowserSurface::see`). It reads the whole page, or the part under a given
root ref when a flow has drilled into one container. Nothing new is added
to the page except a marker attribute on elements it has already seen
(below): no injected UI, no persistent listener.

`Perception::Sight` is the default. `BrowserSurface::observe` tries sight
first, and only reads the accessibility tree when sight fails outright or
runs into something it cannot address with a CSS selector, such as two
shadow roots showing controls, or a large frame sitting in front of the
content. `Perception::Tree`,
set with `BrowserSurface::with_perception`, skips sight entirely and always
reads through the tree; the live examples expose this as
`TINYCOMPUTER_BROWSER_PERCEPTION=tree`.

## What counts as a control

Sight decides what is a control by behavior, not by role:

- Native links, buttons, inputs, checkboxes, radios, and selects are
  controls.
- An ARIA role from a fixed list (`button`, `link`, `checkbox`, `radio`,
  `switch`, `tab`, `menuitem`, `option`, `treeitem`, `slider`, `gridcell`,
  and their `menuitemcheckbox`/`menuitemradio` cousins) is a control.
- An element that shows a pointer cursor, has a click handler, or is a tab
  stop is a control too, but only if its *parent* is not already one of
  those (so a `<span>` inside a button does not become a second control just
  because it inherits the button's pointer cursor).
- A small element (under a quarter of the window) whose React props hold a
  press handler (`onClick`, `onPress`), or whose Preact listeners (`l` once
  minified, `_listeners`) hold a click one, is a control too: a page can wire a
  plain `div` to a click with neither a cursor nor a tab stop. A handler for
  the mouse going down alone (a carousel's track) makes no control, and a
  control made only by such a handler hides nothing pressable inside it.
- Disabled controls are left out entirely, as they are in the tree. "Disabled"
  here is broader than the `disabled` property: it also covers
  `aria-disabled="true"` and a class name that *ends* in `disabled`. That last
  rule exists because plenty of date pickers grey out a past day purely
  through a CSS class, `rdrDay rdrDayDisabled` on a real calendar widget,
  with no ARIA attribute at all. A class behind a variant prefix
  (`placeholder:text-disabled`, `disabled:opacity-50`) is never read this
  way, nor for `selected` below: it styles a part or a state the element may
  not be in.
- Chosen controls read `selected` by the same reasoning: besides
  `aria-selected`, `aria-current`, `aria-checked`, and `aria-pressed`, a class
  name ending in `-selected` or `-checked` (never `unselected` or
  `not-selected`) marks one, as a store's picked size
  (`size-buttons-size-button-selected`) carries no ARIA state at all.
- A control repeated on every card ("ADD" on each product), inside a card
  that is a control itself, is described by that card's name (`in Maggi
  Double Masala 95 g ₹20`) when it has no description of its own: by its
  name alone, every copy is the same, and live, "add the first Maggi"
  pressed the first card's "ADD", on a ramen above the Maggi.

The IndiGo case above is the sharpest illustration of the next rule: **a
claimed text box that takes no text is a button, not a text box.** Sight
checks whether an element can actually receive typed input (a real
`<input>` of a text-like type, a `<textarea>`, or a `contenteditable`
region) before it will call it `textbox` or `searchbox`. A `div
role="combobox"` that wraps no real input is read as a button to press; one
that *does* wrap a real input is read as that input instead.

## What counts as drawn

Only what a person could actually see is kept. Zero-size elements,
`display: none`, `visibility: hidden`, and fully transparent elements never
appear. Two states are worth calling out on their own because they are easy
to get wrong when reading markup instead of pixels:

- **Offscreen.** A control that exists and is visible in every CSS sense,
  but sits outside the current viewport, below the fold, or scrolled past,
  carries the state `offscreen`. It is still reported (a flow may need to
  scroll to it), just marked as not currently in view.
- **Covered.** A control whose middle point is under some other element,
  such as a modal, a sticky header, or a tooltip, carries the state
  `covered`. This is not the same as being off screen: the element is on
  screen and rendered, something else is simply sitting on top of the exact
  point a click would land on.
- **Hidden from screen readers.** What the page marks `aria-hidden` is kept
  when a person plainly sees it: below the fold, or on top at its middle.
  An element that turns pointer events off itself (a seat table's number
  and status cells) is passed through by the hit test, so landing on what
  holds it counts as seeing it; a whole page a modal library turned off
  behind its dialog does not.

Covered is not always the end of the story for a click; see "click-through"
in [interacting.md](interacting.md) for what happens next.

## Naming: what a person would call it

This is the part that fixes the IndiGo bug. A control's name comes from,
roughly in order: its own tied `<label>`, then the page's `aria-label`, its
placeholder, or its title, then the words a person would read beside or
above the box (to its right, for a checkbox or radio), never a divider such
as "OR". A word-less control falls back to
`aria-label`, `title`, or an image's alt text, then the words in its own or
its icon's class/id/test-id (`close`, `search`, `menu`, and a fixed list of
similar terms sight recognizes) with the description "an icon", and for a
plain link with no words at all, where it leads, "leads to sightseeing",
read from the last segment of its `href`.

A page label that says *more* than the control's own words becomes a
description rather than replacing the name: a calendar day drawn as just
"18" whose inner element carries `aria-label="Sunday, 18 October 2026"`
keeps the short name and gets that as its description. A day that shows a
fare after its number ("23 6757") gets its inner label the same way, when
that label names a month and the day's number ("October 23, 2026").

## One control per thing a person sees

A page frequently draws two DOM elements as what a person reads as one
thing: a wrapper `<div>` around a `<button>` with identical text, or a
custom checkbox drawn next to (and visually replacing) a real, hidden
`<input type="checkbox">`. Sight collapses these into one candidate, a text
entry always wins over a wrapper around it, rather than reporting the same
visible control twice under two different refs.

## Containers: what a person sees a control *in*

Each candidate's `path` lists the containers a person would say it sits
inside: dialogs and fixed layers (see below), landmarks (`banner`,
`navigation`, `main`, `form`, and so on, from the matching HTML5 elements or
ARIA roles), named sections and groups, and the cards of a result list. Cards
get an ordinal among their same-role siblings, `listitem #3`, `row #2`, in
the same label format the accessibility tree fallback uses, so grouping and
the shared "digest" logic downstream work the same whichever perception
produced the screen. A table row with no label of its own is named by what
its cells without a control say, `row "01 Handicapped" #2`, so a row's
button reads with what the row is about.

The screen's `surface` field becomes `sheet` when a dialog covers the middle
of the viewport, or `alert` for an `alertdialog`, the same signal a flow
uses to know it is looking at a modal rather than the page underneath it.

## What takes text, decided by behavior

Whether an element takes typed text is never decided by its claimed role.
Sight (and, independently, `BrowserSurface::focused_field_is_editable` in
`surface/fields.rs`) checks the concrete element: a text-like `<input>` that is
neither read-only nor disabled, a `<textarea>`, or a `contenteditable`
region. An ARIA role alone proves nothing, measured directly on a booking
widget where every city row in an autocomplete list is a `<div
role="combobox">` that holds no text at all.

This matters at the point of action, not just observation: before typing
into a target, the surface actually focuses it and checks what ended up
holding the browser's focus (`BrowserSurface::takes_text`), because a page
can give any `<div>` a `combobox` or `textbox` role and have a fill silently
report success while nothing on the page received the text.

## Refs: how sight keeps addressing exact

Every control sight finds gets a `data-tc-seen="N"` attribute the first time
it is seen, and that mark stays on the element for as long as the element
lives. The ref sight hands back is `seen:N`; the crate addresses the same
element again, to click it, focus it, read its bounding box, or scope a
follow-up observation to it, with the CSS selector `[data-tc-seen="N"]`
(`sight::selector`). If the page removes or replaces that element, its mark
goes with it: a stale `seen:` ref then simply matches nothing, and the
action fails rather than silently landing on whatever now occupies that
spot. This is the same "stale ref fails closed" guarantee the tree gives
through agent-browser's own `@eN` refs, just implemented with a DOM
attribute instead of an internal snapshot table.

## Denoising

A person skips ads without a second thought and never sees what a page
deliberately hides from them. Sight does the same, dropping whole blocks
(the element and everything inside it) before anything reaches a flow:

- **Ads.** Frames, images, and links pointing at a fixed list of ad and
  tracking hosts (`doubleclick.net`, `googlesyndication.com`,
  `taboola.com`, `scorecardresearch.com`, and others); elements whose class
  or id contains a recognized ad word (`adslot`, `advert...`, `sponsor...`,
  and the short words `ad`/`ads`/`dfp` only when they stand alone or beside
  a real word, so `header`, `download`, `adults`, and a generated class
  like `css-1ad4k9` are never mistaken for one); elements carrying
  AdSense's or Google Publisher Tag's own data attributes; a label whose
  whole text is "Advertisement", "Sponsored", or "Ad", together with the
  smallest surrounding block that holds it (never a landmark, a form, a
  dialog, or more than 40% of the viewport); and 1x1 tracking pixels.
- **Blank boxes.** An element that is clickable only because of a pointer
  cursor, a tab stop, or a click handler, and has no words, no name, no
  picture, and nothing inside worth acting on, is dropped. Native controls
  and anything with a genuine ARIA role are never dropped this way, even
  when empty.
- **Hidden content.** `inert` elements; visually-hidden screen-reader text
  that is still laid out but clipped to nothing (`clip: rect(0 0 0 0)`,
  `clip-path: inset(50%)`, or a 1x1 box with `overflow: hidden`);
  `aria-hidden="true"` elements that a person genuinely cannot see, slid
  sideways out of the viewport, or sitting behind something else at their
  middle point. `aria-hidden` content a person *can* actually see (plenty of
  sites mark things this way that are still drawn on screen, such as a
  custom list's own shown label) is kept.

One rule cuts across all of the above: **cookie, consent, GDPR, privacy, and
newsletter banners are never treated as noise**, even when their markup
matches an ad or hidden-content rule, because a flow has to see them in
order to answer them. Nor is anything dropped that floats above the page (a
dialog or a fixed layer) or sits inside one: an ad banner in front is an
obstacle to close, not noise to ignore.

Every reading carries a `denoised: {ads, empty, hidden}` count of how many
blocks of each kind were dropped. `BrowserSurface::denoised()` exposes the
last reading's counts; it reads zero before any page has been read, and
zero again whenever the last read fell back to the tree instead of sight.
The counts never become part of the `Screen` itself: they are a debugging
signal, not something a flow decides on.

## Limits

| Constant | Value | What it bounds |
|---|---|---|
| `MAX_CONTROLS` | 800 | controls returned, in page order |
| `MAX_TEXTS` | 400 | text blocks, within a screen of the viewport either side |
| `MAX_LABELS` | 3,000 | visible words weighed as a candidate label for a field |
| `MAX_NAME` / `MAX_TEXT` | 120 / 160 characters | longest name, longest text block |
| `MAX_CONTEXT_LINES` | 60 | distinct context lines kept |

## What sight can't reach

Sight gives way to the accessibility tree (`tree.rs`, see below) when the
reading fails outright, or when it sees a frame that covers a large share of
the viewport, or controls inside two shadow roots: cases where a plain CSS
selector from the top-level page cannot address the element sight found.

One shadow root that shows controls is read beside sight instead. Sight
marks its host and names the first shown layer its controls draw (`popover
"We value your privacy"`, with an `aria-labelledby` resolved inside the
shadow root), and the surface reads the tree under the host alone and adds
its controls, under that label, after everything sight read. The tree reads
the host itself and what the page puts in its slots too, so sight leaves
both to it: nothing is offered twice. A tree
snapshot's refs last until the next snapshot, so only one host's subtree
can be read beside sight. A shadow root counts once its host or any of its
controls shows: a host laid out as `display: contents` has no box of its
own, and live, a consent banner's host was one. Before, its buttons went
unread while the banner lay over "Add To Cart"; giving the whole page to
the tree read the rest of the page worse. Reading through the tree is deliberately
unglamorous: it is the same fallback the crate always had, just demoted from
"the only way" to "the way out when sight cannot help."

## The tree fallback in practice

When sight is off (`Perception::Tree`) or gives up, `BrowserSurface::observe`
asks agent-browser for a `snapshot` command instead and hands the resulting
text to `tree::screen` (`crates/tinycomputer-browser/src/surface/tree.rs`).
That snapshot is agent-browser's own accessibility tree, rendered as
indented lines:

```text
- button "Search flights" [ref=e12]
- textbox "From" [ref=e13]: New Delhi
```

`tree::parse_line` reads one line into a role, an optional quoted name,
bracketed attributes (`ref=e12`, `disabled`, `checked=true`), and a trailing
value. `tree::screen` then walks the whole snapshot, tracking each open
ancestor so it can build the same kind of container `path` sight builds, and
numbering repeated container roles (`listitem`, `row`, `article`, `group`,
`region`, `option`, `gridcell`, `cell`, `treeitem`) the same way sight does,
so a flow written against one perception groups cards identically under the
other.

Two rules in the tree parser exist purely to avoid a privacy mistake sight
also avoids: a value-bearing role (`textbox`, `searchbox`, `combobox`,
`textarea`, `spinbutton`) never has its *value*, what was typed or selected
into it, leak out as some enclosing unnamed container's description, even
though its accessible *name* still can. And an unnamed control with no name
and no value of its own is named, after the fact, by whatever text turns up
nested inside it as the tree is walked (`describe_by_content`): the tree's
equivalent of sight's "own text" rule, just working from a flat line stream
instead of a live DOM.
