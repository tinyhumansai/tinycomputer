//! Sight: a web page read the way a person looks at it, rather than
//! through its accessibility tree.
//!
//! The accessibility tree says what a page *declares*: roles and names from
//! ARIA and label markup, which most sites apply partly or wrongly. A list of
//! cities marked `combobox` with a label that points nowhere, an icon button
//! with no name, a field labelled only by the words printed above it — the
//! tree leaves each of these unnamed or mislabelled, and a flow cannot tell
//! them apart. A person never reads the markup. They see what is drawn and
//! what is on top, read the words on a control or beside a box, and know a
//! box takes text because it has a caret.
//!
//! `sight.js` does the same in one pass over the rendered page, run through
//! the engine's `evaluate`:
//!
//! - **What counts as a control** is decided by behaviour: native controls,
//!   elements that show a pointer cursor or take focus, and ARIA roles — but
//!   a "text box" that takes no text is a button to press, and one that only
//!   wraps a real input is read as that input. A date picker's calendar (a
//!   table of day numbers under its month and year) offers each enabled day
//!   as a `gridcell` described by the date it stands for, though many
//!   pickers show a pointer on a day only under the mouse, and its arrows
//!   read as "next month" and "previous month".
//! - **Only what is drawn** is kept: no zero-size, hidden, or transparent
//!   elements. An element outside the viewport is `offscreen`; one whose
//!   middle is under something else is `covered` (not when the cover is the
//!   same result card's own text).
//! - **Names are the words a person reads:** the text on the control; for a
//!   field, its tied label, then the page's label for it, then the words
//!   beside or above it, then its placeholder; for a picture-only control,
//!   its alternative text, then the icon's class words (`close`, `search`),
//!   then, for a link, where it leads.
//! - **Two elements drawn as one control are one control:** the same box,
//!   a wrapper with the same words, or a page's own radio drawn next to the
//!   real one in the same label.
//! - **Containers are what a person sees a control in:** dialogs and fixed
//!   layers, landmarks, named sections, and the cards of a list with their
//!   ordinal, so result cards group as they do from the tree.

//! - **Noise is left out:** ads (frames and links to ad servers, blocks
//!   named or labelled as ads, tracking pixels), blank clickable boxes, and
//!   what the page hides from people (`inert`, clipped screen-reader text,
//!   and `aria-hidden` content slid out sideways or behind a dialog). An ad
//!   in front, and a cookie, consent, or newsletter banner, is never noise:
//!   a person has to answer it. The reply's `denoised` counts what went, by
//!   kind ([`Denoised`]).
//!
//! Each control is marked with a `data-tc-seen` attribute the first time it
//! is seen, and keeps it for as long as the element lives: a ref
//! (`seen:12`) is that attribute's CSS selector, so an element a page
//! removes or replaces leaves its ref pointing at nothing, and acting on it
//! fails rather than reaching whatever took its place.
//!
//! What a CSS selector from the page cannot address is read through the
//! tree. A shadow root that shows controls (even one whose host draws no
//! box of its own) has its host's subtree read by the tree and merged into
//! the reading, under the label of the layer it draws; sight gives way to
//! the tree for the whole page when two shadow roots show controls, or a
//! large frame is in front.

use serde_json::{Value, json};
use tinycomputer_core::surface::{Candidate, Screen};

/// The script, called as `(root, limits)`.
const SIGHT_JS: &str = include_str!("sight.js");

/// The prefix of a ref sight minted.
const PREFIX: &str = "seen:";

/// The most controls one reading returns, in page order.
const MAX_CONTROLS: usize = 800;
/// The most text blocks one reading returns: the viewport and a screen
/// either side.
const MAX_TEXTS: usize = 400;
/// The most visible words considered as a field's label.
const MAX_LABELS: usize = 3_000;
/// The longest name a control is given, and the longest text block.
const MAX_NAME: usize = 120;
const MAX_TEXT: usize = 160;
/// The most context lines kept, as the tree keeps.
const MAX_CONTEXT_LINES: usize = 60;

/// The expression that reads the page, or the part of it under `root` — a
/// ref from an earlier screen.
#[must_use]
pub(crate) fn script(root: Option<&str>) -> String {
    script_with_form_values(root, true)
}

/// The Sight observation script for a host that keeps form values private.
#[must_use]
pub(crate) fn script_without_form_values(root: Option<&str>) -> String {
    script_with_form_values(root, false)
}

fn script_with_form_values(root: Option<&str>, include_form_values: bool) -> String {
    let root = root.map_or(Value::Null, |reference| Value::String(selector(reference)));
    let limits = json!({
        "controls": MAX_CONTROLS,
        "texts": MAX_TEXTS,
        "labels": MAX_LABELS,
        "name": MAX_NAME,
        "text": MAX_TEXT,
        "hideFormValues": !include_form_values,
    });
    format!(
        "({})({root}, {limits})",
        SIGHT_JS.trim().trim_end_matches(';')
    )
}

/// Whether `reference` was minted by sight.
#[must_use]
pub(crate) fn is_seen(reference: &str) -> bool {
    reference.starts_with(PREFIX)
}

