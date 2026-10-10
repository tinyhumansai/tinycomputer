//! Tests pinning how engine replies and messages become typed results.
//!
//! The error texts come from agent-browser's source; a change there that one
//! of these no longer matches sends a caller down the wrong recovery path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;

use super::{classify, data, image_size, snapshot, text};
use crate::error::Error;

#[test]
fn a_success_yields_its_data_and_a_failure_its_classified_error() {
    assert_eq!(
        data(&json!({"id": "1", "success": true, "data": {"url": "u"}})).unwrap(),
        json!({"url": "u"})
    );
    assert_eq!(data(&json!({"success": true})).unwrap(), json!(null));
    assert!(matches!(
        data(&json!({"success": false, "error": "Unknown ref: e3"})),
        Err(Error::StaleRef { reference }) if reference == "e3"
    ));
    assert!(matches!(
        data(&json!({"success": false})),
        Err(Error::ModuleFailed { .. })
    ));
}

/// Whether a classified error is the expected kind.
type Expect = fn(&Error) -> bool;

#[test]
fn engine_messages_map_to_what_the_caller_should_do() {
    let cases: &[(&str, Expect)] = &[
        (
            "Unknown ref: @e9",
            |e| matches!(e, Error::StaleRef { reference } if reference == "e9"),
        ),
        ("Could not locate element with role=button name=Book", |e| {
            matches!(e, Error::StaleRef { .. })
        }),
        (
            "Domain 'evil.test' is not in the allowed domains list",
            |e| matches!(e, Error::BlockedByPolicy { url } if url == "evil.test"),
        ),
        (
            "Element not found: #book. Verify the selector, role, or name is correct",
            |e| matches!(e, Error::NoSuchElement { .. }),
        ),
        ("No element found by text 'Book'", |e| {
            matches!(e, Error::NoSuchElement { .. })
        }),
        ("No element at index 3", |e| {
            matches!(e, Error::NoSuchElement { .. })
        }),
        ("Element 'Book' is covered by div at its click point", |e| {
            matches!(e, Error::NotActionable { .. })
        }),
        (
            "Element exists but is not visible. Wait for it to become visible",
            |e| matches!(e, Error::NotActionable { .. }),
        ),
        (
            "Element matched multiple results. Use a more specific selector.",
            |e| matches!(e, Error::NotActionable { .. }),
        ),
        ("Operation timed out. The page may still be loading", |e| {
            matches!(e, Error::Timeout { .. })
        }),
        ("Timeout waiting for selector", |e| {
            matches!(e, Error::Timeout { .. })
        }),
        ("Browser not launched", |e| {
            matches!(e, Error::BrowserUnavailable { .. })
        }),
        ("Auto-launch failed: no chrome", |e| {
            matches!(e, Error::BrowserUnavailable { .. })
        }),
        ("CDP connection failed: refused", |e| {
            matches!(e, Error::BrowserUnavailable { .. })
        }),
        (
            "Chrome exited before providing DevTools URL (no stderr output from Chrome)",
            |e| matches!(e, Error::BrowserUnavailable { .. }),
        ),
        ("Timeout waiting for Chrome DevTools URL", |e| {
            matches!(e, Error::BrowserUnavailable { .. })
        }),
        ("Chrome launch task failed: cancelled", |e| {
            matches!(e, Error::BrowserUnavailable { .. })
        }),
        ("Invalid URL: nope", |e| {
            matches!(e, Error::InvalidInput { .. })
        }),
        ("Missing 'url' parameter", |e| {
            matches!(e, Error::InvalidInput { .. })
        }),
        ("Evaluation error: ReferenceError", |e| {
            matches!(e, Error::PageError { .. })
        }),
        (
            "A JavaScript confirm dialog is blocking the page: \"Leave?\"",
            |e| matches!(e, Error::NotActionable { .. }),
        ),
        ("something unexpected", |e| {
            matches!(e, Error::ModuleFailed { .. })
        }),
    ];
    for (message, expected) in cases {
        let error = classify(message);
        assert!(expected(&error), "{message} became {error:?}");
    }
}

#[test]
fn a_snapshot_lists_refs_in_document_order_and_caps_the_tree() {
    let reply = json!({
        "snapshot": "- button \"Search\" [ref=e1]\n- link \"Deals\" [ref=e10]\n- textbox \"From\" [ref=e2]",
        "refs": {
            "e10": {"role": "link", "name": "Deals"},
            "e2": {"role": "textbox", "name": "From"},
            "@e1": {"role": "button", "name": "Search"},
            "misc": {"role": "generic"}
        }
    });
    let parsed = snapshot(&reply, "https://x".to_owned(), "X".to_owned(), 4, 20);
    assert_eq!(
        parsed
            .refs
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["e1", "e2", "e10", "misc"]
    );
    assert_eq!(parsed.refs[0].name, "Search");
    assert_eq!(parsed.refs[3].name, "");
    assert!(parsed.truncated);
    assert_eq!(parsed.tree.chars().count(), 20);
    assert_eq!((parsed.sequence, parsed.url.as_str()), (4, "https://x"));

    let whole = snapshot(&reply, String::new(), String::new(), 1, 10_000);
    assert!(!whole.truncated);
    let empty = snapshot(&json!({}), String::new(), String::new(), 1, 10);
    assert!(empty.refs.is_empty() && empty.tree.is_empty());
    assert_eq!(text(&json!({"a": 1}), "a"), "");
}

#[test]
fn image_dimensions_come_from_each_supported_header() {
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend(1280_u32.to_be_bytes());
    png.extend(800_u32.to_be_bytes());
    assert_eq!(image_size(&png), (1280, 800));

    // SOI, an APP0 segment to skip, then SOF0 with height 600 and width 800.
    let jpeg = [
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x02, 0x58,
        0x03, 0x20, 0x03, 0x00, 0x00,
    ];
    assert_eq!(image_size(&jpeg), (800, 600));
    assert_eq!(
        image_size(&[0xFF, 0xD8, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0]),
        (0, 0)
    );

    let riff = |chunk: &[u8], body: &[u8]| {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend(chunk);
        bytes.extend(body);
        bytes
    };
    // VP8X: flags and reserved bytes, then 24-bit width-1 and height-1.
    let extended = riff(
        b"VP8X",
        &[0, 0, 0, 0, 0, 0, 0, 0, 0x1F, 0x03, 0x00, 0x57, 0x02, 0x00],
    );
    assert_eq!(image_size(&extended), (800, 600));
    // VP8: frame header, then 14-bit width and height at offset 26.
    let mut lossy = riff(b"VP8 ", &[0; 10]);
    lossy.extend(800_u16.to_le_bytes());
    lossy.extend(600_u16.to_le_bytes());
    assert_eq!(image_size(&lossy), (800, 600));
    // VP8L: signature byte, then width-1 and height-1 packed in 14 bits each.
    let (width_less_one, height_less_one): (u32, u32) = (799, 599);
    let packed = width_less_one | (height_less_one << 14);
    let mut lossless = riff(b"VP8L", &[0, 0, 0, 0, 0x2F]);
    lossless.extend(packed.to_le_bytes());
    assert_eq!(image_size(&lossless), (800, 600));
    assert_eq!(image_size(&riff(b"ALPH", &[0; 16])), (0, 0));

    assert_eq!(image_size(b"not an image"), (0, 0));
}
