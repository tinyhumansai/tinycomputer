//! Tests for the browser surface: snapshot parsing, and every surface call
//! turning into the right engine command over a scripted engine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use tinycomputer_bus::browser::SessionOptions;
use tinycomputer_core::surface::Candidate;
use tinycomputer_cursor::{CursorPace, OverlayCommand, OverlaySink, ScreenCursor};

use super::BrowserSurface;
use crate::fake::{Fake, ok};
use crate::sessions::Browser;

mod card_tests;
mod cover_tests;
mod cursor_tests;
mod native_select_tests;
mod operations_tests;
mod origins_tests;
mod perception_tests;
mod tree_tests;

const PAGE: &str = r#"- banner
  - heading "Search flights" [level=1]
  - text: Fares include taxes
- main
  - textbox "From" [required, ref=e1]: Delhi
  - textbox "To" [ref=e2]
  - checkbox "Return trip" [checked=false, ref=e3]
  - checkbox "Flexible dates" [checked=true, ref=e4]
  - button "Search" [ref=e5]
  - button "Book" [disabled, ref=e6]
  - combobox "Cabin" [expanded=true, ref=e7]
    - option "Economy" [selected, ref=e8]
  - textbox "Notes" [ref=e9]
    - text: private note
"#;

struct Harness {
    fake: Fake,
    surface: BrowserSurface,
    _runtime: tokio::runtime::Runtime,
}

fn harness(name: &str, fake: Fake) -> Harness {
    shown_harness(name, fake, SessionOptions::default(), &Drawn::default())
}

fn shown_harness(name: &str, fake: Fake, options: SessionOptions, drawn: &Drawn) -> Harness {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let browser = Browser::with_scratch(
        Arc::new(fake.clone()),
        std::env::temp_dir().join(format!(
            "tinycomputer-surface-test-{}-{name}",
            std::process::id()
        )),
    );
    let surface = BrowserSurface::new(Arc::new(browser), options, runtime.handle().clone())
        .with_cursor(drawn.cursor(CursorPace::Natural));
    Harness {
        fake,
        surface,
        _runtime: runtime,
    }
}

fn page_fake() -> Fake {
    Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "snapshot" => Some(ok(&json!({"snapshot": PAGE}))),
        "inputvalue" if command["selector"] == "@e1" => Some(ok(&json!({"value": "Delhi"}))),
        "inputvalue" => Some(ok(&json!({"value": ""}))),
        "gettext" if command["selector"] == "@e2" => Some(ok(&json!({"text": "Srinagar"}))),
        "gettext" => Some(ok(&json!({"text": " "}))),
        // A text field is focused, so targetless typing is accepted.
        "evaluate" => Some(ok(&json!({"result": true}))),
        _ => None,
    })
}

fn node(reference: &str, actions: &[&str]) -> Candidate {
    Candidate {
        ref_id: reference.to_owned(),
        available_actions: actions.iter().map(|action| (*action).to_owned()).collect(),
        ..Candidate::default()
    }
}

/// Records every command the shared cursor would draw.
#[derive(Clone, Default)]
struct Drawn(Arc<std::sync::Mutex<Vec<OverlayCommand>>>);

impl OverlaySink for Drawn {
    fn send(&mut self, command: &OverlayCommand) -> std::io::Result<()> {
        self.0.lock().unwrap().push(command.clone());
        Ok(())
    }
}

impl Drawn {
    fn cursor(&self, pace: CursorPace) -> Arc<ScreenCursor> {
        Arc::new(ScreenCursor::with_sink(pace, Box::new(self.clone())).without_waiting())
    }

    /// The `[t, x, y]` path of every glide drawn so far.
    fn glides(&self) -> Vec<Vec<[f64; 3]>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|command| match command {
                OverlayCommand::Glide { path, .. } => Some(path.clone()),
                OverlayCommand::Hide => None,
            })
            .collect()
    }
}
