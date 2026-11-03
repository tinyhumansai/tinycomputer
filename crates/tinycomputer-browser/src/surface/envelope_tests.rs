//! Tests for tests behavior.

use tinycomputer_bus::{DesktopError, DesktopResponse};

use super::covered;

#[test]
fn a_host_policy_refusal_is_not_retried_as_a_cover() {
    let refused = DesktopResponse::err(
        "click",
        DesktopError::new(
            "ACTION_REFUSED",
            "Refused: the target is covered by policy.",
        ),
    );
    assert!(!covered(&refused));

    let covered_by_page = DesktopResponse::err(
        "click",
        DesktopError::new("NOT_ACTIONABLE", "The element is covered by a banner."),
    );
    assert!(covered(&covered_by_page));
}
