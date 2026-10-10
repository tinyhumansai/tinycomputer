# Browser types

Source: [`crates/tinycomputer-bus/src/browser/`](../../../crates/tinycomputer-bus/src/browser/)

The 13 browser members are served on the same bus interface and object path
as the desktop members, by the same loaded module, and they have their own
vocabulary: open a session, navigate, snapshot the page's accessibility tree,
act on a ref, read the page as text, screenshot it, close. The engine behind
it is [agent-browser](https://github.com/vercel-labs/agent-browser), linked
in directly as a library. The module holds one `Browser`, shared with the
task runner, so a task's session and screenshots are reachable through these
same members.

## One interface, a `Browser` prefix

```rust,ignore
pub const INTERFACE: &str = crate::names::INTERFACE;     // "ai.tinyhumans.tinycomputer.Desktop"
pub const OBJECT_PATH: &str = crate::names::OBJECT_PATH;  // "/ai/tinyhumans/tinycomputer/Desktop"
```

Each browser member's name carries a `Browser` prefix (`BrowserSnapshot`,
`BrowserScreenshot`, `BrowserPerform`, and so on): a few of them would
otherwise collide with a desktop member of a different shape, and the prefix
keeps the whole family obvious to a model reading a member list. The payload
*types*, though, still live under `tinycomputer_bus::browser`, not at the
crate root, and that is deliberate: `SnapshotRequest` and `ScreenshotRequest`
also exist for the desktop members, with different shapes. Namespacing the
types means neither side ever shadows the other, and a caller cannot
accidentally send a desktop `SnapshotRequest` where a browser one belongs,
they are different Rust types. A browser member that acts on an open session
takes one JSON object, with the session beside the member's own fields;
`BrowserOpenSession` takes the new session's options, `BrowserListSessions`
nothing, and the output members an `output`. All reply in the same
`DesktopResponse` envelope the desktop members use.

These types were ported from a now-superseded `tinybrowser-bus` crate; a host
still written against that crate's separate interface only needs to call the
`Browser…`-prefixed members on this module's one interface instead (see
[docs/technical/specs/unified-agent.md](../../technical/specs/unified-agent.md)).

## Sessions: `SessionId`, `Viewport`, `SessionOptions`

A `SessionId` is an opaque newtype wrapping a string, deliberately not a
bare `String`, so a session id and an `OutputId` (below) cannot be passed to
the wrong parameter and only get caught at runtime. It serializes as the bare
string it wraps, so a host logging one sees the id, not an object wrapper.

`Viewport { width, height, device_scale_factor, mobile }` matters more than
it looks: the accessibility snapshot and every coordinate-based interaction
depend on layout, and a headless default of 800x600 makes a responsive site
serve its mobile tree, at which point an agent cannot find the navigation an
operator sees. The crate's own default is 1280x800 at 1x: wide enough that
mainstream sites serve their desktop layout, small enough that a full-page
screenshot of an ordinary article stays under the module's image size cap.

`SessionOptions` covers how to obtain the browser a session drives, and every
field has a documented default so the common case is
`SessionOptions::default()`:

| Field | Default | What it is for |
|---|---|---|
| `endpoint` | none (launches one) | attach to an already-running browser at a DevTools endpoint instead of launching one, how a host reuses its own managed browser, or points at a Chrome running in another container |
| `executable` | first Chrome/Chromium/Chrome-for-Testing found | the browser binary to launch |
| `headless` | `true` | a module loaded into a daemon usually has no display |
| `viewport` | `Viewport::default()` | see above |
| `user_agent` | browser's own | override |
| `user_data_dir` | a fresh temp dir, removed on close | so one session's cookies never leak into the next |
| `download_dir` | created by the module, never removed | must be an absolute path |
| `allowed_origins` | none (any origin) | pages only: a navigation outside the list is refused before any request is made, and a page a session is taken to outside it is left and reported; `*` admits any public host; the files a page loads are never checked |
| `default_timeout_ms` | `30_000` | for operations that do not carry their own deadline |

`SessionInfo` is what `OpenSession` and `ListSessions` report back:
`id`, the actual `ws://` `endpoint` the module is driving (so an operator can
attach DevTools to the exact same browser rather than guessing which one),
`launched` (whether this module started it, versus attaching to someone
else's, which determines whether closing the session leaves the browser
running), `headless`, `viewport`, and the active page's `url` and `title`.

## Navigating and extracting: `page/`

`NavigateRequest { url, wait_until, timeout_ms }` sends the session's active
page somewhere. A bare host like `example.com` is read as `https://`,
matching what a person would type; anything else must carry its scheme, and
only `http`/`https` are accepted. `WaitUntil` is the trade this crate refuses
to make a single default for everywhere: `Commit` (barely reachable, may
still be blank), `DomContentLoaded`, `Load` (the crate's own default: most
pages are both rendered and interactive here), `NetworkIdle` (waits for a
quiet period or the deadline, whichever comes first, a page that polls in
the background never truly goes idle, so this is bounded, not absolute).

