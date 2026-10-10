# Browser sight: reading a page the way a person looks at it

**Status:** Implemented; the browser surface's default (`Perception::Sight`),
with the accessibility tree as its fallback and as an opt-out
(`Perception::Tree`, `TINYCOMPUTER_BROWSER_PERCEPTION=tree` in the live
examples).
**Code:** `crates/tinycomputer-browser/src/surface/sight/`.
**Evidence:** [`../evals/2026-09-28-jev-call-audit.md`](../evals/2026-09-28-jev-call-audit.md).

## Problem

The browser surface built its screen from agent-browser's accessibility
snapshot, so it saw what a page *declares*: roles and names from ARIA and
label markup. Most sites apply that markup partly or wrongly, and every gap
reached Jev as an unnamed or mislabelled element:

- IndiGo's destination list is nine `div role="combobox"` rows whose
  `aria-labelledby` points nowhere: nine unnamed "text boxes" that take no
  text, which a flow tried to type into and then pressed, choosing Mumbai;
- a field labelled only by the words printed above it has no name;
- an icon button with no `aria-label` has no name;
- the tree lists what is off screen or covered by a dialog as if it could be
  pressed, and does not say which layer is in front.

A person never reads the markup. They see what is drawn and on top, read the
words on a control or beside a box, and know a box takes text because it has
a caret.

## Goals

- Build the browser's `Screen` from the rendered page, by what is drawn,
  where, and on top — independent of ARIA.
- Name each control by the words a person would read for it.
- Decide what takes text by what the element is, not the role it claims.
- Keep the `Screen` model, the flow runtime, and the contract unchanged:
  sight is a new way to fill the same structure.

## Non-goals

- Vision: nothing reads pixels. An icon with no words, no alternative text,
  and no telling class stays unnamed.
- Shadow roots and frames, which a CSS selector from the page cannot reach:
  one shadow root's controls are read from the tree under its host, beside
  sight; otherwise the tree reads the page (below).
- Changing agent-browser. Sight runs through its existing `evaluate` command,
  and acts through its existing CSS-selector targets.

## Behavior

`BrowserSurface::observe` runs `sight.js` over the page (or the part under a
`root` ref) in one `evaluate`, and turns the reply into a `Screen`:

1. **Controls, by behaviour.** Native links, buttons, fields, checkboxes,
   radios, and selects; elements with a control's ARIA role; and elements a
   pointer cursor, a click handler, or a tab stop makes clickable (not the
   children that only inherit the cursor). A claimed `textbox`, `searchbox`,
   `combobox`, or `spinbutton` that takes no text is a `button`, or, when it
   wraps a real input, is read as that input. A label that stands in for a
   hidden checkbox or radio is that checkbox or radio. Disabled controls are
   left out, as in the tree. A region a page marks as holding controls (a
   menu, list, listbox, grid, tab panel, toolbar, dialog, or a landmark) is
   not a control for its tab stop alone, which only moves the focus inside
   it (one with a pointer cursor or a click handler, such as a carousel's
   slide, still is), and a button or link that holds a box to type in is a
   panel (a popover with its own search box). The rows inside either are read as controls of their
   own; read as one button, a panel's name strings every row together and a
   press lands on whatever row sits at its middle.
2. **Only what is drawn.** Zero-size, `display: none`, invisible, and
   transparent elements are left out, and so are disabled controls: the
   `disabled` property, `aria-disabled`, or a class name ending in
   `disabled` (a calendar's past day, `rdrDay rdrDayDisabled`). A control outside the viewport carries
   the state `offscreen`; one whose middle is under another element carries
   `covered`, unless the cover is its own result card's content (the same
   rule the click-through uses).
3. **Names are the words a person reads.** A control's own words (without
   those of a list of controls nested in it); for a field, its tied
   `<label>`, then the page's `aria-label`, placeholder, or title, then the
   words inside its box, left of it on its line, or just above it (right of
   it for a checkbox), never a divider such as "OR"; for a word-less control, its `aria-label`, `title`,
   or pictures' alternative text, then the icon's class, id, or test-id words
   (`close`, `search`, `menu`, …) with the description "an icon", and for a
   link, where it leads ("leads to sightseeing"). A page label that adds to
   the words shown becomes the description — the control's own, or else the
   one labelled element inside it that carries its words: a calendar day
   drawn as "18" whose inner span is labelled "Sunday, 18 October 2026".
