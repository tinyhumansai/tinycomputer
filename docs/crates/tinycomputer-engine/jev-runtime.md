# The Jev runtime

Jev is the small decision model everything in this crate defers to. It does
not see raw pixels. It is handed a screen already turned into a short list of
candidate elements and a small, closed question ("which of these should I
click, if any"), and it answers with a choice and a confidence. Nothing here
guesses on its own: every click, every field, every "this is done" comes from
Jev's answer.

`JevRuntime`, in `src/agentic/runtime.rs`, is the one place in this crate that
talks to Jev. The doc comment on the crate calls it "the one door to Jev",
and the code backs that up: every other module (the flow runtime, the task
controller, `RunGoal`, `ResolveIntent`) asks Jev by calling
`JevRuntime::evaluate` (indirectly, through the loops built on it), never by
holding a client of its own. That matters for three reasons: the budget is
charged in one place, secrets are masked in one place, and the debug journal
sees every exchange, with nothing able to slip past it.

## Configuring a runtime

A `JevRuntime` is built once, from the module's private configuration, with
`JevRuntime::configure`:

```rust
let runtime = JevRuntime::configure(&JevConfig {
    provider: JevProvider::TinyHumansOpenRouter,
    model: Some("jev-latest".to_owned()),
    ..JevConfig::default()
})?;
```

The configuration (`JevConfig`, in `tinycomputer-bus`) names:

| Field | What it controls |
|---|---|
| `provider` | Which decision model, through which API. See below. |
| `api_key` | The credential for that provider. Never printed; `JevRuntime`'s `Debug` implementation shows `"[configured]"` in its place. |
| `endpoint_url` | An exact endpoint to use instead of the provider's own route, checked against an allow-list (see below). |
| `model` | The Jev model or alias to ask for. Defaults to the provider's `JevProvider::default_model()`: `"jev-latest"`, `"openjev"` for OpenJEV, and the fixed `"levanto-sage"` for Sage. |
| `timeout_ms` / `max_retries` | Per-attempt HTTP timeout and how many transient retries the client makes. Sage ignores both. |
| `sdk_name` | Attribution sent only to the TinyHumans proxy, so it knows which host is calling. |
| `fast` | Sage only: score each choice in one pass rather than one per option. |

`JevRuntime::configuration()` returns the non-secret summary
(`JevConfiguration`: provider, model, endpoint override, `fast`), which the
module serves as `Describe`'s `Capabilities.decision_model`.

Once built, a `JevRuntime` is cheap to clone: cloning shares the same
underlying HTTP client, the same pending-confirmation table, and the same
journal handle. The module clones it per call rather than rebuilding it.

### Providers

Five providers are recognised, each with one approved endpoint that
`JevRuntime::configure` checks `endpoint_url` against before it will use it:

| Provider | Wire name | Approved endpoint |
|---|---|---|
| **`TypeSafe`**, TypeSafe's own System One API, the default | `type_safe` | `https://api.typesafe.ai/v1/systemone` |
| **`OpenRouter`**, OpenRouter's Jev-compatible decisions API | `open_router` | `https://openrouter.ai/api/alpha/decisions` |
| **`TinyHumansOpenRouter`**, Tiny Humans' authenticated OpenRouter proxy, the only one that takes an `sdk_name` | `tiny_humans_open_router` | `https://api.tinyhumans.ai/agent-integrations/openrouter/systemone` |
| **`OpenJev`**, OpenJEV's public System One API | `open_jev` (or `openjev`) | `https://api.openjev.sh/v1/systemone` |
| **`Sage`**, Levanto Sage in place of Jev | `sage` | `https://sage.levanto.ai/` (trailing slash optional) |
| **`SelfHosted`**, an operator-declared Jev-compatible decisions endpoint | `self_hosted` (or `selfhosted`) | any non-empty absolute HTTP(S) URL, required |

Passing an `endpoint_url` that is not the provider's own approved route
returns a `JEV_INVALID_CONFIG` error rather than silently sending credentials
somewhere unexpected. `SelfHosted` has no approved route to compare against:
the operator's declared `endpoint_url` *is* the route, so a non-empty value is
trusted, `Client::new` still enforces the URL's shape (absolute HTTP(S), no
embedded credentials, no query or fragment, plain HTTP only on a literal
loopback address), and both `endpoint_url` and `model` are required — there
is no default model to fall back on. Authentication belongs in `api_key`,
not the URL: keep tokens out of the path or query, since `Describe` echoes
the configured `endpoint_url` in its capabilities. `tinyinference-decisions` has no Tiny Humans proxy route
for OpenJEV or Sage, so none is approved. In tests, endpoints on
`http://127.0.0.1:*` are also accepted, so a scripted Jev server can stand in
for the real one.

