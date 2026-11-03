//! agent-browser's snapshot text as a decision loop's [`Screen`].
//!
//! Each line is `- role "name" [attr, attr=value, ref=eN]: value`, indented
//! two spaces per level. Lines with a ref become candidates; readable lines
//! without one — headings, paragraphs, static text — become context, the way
//! labels and status text do on the desktop.

use serde_json::Value;
use tinycomputer_core::surface::{Candidate, Screen};

/// The most context lines kept, and the longest one.
const MAX_CONTEXT_LINES: usize = 60;
const MAX_CONTEXT_CHARS: usize = 160;

/// Roles that contain a page's field content rather than its chrome; text
/// inside them never reaches the shared context.
const FIELD_ROLES: &[&str] = &[
    "textbox",
    "searchbox",
    "combobox",
    "textarea",
    "document",
    "spinbutton",
    "checkbox",
    "radio",
    "switch",
    "menuitemcheckbox",
    "menuitemradio",
];

/// Roles a value is typed into.
const TYPED_ROLES: &[&str] = &["textbox", "searchbox", "combobox", "spinbutton", "textarea"];

/// Roles whose inner text is what was typed into them.
const TEXT_ENTRY_ROLES: &[&str] = &["textbox", "searchbox", "textarea", "spinbutton"];

/// [`TEXT_ENTRY_ROLES`] plus `combobox`: every role whose own *value* is
/// typed or selected content, private unless values are shared, and so must
/// never be propagated as an enclosing unnamed container's content-derived
/// description. `combobox` is not in `TEXT_ENTRY_ROLES` itself because an
/// unnamed, valueless combobox — a city-search row's own container — must
/// still be describable by what it shows; it is only the role's *value*,
/// once it holds one, that is never safe to show as someone else's
/// description.
const VALUE_BEARING_ROLES: &[&str] =
    &["textbox", "searchbox", "textarea", "spinbutton", "combobox"];

/// Longest description an unnamed control takes from the text inside it.
const MAX_CONTENT_NAME: usize = 120;

/// Container roles that repeat as the cards of a list; each gets an ordinal
/// among its same-role siblings (`listitem #3`) so a card's contents share
/// one distinct label in their paths.
const REPEATED_ROLES: &[&str] = &[
    "listitem", "row", "article", "group", "region", "option", "gridcell", "cell", "treeitem",
];

/// Roles that are toggled.
const TOGGLED_ROLES: &[&str] = &[
    "checkbox",
    "radio",
    "switch",
    "menuitemcheckbox",
    "menuitemradio",
];

/// One parsed snapshot line.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Line {
    pub(crate) depth: usize,
    pub(crate) role: String,
    pub(crate) name: Option<String>,
    pub(crate) attributes: Vec<(String, Option<String>)>,
    pub(crate) value: Option<String>,
}

impl Line {
    /// Whether the attribute is present, as a flag or with a value.
    fn has(&self, key: &str) -> bool {
        self.attributes.iter().any(|(name, _)| name == key)
    }

    /// The attribute's value, when it has one.
    fn value_of(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.as_deref())
    }

    fn reference(&self) -> Option<&str> {
        self.value_of("ref")
    }

    fn label(&self) -> String {
        self.name.as_deref().map_or_else(
            || self.role.clone(),
            |name| format!("{} {name:?}", self.role),
        )
    }
}

/// Parses one snapshot line; `None` for anything that is not an item.
#[must_use]
pub(crate) fn parse_line(text: &str) -> Option<Line> {
    let trimmed = text.trim_start_matches(' ');
    let depth = (text.len() - trimmed.len()) / 2;
    let rest = trimmed.strip_prefix("- ")?;
    let role_end = rest.find([' ', ':']).unwrap_or(rest.len());
    let role = rest[..role_end].to_owned();
    let mut rest = &rest[role_end..];
    let mut line = Line {
        depth,
        role,
        ..Line::default()
    };
    if let Some(quoted) = rest.strip_prefix(" \"") {
        let end = closing_quote(quoted)?;
        line.name = serde_json::from_str(&format!("\"{}\"", &quoted[..end])).ok();
        rest = &quoted[end + 1..];
    }
    if let Some(bracketed) = rest.strip_prefix(" [") {
        let end = bracketed.find(']')?;
        line.attributes = bracketed[..end]
            .split(", ")
            .filter(|part| !part.is_empty())
            .map(|part| match part.split_once('=') {
                Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                None => (part.to_owned(), None),
            })
            .collect();
        rest = &bracketed[end + 1..];
    }
    if let Some((_, value)) = rest.split_once(": ") {
        line.value = Some(value.to_owned());
    } else if let Some(value) = rest.strip_prefix(':') {
        line.value = Some(value.trim().to_owned()).filter(|value| !value.is_empty());
    }
    Some(line)
}

/// The index of the unescaped quote that closes a JSON string body.
fn closing_quote(body: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in body.char_indices() {
        match character {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(index),
            _ => escaped = false,
        }
    }
    None
}

/// A whole snapshot as a [`Screen`] of the page titled `title`.
#[must_use]
pub(crate) fn screen(tree: &str, title: &str) -> Screen {
    screen_with_form_values(tree, title, true)
}

/// Parses a tree while keeping form content out of every candidate and
/// description that could be shown to a task model.
#[must_use]
pub(crate) fn screen_without_form_values(tree: &str, title: &str) -> Screen {
    screen_with_form_values(tree, title, false)
}

/// Whether a role can contain user-entered or restored form content.
pub(crate) fn is_form_role(role: &str) -> bool {
    FIELD_ROLES.contains(&role)
}

