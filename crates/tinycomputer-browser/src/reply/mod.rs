//! agent-browser replies to typed results.
//!
//! Every engine reply is `{id, success, data}` or `{id, success: false,
//! error}`, where `error` is a sentence rather than a code. [`data`] unwraps a
//! success and classifies a failure into the [`Error`] variant that tells a
//! caller what to do next. The classification keys on the engine's own
//! message texts, so each one is pinned by a test.

use serde_json::Value;
use tinycomputer_bus::browser::{ElementRef, Snapshot};

use crate::error::{Error, Result};

/// The `data` of a successful reply, or the failure classified.
///
/// # Errors
///
/// The [`Error`] variant [`classify`] picks for the engine's message.
pub(crate) fn data(reply: &Value) -> Result<Value> {
    if reply.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(reply.get("data").cloned().unwrap_or(Value::Null));
    }
    let message = reply
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("the browser engine failed without a message");
    Err(classify(message))
}

/// Maps an engine error message to the variant a caller acts on.
#[must_use]
pub(crate) fn classify(message: &str) -> Error {
    let lower = message.to_ascii_lowercase();
    if let Some(reference) = message
        .strip_prefix("Unknown ref: ")
        .map(|rest| rest.split_whitespace().next().unwrap_or(rest))
    {
        return Error::StaleRef {
            reference: reference.trim_start_matches('@').to_owned(),
        };
    }
    if lower.starts_with("could not locate element with role=") {
        return Error::StaleRef {
            reference: message.to_owned(),
        };
    }
    if lower.contains("is not in the allowed domains list") {
        return Error::BlockedByPolicy {
            url: quoted(message).unwrap_or(message).to_owned(),
        };
    }
    if lower.starts_with("element not found")
        || lower.starts_with("no element found by")
        || lower.starts_with("no element at index")
    {
        return Error::NoSuchElement {
            target: message.to_owned(),
        };
    }
    if lower.contains("is covered by")
        || lower.contains("not visible")
        || lower.contains("matched multiple results")
        || lower.contains("not interactable")
    {
        return Error::not_actionable(message);
    }
    // A binary that never comes up as a browser (not Chrome at all, or one
    // that dies at once) is a browser that could not be started, though its
    // wait for the browser's address ran out.
    if lower.contains("devtools url") || lower.starts_with("chrome launch task failed") {
        return Error::browser_unavailable(message);
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return Error::timeout(message, 0);
    }
    if lower.starts_with("browser not launched")
        || lower.starts_with("auto-launch failed")
        || lower.starts_with("cdp connection failed")
        || lower.contains("failed to launch")
        || lower.contains("chrome not found")
    {
        return Error::browser_unavailable(message);
    }
    if lower.starts_with("invalid url")
        || lower.starts_with("no hostname in url")
        || lower.starts_with("missing '")
        || lower.starts_with("unknown action")
    {
        return Error::invalid_input(message);
    }
    // A dialog in front of the page passes once someone answers it, so the
    // command is not wrong — the page is not ready for it yet.
    if lower.contains("dialog is blocking the page") {
        return Error::not_actionable(message);
    }
    if lower.starts_with("evaluation error") {
        return Error::page(message);
    }
    Error::failed(message)
}

/// The first single-quoted span in `message`: the host a domain refusal names.
fn quoted(message: &str) -> Option<&str> {
    let start = message.find('\'')? + 1;
    let length = message[start..].find('\'')?;
    Some(&message[start..start + length])
}

/// A string field of a reply's data, or empty.
#[must_use]
pub(crate) fn text(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A snapshot reply as the contract's [`Snapshot`], its tree capped at
/// `max_chars`.
#[must_use]
pub(crate) fn snapshot(
    data: &Value,
    url: String,
    title: String,
    sequence: u64,
    max_chars: usize,
) -> Snapshot {
    let full = text(data, "snapshot");
    let truncated = full.chars().count() > max_chars;
    let tree = if truncated {
        full.chars().take(max_chars).collect()
    } else {
        full
    };
    let mut refs = data
        .get("refs")
        .and_then(Value::as_object)
        .map(|refs| {
            refs.iter()
                .map(|(id, element)| ElementRef {
                    id: id.trim_start_matches('@').to_owned(),
                    role: text(element, "role"),
                    name: text(element, "name"),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    refs.sort_by_key(|element| ref_order(&element.id));
    Snapshot {
        url,
        title,
        sequence,
        tree,
        refs,
        truncated,
    }
}

/// Sorts `e2` before `e10`, and anything unnumbered last.
fn ref_order(id: &str) -> (u64, String) {
    let number = id
        .trim_start_matches(|character: char| !character.is_ascii_digit())
        .parse()
        .unwrap_or(u64::MAX);
    (number, id.to_owned())
}

/// Pixel dimensions from a PNG, JPEG, or WebP header; `(0, 0)` when the bytes
/// are none of those.
#[must_use]
pub(crate) fn image_size(bytes: &[u8]) -> (u32, u32) {
    png_size(bytes)
        .or_else(|| jpeg_size(bytes))
        .or_else(|| webp_size(bytes))
        .unwrap_or((0, 0))
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(bytes.get(16..20)?), be32(bytes.get(20..24)?)))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while at + 9 < bytes.len() {
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        // Start-of-frame markers, excluding DHT (C4), JPG (C8), and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]);
            let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]);
            return Some((u32::from(width), u32::from(height)));
        }
        at += 2 + length;
    }
    None
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    match bytes.get(12..16)? {
        b"VP8X" => {
            let width = 1 + le24(bytes.get(24..27)?);
            let height = 1 + le24(bytes.get(27..30)?);
            Some((width, height))
        }
        b"VP8 " => {
            let width = u32::from(u16::from_le_bytes([*bytes.get(26)?, *bytes.get(27)?]) & 0x3FFF);
            let height = u32::from(u16::from_le_bytes([*bytes.get(28)?, *bytes.get(29)?]) & 0x3FFF);
            Some((width, height))
        }
        b"VP8L" => {
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

fn be32(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0, |value, byte| (value << 8) | u32::from(*byte))
}

fn le24(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .rev()
        .fold(0, |value, byte| (value << 8) | u32::from(*byte))
}

#[cfg(test)]
mod reply_tests;
