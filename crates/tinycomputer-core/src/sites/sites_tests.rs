//! A finished browser task leaves its plan and the elements it found for the
//! next task on the same site, the plan only for the same goal and facts;
//! nothing that holds a fact, reads like page text, needed a rescue, or
//! failed when reused is kept, and none of it outlives its use.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value
)]

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tinycomputer_bus::agent::TaskReport;

use super::*;

fn facts() -> BTreeMap<String, String> {
    BTreeMap::from([("name".into(), "Asha Raina".into())])
}

fn follow(goal: &str, facts: &BTreeMap<String, String>) -> Following {
    Following::new("shop.test".into(), goal, facts, false)
}

fn flow(steps: Value) -> Value {
    json!({"app": "browser", "steps": steps})
}

fn hint(key: &str, name: &str) -> Value {
    json!({"app": "browser", "key": key, "role": "button", "name": name, "path": ["main", "product"]})
}

fn report(flow: Value, learned: Value, rescues: Value) -> TaskReport {
    serde_json::from_value(json!({
        "view": {"id": "t", "status": {"state": "done", "answer": "ordered", "records": {}},
                 "summary": "", "progress": 1.0, "next": []},
        "flow": flow, "steps": [], "records": {}, "artifacts": [],
        "learned": learned, "trace": [], "rescues": rescues
    }))
    .unwrap()
}

fn rescue() -> Value {
    json!([{"step": 1, "failure": "covered", "reason": "a size comes first",
            "steps": ["choose a size"], "covers": 0, "outcome": "recovered"}])
}

#[test]
fn a_site_is_its_host_without_www() {
    assert_eq!(
        site_of("https://www.Amazon.in/s?k=x").as_deref(),
        Some("amazon.in")
    );
    assert_eq!(
        site_of(" http://shop.test:8080/cart ").as_deref(),
        Some("shop.test")
    );
    assert_eq!(site_of("file:///tmp/page.html"), None);
    assert_eq!(site_of("https://[::1]/"), None);
    assert_eq!(site_of("not an address"), None);
    assert_eq!(site_named(".hidden"), None);
}

#[test]
fn site_names_reject_adjacent_dots() {
    assert_eq!(site_named("a..b"), None);
}

#[test]
fn site_names_reject_overlong_file_stems() {
    assert_eq!(site_named(&"a".repeat(254)), None);
}

#[test]
fn site_urls_keep_url_parser_idna_and_ip_address_behavior() {
    assert_eq!(
        site_of("https://www.bücher.example/").as_deref(),
        Some("xn--bcher-kva.example")
    );
    assert_eq!(site_of("https://[::1]/"), None);
    assert_eq!(site_of("https://127.0.0.1/"), None);
    assert_eq!(site_of("http://10.0.0.1:8080/login"), None);
    assert_eq!(site_named("192.168.1.1"), None);
    assert_eq!(site_named("123.example").as_deref(), Some("123.example"));
    assert_eq!(site_named("bücher.example"), None);
}

#[test]
fn site_names_are_derived_from_the_url_authority_only() {
    assert_eq!(
        site_of("https://www.shop.test:8443/a/path?next=evil.example#section").as_deref(),
        Some("shop.test")
    );
}

#[test]
fn goals_are_matched_whatever_their_case_and_spacing() {
    assert_eq!(
        goal_key("  Order   MILK\non shop.test "),
        "order milk on shop.test"
    );
}

#[test]
fn the_facts_digest_is_stable_and_holds_no_value() {
    let id = facts_id(&facts());
    assert_eq!(id.len(), 64);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(id, facts_id(&facts()));
    assert!(!id.contains("Asha"));
}