Every navigating member returns a `PageState { url, title, status }` so a
caller never has to make a second call to find out where an action left the
page. `status` is absent when the page was not reached over HTTP at all
(`about:blank`, a `data:` URL, a same-document navigation).

`ReadRequest { format, selector, max_chars }` extracts the page. `ReadFormat`
has three options: `Text` (rendered text, chrome dropped), `Markdown` (the
default, keeps headings, links, lists, code blocks, and is what a model
reads best), `Html` (the live serialized DOM after scripts ran, not the
response body, what the page *became*, by far the most expensive of the
three, kept for a host that needs structure the other formats discard).
`PageText.truncated` says whether `max_chars` (default 200,000) cut the
content short, because showing a model truncated content without saying so
invites it to assume the rest of the page simply does not exist.

`EvaluateRequest { expression, await_promise, timeout_ms }` runs JavaScript
in the page and returns its completion value. `await_promise` defaults to
`true`; an unawaited promise serializes as an empty object, which reads like
a bug in the module rather than a mistake in the expression.

## The accessibility tree: `snapshot/`

`SnapshotRequest { selector, interactive_only, compact, depth, include_urls,
max_chars }` mirrors the desktop `Snapshot` in spirit but is its own type.
`interactive_only` (a constructor, `SnapshotRequest::interactive()`, sets
just this) keeps only links, buttons, fields, and their labels, dropping the
prose between them, roughly a tenth the size, and usually the right
question to ask ("what can I click here").

`Snapshot { url, title, sequence, tree, refs, truncated }` is the result.
`sequence` is worth understanding precisely: it is a monotonic counter that
advances on every snapshot *and* every navigation, because a navigation
replaces the whole document and makes every ref from before it exactly as
dead as if a new snapshot had replaced them. So the first snapshot after a
navigation is not "number one", reporting the true counter lets a host say
"this ref is two generations old" instead of discovering that fact by acting
on the wrong element.

## Naming and acting on an element: `action/`

