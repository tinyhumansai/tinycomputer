# Surfaces and screens

Source: [`crates/tinycomputer-core/src/surface/mod.rs`](../../../crates/tinycomputer-core/src/surface/mod.rs),
[`screen.rs`](../../../crates/tinycomputer-core/src/surface/screen.rs).

## The `Surface` trait

tinycomputer can drive two very different kinds of things: a desktop
application (through the vendored `agent-desktop` engine) and a browser tab
(through `agent-browser`). Rather than write the decision loops twice, the
whole engine is written once against a trait called `Surface`. Anything that
implements it can be observed and acted on the same way.

A `Surface` has to be able to:

- `observe` an application and hand back a `Screen` (what is there right now)
- `execute` one closed operation such as click, type, expand, or check
- `read_value` an element's current value, when the platform exposes one
- `paste` text into a field through the clipboard, for fields that ignore a
  plain set-value
- `press` a key combination
- `launch` an application, or bring it to the front if it is already running

Three more methods have a default that most surfaces never need to
override: `navigate` (load a URL) and `back` (go back in history) both refuse
by default, because a desktop application has no address bar and no history,
and so does `dismiss_cover` (press the empty layer lying over a target, as a
person clicks outside a popup), because a desktop application cannot say what
lies over an element. `tinycomputer-browser`'s implementation overrides all
three; `tinycomputer-desktop` leaves them as the default refusal.

There is also `settle()`, which does nothing by default. A surface overrides
it to give an application a moment to react: closing a banner a beat after a
click, or turning a typed address into a token in an autocomplete field. The
engine calls it after an action, before the next observation, and before
reading a value back, so a surface's own idea of "how long is a moment" stays
in one place rather than being copied into every caller. After an action that
fetches nothing (a launch, Escape) the engine calls `settle_briefly()`
instead, which settles in full unless the surface overrides it:
`tinycomputer-browser` then waits only while the page changes.

And `await_change(ms)`, which waits up to `ms` for the application to change
by itself and says whether it did. A flow uses it while it watches for
something to appear, such as the suggestions a place box lists for the text
just typed: it looks again as soon as the page changes, and stops once the
page stays still. By default it pauses as a `Wait` does and says the
application may have changed; `tinycomputer-browser` watches the page's DOM
instead.

Every member returns a `DesktopResponse`, never a plain `Result`. That is a
deliberate rule of the whole repository, not just this trait: a denied
permission or a stale reference is a result a caller can act on (retry,
re-observe, ask for approval), so it travels in the envelope with a code and
a hint, rather than as an error that unwinds the call. `observe` is the one
exception, and even there the error it returns is the same `DesktopResponse`,
just boxed.

```rust
pub trait Surface: Clone + Send + 'static {
    fn observe(&self, app: &str, root: Option<&str>, depth: Depth)
        -> Result<Screen, Box<DesktopResponse>>;
    fn execute(&self, operation: JevOperation, target: Option<Candidate>, text: Option<String>)
        -> DesktopResponse;
    fn read_value(&self, target: &Candidate) -> Option<String>;
    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse;
    fn press(&self, app: &str, combo: &str) -> DesktopResponse;
    fn launch(&self, app: &str) -> DesktopResponse;
    fn settle(&self) {}
    fn settle_briefly(&self) { /* settles */ }
    fn await_change(&self, ms: u64) -> bool { /* pauses, and says it may have */ }
    fn navigate(&self, url: &str) -> DesktopResponse { /* refuses by default */ }
    fn back(&self, app: &str) -> DesktopResponse { /* refuses by default */ }
    fn dismiss_cover(&self, target: &Candidate) -> DesktopResponse { /* refuses by default */ }
}
```

If you are adding a third kind of surface one day, this trait is the whole
contract. Everything else in this crate is written against it, or against
the `Screen` it returns, and does not care which surface produced that
screen.

## `Screen`: what an observation hands back

```rust
pub struct Screen {
    pub app: String,
    pub window: Option<String>,
    pub surface: String,
    pub candidates: Vec<Candidate>,
    pub context: Vec<String>,
    pub unexplored: Vec<String>,
    pub text_nodes: Vec<Candidate>,
}
```

- `candidates` are the ref-bearing, actionable nodes, in document order. A
  "ref" is the id the underlying engine assigns so a later click can say
  "this exact one", even though the tree gets re-observed between actions.
- `context` is the visible text that is not actionable: headings, labels,
  status messages.