#[test]
fn a_finished_task_leaves_its_plan_and_elements_for_the_next_task() {
    let order = follow("Start at https://shop.test. Order milk", &facts());
    let mut memory = SiteMemory::default();
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_none());

    let ran = flow(json!([{"browse": "https://shop.test"}, "add milk to the cart"]));
    let found = json!([
        hint("add milk to the cart", "Add to cart"),
        hint("deliver", "Deliver to Asha Raina"),
        hint("read", &"long page text ".repeat(10)),
    ]);
    memory.learn(&order, &report(ran.clone(), found, json!([])), 10);

    let plan = memory.plan_for(order.goal(), order.facts_id()).unwrap();
    assert_eq!(serde_json::to_value(plan).unwrap(), ran);
    let names: Vec<_> = memory
        .hints
        .iter()
        .map(|saved| saved.hint.name.clone())
        .collect();
    assert_eq!(
        names,
        [Some("Add to cart".to_owned())],
        "no fact, no page text"
    );

    // Another goal on the same site gets the elements, not the plan.
    let eggs = follow("Start at https://shop.test. Order eggs", &facts());
    assert!(memory.plan_for(eggs.goal(), eggs.facts_id()).is_none());
}

#[test]
fn a_plan_that_needed_a_rescue_or_holds_a_fact_is_not_kept() {
    let order = follow("Start at https://shop.test. Order milk", &facts());
    let mut memory = SiteMemory::default();
    let ran = flow(json!(["add milk to the cart"]));
    memory.learn(
        &order,
        &report(ran, json!([hint("add", "Add")]), rescue()),
        10,
    );
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_none());
    assert_eq!(memory.hints.len(), 1, "its elements are");

    // A plan typing a fact the goal does not name could type the wrong one.
    let typed = flow(json!([{"enter": {"name": "Asha Raina"}}]));
    memory.learn(&order, &report(typed.clone(), json!([]), json!([])), 11);
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_none());

    // Named in the goal, the same value is part of what the plan is for.
    let named = follow(
        "Start at https://shop.test. Book a table for Asha Raina",
        &facts(),
    );
    memory.learn(&named, &report(typed, json!([]), json!([])), 12);
    assert!(memory.plan_for(named.goal(), named.facts_id()).is_some());
}

#[test]
fn a_plan_is_reused_only_with_the_facts_it_ran_with() {
    let order = follow("Start at https://shop.test. Order a shirt", &facts());
    let mut memory = SiteMemory::default();
    let ran = flow(json!(["choose Medium", "add the shirt to the cart"]));
    memory.learn(
        &order,
        &report(ran, json!([hint("add", "Add to bag")]), json!([])),
        10,
    );

    let mut more = facts();
    more.insert("size".into(), "L".into());
    let larger = follow("Start at https://shop.test. Order a shirt", &more);
    assert!(memory.plan_for(larger.goal(), larger.facts_id()).is_none());
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_some());
    let saved = serde_json::to_string(&memory).unwrap();
    assert!(!saved.contains("Asha Raina"), "facts are kept as a digest");
}

#[test]
fn a_reused_plan_that_needs_a_rescue_is_forgotten() {
    let mut order = follow("Start at https://shop.test. Order milk", &facts());
    let mut memory = SiteMemory::default();
    let ran = flow(json!(["add milk to the cart"]));
    memory.learn(&order, &report(ran.clone(), json!([]), json!([])), 10);
    // A run that was not on a reused plan leaves a saved one alone.
    memory.learn(&order, &report(ran.clone(), json!([]), rescue()), 11);
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_some());

    order = Following::new(
        "shop.test".into(),
        "Start at https://shop.test. Order milk",
        &facts(),
        true,
    );
    assert!(order.reuses_plan());
    memory.learn(&order, &report(ran, json!([]), rescue()), 12);
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_none());
}

#[test]
fn a_refused_or_failed_plan_is_forgotten_by_goal_and_facts_alone() {
    let mut order = Following::new("shop.test".into(), "Order milk", &facts(), true);
    let mut memory = SiteMemory::default();
    memory.learn(
        &order,
        &report(flow(json!(["a"])), json!([]), json!([])),
        10,
    );
    let other = follow("Order eggs", &facts());
    memory.learn(
        &other,
        &report(flow(json!(["b"])), json!([]), json!([])),
        10,
    );
    memory.forget_plan(&order);
    assert!(memory.plan_for(order.goal(), order.facts_id()).is_none());
    assert!(memory.plan_for(other.goal(), other.facts_id()).is_some());
    order.plan_refused();
    assert!(!order.reuses_plan());
}