The `sage` provider builds a runtime that answers the same loops with Levanto
Sage instead of Jev; `JevRuntime::sage(api_key, fast)` builds the same one
directly, for measuring one decision model against the other behind
identical code. Every call still goes through `JevRuntime::evaluate` below,
exactly as a Jev-backed runtime's would. See [sage.md](sage.md).

### Why a Jev call can fail

`JevRuntime::configure` itself can fail (a bad key, an unapproved endpoint)
before anything runs. Once running, every Jev call funnels its failure
through the same small set of error codes:

| Code | Cause |
|---|---|
| `JEV_AUTHENTICATION` | The key was rejected. |
| `JEV_RATE_LIMITED` | Too many requests too fast. |
| `JEV_TIMEOUT` | The provider did not answer in time. |
| `JEV_INVALID_RESPONSE` | The reply could not be parsed, or answered a question that was not asked. |
| `JEV_PROVIDER_FAILED` | Anything else. |

## The evaluate call

`JevRuntime::evaluate` takes an optional step label (used only for the
journal) and an `EvaluationRequest` (Jev's own request shape, from
`tinyinference_decisions`), and returns either an `EvaluationResult` or an
`EvaluationFailure`. Every caller in this crate builds that request the same
way it builds any other: state (the goal, the screen's app and window, recent
actions), and one or more `Question`s, each a closed choice with named
criteria.

The function itself is short, because most of the interesting work (turning
a screen into candidates, turning candidates into a question, gating the
answer against a confidence threshold) lives in the modules that call it
(`agentic::screen`, `agentic::policy`, and, for whole flows, `agentic::flow`;
see [goals-and-intents.md](goals-and-intents.md) for the two loops that live
directly in `agentic/` (`resolve.rs`, `goal.rs`), and the flow module's own docs for the rest).
`evaluate` itself does exactly two things: call the client, and hand the
request and outcome to the journal, win or lose.

```rust
async fn evaluate(
    &self,
    step: Option<&str>,
    request: &EvaluationRequest,
) -> std::result::Result<EvaluationResult, EvaluationFailure> {
    let outcome = self.client.evaluate(request).await;
    self.journal.exchange(step, request, outcome.as_ref());
    outcome
}
```

Because the journal write happens here and nowhere else, a caller that
somehow bypassed `evaluate` (there is no supported way to) would also bypass
the journal. That is the enforcement mechanism for "one door": there is
exactly one function in this crate that can produce a Jev answer, and it
always logs.

## Run identity and the journal

A `JevRuntime` carries a journal handle (see [journal.md](journal.md)) that
starts a new run each time one of the top-level entry points
(`resolve_intent`, `run_goal`, `run_flow`) begins. `JevRuntime::begin_run(kind,
label)` opens (or continues) a run in the journal and returns a new runtime
pointed at it. `JevRuntime::journaled_as(run_id)` lets a caller, such as the
task controller, which runs one task across several separate flow runs, keep
every run of one task in the same journal file, by giving them all the same
run id.

`JevRuntime::with_journal(dir)` switches the journal on unconditionally,
overriding the `TINYCOMPUTER_JEV_JOURNAL` environment variable that
`JevRuntime::configure` otherwise reads. `journal_dir()` reports where the
current run is writing, if it is writing anywhere.

## Confirmation handles

`RunGoal` can pause in front of an action Jev judged hard to undo, and hand
the caller a one-use confirmation id rather than performing it. `JevRuntime`
holds the table of these pending actions (`pending: Arc<Mutex<HashMap<String,
PendingRun>>>`) so that a later `RunGoal` call carrying a `GoalContinuation`
can find the exact action it is being asked to approve or decline. Entries
older than ten minutes are dropped, and the table is capped at 32 pending
actions at a time, past which a new confirmation is refused rather than
silently evicting an older one a caller might still be about to answer. See
[goals-and-intents.md](goals-and-intents.md) for what a caller does with a
confirmation id.

## Source

- `crates/tinycomputer-engine/src/agentic/runtime.rs`, `JevRuntime`,
  `evaluate`, provider configuration; `pending.rs`, confirmation handles.
- `crates/tinycomputer-bus/src/agentic/`, `JevConfig`, `JevProvider`, and the
  other payload types that cross the bus.
- [`docs/technical/jev-harness.md`](../../technical/jev-harness.md), the
  full Jev stack, one decision end to end, and the levers that change its
  latency.
