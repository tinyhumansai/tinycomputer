//! Accessibility snapshot parsing and compact Jev candidate descriptions.

use serde::Deserialize;
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};
#[cfg(feature = "desktop")]
use tinycomputer_bus::Surface;
#[cfg(feature = "desktop")]
use tinycomputer_bus::{DesktopResponse, SnapshotRequest};

#[cfg(feature = "desktop")]
use crate::Desktop;

const MAX_TREE_DEPTH: usize = 64;
const MAX_VISITED_NODES: usize = 4_096;
const MAX_FINGERPRINT_NODES: usize = MAX_VISITED_NODES;
const MAX_FINGERPRINT_ITEMS: usize = 16;
const MAX_FINGERPRINT_BYTES: usize = 128;
const MAX_FINGERPRINT_VALUE_DEPTH: usize = 4;

/// One ref-bearing accessibility node offered to Jev.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct Candidate {
    pub(super) ref_id: String,
    pub(super) role: String,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) native_id: Option<NativeId>,
    pub(super) value: Option<Value>,
    pub(super) states: Vec<String>,
    pub(super) available_actions: Vec<String>,
    pub(super) children_count: Option<usize>,
    pub(super) bounds: Option<Value>,
    pub(super) children: Vec<Candidate>,
    #[serde(skip)]
    pub(super) path: Vec<String>,
}

/// Engine-provided platform identifier for an otherwise unlabeled element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(super) struct NativeId {
    pub(super) kind: String,
    pub(super) value: String,
}

impl Candidate {
    pub(super) fn labels(&self) -> impl Iterator<Item = &str> {
        [
            self.name.as_deref(),
            self.description.as_deref(),
            self.native_id.as_ref().map(|id| id.value.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|label| !label.is_empty())
    }

    pub(super) fn label(&self) -> Option<&str> {
        self.labels().next()
    }
}

/// Parsed current surface.
#[derive(Debug, Clone)]
pub(super) struct Screen {
    pub(super) app: String,
    pub(super) window: Option<String>,
    pub(super) window_id: Option<String>,
    pub(super) surface: String,
    pub(super) root: Option<String>,
    pub(super) candidates: Vec<Candidate>,
    pub(super) observed: Vec<Candidate>,
}

#[cfg(feature = "desktop")]
pub(super) fn observe(
    desktop: &Desktop,
    app: &str,
    window_id: Option<&str>,
    root: Option<&str>,
) -> Result<Screen, Box<DesktopResponse>> {
    let request = snapshot_request(app, window_id, root);
    let mut reply = desktop.snapshot(request);
    if !reply.ok && root.is_none() && window_id.is_none() {
        reply = desktop.snapshot(SnapshotRequest {
            app: Some(app.to_owned()),
            max_depth: Some(4),
            include_bounds: true,
            compact: true,
            ..SnapshotRequest::default()
        });
    }
    parse_reply(desktop, app, window_id, root, reply)
}

#[cfg(feature = "desktop")]
pub(super) fn snapshot_request(
    app: &str,
    window_id: Option<&str>,
    root: Option<&str>,
) -> SnapshotRequest {
    SnapshotRequest {
        app: Some(app.to_owned()),
        window_id: window_id.map(str::to_owned),
        include_bounds: true,
        interactive_only: false,
        compact: true,
        root_ref: root.map(str::to_owned),
        ..SnapshotRequest::default()
    }
}

#[cfg(feature = "desktop")]
pub(super) fn parse_reply(
    desktop: &Desktop,
    app: &str,
    window_id: Option<&str>,
    root: Option<&str>,
    mut reply: DesktopResponse,
) -> Result<Screen, Box<DesktopResponse>> {
    if !reply.ok {
        return Err(Box::new(reply));
    }
    let mut surface = "window".to_owned();
    if root.is_none()
        && let Some(role) = overlay_role(reply.data.as_ref().and_then(|data| data.get("tree")))
    {
        let overlay = match role {
            "sheet" => Surface::Sheet,
            "alert" => Surface::Alert,
            "menu" => Surface::Menu,
            "popover" => Surface::Popover,
            _ => Surface::Window,
        };
        let scoped = desktop.snapshot(SnapshotRequest {
            app: Some(app.to_owned()),
            window_id: window_id.map(str::to_owned),
            include_bounds: true,
            compact: true,
            surface: overlay,
            ..SnapshotRequest::default()
        });
        if scoped.ok {
            reply = scoped;
            role.clone_into(&mut surface);
        }
    }
    let Some(data) = reply.data.as_ref() else {
        return Err(Box::new(DesktopResponse::err(
            "snapshot",
            tinycomputer_bus::DesktopError::new("INTERNAL", "successful snapshot carried no data"),
        )));
    };
    let tree = data.get("tree").cloned().unwrap_or(Value::Null);
    let mut root_node: Candidate = serde_json::from_value(tree).unwrap_or_default();
    let mut candidates = Vec::new();
    let mut visited = 0_usize;
    collect(&mut root_node, &[], &mut candidates, 0, &mut visited);
    let observed = candidates.clone();
    candidates.retain(|node| !node.ref_id.is_empty() && offerable(node));
    candidates.truncate(254);

    Ok(Screen {
        app: data
            .get("app")
            .and_then(Value::as_str)
            .unwrap_or(app)
            .to_owned(),
        window: data
            .get("window")
            .and_then(|window| window.get("title"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        window_id: data
            .get("window")
            .and_then(|window| window.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        surface,
        root: root.map(str::to_owned),
        candidates,
        observed,
    })
}

fn collect(
    node: &mut Candidate,
    path: &[String],
    out: &mut Vec<Candidate>,
    depth: usize,
    visited: &mut usize,
) {
    if depth > MAX_TREE_DEPTH || *visited >= MAX_VISITED_NODES {
        return;
    }
    *visited = visited.saturating_add(1);
    let label = node.label().map_or_else(
        || node.role.clone(),
        |name| format!("{} {name:?}", node.role),
    );
    node.path = path.to_vec();
    out.push(Candidate {
        ref_id: node.ref_id.clone(),
        role: node.role.clone(),
        name: node.name.clone(),
        description: node.description.clone(),
        native_id: node.native_id.clone(),
        value: node.value.clone(),
        states: node.states.clone(),
        available_actions: node.available_actions.clone(),
        children_count: node.children_count,
        bounds: node.bounds.clone(),
        children: Vec::new(),
        path: node.path.clone(),
    });
    let mut child_path = path.to_vec();
    if !node.children.is_empty() {
        child_path.push(label);
    }
    for child in &mut node.children {
        collect(child, &child_path, out, depth.saturating_add(1), visited);
    }
}

fn offerable(node: &Candidate) -> bool {
    !node.available_actions.is_empty()
        && !node
            .states
            .iter()
            .any(|state| matches!(state.as_str(), "disabled" | "hidden"))
}

fn overlay_role(tree: Option<&Value>) -> Option<&'static str> {
    let mut pending = vec![(tree?, 0_usize)];
    let mut visited = 0_usize;
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_TREE_DEPTH || visited >= MAX_VISITED_NODES {
            continue;
        }
        visited = visited.saturating_add(1);
        if let Some(role) = node.get("role").and_then(Value::as_str)
            && matches!(role, "sheet" | "alert" | "menu" | "popover")
        {
            return Some(match role {
                "sheet" => "sheet",
                "alert" => "alert",
                "menu" => "menu",
                _ => "popover",
            });
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            pending.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (child, depth.saturating_add(1))),
            );
        }
    }
    None
}

