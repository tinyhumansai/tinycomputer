# The goal loop

Source: [`crates/tinycomputer-bus/src/agentic/types/`](../../../crates/tinycomputer-bus/src/agentic/types/mod.rs)

`RunGoal` and `ResolveIntent` are the desktop interface's own bounded
decide-and-act loop: older, lower-level, and more tightly closed than a
`Flow`. If you are choosing which of the two to build against, read
[How it decides](../../how-it-decides.md) first; this page is the reference
for the wire types once you know you want this one.

## `ResolveIntent`: decide, but do not necessarily act

`ResolveIntentRequest` asks the loop to resolve one natural-language intent
against the *current* state of one application, once:

```rust,ignore
pub struct ResolveIntentRequest {
    pub app: String,
    pub intent: String,
    pub text: Option<String>,      // supplied text, if the resolved action needs it
    pub root: Option<String>,      // narrow observation to this container ref
    pub execute: bool,             // actually perform a safe resolved action?
    pub include_values: bool,
}
```

With `execute: false` this is a dry run: it tells you what it would do
without doing it. With `execute: true`, a resolved action that is judged
safe is performed.

## `RunGoal`: a bounded loop toward a visible end state

`RunGoalRequest` is the multi-step version: keep deciding and acting, within
firm limits, until a visible goal is reached or the loop gives up.

```rust,ignore
pub struct RunGoalRequest {
    pub app: String,
    pub goal: String,                          // the visible end state to reach
    pub text: Vec<String>,                      // values consumed by text actions, in order
    pub root: Option<String>,
    pub window: Option<String>,                 // exact window title to hold to
    pub window_id: Option<String>,               // exact window id; binds every observation
    pub allowed_operations: Vec<JevOperation>,   // empty keeps the legacy operation set
    pub allowed_targets: Vec<String>,            // empty keeps the legacy target set
    pub text_slots: BTreeMap<String, String>,    // prepared text by field name
    pub success: Vec<VisiblePredicate>,          // empty preserves legacy completion behavior
    pub include_values: bool,
    pub max_steps: u32,          // default 40, capped at 40
    pub max_model_calls: u32,    // default 80, capped at 80
    pub max_elapsed_ms: u64,     // default 120_000 (two minutes), capped at five minutes
    pub require_confirmations: bool, // default true
    pub continuation: Option<GoalContinuation>,
}
```

Like `RunFlowRequest`, this requires confidential bus delivery, since `text`
and `text_slots` may carry values that should never sit in a plain log.

### Closed operations, not open-ended clicks

`RunGoal` does not let the decision model do anything a screen offers; it
picks from a fixed, closed vocabulary: `Click`, `TypeText`, `Check`,
`Uncheck`, `Expand`, `Collapse`, `Scroll`, `Drill` (inspect one truncated
container), `Widen` (return to the full surface), `Wait`, `Done`, `Blocked`.
`allowed_operations` can narrow that further for one call; leaving it empty
keeps the legacy full set. That closedness is what makes the loop boundable
at all: there are finitely many things it can decide to do next, so a budget
on "how many decisions" and "how many actions" is a meaningful limit rather
than an open-ended one.

### Confirmation and verification, together

`require_confirmations: true` (the default) means a consequential action
stops the loop before it happens and hands back a one-use
`confirmation_id`. A host resumes with `RunGoalRequest.continuation =
GoalContinuation { id, approve }`. Approving does not blindly replay the
original decision: the module re-observes the target first, and a stale or
now-ambiguous target is refused rather than pressed anyway.

For a host that manages its own approvals outside this loop, setting
`require_confirmations: false` turns confirmation off, but only if `success`
is populated with `VisiblePredicate`s. Without confirmations to fall back on,
the loop needs some other way to know it actually got there, and that is
what `success` is for: a deterministic condition, checked against a *fresh*
accessibility snapshot after acting, not merely inferred from Jev's own
sense of completion.

```rust,ignore
pub enum VisiblePredicate {
    NamePresent { name: String },
    NameContains { fragment: String, within: String },
    ValueEquals { name: String, value: String },
    ValueContains { name: String, value: String },
    StateContains { name: String, state: String },
}
```

