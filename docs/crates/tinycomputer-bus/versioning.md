# Versioning and compatibility

Source: [`crates/tinycomputer-bus/src/version/mod.rs`](../../../crates/tinycomputer-bus/src/version/mod.rs)

There are two version numbers in this crate, and they answer two different
questions. Confusing them is the easiest mistake to make when reading this
code, so this page exists mostly to keep them apart.

## `CONTRACT_VERSION`: does this host understand this module

```rust,ignore
pub const CONTRACT_VERSION: (u32, u32) = (2, 29);
```

This describes the *vocabulary*: the member set and the payload shapes,
not the crate's own package version (updated separately through a reviewed
PR; see the root `AGENTS.md`'s "Releases" section). It is a plain
`(major, minor)` pair with no pre-release component, and the rule for
bumping it is fixed:

- **Minor bump**: a member was added, or an optional field was added to an
  existing payload. A host that does not know about the new member or field
  keeps working exactly as before.
- **Major bump**: a payload's wire form changed incompatibly, or a member
  was removed or renamed. A host built against the old major version can no
  longer assume it is talking to something it understands.

Contract 2.29 publishes the typed `ComputerConfig`, shared strict site-name
normalization, and versioned consistency-checked extraction for desktop and
Agent responses. The Agent reply gains its own `version` field; this is separate
from `ENVELOPE_VERSION`, which tracks the desktop response envelope.

### The bind rule

```rust,ignore
pub fn is_compatible(module: (u32, u32)) -> bool {
    binds(CONTRACT_VERSION, module)
}

fn binds(host: (u32, u32), module: (u32, u32)) -> bool {
    let (host_major, host_minor) = host;
    let (module_major, module_minor) = module;
    module_major == host_major && module_minor >= host_minor
}
```

Two conditions, both necessary:

1. **The majors must match exactly.** A major bump means something the host
   relies on changed shape or disappeared, so a mismatched major is refused
   outright, in either direction.
2. **The module must be at least as new, minor-wise, as the host.** A host
   built against `(2, 29)` expects at least the members and fields that
   existed at `(2, 29)`. A module reporting `(2, 28)` lacks the typed
   `ComputerConfig` and versioned response extraction, so a 2.29 host rejects
   it. A module reporting `(2, 26)` lacks host-context relay for
   `ConsequenceHost`, and one reporting `(2, 25)` predates the `TraceSinkHost`
   callback added in 2.26. A newer module with the same major is accepted.

```rust,ignore
assert!(is_compatible(CONTRACT_VERSION));   // (2, 29), the exact version this crate ships
assert!(is_compatible((2, 30)));            // a newer, still-compatible module
assert!(!is_compatible((2, 11)));            // an older module, missing something
assert!(!is_compatible((2, 26)));            // predates ConsequenceHost host-context relay
assert!(!is_compatible((1, 10)));           // a different major, all bets off
```

Call `is_compatible` before a host's first real call to a freshly loaded
module: the `Version` member reports the number to check, for the desktop and
the browser members alike, because both are served on the one interface and
share the one `CONTRACT_VERSION`. Getting this check wrong in either
direction is a real failure mode: skip it and a host can call a member that
does not exist yet on an old module, or silently miss a field a new module
added because it never re-read its own assumptions.

## `ENVELOPE_VERSION`: what does a reply's shape look like

```rust,ignore
pub const ENVELOPE_VERSION: &str = "2.5";
```

This is a different axis entirely: a string, not a tuple, and it describes
the shape of `DesktopResponse` itself, its field names, what is optional,
the exact shape of `disposition`. It tracks the vendored `agent-desktop`
engine's own output format, byte for byte, because `DesktopResponse` *is*
that format, a host that already parses that engine's CLI JSON output needs
no second parser to talk to this module. See
[The envelope and errors](envelope-and-errors.md) for the full shape.

`ENVELOPE_VERSION` and `CONTRACT_VERSION` move independently. A member being
added to this crate (a minor `CONTRACT_VERSION` bump) says nothing about
whether the reply envelope's own shape changed, and vice versa. Do not read
one as implying anything about the other; check the crate's changelog or the
git history of `crates/tinycomputer-bus/src/envelope/` and
`crates/tinycomputer-bus/src/version/` for what actually happened.

The Agent interface has a separate `AGENT_ENVELOPE_VERSION`, carried by every
`AgentResponse`. `DesktopResponse::into_data` and `AgentResponse::into_data`
check their respective version and require exactly one consistent outcome
(data on success, structured error on failure) before returning a value.

## No members ever silently disappear

The crate's own `lib.rs` doc comment is explicit about one more thing worth
repeating here: this crate deliberately serves no session lifecycle, trace
read/export, or documentation-loader members, even though the underlying
engine has all three. Those are process-lifecycle concerns of a command-line
tool, not of a module that is configured once at load time. They are
candidates for a *future minor* bump, which is exactly why the bind rule
above treats "the module has an extra member the host does not use" as
compatible rather than as an error: it leaves room for exactly that kind of
addition without breaking every existing host the day it lands.

## Where the wire form itself is pinned

Every payload family in this crate keeps its own test module
(`<family>_tests.rs` beside the folder's `mod.rs`) that serializes a real
value and asserts the exact JSON it produces, and in several places decodes a literal
JSON fixture taken from the real engine's own output. That is not incidental
test coverage: it is where the wire form is actually pinned. A host and a
module that disagree about a field's name fail at runtime with a decode
error, so the shape is asserted in these tests rather than merely assumed to
hold. If you are ever unsure whether a field is really optional, or really
snake_case, or really named what the doc comment says, the `*_tests.rs` file
next to its `types.rs` (or `types/` folder) is the fastest way to find out
for certain.