4. **One control per thing a person sees.** Two related elements drawn in
   nearly the same box (intersection over union 0.6), a wrapper with the same
   words as the control inside it, or two controls of one kind in the same
   label, are one control; a text entry wins over its wrapper.
5. **Containers.** Each element's path lists what a person sees it in:
   dialogs (`dialog`, `alertdialog`, `aria-modal`) and fixed layers (a
   `dialog` when it covers 30% of the viewport, else a `popover`; not a top
   bar with links), landmarks (`banner`, `navigation`, `main`, `form`, …),
   named sections and groups, lists (an unnamed one after the first of its
   kind numbered in page order, `list 2`, so two lists' first cards stay
   apart), and cards (`listitem #3`, `row #2`, `article #1`; an unlabelled
   table row by its control-free cells' words, `row "01 Handicapped" #2`),
   in the tree's label format so card grouping and the digest
   work unchanged. The screen's surface is `sheet` (or `alert`) when a dialog
   is on top at the middle of the viewport.
6. **Text.** Visible words outside controls and fields, within a screen of
   the viewport, become text nodes in page order; the first 60 distinct lines
   become the context. Words inside a field are its value, private unless
   values are shared, and never context. A password's value is never read.
7. **Refs.** Each control is marked `data-tc-seen="N"` the first time it is
   seen and keeps the mark for its life. Its ref is `seen:N`; the surface
   addresses it with the selector `[data-tc-seen="N"]` for every action,
   bounding box, value read, and scoped observation. An element the page
   removes takes its mark with it, so a stale ref fails rather than reaching
   what replaced it.
8. **Fallback.** When the reading fails, sees controls inside two shadow
   roots or a frame of a fifth of the viewport in front, or the tree cannot
   read the one shadow root's host, the surface reads the accessibility tree
   for that observation, as before. One shadow root is read beside sight:
   the tree reads its host, its controls, and what the page puts in its
   slots, which sight leaves out so that nothing is offered twice.

9. **Denoising.** Noise is left out before anything is returned; see
   [Denoising](#denoising).

A covered click on a `seen:` ref finds its target by its mark, not by its
name, so an unnamed card link is clicked through its own card too. A click
on a `seen:` tab, radio, or option that leaves it on screen unselected is pressed once more by
the element's own `click()`: a page can ignore a trusted click it has not yet
wired up (Emirates' trip tabs, freshly loaded), and selecting is idempotent.

A native dropdown (`<select>`) draws its options in a menu the browser shows
outside the page, so a click on it changes nothing the reader can see and its
choices never appear. A text box's list of suggestions (`<input list>` with a
`<datalist>`) is the same: Chrome draws it outside the page, and only for a
person typing. The reading therefore lists each enabled choice, up to 60 per
control, as an `option` inside its control: drawn in the control's box, under
a `listbox "<control>"` container, and `selected` when chosen. A dropdown's
choice is named by its label; a suggestion by the value it fills in, with a
differing label as its description. Suggestions two boxes share are listed
only under the one that has focus, and the reader marks each suggestion with
the box it was offered under (`data-tc-for`), so pressing one names a single
box. A click on such an option chooses it the way a person does, without
opening the menu: the dropdown's option is selected, or the box's value is
set through the input's own value setter (so a page that tracks the value
itself sees it); the control fires `input` and `change`, and the reply says
so only once the choice holds (`native_select.rs`). A click on any other
`option`, such as a page's own `role="option"` row, is pressed as before.
BlazeDemo's cities and Selenium's "Dropdown (select)" and "Dropdown
(datalist)" could not be chosen before this.

A date picker's calendar is a table of day numbers under its month and year,
and many pickers (Bootstrap's among them) draw each day as a plain cell that
shows a pointer only under the mouse, so neither a role, a tab stop, nor the
cursor marks it as pressable. The reading therefore treats a shown table with
at least 28 day-number cells, whose month and year ("November 2026") its own
heading or a short header just before it names, as a calendar: each enabled
day is a `gridcell` named by its number and described by the date it stands
for ("15 November 2026"), the days before the month's first and after its last
dated in the months either side, and a week-number column left out. Inside a
calendar's container an arrow glyph or a bare "Next" reads as "next month"
(and its opposite as "previous month"), which is how `choose` pages a calendar
to the requested day. Selenium's date picker could be opened but no day picked
before this; the flow looped and rescued for minutes.

A calendar drawn without a table is read the same way: a block whose cells
each begin with a day number (a fare may follow, "22 6529"), numbered from 1
to the month's last day, under a short text naming its month and year. A
calendar already found before it is passed over, but a header beyond it
titles the block only while it has a month left to give. One header naming
two months titles two such blocks in order, and only when it found a block
for each month: one block missed (a "Today" over its first day) would give
the next block's days the month before's name, so then no day of either is
dated. The cells may sit in the block itself or in its rows of a week (four
to six weeks, after a row of day names when the month draws one there), as
React-style pickers draw them. A day is described by its date unless the
page's own label for it already names a month ("Thu Oct 01 2026"), which
stands alone; a label that names none ("Sold out", or only the day's
number) follows the date ("24 October 2026, Sold out"). Live, a hotel site
drew its open days as a number over a fare in rows of a week: none read as a
date, and the step paged the calendar a year past the month it wanted. The
one labelled element inside a day is its date also when a fare follows its
number ("23 6757" holding a label "October 23, 2026"), as long as that label
names a month and the day's number. The title may sit before the grid or
before any of its five nearest ancestors, and the title's own block is a
calendar too, so arrows drawn in it page the month, unless it holds a form's
fields; it is added after the pickers, so no block around a title and its
grid reads as a picker of two months, and a wizard's "Next" beside the grid
stays its own. Live, a flight site drew "‹ October 2026 – November 2026 ›"
above both months, outside either, and its priced days carried their date
only in an inner label.

A label that wraps its field, and any text block holding a dropdown, is read
without the dropdown's own text: a closed dropdown shows one choice, but its
text holds them all, so Selenium's dropdown was named "Dropdown (select) Open
this select menu One Two Three" before.

## Denoising

A person skips ads and never sees what a page hides from them, so sight
leaves both out, and blank boxes with them. Each rule leaves a whole block
out: the element and everything inside it.

- **Ads.**
  - Frames, pictures, and links whose address is on an ad or tracking
    host: `doubleclick.net`, `googlesyndication.com`,
    `googleadservices.com`, `adservice.google.*`, `amazon-adsystem.com`,
    `taboola.com`, `outbrain.com`, `criteo.*`, `adnxs.com`, `moatads.com`,
    `pubmatic.com`, `rubiconproject.com`, `scorecardresearch.com`, and their
    subdomains.
  - Elements whose class or id holds an ad word. A class is split into
    words at `-` and `_` only, never inside a word, so `header`, `shadow`,
    `download`, `adults`, and generated classes such as `css-1ad4k9` never
    match. `adsbygoogle`, `adslot`, `adunit`, `adcontainer` (and the other
    `ad…` compounds in `sight.js`), `advert…`, and `sponsor…` match in any
    case. The short words `ad`, `ads`, and `dfp` must be in one case and
    stand alone (`ads`) or beside a real word of three letters or more
    (`ad-slot`, `top-ad`, `div-gpt-ad-1234-0`). Google's generated `gb_Ad`
    and `gb_ad` are not ads.
  - Elements with AdSense's `data-ad-slot` or `data-ad-client`, or Google
    Publisher Tag's `data-google-query-id`.
  - A shown label whose whole text is "Advertisement", "Sponsored", or "Ad"
    (any case), together with the nearest block around it that holds more:
    another word, a control, a picture, or a frame. The block is never the
    page root, a landmark, a form, a dialog, anything that holds a field, or
    more than 40% of the viewport. A control whose whole name is the label
    is a control, not a label.
  - Tracking pixels: a loaded image of at most 1×1 pixels drawn at most
    1×1.
  - An ad frame no longer counts toward the frame-in-front fallback, so an
    inline banner does not send the reading to the tree.
- **Blank boxes.** A box that is clickable only by its pointer cursor, tab
  stop, or click handler is left out when it has no words, no name, no
  picture, and nothing inside to act on or type into. Native controls,
  controls with an ARIA role, and pictures that are controls stay. Sight
  never returned decorative pictures or wrapper elements, so there is
  nothing else to drop or collapse.
- **Hidden.**
  - `inert` blocks.
  - Visually hidden screen-reader text, still laid out but clipped away:
    positioned absolutely or fixed, and clipped by `clip: rect(0 0 0 0)`,
    `clip-path: inset(50%)`, or a 1×1 box with `overflow: hidden`.
  - `aria-hidden="true"` blocks a person cannot see: those slid out of the
    viewport sideways (a carousel's clones), and those in the viewport with
    something else in front at their middle (the page behind a dialog).
    Pages mark plenty they draw with `aria-hidden`, such as a custom list's
    shown label, a pill below the fold, or a page a modal library forgot to
    unmark. What is in front, or above or below the viewport, stays.
  - A checkbox or radio hidden by any of these inside its own label still
    makes the label its stand-in, as a checkbox hidden by style does.
- **Never noise.** An ad rule never drops a block that floats above the page
  (a dialog or a fixed layer) or sits in one: an ad in front is an obstacle
  a person must close. Nor does it drop a block whose class, id, label, or
  first 600 characters of text mention cookies, consent, GDPR, privacy,
  newsletters, or subscribing: the obstacle loop must see those banners to
  answer them.
- **Words.** The words in an ad never become a field's label.

The reply carries `denoised: {ads, empty, hidden}`, the number of blocks of
each kind that held something sight would otherwise have returned: a
control, a text block, a shadow root or frame that would have sent the
reading to the tree, or an ad's own frame or picture. A reading from before
denoising has no such field, and every count reads as zero. The surface
keeps the last reading's summary as `BrowserSurface::denoised()`, which is
zero until a page is read, and zero again when the tree was read instead.
The `Screen` does not carry it.

## Limits

| Constant | Value | Why |
|---|---|---|
| `MAX_CONTROLS` | 800 | a long results page, in page order |
| `MAX_TEXTS` | 400 | text blocks within a screen of the viewport |
| `MAX_LABELS` | 3,000 | visible words weighed as a field's label |
| `MAX_NAME` / `MAX_TEXT` | 120 / 160 characters | as the tree's content names and context lines |
| nearby label | 200 px left, 40 px above, 40 px right of a checkbox | the distances at which a person still reads words as a field's label |
| ad label's block | at most 40% of the viewport | an ad, not the page section it sits in |
| banner words read | first 600 characters of a block's text | enough to find a consent or newsletter banner's words cheaply |

## Invariants

- Screen text stays data: everything sight returns reaches Jev only through
  the flow runtime's `untrusted_accessibility_data` wrapping and masking.
- Typing still passes the surface's editable check (`takes_text`); sight
  only stops offering `SetValue` for what cannot take it.
- A ref never silently moves to another element; a control a page claims
  around a native button (a `td role="gridcell"` holding a `<button>`) is
  marked on that button, which a press must reach, unless the control says
  whether it is chosen (a tab, a radio, an option).
- Reading adds nothing to the page but the marks. Pressing adds, best
  effort, only this: the pressed link or form, when it would open a new
  tab, is aimed at the page's own; for two seconds a script's
  `window.open` of an address on the same site opens it in place (another
  site's still opens its own window, and the patched `open` returns no
  window); and a flag notes that the page began to unload, so a slow link is
  not followed twice.

## Acceptance

- Unit tests: the reply's controls, text, context, states, actions, and
  bounds; the fallback when a reading fails or sees what it cannot reach;
  sight refs addressed by their marks in fill, focus, the covered click, and
  a scoped observation; `Perception::Tree` reading the tree alone.
  The `denoised` summary parsed, defaulted when absent, and kept by the
  surface.
- Live fixture tests (`live_*` in `sight/sight_tests/live_tests.rs`, with the `agent-browser`
  feature and `TINYCOMPUTER_LIVE_BROWSER=1`, since CI has no browser): ad
  frames, ad-named and "Sponsored" blocks, ad links, and pixels are removed,
  while `header`, `shadow`, `download`, `adults`, and generated classes are
  kept; blank boxes are dropped while picture boxes and native buttons stay;
  a panel holding a search box and a tab panel taking a tab stop leave their
  rows to be read one by one;
  consent, cookie, and newsletter banners are kept whole; `inert`, clipped,
  and sideways `aria-hidden` content and the page behind a dialog are
  dropped, while `aria-hidden` content a person sees stays and hidden
  checkboxes keep their labels as stand-ins.
- Live, read with a local headless Chrome: IndiGo's city rows named by their
  cities as buttons, its form's radios, date and passenger controls read
  once each, Google Flights' fields named "Where from? New Delhi DEL" and
  its result cards grouped, the travel fixture's fields named by their
  labels.
- Live runs: see the eval.
