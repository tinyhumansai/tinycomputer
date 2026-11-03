//! Tests for tests behavior.

use serde_json::json;
use tinycomputer_bus::browser::{Action, ElementRef, Target};

use super::{ActionContext, ActionRequest, action_target, resolve_reference};

fn refs() -> Vec<ElementRef> {
    vec![ElementRef {
        id: "e1".to_owned(),
        role: "button".to_owned(),
        name: "Place order".to_owned(),
    }]
}

#[test]
fn only_explicit_snapshot_refs_resolve_to_snapshot_metadata() {
    let refs = refs();
    assert_eq!(
        resolve_reference("@e1", &refs).map(|target| target.unique),
        Some(true)
    );
    assert_eq!(
        resolve_reference("e1", &refs).map(|target| target.unique),
        Some(true)
    );
    assert!(
        action_target(
            &Action::Click {
                target: Target::selector("e1"),
                new_tab: false,
            },
            &refs,
        )
        .is_none()
    );
}

#[test]
fn duplicate_snapshot_targets_are_not_reported_as_unique() {
    let mut refs = refs();
    refs.push(ElementRef {
        id: "e2".to_owned(),
        role: "button".to_owned(),
        name: "Place order".to_owned(),
    });
    assert_eq!(
        resolve_reference("e1", &refs).map(|target| target.unique),
        Some(false)
    );
    assert_eq!(
        resolve_reference("e2", &refs).map(|target| target.unique),
        Some(false)
    );
}

#[test]
fn raw_css_selectors_do_not_alias_snapshot_refs() {
    let refs = refs();
    let css = ActionRequest::raw(
        json!({"action": "click", "selector": "e1"}),
        Some("https://shop.test/".to_owned()),
        None,
        &ActionContext::default(),
        &refs,
    );
    let reference = ActionRequest::raw(
        json!({"action": "click", "selector": "@e1"}),
        Some("https://shop.test/".to_owned()),
        None,
        &ActionContext::default(),
        &refs,
    );
    assert!(css.target.is_none());
    assert_eq!(
        reference
            .target
            .as_ref()
            .and_then(|target| target.name.as_deref()),
        Some("Place order")
    );
}
