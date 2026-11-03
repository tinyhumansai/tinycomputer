//! Unit tests for the contract version and its bind rule.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{CONTRACT_VERSION, binds, is_compatible};

#[test]
fn the_shipped_contract_version_is_pinned() {
    assert_eq!(CONTRACT_VERSION, (2, 29));
}

#[test]
fn the_contract_binds_to_itself() {
    assert!(is_compatible(CONTRACT_VERSION));
}

#[test]
fn a_newer_minor_on_the_module_side_binds() {
    assert!(is_compatible((2, 29)));
    assert!(is_compatible((2, 97)));
    // 2.24 adds same-task flow-to-planner handoff to StartTask.
    // 2.22 adds optional host-held secret handles to StartTask and RunFlow.
    // A host-only handle is ignored by older modules, which then refuse it
    // instead of trying to type its opaque identifier as a value.
    // 2.12 adds replayable GlobeRead and joined terminal GlobeShutdown.
    // A 2.11 module does not implement those members.
    // 2.10 gives `StartTask` constraints `browser_executable` and
    // `browser_profile`, and `*` among the origins: a 2.10 host may send
    // them, which a 2.9 module would ignore, or hand `*` to its engine.
    // 2.13 adds the SiteMemoryHost host-callback interface (Load/Save/Forget):
    // a 2.13 host serves it, which a 2.12 module does not call.
    // 2.14 adds BrowserClaimProfile and BrowserReleaseProfile.
    // 2.15 adds browser::tool: the recorded `browser` tool schema and ToolArgs.
    // 2.16 adds desktop_tool: the eight desktop tool declarations and request builders.
    // 2.17 adds `StartTask.site` and the `epoch` on `SiteMemoryHost` Load/Save.
    // 2.18 gives `Press` an optional `target`, which the module focuses and
    // checks before the key goes: a 2.17 module would ignore the field and
    // press the key wherever the focus is.
    // 2.19 adds the ConsequenceHost host-callback interface (Review): a 2.19
    // host serves it, which a 2.18 module does not call, so its approvals
    // would never reach the host's policy.
    // 2.20 adds SessionOptions identity fields.
    // 2.25 adds BrowserEgressHost.
    // 2.26 adds TraceSinkHost, so a 2.25 module lacks that callback.
    // 2.27 relays opaque host context to ConsequenceHost reviews. A 2.26
    // module would drop that context, so a host requiring it cannot bind.
    // 2.28 adds `max_do_turns` to TaskBudget and RunFlowRequest. A 2.27
    // module ignores it and keeps every `do` step at 8 turns.
    // 2.29 adds typed ComputerConfig, shared strict site normalization, and
    // versioned consistency-checked Desktop and Agent response extraction.
    assert!(!is_compatible((2, 28)));
    assert!(!is_compatible((2, 27)));
    assert!(!is_compatible((2, 26)));
    assert!(!is_compatible((2, 25)));
    assert!(!is_compatible((2, 24)));
    assert!(!is_compatible((2, 23)));
    assert!(!is_compatible((2, 22)));
    // BrowserEgressHost authorizes, consumes, and validates each browser dial.
    assert!(!is_compatible((2, 24)));
    assert!(!is_compatible((2, 20)));
    assert!(!is_compatible((2, 19)));
    assert!(!is_compatible((2, 18)));
    assert!(!is_compatible((2, 17)));
    assert!(!is_compatible((2, 16)));
    assert!(!is_compatible((2, 15)));
    assert!(!is_compatible((2, 14)));
    assert!(!is_compatible((2, 13)));
    assert!(!is_compatible((2, 12)));
    assert!(!is_compatible((2, 11)));
    // 2.9 gives `RunFlow` and its result `dialog_left_open`: a 2.9 host may
    // send it, which a 2.8 module would ignore.
    assert!(!is_compatible((2, 8)));
    // 2.8 accepts the `open_jev` and `sage` decision providers and the
    // planner's `tiny_humans` route: a 2.8 host may configure them, which a
    // 2.7 module refuses.
    assert!(!is_compatible((2, 7)));
    // 2.7 gives `TaskReport` a request a confidential call can carry, and
    // `trace`: a 2.7 host may ask a report without it, which a 2.6 module
    // would ignore.
    assert!(!is_compatible((2, 6)));
    assert!(!is_compatible((2, 5)));
    assert!(!is_compatible((2, 4)));
    assert!(!is_compatible((2, 3)));
    assert!(!is_compatible((2, 0)));
}

#[test]
fn an_older_minor_on_the_module_side_is_rejected() {
    // A host built against 1.8 cannot call a 1.7 module: the brief and vote
    // fields it sends are not understood there.
    assert!(!binds((1, 8), (1, 7)));
    assert!(binds((1, 8), (1, 8)));
    assert!(!binds((2, 1), (2, 0)));
}

#[test]
fn a_different_major_is_rejected() {
    assert!(!is_compatible((0, 0)));
    // 2.0 renamed the interface from `tinydesktop` to `tinycomputer`: a
    // 1.x host calls names a 2.x module does not serve.
    assert!(!is_compatible((1, 8)));
    assert!(!is_compatible((3, 0)));
}
