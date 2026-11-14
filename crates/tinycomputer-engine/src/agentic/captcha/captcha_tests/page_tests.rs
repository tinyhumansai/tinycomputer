//! The browser-backed page: its scan, screenshot and press against a
//! scripted engine.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use tinycomputer_browser::{Browser, Engine, Launcher, Reply, SessionOptions};

use super::*;

#[derive(Debug, Clone, Default)]
struct Engines {
    commands: Arc<Mutex<Vec<Value>>>,
    failing: Arc<AtomicBool>,
}

struct PageEngine(Engines);

impl Engine for PageEngine {
    fn execute(&mut self, command: Value) -> Reply<'_> {
        self.0.commands.lock().unwrap().push(command.clone());
        let action = command["action"].as_str().unwrap_or_default().to_owned();
        let failing = self.0.failing.load(Ordering::SeqCst)
            && matches!(action.as_str(), "evaluate" | "screenshot" | "mouse");
        if failing {
            return Box::pin(async { json!({"id": "", "success": false, "error": "refused"}) });
        }
        let data = match action.as_str() {
            "evaluate" => json!({"result": {
                "widgets": [{
                    "vendor": "recaptcha", "role": "checkbox", "visible": true,
                    "rect": {"x": 1.0, "y": 2.0, "width": 304.0, "height": 78.0}
                }],
                "solved": false,
                "viewport": {"width": 400.0, "height": 300.0}
            }}),
            "screenshot" => {
                let path = command["path"].as_str().unwrap_or_default();
                std::fs::write(path, png(8, 6)).unwrap();
                json!({"path": path})
            }
            _ => json!({}),
        };
        Box::pin(async move { json!({"id": "", "success": true, "data": data}) })
    }
}

impl Launcher for Engines {
    fn open(&self, _session: &str) -> Box<dyn Engine> {
        Box::new(PageEngine(self.clone()))
    }
}

async fn open() -> (BrowserCaptchaPage, Engines) {
    let engines = Engines::default();
    let browser = Arc::new(Browser::new(Arc::new(engines.clone())));
    let session = browser
        .open_session(SessionOptions {
            headless: true,
            ..SessionOptions::default()
        })
        .await
        .unwrap()
        .id;
    (BrowserCaptchaPage::new(browser, session), engines)
}

#[tokio::test]
async fn the_page_scans_shoots_and_presses_through_the_browser_session() {
    let (page, engines) = open().await;
    let scan = page.scan().await.expect("a scan");
    assert_eq!(scan.widgets.len(), 1);
    assert!(!scan.solved);
    assert!(page.shoot().await.is_some_and(|bytes| !bytes.is_empty()));
    assert!(page.press(10.0, 20.0).await);
    page.pause(1).await;
    let commands = engines.commands.lock().unwrap();
    let presses = commands
        .iter()
        .filter(|command| command["action"] == "mouse" && command["eventType"] == "mousePressed")
        .count();
    assert_eq!(presses, 1);
    assert!(
        commands
            .iter()
            .any(|command| command["action"] == "evaluate"),
        "the scan script ran in the page"
    );
}

#[tokio::test]
async fn a_browser_that_refuses_leaves_the_page_unread_and_unpressed() {
    let (page, engines) = open().await;
    engines.failing.store(true, Ordering::SeqCst);
    assert!(page.scan().await.is_none());
    assert!(page.shoot().await.is_none());
    assert!(!page.press(10.0, 20.0).await);
    assert!(!page.press(f64::NAN, 1.0).await);
}