pub(super) fn describe(node: &Candidate, include_values: bool) -> Value {
    let mut value = json!({
        "what": format!(
            "{}{}",
            node.role,
            node.label()
                .map_or_else(String::new, |name| format!(" {name:?}"))
        ),
        "where": if node.path.is_empty() { "top level".to_owned() } else { node.path.join(" > ") },
        "supports": node.available_actions,
    });
    if include_values && let Some(held) = node.value.as_ref().filter(|value| !value.is_null()) {
        value["holds"] = Value::String(held.to_string().chars().take(120).collect());
    }
    if !node.states.is_empty() {
        value["state"] = Value::String(node.states.join(", "));
    }
    if let Some(count) = node.children_count {
        value["contains"] = json!(count);
    }
    if node.label().is_none()
        && node.value.is_none()
        && let Some(bounds) = &node.bounds
    {
        value["bounds"] = bounds.clone();
    }
    json!({"untrusted_accessibility_data": value})
}

pub(super) fn fingerprint(screen: &Screen) -> String {
    let visible = if screen.observed.is_empty() {
        &screen.candidates
    } else {
        &screen.observed
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    visible.len().hash(&mut hash);
    for node in visible.iter().take(MAX_FINGERPRINT_NODES) {
        hash_text(&node.role, &mut hash);
        hash_text(node.label().unwrap_or_default(), &mut hash);
        hash_texts(&node.path, &mut hash);
        hash_texts(&node.states, &mut hash);
        hash_texts(&node.available_actions, &mut hash);
        node.children_count.hash(&mut hash);
        hash_value(node.value.as_ref(), 0, &mut hash);
    }
    format!("{:016x}", hash.finish())
}

fn hash_text(text: &str, hash: &mut impl Hasher) {
    text.len().hash(hash);
    let mut end = text.len().min(MAX_FINGERPRINT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].hash(hash);
}

fn hash_texts(texts: &[String], hash: &mut impl Hasher) {
    texts.len().hash(hash);
    for text in texts.iter().take(MAX_FINGERPRINT_ITEMS) {
        hash_text(text, hash);
    }
}

fn hash_value(value: Option<&Value>, depth: usize, hash: &mut impl Hasher) {
    if depth >= MAX_FINGERPRINT_VALUE_DEPTH {
        return;
    }
    match value {
        None => 0_u8.hash(hash),
        Some(Value::Null) => 1_u8.hash(hash),
        Some(Value::Bool(value)) => {
            2_u8.hash(hash);
            value.hash(hash);
        }
        Some(Value::Number(value)) => {
            3_u8.hash(hash);
            value.to_string().hash(hash);
        }
        Some(Value::String(value)) => {
            4_u8.hash(hash);
            hash_text(value, hash);
        }
        Some(Value::Array(values)) => {
            5_u8.hash(hash);
            values.len().hash(hash);
            for value in values.iter().take(MAX_FINGERPRINT_ITEMS) {
                hash_value(Some(value), depth + 1, hash);
            }
        }
        Some(Value::Object(values)) => {
            6_u8.hash(hash);
            values.len().hash(hash);
            for (key, value) in values.iter().take(MAX_FINGERPRINT_ITEMS) {
                hash_text(key, hash);
                hash_value(Some(value), depth + 1, hash);
            }
        }
    }
}

#[cfg(test)]
mod screen_tests;