/// How the engine addresses `reference`: sight's refs as the CSS selector
/// of their mark, the tree's as `@eN`.
#[must_use]
pub(crate) fn selector(reference: &str) -> String {
    reference.strip_prefix(PREFIX).map_or_else(
        || format!("@{}", reference.trim_start_matches('@')),
        |id| format!("[data-tc-seen={}]", Value::String(id.to_owned())),
    )
}

/// What one sight reading left out as noise, by kind: each count is a
/// block that held something sight would otherwise have returned.
///
/// The screen carries no trace of it; the surface keeps the last reading's
/// summary ([`BrowserSurface::denoised`](super::BrowserSurface::denoised)).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Denoised {
    /// Advertising: frames and links to ad servers, blocks the page marks
    /// as ads, blocks labelled "Advertisement" or "Sponsored", and tracking
    /// pixels.
    pub ads: u64,
    /// Blank clickable boxes: no words, no name, no picture, nothing inside
    /// to act on.
    pub empty: u64,
    /// Content the page hides from people: `aria-hidden`, `inert`, and
    /// visually hidden (clipped) text.
    pub hidden: u64,
}

/// The `denoised` summary of a reading; zero for a count that is missing,
/// as in a reading from before sight denoised.
#[must_use]
pub(crate) fn denoised(result: &Value) -> Denoised {
    let count = |key: &str| {
        result
            .get("denoised")
            .and_then(|summary| summary.get(key))
            .and_then(Value::as_u64)
            .unwrap_or_default()
    };
    Denoised {
        ads: count("ads"),
        empty: count("empty"),
        hidden: count("hidden"),
    }
}

/// A shadow root that shows controls, which a selector from the page cannot
/// address: its host's ref, and the label of the layer it draws over the
/// page, if it draws one (`popover "We value your privacy"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shadow {
    /// The host's ref, minted by sight.
    pub(crate) host: String,
    /// The container label its controls are read under, if any.
    pub(crate) label: Option<String>,
}

/// The shadow roots a reading saw showing controls, in page order.
#[must_use]
pub(crate) fn shadows(result: &Value) -> Vec<Shadow> {
    result
        .get("shadows")
        .and_then(Value::as_array)
        .map(|shadows| {
            shadows
                .iter()
                .filter_map(|shadow| {
                    let id = shadow.get("id").and_then(Value::as_str)?;
                    Some(Shadow {
                        host: format!("{PREFIX}{id}"),
                        label: shadow
                            .get("label")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What `evaluate` returned as a [`Screen`]; `None` when the reading failed
/// or saw a control it cannot reach, so the tree is read instead.
#[must_use]
pub(crate) fn screen(result: &Value) -> Option<Screen> {
    let text = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let strings = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let unreachable = result
        .get("unreachable")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if result.get("ok") != Some(&Value::Bool(true)) || unreachable > 0 {
        return None;
    }
    let mut candidates = Vec::new();
    let mut text_nodes = Vec::new();
    let mut context = Vec::new();
    let nodes = result.get("nodes").and_then(Value::as_array)?;
    for (order, node) in nodes.iter().enumerate() {
        let path = strings(node, "path");
        if let Some(shown) = node.get("text").and_then(Value::as_str) {
            if context.len() < MAX_CONTEXT_LINES && !context.iter().any(|line| line == shown) {
                context.push(shown.to_owned());
            }
            text_nodes.push(Candidate {
                role: "text".to_owned(),
                name: Some(shown.to_owned()),
                path,
                order,
                ..Candidate::default()
            });
        } else if let Some(id) = node.get("id").and_then(Value::as_str) {
            let role = text(node, "role");
            let available_actions = match role.as_str() {
                "textbox" | "searchbox" => vec!["Click".to_owned(), "SetValue".to_owned()],
                "checkbox" | "radio" | "switch" => vec!["Click".to_owned(), "Check".to_owned()],
                _ => vec!["Click".to_owned()],
            };
            let bounds = node
                .get("box")
                .and_then(Value::as_array)
                .map(|numbers| numbers.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
                .and_then(|numbers| match numbers[..] {
                    [x, y, width, height] => {
                        Some(json!({"x": x, "y": y, "width": width, "height": height}))
                    }
                    _ => None,
                });
            candidates.push(Candidate {
                ref_id: format!("{PREFIX}{id}"),
                role,
                name: Some(text(node, "name")).filter(|name| !name.is_empty()),
                description: Some(text(node, "description"))
                    .filter(|description| !description.is_empty()),
                value: Some(text(node, "value"))
                    .filter(|value| !value.is_empty())
                    .map(Value::String),
                states: strings(node, "states"),
                available_actions,
                bounds,
                path,
                order,
                ..Candidate::default()
            });
        }
    }
    let surface = text(result, "surface");
    Some(Screen {
        app: "browser".to_owned(),
        window: Some(text(result, "title")).filter(|title| !title.is_empty()),
        surface: if surface.is_empty() {
            "window".to_owned()
        } else {
            surface
        },
        candidates,
        context,
        unexplored: Vec::new(),
        text_nodes,
    })
}

#[cfg(test)]
mod sight_tests;
