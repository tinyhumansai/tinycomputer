//! Tests for reading the accessibility tree: lines, candidates, dialogs,
//! numbered containers, and names.

use serde_json::json;

use super::PAGE;
use crate::surface::tree::{parse_line, screen, screen_without_form_values};

#[test]
fn a_line_parses_its_role_name_attributes_and_value() {
    let line = parse_line(r#"    - textbox "Say \"hi\"" [required, ref=e1]: typed"#).unwrap();
    assert_eq!(line.depth, 2);
    assert_eq!(line.role, "textbox");
    assert_eq!(line.name.as_deref(), Some("Say \"hi\""));
    assert_eq!(
        line.attributes,
        [
            ("required".to_owned(), None),
            ("ref".to_owned(), Some("e1".to_owned()))
        ]
    );
    assert_eq!(line.value.as_deref(), Some("typed"));

    let text = parse_line("- text: Hello there").unwrap();
    assert_eq!(
        (text.role.as_str(), text.value.as_deref()),
        ("text", Some("Hello there"))
    );
    let bare = parse_line("- separator").unwrap();
    assert!(bare.name.is_none() && bare.value.is_none());
    assert_eq!(parse_line("- text:").unwrap().value, None);
    assert!(parse_line("not an item").is_none());
    assert!(parse_line(r#"- button "unterminated"#).is_none());
    assert!(parse_line(r"- button [ref=e1").is_none());
}

#[test]
fn a_page_becomes_candidates_and_context() {
    let parsed = screen(PAGE, "Flights");
    let names = parsed
        .candidates
        .iter()
        .map(|node| node.name.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "From",
            "To",
            "Return trip",
            "Flexible dates",
            "Search",
            "Cabin",
            "Economy",
            "Notes"
        ],
        "the disabled Book button is not offered"
    );
    assert_eq!(parsed.window.as_deref(), Some("Flights"));
    assert_eq!(parsed.surface, "window");
    assert_eq!(parsed.context, ["Search flights", "Fares include taxes"]);
    assert!(
        parsed
            .text_nodes
            .iter()
            .any(|node| node.value.as_ref() == Some(&json!("private note"))),
        "field content stays reachable but out of context"
    );

    let from = &parsed.candidates[0];
    assert_eq!(from.ref_id, "e1");
    assert_eq!(from.value, Some(json!("Delhi")));
    assert_eq!(from.states, ["required"]);
    assert_eq!(from.available_actions, ["Click", "SetValue"]);
    assert_eq!(from.path, ["main"]);
    assert_eq!(parsed.candidates[2].available_actions, ["Click", "Check"]);
    assert_eq!(parsed.candidates[2].states, [] as [std::string::String; 0]);
    assert_eq!(parsed.candidates[3].states, ["checked"]);
    assert_eq!(parsed.candidates[4].available_actions, ["Click"]);
    assert_eq!(parsed.candidates[5].states, ["expanded"]);
    assert_eq!(parsed.candidates[6].states, ["selected"]);
    assert_eq!(parsed.candidates[6].path, ["main", "combobox \"Cabin\""]);
}

#[test]
fn dialogs_are_the_surface_in_front() {
    let dialog = screen("- dialog \"Sign in\"\n  - button \"Close\" [ref=e1]\n", "");
    assert_eq!(dialog.surface, "sheet");
    assert_eq!(dialog.window, None);
    let alert = screen("- dialog\n- alertdialog \"Leave?\"\n", "");
    assert_eq!(alert.surface, "alert");
    let many = (0..80)
        .map(|index| format!("- heading \"line {index}\"\n- text: \n"))
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(screen(&many, "").context.len(), 60, "context is capped");
    let long = format!(
        "- heading \"{}\"\n- heading \"line 1\"\n- heading \"line 1\"\n",
        "x".repeat(400)
    );
    let capped = screen(&long, "");
    assert_eq!(capped.context[0].chars().count(), 160);
    assert_eq!(capped.context.len(), 2, "duplicates are kept once");
}

#[test]
fn unreferenced_editable_values_do_not_become_shared_context() {
    let parsed = screen(
        "- heading \"Checkout\"\n- spinbutton: restored-control-value\n- text: Payment is ready\n",
        "Shop",
    );

    assert_eq!(parsed.context, ["Checkout", "Payment is ready"]);
    assert!(parsed.text_nodes.iter().any(|node| {
        node.role == "spinbutton"
            && node
                .value
                .as_ref()
                .is_some_and(|value| value == "restored-control-value")
    }));
}

#[test]
fn private_tree_observations_hide_document_descendants_and_toggle_values() {
    const UNKNOWN: &str = "profile-autofill-unknown-7Qz9";
    let tree = format!(
        r#"- heading "Checkout"
- group
  - document [ref=e1]: {UNKNOWN}
    - text: {UNKNOWN} document body
    - text "{UNKNOWN} named document body"
    - generic "{UNKNOWN} ancestor label"
      - button "{UNKNOWN} action" [ref=e5]
  - menuitemcheckbox [ref=e2]: {UNKNOWN}
    - text: {UNKNOWN} checkbox echo
  - menuitemradio [ref=e3]: {UNKNOWN}
    - text: {UNKNOWN} radio echo
  - textbox "Email" [ref=e4]: {UNKNOWN}
- text: Order total is $12.50
"#
    );

    let private = screen_without_form_values(&tree, "Checkout");
    let model_view = private
        .context
        .iter()
        .cloned()
        .chain(
            private
                .candidates
                .iter()
                .chain(&private.text_nodes)
                .map(|node| tinycomputer_core::surface::describe(node, true).to_string()),
        )
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !model_view.contains(UNKNOWN),
        "serialized candidate descriptions contain restored field text"
    );
    assert!(model_view.contains("Email"));
    assert!(model_view.contains("Order total is $12.50"));
    assert_eq!(private.candidates[0].role, "document");
    assert_eq!(private.candidates[0].value, None);
    assert_eq!(private.candidates[1].role, "menuitemcheckbox");
    assert_eq!(private.candidates[1].value, None);
    assert_eq!(private.candidates[2].role, "menuitemradio");
    assert_eq!(private.candidates[2].value, None);
    assert!(private.candidates[3].value.is_none());
    assert!(private.candidates.iter().all(|node| node.ref_id != "e5"));
    assert!(private.candidates[0].description.is_none());
    assert!(private.text_nodes.iter().all(|node| {
        node.value
            .as_ref()
            .is_none_or(|value| !value.to_string().contains(UNKNOWN))
    }));

    // The non-private surface keeps the normal tree's existing value
    // behavior; the masking is enabled only for a host with SecretHost.
    let public = screen(&tree, "Checkout");
    assert_eq!(public.candidates[0].value.as_ref(), Some(&json!(UNKNOWN)));
    assert!(
        public
            .text_nodes
            .iter()
            .any(|node| node.value.as_ref() == Some(&json!(format!("{UNKNOWN} document body"))))
    );
    assert!(
        public
            .text_nodes
            .iter()
            .any(|node| node.name.as_deref() == Some(&format!("{UNKNOWN} named document body")))
    );
    assert!(
        public.candidates.iter().any(|node| node.ref_id == "e5"
            && node.name.as_deref() == Some(&format!("{UNKNOWN} action")))
    );
    assert!(
        public
            .context
            .iter()
            .any(|line| line == "Order total is $12.50")
    );
}

#[test]
fn repeated_containers_are_numbered_so_their_cards_group() {
    let page = "- list \"Results\"\n  - listitem\n    - text: IndiGo\n    - text: ₹6,840\n    - button \"Select\" [ref=e1]\n  - listitem\n    - text: Vistara\n    - text: ₹7,210\n    - button \"Select\" [ref=e2]\n- list \"More\"\n  - listitem\n    - button \"Next page\" [ref=e3]\n";
    let parsed = screen(page, "Results");
    assert_eq!(
        parsed.candidates[0].path,
        ["list \"Results\"", "listitem #1"]
    );
    assert_eq!(
        parsed.candidates[1].path,
        ["list \"Results\"", "listitem #2"]
    );
    assert_eq!(
        parsed.candidates[2].path,
        ["list \"More\"", "listitem #1"],
        "numbering restarts under a new parent"
    );
    let groups = tinycomputer_core::surface::result_groups(&parsed);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].fields, ["IndiGo", "₹6,840", "Select"]);
    assert_eq!(groups[1].primary.as_ref().unwrap().ref_id, "e2");
}

