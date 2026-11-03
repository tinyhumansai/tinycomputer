//! Tests for canonical site normalization shared by the host and module.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::site_named;

#[test]
fn site_names_are_ascii_normalized_and_safe_file_stems() {
    assert_eq!(
        site_named("  WWW.Example.COM  ").as_deref(),
        Some("example.com")
    );
    assert_eq!(site_named("example.com.").as_deref(), Some("example.com."));
    assert_eq!(site_named("bücher.example"), None);
    assert_eq!(site_named(".hidden"), None);
    assert_eq!(site_named("a..b"), None);
    assert_eq!(site_named(&"a".repeat(254)), None);
    let longest = "a".repeat(253);
    assert_eq!(site_named(&longest).as_deref(), Some(longest.as_str()));
}