fn screen_with_form_values(tree: &str, title: &str, include_form_values: bool) -> Screen {
    let mut candidates = Vec::new();
    let mut context: Vec<String> = Vec::new();
    let mut text_nodes = Vec::new();
    let mut surface = "window".to_owned();
    // Labels of the open ancestors at each depth.
    let mut ancestors: Vec<(usize, String, String)> = Vec::new();
    // Per depth, how many siblings of each role have been seen under the
    // current parent.
    let mut siblings: Vec<std::collections::BTreeMap<String, usize>> = Vec::new();
    // Unnamed controls still open, by depth and index in `candidates`: the
    // text inside one is what it shows, and becomes its description.
    let mut unnamed: Vec<(usize, usize)> = Vec::new();
    for (order, line) in tree.lines().filter_map(parse_line).enumerate() {
        ancestors.retain(|(depth, _, _)| *depth < line.depth);
        unnamed.retain(|(depth, _)| *depth < line.depth);
        let inside_field = ancestors
            .iter()
            .any(|(_, _, role)| FIELD_ROLES.contains(&role.as_str()));
        // A value-bearing line's value is what was typed or selected into
        // it: private unless values are shared, and never a safe
        // description for an enclosing unnamed container. Its accessible
        // name — a label, not content — still is.
        let describable = if !include_form_values
            && (inside_field || FIELD_ROLES.contains(&line.role.as_str()))
        {
            line.name.as_deref().filter(|_| !inside_field)
        } else if VALUE_BEARING_ROLES.contains(&line.role.as_str()) {
            line.name.as_deref()
        } else {
            line.name.as_deref().or(line.value.as_deref())
        };
        if let Some(text) = describable {
            for (_, index) in &unnamed {
                describe_by_content(&mut candidates[*index], text);
            }
        }
        let path = ancestors
            .iter()
            .map(|(_, label, _)| label.clone())
            .collect::<Vec<_>>();
        match line.role.as_str() {
            "dialog" if surface == "window" => "sheet".clone_into(&mut surface),
            "alertdialog" => "alert".clone_into(&mut surface),
            _ => {}
        }
        if include_form_values || !inside_field {
            let mut node = candidate(&line, path, order);
            if !include_form_values && (inside_field || FIELD_ROLES.contains(&line.role.as_str())) {
                node.value = None;
                if inside_field && line.reference().is_none() {
                    // Unreferenced descendants become text_nodes. Their tree
                    // name can carry the same rich-field body as their value.
                    node.name = None;
                }
            }
            if line.reference().is_some() {
                if !line.has("disabled") {
                    // A control with no name of its own — a list row whose
                    // `aria-labelledby` points nowhere — is named by what it
                    // shows. A text field — or a control holding a value — is not:
                    // what is inside it is its content, private unless values
                    // are shared.
                    if node.name.is_none()
                        && node.value.is_none()
                        && !TEXT_ENTRY_ROLES.contains(&line.role.as_str())
                    {
                        unnamed.push((line.depth, candidates.len()));
                    }
                    candidates.push(node);
                }
            } else {
                let is_field = FIELD_ROLES.contains(&line.role.as_str())
                    || VALUE_BEARING_ROLES.contains(&line.role.as_str());
                if !inside_field && !is_field {
                    remember(&line, &mut context);
                }
                text_nodes.push(node);
            }
        }
        siblings.truncate(line.depth + 1);
        siblings.resize_with(line.depth + 1, Default::default);
        let seen = siblings[line.depth].entry(line.role.clone()).or_default();
        *seen += 1;
        let label = if REPEATED_ROLES.contains(&line.role.as_str()) {
            format!("{} #{seen}", line.label())
        } else {
            line.label()
        };
        ancestors.push((line.depth, label, line.role));
    }
    Screen {
        app: "browser".to_owned(),
        window: Some(title.to_owned()).filter(|title| !title.is_empty()),
        surface,
        candidates,
        context,
        unexplored: Vec::new(),
        text_nodes,
    }
}

fn candidate(line: &Line, path: Vec<String>, order: usize) -> Candidate {
    let states = ["checked", "expanded", "selected", "required"]
        .into_iter()
        .filter(|state| {
            line.value_of(state)
                .map_or_else(|| line.has(state), |value| value == "true")
        })
        .map(str::to_owned)
        .collect();
    let available_actions = if TYPED_ROLES.contains(&line.role.as_str()) {
        vec!["Click".to_owned(), "SetValue".to_owned()]
    } else if TOGGLED_ROLES.contains(&line.role.as_str()) {
        vec!["Click".to_owned(), "Check".to_owned()]
    } else {
        vec!["Click".to_owned()]
    };
    Candidate {
        ref_id: line.reference().unwrap_or_default().to_owned(),
        role: line.role.clone(),
        name: line.name.clone().filter(|name| !name.is_empty()),
        value: line.value.clone().map(Value::String),
        states,
        available_actions,
        path,
        order,
        ..Candidate::default()
    }
}

/// Adds `text` to what an unnamed control shows, up to
/// [`MAX_CONTENT_NAME`] characters.
fn describe_by_content(candidate: &mut Candidate, text: &str) {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return;
    }
    let mut shown = candidate.description.take().unwrap_or_default();
    if shown.chars().count() < MAX_CONTENT_NAME && !shown.contains(&text) {
        if !shown.is_empty() {
            shown.push(' ');
        }
        shown.push_str(&text);
    }
    candidate.description = Some(shown.chars().take(MAX_CONTENT_NAME).collect());
}

fn remember(line: &Line, context: &mut Vec<String>) {
    if context.len() >= MAX_CONTEXT_LINES {
        return;
    }
    let Some(text) = line.name.as_deref().or(line.value.as_deref()) else {
        return;
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return;
    }
    let text: String = text.chars().take(MAX_CONTEXT_CHARS).collect();
    if !context.contains(&text) {
        context.push(text);
    }
}
