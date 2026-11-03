//! Tests for the calls on a session's page: navigation, snapshots, actions,
//! reading, evaluation, and raw commands.

use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tinycomputer_bus::browser::{
    Action, EvaluateRequest, LocateBy, Locator, NavigateRequest, ReadFormat, ReadRequest,
    ScreenshotRequest, SessionOptions, SnapshotRequest, Target,
};

use super::open;
use crate::error::Error;
use crate::fake::{Fake, failure, ok};
use crate::{ActionContext, ActionGate, ActionRequest, ActionVerdict, Browser, ObservationKind};
use crate::{ActionObservation, ActionObserver, ObservedAction};

type GateObservation = (String, Option<ObservationKind>, Option<String>);

#[derive(Default)]
struct CaptureGate(std::sync::Mutex<Vec<GateObservation>>);

impl ActionGate for CaptureGate {
    fn check(&self, request: &ActionRequest) -> ActionVerdict {
        self.0.lock().unwrap().push((
            request.payload["action"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            request.observation,
            request.current_url.clone(),
        ));
        ActionVerdict::Allow
    }
}

#[tokio::test]
async fn only_the_fixed_page_stability_helper_is_classified_as_an_observation() {
    let fake = Fake::scripted(|command| match command["action"].as_str() {
        Some("url") => Some(ok(&json!({"url": "https://shop.test/"}))),
        Some("navigate") => Some(ok(&json!({
            "url": command["url"],
            "title": "Shop"
        }))),
        Some("evaluate") => Some(ok(&json!({"result": true}))),
        _ => None,
    });
    let gate = Arc::new(CaptureGate::default());
    let browser = Browser::with_scratch(
        Arc::new(fake),
        std::env::temp_dir().join(format!("tinycomputer-stability-{}", std::process::id())),
    )
    .with_action_gate(gate.clone());
    let id = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap()
        .id;
    browser
        .navigate(&id, NavigateRequest::new("https://shop.test/"))
        .await
        .unwrap();
    gate.0.lock().unwrap().clear();

    let script =
        "new Promise(resolve => { const watcher = new MutationObserver(() => resolve(true)); })";
    browser
        .observe_page_stability(&id, script.to_owned())
        .await
        .unwrap();
    assert_eq!(
        gate.0.lock().unwrap().as_slice(),
        [
            (
                "observe".to_owned(),
                Some(ObservationKind::PageStability),
                Some("https://shop.test/".to_owned())
            ),
            (
                "observe".to_owned(),
                Some(ObservationKind::PageStability),
                Some("https://shop.test/".to_owned())
            )
        ]
    );

    gate.0.lock().unwrap().clear();
    browser
        .observe_target_check(&id, script.to_owned())
        .await
        .unwrap();
    assert_eq!(
        gate.0.lock().unwrap().as_slice(),
        [
            (
                "observe".to_owned(),
                Some(ObservationKind::TargetCheck),
                Some("https://shop.test/".to_owned())
            ),
            (
                "observe".to_owned(),
                Some(ObservationKind::TargetCheck),
                Some("https://shop.test/".to_owned())
            )
        ]
    );

    gate.0.lock().unwrap().clear();
    browser.observe_sight(&id, script.to_owned()).await.unwrap();
    assert_eq!(
        gate.0.lock().unwrap().as_slice(),
        [
            (
                "observe".to_owned(),
                Some(ObservationKind::Snapshot),
                Some("https://shop.test/".to_owned())
            ),
            (
                "observe".to_owned(),
                Some(ObservationKind::Snapshot),
                Some("https://shop.test/".to_owned())
            )
        ]
    );

    gate.0.lock().unwrap().clear();
    browser
        .command(&id, json!({"action": "evaluate", "script": script}))
        .await
        .unwrap();
    assert_eq!(
        gate.0.lock().unwrap().as_slice(),
        [(
            "evaluate".to_owned(),
            None,
            Some("https://shop.test/".to_owned())
        )]
    );
}

#[tokio::test]
async fn navigation_reports_the_page_and_refusals_are_typed() {
    let fake = Fake::scripted(|command| {
        (command["url"] == "https://evil.test/")
            .then(|| failure("Domain 'evil.test' is not in the allowed domains list"))
    });
    let (browser, id) = open(&fake, "navigate").await;
    let page = browser
        .navigate(&id, NavigateRequest::new("https://flights.test/search"))
        .await
        .unwrap();
    assert_eq!(page.url, "https://flights.test/search");
    assert_eq!(page.title, "Loaded");
    assert_eq!(
        browser.list_sessions().await.unwrap()[0].url,
        "https://flights.test/search"
    );
    assert!(matches!(
        browser.navigate(&id, NavigateRequest::new("https://evil.test/")).await,
        Err(Error::BlockedByPolicy { url }) if url == "evil.test"
    ));
    assert!(matches!(
        browser.navigate(&id, NavigateRequest::new("")).await,
        Err(Error::InvalidInput { .. })
    ));
}

struct RefuseNamedAction(&'static str);

impl ActionGate for RefuseNamedAction {
    fn check(&self, request: &ActionRequest) -> ActionVerdict {
        let action = request.payload["action"].as_str().unwrap_or_default();
        if action == self.0 {
            ActionVerdict::Refuse {
                reason: "host policy refused this action".to_owned(),
            }
        } else {
            ActionVerdict::Allow
        }
    }
}

#[derive(Default)]
struct CaptureActionObserver(std::sync::Mutex<Vec<ActionObservation>>);

impl ActionObserver for CaptureActionObserver {
    fn observed(&self, action: ActionObservation) {
        self.0.lock().unwrap().push(action);
    }
}

fn observer_shop_fake(reject_click: Arc<AtomicBool>) -> Fake {
    Fake::scripted(move |command| match command["action"].as_str() {
        Some("url") => Some(ok(&json!({"url": "https://shop.test/"}))),
        Some("navigate") => Some(ok(&json!({
            "url": command["url"],
            "title": "Shop"
        }))),
        Some("snapshot") => Some(ok(&json!({
            "snapshot": "- button \"Add to cart\" [ref=e1]\n- button \"Add to cart\" [ref=e2]\n- textbox \"Email\" [ref=e3]",
            "refs": {
                "e1": {"role": "button", "name": "Add to cart"},
                "e2": {"role": "button", "name": "Add to cart"},
                "e3": {"role": "textbox", "name": "Email"}
            }
        }))),
        Some("click") if reject_click.load(Ordering::SeqCst) => {
            Some(failure("the scripted click failed"))
        }
        _ => None,
    })
}

async fn click_reference(browser: &Browser, id: &tinycomputer_bus::browser::SessionId) {
    browser
        .perform(
            id,
            Action::Click {
                target: Target::reference("e1"),
                new_tab: false,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn observer_sees_only_successful_metadata_and_snapshot_uniqueness() {
    let reject_click = Arc::new(AtomicBool::new(true));
    let fake = observer_shop_fake(Arc::clone(&reject_click));
    let observer = Arc::new(CaptureActionObserver::default());
    let browser = Browser::with_scratch(Arc::new(fake), super::scratch("action-observer"));
    browser.set_action_observer(Some(observer.clone()));
    let id = browser
        .open_session_with_context(
            SessionOptions::default(),
            ActionContext {
                task_id: Some("task-a".to_owned()),
                ..ActionContext::default()
            },
        )
        .await
        .unwrap()
        .id;
    browser
        .navigate(&id, NavigateRequest::new("https://shop.test/"))
        .await
        .unwrap();
    browser
        .snapshot(&id, SnapshotRequest::default())
        .await
        .unwrap();

    assert!(
        browser
            .perform(
                &id,
                Action::Click {
                    target: Target::reference("e1"),
                    new_tab: false,
                },
            )
            .await
            .is_err()
    );
    assert_eq!(
        observer.0.lock().unwrap().len(),
        1,
        "failed dispatch is not observed"
    );

    reject_click.store(false, Ordering::SeqCst);
    click_reference(&browser, &id).await;
    browser
        .perform(
            &id,
            Action::Type {
                target: Some(Target::reference("e3")),
                text: "private@example.test".to_owned(),
                delay_ms: None,
            },
        )
        .await
        .unwrap();
    let observations = observer.0.lock().unwrap().clone();
    assert_eq!(observations.len(), 3);
    assert!(
        observations
            .iter()
            .all(|item| item.task_id.as_deref() == Some("task-a"))
    );
    assert!(
        observations
            .iter()
            .all(|item| item.session_id == id.to_string())
    );
    assert_eq!(observations[0].action, ObservedAction::Navigate);
    assert_eq!(observations[0].after_url, "https://shop.test/");
    assert_eq!(observations[1].action, ObservedAction::Click);
    let target = observations[1].target.as_ref().unwrap();
    assert_eq!(target.reference.as_deref(), Some("e1"));
    assert!(
        !target.unique,
        "the latest snapshot has two matching controls"
    );
    assert_eq!(observations[2].action, ObservedAction::Type);
    assert!(
        observations
            .iter()
            .all(|event| event.before_url != "private@example.test"
                && event.after_url != "private@example.test")
    );
    browser.close_session(&id).await.unwrap();
}

struct RefuseUnknownCurrentUrl;

impl ActionGate for RefuseUnknownCurrentUrl {
    fn check(&self, request: &ActionRequest) -> ActionVerdict {
        if request.current_url.is_none() && request.destination_url.is_none() {
            ActionVerdict::Refuse {
                reason: "current page URL is unavailable".to_owned(),
            }
        } else {
            ActionVerdict::Allow
        }
    }
}

struct RefuseUntrustedObservations;

impl ActionGate for RefuseUntrustedObservations {
    fn check(&self, request: &ActionRequest) -> ActionVerdict {
        if request.observation.is_some()
            && request
                .current_url
                .as_deref()
                .is_some_and(|url| !url.starts_with("https://") || url.contains("127.0.0.1"))
        {
            ActionVerdict::Refuse {
                reason: "page URL is not public HTTPS".to_owned(),
            }
        } else {
            ActionVerdict::Allow
        }
    }
}

#[tokio::test]
async fn untrusted_page_observations_are_refused_before_content_reaches_the_engine() {
    for url in ["http://shop.test/", "https://127.0.0.1/"] {
        let url = url.to_owned();
        let fake = Fake::scripted(move |command| {
            (command["action"] == "url").then(|| ok(&serde_json::json!({"url": url.clone()})))
        });
        let browser = Browser::with_scratch(
            Arc::new(fake.clone()),
            super::scratch("blocked-observation"),
        )
        .with_action_gate(Arc::new(RefuseUntrustedObservations));
        let id = browser
            .open_session(SessionOptions::default())
            .await
            .unwrap()
            .id;

        assert!(matches!(
            browser.snapshot(&id, SnapshotRequest::default()).await,
            Err(Error::ActionRefused { .. })
        ));
        assert!(matches!(
            browser.read_page(&id, ReadRequest::default()).await,
            Err(Error::ActionRefused { .. })
        ));
        assert!(matches!(
            browser.screenshot(&id, ScreenshotRequest::default()).await,
            Err(Error::ActionRefused { .. })
        ));
        let actions = fake.actions();
        assert!(!actions.contains(&"snapshot".to_owned()), "{actions:?}");
        assert!(!actions.contains(&"read".to_owned()), "{actions:?}");
        assert!(!actions.contains(&"screenshot".to_owned()), "{actions:?}");
        browser.close_session(&id).await.unwrap();
    }
}

#[tokio::test]
async fn a_snapshot_is_discarded_if_the_live_url_changes_during_observation() {
    let url = Arc::new(std::sync::Mutex::new("https://shop.test/".to_owned()));
    let page_url = url.clone();
    let fake = Fake::scripted(move |command| match command["action"].as_str() {
        Some("url") => Some(ok(&serde_json::json!({
            "url": page_url.lock().unwrap().clone(),
        }))),
        Some("snapshot") => {
            *page_url.lock().unwrap() = "http://127.0.0.1/".to_owned();
            Some(ok(&serde_json::json!({
                "snapshot": "text from the page that must not be returned",
                "refs": {},
            })))
        }
        _ => None,
    });
    let browser = Browser::with_scratch(Arc::new(fake), super::scratch("redirected-observation"))
        .with_action_gate(Arc::new(RefuseUntrustedObservations));
    let id = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap()
        .id;

    assert!(matches!(
        browser.snapshot(&id, SnapshotRequest::default()).await,
        Err(Error::ActionRefused { .. })
    ));
}

#[test]
fn browser_surface_never_exposes_an_untrusted_page_snapshot_to_its_runner() {
    use crate::surface::{BrowserSurface, Perception};
    use tinycomputer_core::surface::{Depth, Surface};

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fake = Fake::scripted(|command| {
        (command["action"] == "url").then(|| ok(&serde_json::json!({"url": "https://127.0.0.1/"})))
    });
    let browser =
        Browser::with_scratch(Arc::new(fake.clone()), super::scratch("runner-observation"))
            .with_action_gate(Arc::new(RefuseUntrustedObservations));
    let surface = BrowserSurface::new(
        Arc::new(browser),
        SessionOptions::default(),
        runtime.handle().clone(),
    )
    .with_perception(Perception::Tree);

    assert!(surface.observe("browser", None, Depth::Full).is_err());
    assert!(
        !fake.actions().contains(&"snapshot".to_owned()),
        "the runner must be refused before the accessibility tree is read"
    );
}

#[derive(Default)]
struct CaptureActionRequest(std::sync::Mutex<Vec<String>>);

impl ActionGate for CaptureActionRequest {
    fn check(&self, request: &ActionRequest) -> ActionVerdict {
        self.0.lock().unwrap().push(format!("{request:?}"));
        if request.payload["action"] == "fill" {
            assert_eq!(request.payload["value"], "private@example.test");
        }
        ActionVerdict::Allow
    }
}

#[tokio::test]
async fn gate_receives_task_page_destination_and_value_without_debugging_the_value() {
    let page_url = Arc::new(std::sync::Mutex::new("about:blank".to_owned()));
    let page_url_for_script = Arc::clone(&page_url);
    let fake = Fake::scripted(move |command| match command["action"].as_str() {
        Some("url") => Some(ok(
            &json!({"url": page_url_for_script.lock().unwrap().clone()}),
        )),
        Some("navigate") => {
            *page_url_for_script.lock().unwrap() =
                command["url"].as_str().unwrap_or_default().to_owned();
            Some(ok(
                &json!({"url": page_url_for_script.lock().unwrap().clone(), "title": "Checkout"}),
            ))
        }
        _ => None,
    });
    let gate = Arc::new(CaptureActionRequest::default());
    let browser = Browser::with_scratch(Arc::new(fake), super::scratch("gate-context"))
        .with_action_gate(gate.clone());
    let id = browser
        .open_session_with_context(
            SessionOptions::default(),
            ActionContext {
                task_id: Some("task-17".to_owned()),
                session_id: Some("untrusted-session-id".to_owned()),
                payment_in_play: true,
            },
        )
        .await
        .unwrap()
        .id;
    browser
        .navigate(&id, NavigateRequest::new("https://shop.test/checkout"))
        .await
        .unwrap();
    browser
        .perform(
            &id,
            Action::Fill {
                target: Target::locator(Locator::new(LocateBy::Role, "textbox").with_name("Email")),
                value: "private@example.test".to_owned(),
            },
        )
        .await
        .unwrap();
    let requests = gate.0.lock().unwrap();
    let navigation = requests
        .iter()
        .find(|request| request.contains("destination_url: Some(\"https://shop.test/checkout\")"))
        .unwrap();
    assert!(navigation.contains("task_id: Some(\"task-17\")"));
    assert!(navigation.contains(&format!("session_id: Some(\"{id}\")")));
    assert!(!navigation.contains("untrusted-session-id"));
    assert!(navigation.contains("payment_in_play: true"));
    let fill = requests
        .iter()
        .find(|request| request.contains("action: Some(\"fill\")"))
        .unwrap();
    assert!(fill.contains("current_url: Some(\"https://shop.test/checkout\")"));
    assert!(fill.contains("role: Some(\"textbox\")"));
    assert!(fill.contains("name: Some(\"Email\")"));
    assert!(!fill.contains("private@example.test"));
}

#[tokio::test]
async fn updating_action_context_cannot_replace_the_actual_session_id() {
    let fake = Fake::scripted(|_| None);
    let gate = Arc::new(CaptureActionRequest::default());
    let browser = Browser::with_scratch(Arc::new(fake), super::scratch("gate-context-update"))
        .with_action_gate(gate.clone());
    let id = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap()
        .id;
    browser
        .update_action_context(
            &id,
            ActionContext {
                task_id: Some("task-18".to_owned()),
                session_id: Some("spoofed-session-id".to_owned()),
                payment_in_play: true,
            },
        )
        .await
        .unwrap();
    browser
        .navigate(&id, NavigateRequest::new("https://shop.test/checkout"))
        .await
        .unwrap();
    let requests = gate.0.lock().unwrap();
    let navigation = requests
        .iter()
        .find(|request| request.contains("destination_url: Some(\"https://shop.test/checkout\")"))
        .unwrap();
    assert!(navigation.contains(&format!("session_id: Some(\"{id}\")")));
    assert!(!navigation.contains("spoofed-session-id"));
}

#[tokio::test]
async fn an_unknown_current_url_is_not_replaced_with_the_last_known_page() {
    let url_unavailable = Arc::new(AtomicBool::new(false));
    let unavailable_for_script = Arc::clone(&url_unavailable);
    let fake = Fake::scripted(move |command| {
        (command["action"] == "url" && unavailable_for_script.load(Ordering::SeqCst))
            .then(|| failure("page URL is temporarily unavailable"))
    });
    let browser = Browser::with_scratch(Arc::new(fake.clone()), super::scratch("gate-unknown-url"))
        .with_action_gate(Arc::new(RefuseUnknownCurrentUrl));
    let id = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap()
        .id;
    browser
        .navigate(&id, NavigateRequest::new("https://shop.test/checkout"))
        .await
        .unwrap();

    url_unavailable.store(true, Ordering::SeqCst);
    let error = browser
        .perform(
            &id,
            Action::Click {
                target: Target::reference("e1"),
                new_tab: false,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::ActionRefused { ref reason } if reason == "current page URL is unavailable")
    );
    assert!(!fake.actions().iter().any(|action| action == "click"));
}

#[tokio::test]
async fn refused_order_click_never_reaches_the_page_and_reports_the_reason() {
    let fake = Fake::new();
    let browser = Browser::with_scratch(Arc::new(fake.clone()), super::scratch("gate-click"))
        .with_action_gate(Arc::new(RefuseNamedAction("click")));
    let id = browser
        .open_session_with_context(
            SessionOptions::default(),
            ActionContext {
                task_id: Some("task-17".to_owned()),
                session_id: None,
                payment_in_play: true,
            },
        )
        .await
        .unwrap()
        .id;
    browser
        .navigate(&id, NavigateRequest::new("https://shop.test/checkout"))
        .await
        .unwrap();
    let error = browser
        .perform(
            &id,
            Action::Click {
                target: Target::locator(
                    Locator::new(LocateBy::Role, "button").with_name("Place order"),
                ),
                new_tab: false,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::ActionRefused { ref reason } if reason == "host policy refused this action")
    );
    assert!(!fake.actions().iter().any(|action| action == "click"));
}

#[tokio::test]
async fn refused_fill_leaves_the_page_field_empty() {
    let field = Arc::new(std::sync::Mutex::new(String::new()));
    let field_for_script = Arc::clone(&field);
    let fake = Fake::scripted(move |command| match command["action"].as_str() {
        Some("fill") => {
            *field_for_script.lock().unwrap() =
                command["value"].as_str().unwrap_or_default().to_owned();
            Some(ok(&json!({})))
        }
        Some("inputvalue") => Some(ok(
            &json!({"value": field_for_script.lock().unwrap().clone()}),
        )),
        _ => None,
    });
    let browser = Browser::with_scratch(Arc::new(fake.clone()), super::scratch("gate-fill"))
        .with_action_gate(Arc::new(RefuseNamedAction("fill")));
    let id = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap()
        .id;
    let error = browser
        .perform(
            &id,
            Action::Fill {
                target: Target::selector("#email"),
                value: "private@example.test".to_owned(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::ActionRefused { .. }));
    assert_eq!(*field.lock().unwrap(), "");
    assert!(!fake.actions().iter().any(|action| action == "fill"));
}

#[tokio::test]
async fn raw_commands_and_evaluate_are_consulted_by_the_gate() {
    let fake = Fake::new();
    let browser = Browser::with_scratch(Arc::new(fake.clone()), super::scratch("gate-raw"))
        .with_action_gate(Arc::new(RefuseNamedAction("evaluate")));
    let id = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap()
        .id;
    let command = browser
        .command(
            &id,
            json!({"action": "evaluate", "script": "document.body.remove()"}),
        )
        .await;
    assert!(matches!(command, Err(Error::ActionRefused { .. })));
    let evaluate = browser
        .evaluate(&id, EvaluateRequest::new("document.body.remove()"))
        .await;
    assert!(matches!(evaluate, Err(Error::ActionRefused { .. })));
    assert!(!fake.actions().iter().any(|action| action == "evaluate"));
}

#[tokio::test]
async fn snapshots_count_up_and_carry_the_page() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "snapshot").await;
    let first = browser
        .snapshot(&id, SnapshotRequest::interactive())
        .await
        .unwrap();
    let second = browser
        .snapshot(&id, SnapshotRequest::default())
        .await
        .unwrap();
    assert_eq!((first.sequence, second.sequence), (1, 2));
    assert_eq!(first.refs[0].id, "e1");
    assert_eq!(first.title, "Flights");
    assert_eq!(fake.last("snapshot")["interactive"], false);
}

#[tokio::test]
async fn actions_return_read_values_and_the_page_they_left() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "perform").await;
    let text = browser
        .perform(
            &id,
            Action::GetText {
                target: Target::reference("e1"),
            },
        )
        .await
        .unwrap();
    assert_eq!(text.value, json!("IndiGo ₹6,840"));
    assert_eq!(text.page.url, "https://flights.test/");
    let attribute = browser
        .perform(
            &id,
            Action::GetAttribute {
                target: Target::reference("e1"),
                attribute: "href".to_owned(),
            },
        )
        .await
        .unwrap();
    assert_eq!(attribute.value, json!("/book"));
    let visible = browser
        .perform(
            &id,
            Action::IsVisible {
                target: Target::reference("e1"),
            },
        )
        .await
        .unwrap();
    assert_eq!(visible.value, json!(true));
    let clicked = browser
        .perform(
            &id,
            Action::Click {
                target: Target::locator(Locator::new(LocateBy::Role, "button")),
                new_tab: false,
            },
        )
        .await
        .unwrap();
    assert!(clicked.value.is_null());
    assert_eq!(clicked.matched.as_deref(), Some("Role \"button\""));
    let filled = browser
        .perform(
            &id,
            Action::Fill {
                target: Target::locator(Locator::new(LocateBy::Label, "From")),
                value: "DEL".to_owned(),
            },
        )
        .await
        .unwrap();
    assert!(filled.matched.is_some());
    let pressed = browser
        .perform(
            &id,
            Action::Press {
                key: "Enter".to_owned(),
                target: None,
            },
        )
        .await
        .unwrap();
    assert!(pressed.matched.is_none());
    assert_eq!(
        browser
            .perform(
                &id,
                Action::WaitFor {
                    target: None,
                    text: Some("Results".to_owned()),
                    state: tinycomputer_bus::browser::WaitState::Visible,
                    ms: None,
                    timeout_ms: None,
                },
            )
            .await
            .unwrap()
            .value,
        Value::Null
    );
    assert_eq!(fake.last("wait")["timeout"], 30_000);
}

#[tokio::test]
async fn a_stale_ref_and_an_inexpressible_action_are_typed_failures() {
    let fake = Fake::scripted(|command| {
        (command["action"] == "click").then(|| failure("Unknown ref: e7"))
    });
    let (browser, id) = open(&fake, "stale").await;
    assert!(matches!(
        browser
            .perform(
                &id,
                Action::Click {
                    target: Target::reference("e7"),
                    new_tab: false,
                },
            )
            .await,
        Err(Error::StaleRef { reference }) if reference == "e7"
    ));
    assert!(matches!(
        browser
            .perform(
                &id,
                Action::Focus {
                    target: Target::locator(Locator::new(LocateBy::Text, "Book")),
                },
            )
            .await,
        Err(Error::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn reading_a_page_truncates_and_reports_its_format() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "read").await;
    let markdown = browser
        .read_page(
            &id,
            ReadRequest {
                max_chars: 5,
                ..ReadRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(markdown.content, "# Fli");
    assert!(markdown.truncated);
    assert_eq!(markdown.format, ReadFormat::Markdown);
    let html = browser
        .read_page(
            &id,
            ReadRequest {
                format: ReadFormat::Html,
                ..ReadRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(html.content, "<h1>Flights</h1>");
    assert!(!html.truncated);
    let scoped = browser
        .read_page(
            &id,
            ReadRequest {
                selector: Some("main".to_owned()),
                ..ReadRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(scoped.format, ReadFormat::Text);
    assert_eq!(scoped.content, "IndiGo ₹6,840");
}

#[tokio::test]
async fn evaluation_returns_the_scripts_value() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "evaluate").await;
    assert_eq!(
        browser
            .evaluate(&id, EvaluateRequest::new("6 * 7"))
            .await
            .unwrap(),
        json!(42)
    );
    assert!(matches!(
        browser.evaluate(&id, EvaluateRequest::new(" ")).await,
        Err(Error::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn a_raw_command_needs_an_action_and_returns_its_data() {
    let fake = Fake::scripted(|command| {
        (command["action"] == "inputvalue").then(|| ok(&json!({"value": "Delhi"})))
    });
    let (browser, id) = open(&fake, "command").await;
    assert_eq!(
        browser
            .command(&id, json!({"action": "inputvalue", "selector": "@e2"}))
            .await
            .unwrap(),
        json!({"value": "Delhi"})
    );
    assert!(matches!(
        browser.command(&id, json!({"selector": "@e2"})).await,
        Err(Error::InvalidInput { .. })
    ));
}