- `unexplored` lists refs of subtrees the engine cut short to stay inside a
  size budget. Observing one of those refs as a new root reads what was left
  out, the same way you would click "show more".
- `text_nodes` holds ref-less text that carries something a field actually
  holds, such as a rich-text email body or a token field's list of
  attachments. It is kept apart from `context` on purpose: `context` goes to
  every request unconditionally, while field contents are only shown when a
  caller has explicitly asked to see values (`include_values`). Mixing the
  two would leak a typed password into a screen description that has nothing
  to do with reading values.

`Depth` controls how much of the tree one observation reads: `Full` (the
engine's own default) or `Skeleton`, a shallow overview whose cut-off
containers can be drilled into with a follow-up `observe` rooted at one of
them.

## `Candidate`: one element

A `Candidate` is one node: its `ref_id`, `role` ("button", "textfield", …),
`name` and `description`, its current `value`, `states` such as `focused` or
`disabled`, the `available_actions` the engine will let you perform on it,
and (when relevant) `children_count` and `bounds`. It also carries `path`,
the labels of its ancestors outermost first, and `order`, its position in
document order. Both are computed locally (`#[serde(skip)]`) rather than
sent over the wire, because they are cheaper to derive once the tree has
already arrived than to duplicate in every node's JSON.

## Turning a candidate into something Jev can read

`describe()` and `element_line()` both take a `Candidate` and produce a
description for the decision model: one as a JSON value wrapped in
`untrusted_accessibility_data`, the other as a single line of text. Both add
two things a raw accessibility tree does not give you for free:

- `near`, for an element with no name of its own: the nearest named container
  it sits in, so two unlabelled search boxes inside two different dropdowns
  do not look identical.
- `says`, for a named element whose description adds something the name
  does not: a calendar names each day by its number ("18"), and only the
  description ("Sunday, 18 October 2026") tells one month's 18th from the
  next's.

Everything that comes off a screen is wrapped as
`untrusted_accessibility_data` before it reaches Jev. That phrase is not
decoration: see [safety and privacy](../../safety-and-privacy.md) for why
screen text is always treated as data, never as instructions, no matter how
much it looks like one.

## Telling two screens apart without asking a model

Re-observing a screen mints new refs every time, even when nothing visible
changed. So the crate never compares refs to detect a change; it compares
`fingerprint()`, a string built from every candidate's `signature()` (role,
label, value, states, and where it sits in the tree, deliberately leaving
out the ref), plus the window title, the surface kind, and the context text. Two
screens with the same fingerprint are the same screen as far as anyone
watching is concerned, even if every ref in them is brand new.

`difference()` and `change_note()` build on the same idea to describe *what*
changed, in words a decision model can match against next turn:

```rust
// after clicking "Accept all" on a cookie banner
change_note(&before, &after, true)
// => "gone: button \"Accept all\", button \"Essential only\""
```

`label()` gives a short human label for one element (role plus name, e.g.
`button "Book"`), and `exact_named_match()` answers a narrower question: does
a goal's text name this exact candidate, by containing a run of at least two
of its name's words in order? That is deliberately strict. A single word in
common is not enough to count as "the goal named this element", because it
guards against acting on the wrong element just because two labels share one
common word.

Finally, `target_payload()` turns a `Candidate` into the small `JevTarget`
shape (`ref_id`, `role`, `name`) that a flow's step report records: enough to
say what was acted on, without repeating the whole candidate.

## `uses_pointer`

One small helper lives at the top level of `surface/mod.rs` rather than in
`screen.rs`: `uses_pointer(operation)` answers whether a person would use the
mouse for this operation (click, expand, collapse, check, uncheck). A surface
that draws an on-screen cursor for the agent (see
[`crates/tinycomputer-cursor`](../../../crates/tinycomputer-cursor/README.md))
uses this to decide whether to glide the cursor onto the target before the
action lands, rather than moving it for actions like typing that a person
would not point at first.

## Where this fits

The flow runtime in `tinycomputer-engine` observes through `Surface`, asks
Jev questions built from what `describe`/`element_line`/`fingerprint` produce,
and acts back through `Surface::execute`. None of that loop is in this crate;
this crate only supplies the shapes and the deterministic judgments the loop
leans on. See [how tinycomputer decides](../../how-it-works.md) for the loop
itself, and [watching a run](../../watching-a-run.md) for how a trace shows
these same fingerprints and change notes back to you.
