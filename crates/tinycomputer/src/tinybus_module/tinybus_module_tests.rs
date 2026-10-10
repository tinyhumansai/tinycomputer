//! Tests for the `TinyBus` module adapter and its declared surface.
//!
//! These run against the in-memory transport rather than a loaded `cdylib`, so
//! they exercise the dispatch table and the envelope without needing a display
//! server or a granted permission. The `tinycomputer-examples` crate's
//! `verify_module` binary covers the real dynamic loader.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::DesktopService;
use serde_json::json;

mod browser_tests;
mod config_tests;
mod manifest_tests;
mod tasks_tests;
mod wire_tests;

fn service() -> DesktopService {
    DesktopService::from_config(&json!({}))
        .expect("an empty configuration is valid")
        .with_native_fixture()
}