#[test]
fn an_unnamed_control_is_named_by_what_it_shows_but_a_field_never_is() {
    // A list of cities whose rows carry a `combobox` role and an
    // `aria-labelledby` that points nowhere: no accessible name at all.
    let tree = r#"- main
  - combobox [ref=e10]
    - generic
      - text: Mumbai
      - text: Chhatrapati Shivaji Maharaj International Airport
    - text: BOM
  - combobox [ref=e11]
    - text: Srinagar
  - textbox [ref=e12]
    - text: what was typed
  - combobox [ref=e13]: typed value
    - text: suggestion
  - button "Search" [ref=e14]
    - text: Search
"#;
    let parsed = screen(tree, "Flights");
    let described = |reference: &str| {
        parsed
            .candidates
            .iter()
            .find(|candidate| candidate.ref_id == reference)
            .unwrap()
            .description
            .clone()
    };
    assert_eq!(
        described("e10").as_deref(),
        Some("Mumbai Chhatrapati Shivaji Maharaj International Airport BOM")
    );
    assert_eq!(described("e11").as_deref(), Some("Srinagar"));
    assert_eq!(
        described("e12"),
        None,
        "a text field's content stays private"
    );
    assert_eq!(
        described("e13"),
        None,
        "a control holding a value is a field"
    );
    assert_eq!(described("e14"), None, "a named control keeps its name");
}

#[test]
fn a_combobox_s_own_value_never_becomes_an_ancestors_description() {
    // An unnamed wrapper (a `div role="combobox"` some pages give a whole
    // autocomplete widget) around a nested combobox that already holds a
    // typed or selected value: the wrapper must not inherit that value as
    // its own description, even though it is itself named by what it shows.
    let tree = r"- main
  - combobox [ref=e20]
    - combobox [ref=e21]: Springfield, IL
    - text: label
";
    let parsed = screen(tree, "Flights");
    let outer = parsed
        .candidates
        .iter()
        .find(|candidate| candidate.ref_id == "e20")
        .unwrap();
    assert_eq!(
        outer.description.as_deref(),
        Some("label"),
        "the wrapper is still named by ordinary text, just not by the nested value"
    );
    assert!(
        !outer
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("Springfield"),
        "a nested combobox's own value must never reach an ancestor's description: {:?}",
        outer.description
    );
}
