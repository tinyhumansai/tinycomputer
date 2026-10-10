# Errors: what a caller should do next

Code: `crates/tinycomputer-browser/src/error/mod.rs` (the `Error` enum),
`crates/tinycomputer-browser/src/reply/mod.rs` (`classify`, which maps
agent-browser's own error strings onto it), `crates/tinycomputer-bus/src/
browser/errors/mod.rs` (the published wire names).

## The taxonomy is about the remedy, not the cause

`Error`'s variants are not a map of *where inside this crate* something went
wrong. A caller on the other side of the bus cannot use that information
anyway. They are a map of *what the caller should do about it*, which is the
one distinction that actually survives the trip across the bus. The crate's
own doc comment says this explicitly: every variant maps to exactly one
published wire name in `tinycomputer_bus::browser::errors`, and that mapping
(`Error::wire_name`) lives in this crate rather than at the bus boundary, so
a new variant can never be added without someone deciding, on the spot, what
a host is supposed to see and do about it.

## The variants

| Variant | Wire name | What a caller should do |
|---|---|---|
| `InvalidInput` | `InvalidInput` | Fix the request: an unparseable URL, an empty expression, a screenshot quality outside 1 to 100. |
| `NoSuchSession` | `NoSuchSession` | Open a new session; this one is gone. |
| `NoSuchElement` | `NoSuchElement` | Take a fresh snapshot and choose again. |
| `StaleRef` | `StaleRef` | Same remedy as above, spelled out explicitly: the ref belonged to an earlier reading of this page. |
| `NotActionable` | `NotActionable` | The element exists but could not be acted on right now: covered, disabled, off-document, or behind a JavaScript dialog that has to be answered first. The message names the obstruction where the browser could identify it. |
| `Timeout` | `Timeout` | The operation ran out of time. A click or a submission may already have landed, so look at the page (take a fresh snapshot) before retrying, perhaps with a longer deadline; never repeat it blind. A timeout on a member that only looks — open a session, snapshot, read, screenshot, wait for a download — changed nothing, so over the bus its hint is a plain retry. |
| `BlockedByPolicy` | `BlockedByPolicy` | Never retry. The session's `allowed_origins` refused this destination, or the page the session showed, before the call was sent, and the answer will not change. |
| `LeftRefusedPage` | `BlockedByPolicy` (same wire name) | The call ran, and its work (a click, a key, a script, a redirect) took the session to a page `allowed_origins` refuses; the session left it. Its effect may stand: check before repeating it. Kept apart from `BlockedByPolicy` so its delivery is never claimed. |
| `BrowserUnavailable` | `BrowserUnavailable` | Not something a caller can fix by choosing differently: this is a host or deployment problem (no browser could be launched or reached). A browser the module launches says what to set: no Chrome or Chromium found ("give the path of the browser to use"), or a binary given that would not start ("check that its path names Chrome or Chromium"); one attached to keeps the engine's words. |
| `PageError` | `PageError` | The page itself raised a JavaScript exception, or the browser rejected a command. The same request fails the same way again: inspect the page and revise the request (its hint is `inspect_state_then_revise_request`, not retryable). |
| `NoSuchOutput` | `NoSuchOutput` | The held screenshot or PDF being asked for is unknown or has expired. |
| `LimitExceeded` | `LimitExceeded` | A bound was hit: too many sessions, too many held outputs, or an output larger than the module will hold. |
| `ConnectionLost` | `NoSuchSession` (same wire name as `NoSuchSession`) | Open a new session. Kept as its own Rust variant, rather than folded into `NoSuchSession` at construction time, because it says something more specific ("the transport died") that is useful for the crate's own diagnostics. A caller across the bus is told exactly the same thing either way: don't retry into a socket that will never answer, open a fresh session instead. |
| `ModuleFailed` | `ModuleFailed` | Anything that does not fit the categories above. |

`Error::envelope` marks a failure `not_delivered` (so retrying is safe) only
when it is decided before anything reaches the page: `NoSuchSession` and
`NoSuchOutput`, looked up locally, and `StaleRef` and `BlockedByPolicy`,
refused by agent-browser's ref lookup or the session's allowed origins before
any input is sent. A page refused after a call's work is `LeftRefusedPage`:
the page is left, and its delivery stays `unknown`, since a click on a listed
site's "Pay" that lands on an unlisted bank page has paid. Every other
variant can follow work already done, so its delivery is left `unknown`
rather than claimed.

`errors::is_agent_recoverable` (in the bus crate) is the one further
decision that gets made on top of this table: whether a model can plausibly
recover by choosing differently, versus needing a human or an operator.
`InvalidInput`, `NoSuchElement`, `StaleRef`, `NotActionable`, `Timeout`, and
`PageError` are recoverable this way; `NoSuchSession`,
`BlockedByPolicy` (and `LeftRefusedPage`, which shares its name),
`BrowserUnavailable`, `NoSuchOutput`, `LimitExceeded`, and `ModuleFailed`
are not.

## Where the classification actually happens

agent-browser's own replies do not carry a taxonomy. They carry
`{success: false, error: "<sentence>"}`, a human-readable message and
nothing else. `reply::classify` (`reply/mod.rs`) is where those sentences
get sorted into the table above, matched against the engine's actual
message texts: `"Unknown ref: @e12 ..."` becomes `StaleRef`, anything
starting with `"could not locate element with role="` becomes `StaleRef`
too, `"... is not in the allowed domains list"` becomes `BlockedByPolicy`
(with the refused host pulled out of the message's own quoting; the
session refuses pages outside its allowed origins itself and never hands
them to agent-browser, so this only comes from agent-browser's own domain
filter, set by a raw `launch` command that carries `allowedDomains`),
`"... is covered by ..."` or `"not interactable"` becomes `NotActionable`,
and so on down the list in `classify`.

This is exactly as fragile as it sounds if agent-browser ever reworded one
of those messages. That fragility is why every one of these mappings is
pinned by a test in `reply/reply_tests.rs`, so a message change upstream that would
silently break the classification instead breaks the build.

## Where errors surface for `BrowserSurface`

When a decision loop is driving the page through `BrowserSurface` rather
than calling `Browser` directly, an `Error` becomes a `DesktopResponse`
failure (`surface::mod::failure`), whose `code` is the error's wire name
rewritten from `PascalCase` to `SCREAMING_SNAKE_CASE`: `StaleRef` becomes
`STALE_REF`, matching the convention the desktop adapter's own errors use,
so a flow reading error codes does not need to know which surface it is
talking to.

## Where errors surface for the `Browser…` bus members

The same idea applies when a caller reaches `Browser` directly over the bus,
through one of the 13 `Browser…` members. `crates/tinycomputer`'s dispatch
wraps each result with `Error::envelope` (`error/mod.rs`), which builds a
`DesktopError` the same way: `code` is `errors::code` of the wire name (the
desktop's own spelling wherever the meaning is shared), the full wire name
rides in `details.name`, and `recovery` comes from `errors::recovery`. Only a
failure decided before anything reaches the page is marked not delivered (see
above for the four), so a caller knows retrying it cannot repeat an effect;
`reply::classify` names failures from the engine's replies, and marks none of
them delivered or not on its own. This is a separate code path from
`BrowserSurface`'s (a flow or task drives the page through the engine's
`Surface` trait; a direct `Browser…` call goes straight through the module's
dispatch), but both end up at the same `DesktopResponse` shape and the same
codes, so a caller does not need to know which path produced a given
failure.