`Target` is how an element is named, with three variants: `Ref` (a snapshot
ref, e.g. `e12`), `Selector` (CSS, matched against the first element found),
`Locator` (semantic: role, text, label, placeholder, `data-testid`, alt text,
or title attribute, `LocateBy`'s seven variants). `Target::parse` reads the
string form a host tool typically receives from a model: a leading `@` means
a ref, anything else is a CSS selector, chosen specifically because `@` is
never valid at the start of a CSS selector, so the two vocabularies cannot
collide even when squeezed into one string parameter.

`Action` is a tagged enum whose variants deliberately mirror the verbs an
agent-facing browser tool already exposes (`Click`, `DoubleClick`, `Hover`,
`Focus`, `Fill`, `Type`, `Press`, `Select`, `Check`, `Scroll`, `GetText`,
`GetAttribute`, `IsVisible`, `WaitFor`, `Back`, `Forward`, `Reload`), so a
host's dispatch from tool call to `Action` is a rename, not a translation
layer with its own bugs to keep in sync.

Two distinctions worth knowing:

- **`Fill` vs `Type`.** `Fill` clears a field and sets its value in one step.
  `Type` sends a sequence of key events instead, leaving anything already
  there, the only way to trigger the per-keystroke handlers an
  autocomplete or search-as-you-type box depends on, which a bulk value
  assignment skips entirely.
- **`Click` fails loudly, not quietly, when covered.** A click on an element
  another element covers (a consent banner, a modal) fails and names the
  covering element, rather than reporting success while the click actually
  landed on the overlay. The crate's own comment calls this out as
  deliberately avoided: "the single most expensive failure mode an agent can
  be handed" is a click that reports success and does nothing.

`ActionOutcome { value, page, matched }` is the one result shape every
action returns, reading and acting alike, a reading action (`GetText`,
`IsVisible`) puts its answer in `value`; an acting one leaves `value` null
and is described by the `page` state it left behind. `matched` reports which
element the action actually resolved to, useful when a `Locator` was named
loosely.

## Screenshots and the output-handle protocol: `output/`

`ScreenshotRequest { target, full_page, format, quality }` defaults to
capturing the viewport as `ImageFormat::Png`, lossless enough to read text
out of, bounded by the viewport rather than by however long the document
happens to be. `ImageFormat` also offers `Jpeg` and `Webp` for a long
full-page capture where layout matters more than legibility.

Screenshots (and only screenshots, on the browser side) are not returned
inline. `Screenshot` hands back an `OutputRef { id, total_bytes, sha256,
media_type, width, height }` naming a held image; the actual bytes are
pulled with `ReadOutput` (returns an `OutputChunk { id, offset, data, eof }`,
base64-encoded, four bytes of overhead per byte of image, chosen because it
still parses far faster than a JSON array of numbers would) and released
with `ReleaseOutput` when done, which, like `CloseSession`, succeeds even
if the output is already gone, so a host retrying a cleanup call never has
to distinguish "never existed" from "already cleaned up." Treat
`OutputRef.total_bytes` as a bound to check against, not a value to trust
blindly when sizing a buffer: it is a number the module sent, and a wrong
value that is trusted turns into a failed allocation, which aborts a process
rather than returning a tidy error.

## Downloads: `download/`

A `DownloadId` wraps Chrome's own stable download guid. `DownloadState` is
`InProgress` (default), `Completed`, or `Cancelled`, with `is_terminal()`
true for the latter two. `DownloadInfo` reports a monotonic per-session
`sequence`, the source `url`, `suggested_filename`, current `state`, bytes
received and (when known) expected, and the expected local `path`, present
only when the session configured a `download_dir` and the suggested filename
was judged safe. `WaitDownload` (`DownloadWaitRequest { timeout_ms }`) blocks
for the next terminal download this session has not already returned from an
earlier wait, so polling `ListDownloads` in a loop is never necessary just to
notice when one finishes.

## Errors: `errors/`

Source: [`crates/tinycomputer-bus/src/browser/errors/mod.rs`](../../../crates/tinycomputer-bus/src/browser/errors/mod.rs)

A browser member fails with the same `DesktopResponse`/`DesktopError`
envelope a desktop member does, reusing the desktop's own error code wherever
the meaning is shared (`STALE_REF`, `ELEMENT_NOT_FOUND`, `TIMEOUT`,
`POLICY_DENIED`, `INVALID_ARGS`), so one handler serves both surfaces. This
crate additionally defines a stable, more specific *name* for each browser
failure (a constant string with the prefix
`ai.tinyhumans.tinycomputer.Browser.Error`), carried in `details.name`, with
`errors::code(name)` giving the shared envelope code it maps to. The
reasoning is the same as everywhere else in this crate: a host does not hand
a model a raw failure string, it decides what *kind* of failure happened and
shapes its tool result accordingly, and matching on prose breaks the moment a
message is reworded.

| Name | Meaning | An agent can act on this |
|---|---|---|
| `InvalidInput` | malformed or self-contradictory request | yes |
| `NoSuchSession` | the session does not exist or is closed | no: open a new one, do not retry |
| `NoSuchElement` | no element matched the target | yes: re-snapshot and choose again |
| `StaleRef` | the ref belongs to an earlier snapshot | yes: same remedy as above, and the name says so directly |
| `NotActionable` | found, but covered, disabled, or off-document | yes |
| `Timeout` | ran out of time | yes |
| `BlockedByPolicy` | outside `SessionOptions.allowed_origins` | no: never retry, the answer will not change |
| `BrowserUnavailable` | no browser could be launched or reached | no: a host or deployment problem |
| `PageError` | a JavaScript exception, or the browser rejected a command | yes |
| `NoSuchOutput` | the held output does not exist or expired | no |
| `LimitExceeded` | too many sessions, too many held outputs, or an output too large to hold | no |
| `ModuleFailed` | everything else | no |

`errors::is_agent_recoverable(name)` is this exact table's "yes" column,
expressed as one function so every caller derives the same answer instead of
each one re-deriving it slightly differently:

```rust,ignore
assert!(errors::is_agent_recoverable(errors::NO_SUCH_ELEMENT));
assert!(!errors::is_agent_recoverable(errors::BROWSER_UNAVAILABLE));
```

## Calling a browser member directly

A caller that wants to drive a page itself, rather than through a task, can
call these members directly. Open a session, act on it, then read a
screenshot back in chunks:

```rust,ignore
use tinycomputer_bus::{DesktopResponse, browser::names::methods};

let opened: DesktopResponse = proxy.call(methods::OPEN_SESSION, (json!({}),)).await?;
let session = opened.data.as_ref().unwrap()["id"].clone();

proxy
    .call(methods::NAVIGATE, (json!({"session": session, "url": "https://example.com/"}),))
    .await?;

let clicked: DesktopResponse = proxy
    .call(
        methods::PERFORM,
        (json!({"session": session, "action": "click", "target": {"kind": "ref", "value": "e3"}}),),
    )
    .await?;

let shot: DesktopResponse = proxy.call(methods::SCREENSHOT, (json!({"session": session}),)).await?;
let output = shot.data.as_ref().unwrap()["id"].clone();
let chunk: DesktopResponse = proxy
    .call(methods::READ_OUTPUT, (json!({"output": output}),))
    .await?;
```

(`crates/tinycomputer/src/tinybus_module/tinybus_module_tests/browser_tests.rs` runs this same
sequence against the in-memory bus.)

## Where the browser fits into flows and tasks

A `Flow` step's `browse` action and a `StartTask`'s browser surface are both
built on exactly these members underneath, and `BrowserListSessions` shows a
running task's session beside any a caller opened directly. Most callers
still reach a page through a flow or a task rather than calling these members
directly, since a flow's `browse`/`extract`/`read` steps already cover
picking, reading, and choosing without naming a ref by hand, see
[Writing flows](flows.md) and [The Agent and task types](agent-and-tasks.md).
These members' contract version is `crate::CONTRACT_VERSION`, the same one
the desktop members use: the whole module ships and versions as one (see
[Versioning and compatibility](versioning.md)).
