//! Unit tests for the contract version and its bind rule.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{CONTRACT_VERSION, binds, is_compatible};

#[test]
fn the_shipped_contract_version_is_pinned() {
    assert_eq!(CONTRACT_VERSION, (2, 10));
}

#[test]
fn the_contract_binds_to_itself() {
    assert!(is_compatible(CONTRACT_VERSION));
}

#[test]
fn a_newer_minor_on_the_module_side_binds() {
    assert!(is_compatible((2, 10)));
    assert!(is_compatible((2, 97)));
    // 2.10 gives `StartTask` constraints `browser_executable` and
    // `browser_profile`, and `*` among the origins: a 2.10 host may send
    // them, which a 2.9 module would ignore, or hand `*` to its engine.
    assert!(!is_compatible((2, 9)));
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