#[test]
fn a_repeat_run_counts_and_replaces_its_plan() {
    let order = follow("Order milk", &facts());
    let mut memory = SiteMemory::default();
    memory.learn(
        &order,
        &report(flow(json!(["a"])), json!([]), json!([])),
        10,
    );
    memory.learn(
        &order,
        &report(flow(json!(["b"])), json!([]), json!([])),
        20,
    );
    assert_eq!(memory.plans.len(), 1);
    assert_eq!(memory.plans[0].runs, 2);
    assert_eq!(memory.plans[0].ok_at, 20);
}

#[test]
fn values_are_found_however_the_module_spells_them() {
    let mut signup_facts = BTreeMap::new();
    signup_facts.insert("email".to_owned(), "asha.r@example.com".to_owned());
    signup_facts.insert("born".to_owned(), "1990-05-12".to_owned());
    let mut signup = follow("Start at https://shop.test. Sign up", &signup_facts);
    // A value given when the paused task resumes is not kept either.
    signup.note_inputs([&"98-7654-3210".to_owned()]);
    let found = json!([
        hint("type asha r example com into the email box", "Email"),
        {"app": "browser", "key": "pick the day", "role": "button", "name": "12", "path": ["1990 05 12"]},
        hint("enter 98 7654 3210", "Phone"),
        hint("press sign up", "Sign up"),
    ]);
    let mut memory = SiteMemory::default();
    memory.learn(&signup, &report(flow(json!(["a"])), found, json!([])), 10);
    let keys: Vec<_> = memory
        .hints
        .iter()
        .map(|saved| saved.hint.key.as_str())
        .collect();
    assert_eq!(keys, ["press sign up"]);
}

#[test]
fn old_entries_go_and_limits_hold() {
    let order = follow("Order milk", &facts());
    let found: Vec<Value> = (0..MAX_HINTS + 5)
        .map(|index| hint(&format!("step {index}"), &format!("Button {index}")))
        .collect();
    let mut memory = SiteMemory::default();
    memory.learn(
        &order,
        &report(flow(json!(["a"])), json!(found), json!([])),
        100,
    );
    assert_eq!(memory.hints.len(), MAX_HINTS);
    assert_eq!(memory.hints[0].hint.key, "step 5", "the oldest went first");
    for index in 0..MAX_PLANS + 2 {
        let goal = follow(&format!("goal {index}"), &facts());
        let at = 200 + u64::try_from(index).unwrap();
        memory.learn(&goal, &report(flow(json!(["a"])), json!([]), json!([])), at);
    }
    assert_eq!(memory.plans.len(), MAX_PLANS);
    assert!(
        memory
            .plans
            .iter()
            .all(|plan| plan.goal != "goal 0" && plan.goal != "goal 1")
    );
    memory.expire(100 + KEEP_SECS);
    assert_eq!(memory.hints.len(), 0, "unused for 30 days");
    assert_eq!(memory.plans.len(), MAX_PLANS);
    memory.expire(300 + KEEP_SECS + 100);
    assert!(memory.is_empty());
}

#[test]
fn a_memory_is_fitted_to_the_size_a_host_keeps() {
    let order = follow("Order milk", &facts());
    let mut memory = SiteMemory::default();
    memory.learn(
        &order,
        &report(flow(json!(["a"])), json!([hint("a", "A")]), json!([])),
        10,
    );
    let bytes = memory.fit().unwrap();
    assert!(u64::try_from(bytes.len()).unwrap() <= MAX_FILE_BYTES);
    let back: SiteMemory = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back, memory);

    // Oversized: the oldest plans go first, then the oldest elements.
    let big = "x".repeat(200_000);
    for index in 0..20 {
        memory.plans.push(SavedPlan {
            goal: format!("big {index}"),
            facts_id: String::new(),
            flow: serde_json::from_value(flow(json!([big.clone()]))).unwrap(),
            ok_at: 5 + index,
            runs: 1,
        });
    }
    let bytes = memory.fit().unwrap();
    assert!(u64::try_from(bytes.len()).unwrap() <= MAX_FILE_BYTES);
    assert!(memory.plans.len() < 21);
    assert!(memory.plans.iter().all(|plan| plan.goal != "big 0"));
}