`NameContains` is the one built for a common, specific problem: a visible
result that adds a timestamp or a delivery status to an otherwise stable
piece of text. Rather than requiring an exact match on text that legitimately
varies, it asks whether `fragment` occurs somewhere inside one descendant's
accessible name, underneath an ancestor whose name exactly matches `within`.

`JevRunResult.verified` is `true` only when every requested predicate held on
that fresh snapshot; `JevRunResult.final_observation` (a `JevObservation`)
is the compact per-predicate evidence behind that verdict, which element was
found, what value or state it actually had, so a caller does not have to
trust the boolean blindly.

### Why the loop stops, precisely

`JevStopReason` spells out every way a `RunGoal` call can end, deliberately
distinguishing cases that look similar but call for different next steps:
`Done`, `Blocked` (no offered operation could advance the goal), `
ConfirmationRequired`, `Cancelled` (the host declined), `StaleTarget` (the
approved target no longer matched reality), `LowConfidence`, `NeedsText` (ran
out of supplied text for a text action), `ActionBudget`, `ModelBudget`,
`Stalled` (three turns in a row changed nothing), `ActionFailed`,
`VerificationFailed` (Jev said done, but the success predicates disagreed),
`TimeBudget`, `ScopeChanged` (the observed app or window left the caller's
declared scope), and `ActionUncertain` (a mutation may have been delivered
and must not be blindly replayed). Two of these, `StaleTarget` and
`ActionUncertain`, exist specifically so a caller never has to guess whether
retrying is safe; contrast this with `DesktopError.disposition` in
[The envelope and errors](envelope-and-errors.md), which answers the same
question at the level of one desktop command rather than a whole loop.

### `JevTurn` and the audit trail

Every executed step is recorded as a `JevTurn`: which operation ran, which
`JevTarget` it used, the confidence behind that choice, whether the desktop
command reported success, and whether the observed surface actually
changed. `JevRunResult.turns` is the ordered list of these, which is what
makes a `RunGoal` result auditable after the fact rather than a black box
that reports only its final state.

## Configuring the decision client: `JevConfig`

`JevConfig` is how a host supplies the API key and provider settings the
loaded module uses for every Jev call, for both `RunGoal`/`ResolveIntent` and
for flows and tasks underneath. It must travel with confidential bus
delivery, and its `Debug` implementation is hand-written specifically to
never print the key:

```rust,ignore
impl std::fmt::Debug for JevConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JevConfig")
            .field("api_key", &"[REDACTED]")
            // ...
    }
}
```

`JevProvider` names the decision service, and so the decision model, to talk
to: `TypeSafe` (the default, `TypeSafe`'s own System One API), `OpenRouter`,
or `TinyHumansOpenRouter` (Tiny Humans' authenticated OpenRouter proxy) for
Jev; `OpenJev` (`open_jev`, OpenJEV's Jev-compatible API, model `openjev`);
`Sage` (Levanto Sage in place of Jev, with `JevConfig::fast`), both added
in contract 2.8; or `SelfHosted` (`self_hosted`, an operator-declared
Jev-compatible decisions endpoint with no default model, where both
`JevConfig::endpoint_url` and `JevConfig::model` are required), added in
contract 2.9. `JevProvider::default_model` names the model each answers as
when `model` is absent. `JevConfiguration` is the non-secret summary of a
retained client (provider, model, endpoint override, Sage's `fast`) a caller
can ask for without ever seeing the key back; `Describe` serves it as
`Capabilities.decision_model`. The Jev client itself lives upstream in `vendor/tinyinference`, not in
this crate, see the root `CLAUDE.md`'s "Read The Right Document First"
table.

## Where this fits next to flows and tasks

`RunGoal` is the primitive `Flow`s are grounded on: a flow step like "start a
new email message" is turned, behind the scenes, into exactly this kind of
bounded observe-decide-act loop. Most callers should reach for a `Flow`
(see [Writing flows](flows.md)) or a task (see
[The Agent and task types](agent-and-tasks.md)) rather than calling
`RunGoal` directly, those two give you budgets, pausing, and a
plain-language status for free. `RunGoal` and `ResolveIntent` are here for a
caller that wants that single closed loop with nothing built on top of it.
