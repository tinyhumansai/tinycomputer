//! Tests for opening, capping, and naming sessions, and their scratch space.

use std::{sync::Arc, time::Duration};

use tinycomputer_bus::browser::{NavigateRequest, SessionId, SessionOptions, SnapshotRequest};

use super::{open, scratch};
use crate::error::Error;
use crate::fake::{Fake, failure};
use crate::sessions::{Browser, MAX_SESSIONS};

#[tokio::test]
async fn opening_launches_explicitly_then_sets_the_viewport() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "open").await;
    assert_eq!(fake.actions(), ["launch", "viewport", "url", "title"]);
    let listed = browser.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].url, "https://flights.test/");
    assert!(listed[0].launched);

    browser.close_session(&id).await.unwrap();
    assert_eq!(fake.last("close")["action"], "close");
    assert_eq!(
        browser.list_sessions().await.unwrap(),
        [] as [tinycomputer_bus::browser::SessionInfo; 0]
    );
    browser
        .close_session(&id)
        .await
        .expect("closing twice succeeds");
}

#[tokio::test]
async fn a_second_close_waits_for_the_browser_to_finish_closing() {
    let fake = Fake::new().delaying_close(Duration::from_millis(100));
    let (browser, id) = open(&fake, "concurrent-close").await;
    let browser = Arc::new(browser);
    let first_browser = browser.clone();
    let first_id = id.clone();
    let first = tokio::spawn(async move { first_browser.close_session(&first_id).await });

    tokio::time::timeout(Duration::from_secs(1), async {
        while fake.actions().last().map(String::as_str) != Some("close") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first close reached the engine");

    let second_browser = browser.clone();
    let (closed, wait) = tokio::sync::oneshot::channel();
    let second = tokio::spawn(async move {
        let result = second_browser.close_session(&id).await;
        let _ = closed.send(());
        result
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), wait)
            .await
            .is_err(),
        "a concurrent close must wait for the in-flight close"
    );

    first.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), second)
        .await
        .expect("second close follows the first")
        .unwrap()
        .unwrap();
    assert_eq!(
        browser.list_sessions().await.unwrap(),
        Vec::<tinycomputer_bus::browser::SessionInfo>::new()
    );
}

#[tokio::test]
async fn a_failed_close_keeps_the_session_unconfirmed() {
    let fake = Fake::new()
        .delaying_close(Duration::from_millis(20))
        .failing_close();
    let (browser, id) = open(&fake, "failed-close").await;
    let error = browser.close_session(&id).await.unwrap_err();

    assert!(error.to_string().contains("browser close failed"));
    assert_eq!(browser.list_sessions().await.unwrap().len(), 1);
    let close_count = fake
        .actions()
        .iter()
        .filter(|action| *action == "close")
        .count();
    assert!(
        matches!(
            browser.snapshot(&id, SnapshotRequest::default()).await,
            Err(crate::error::Error::NoSuchSession { .. })
        ),
        "normal page operations stay refused after an uncertain close"
    );
    assert_eq!(
        fake.actions()
            .iter()
            .filter(|action| *action == "close")
            .count(),
        close_count,
        "a snapshot after failure must not reach the browser"
    );
    assert!(browser.close_session(&id).await.is_err());
    assert_eq!(
        fake.actions()
            .iter()
            .filter(|action| *action == "close")
            .count(),
        close_count + 1,
        "an explicit close call may retry the failed close"
    );
}

#[tokio::test]
async fn a_failed_launch_opens_nothing() {
    let fake = Fake::scripted(|command| {
        (command["action"] == "launch").then(|| failure("Auto-launch failed: no chrome"))
    });
    let browser = Browser::with_scratch(Arc::new(fake), scratch("failed"));
    let error = browser
        .open_session(SessionOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::BrowserUnavailable { .. }),
        "{error:?}"
    );
    assert_eq!(
        browser.list_sessions().await.unwrap(),
        [] as [tinycomputer_bus::browser::SessionInfo; 0]
    );
}

#[tokio::test]
async fn a_browser_that_cannot_start_says_what_to_set() {
    // Live, a task failed with a bare BROWSER_UNAVAILABLE on a machine whose
    // Chrome was not where it is looked for, and no path had been given.
    let refusing = |said: &'static str| {
        Fake::scripted(move |command| (command["action"] == "launch").then(|| failure(said)))
    };
    let refused = |fake: Fake, options: SessionOptions| async move {
        Browser::with_scratch(Arc::new(fake), scratch("unstartable"))
            .open_session(options)
            .await
            .unwrap_err()
            .to_string()
    };
    let missing = refused(
        refusing("Chrome not found. Checked:\n  - System Chrome installations"),
        SessionOptions::default(),
    )
    .await;
    assert_eq!(
        missing,
        "browser unavailable: no Chrome or Chromium was found on this machine; give the path of the browser to use"
    );
    // A binary that is no browser exits before the browser's address shows.
    let wrong = refused(
        refusing("Chrome exited before providing DevTools URL (no stderr output from Chrome)"),
        SessionOptions {
            executable: Some("/opt/tools/notes".to_owned()),
            ..SessionOptions::default()
        },
    )
    .await;
    assert_eq!(
        wrong,
        "browser unavailable: the browser binary given could not be started (Chrome exited before providing DevTools URL (no stderr output from Chrome)); check that its path names Chrome or Chromium"
    );
    // A browser attached to is reached, not started: its words stay.
    let attached = refused(
        refusing("CDP connection failed: refused"),
        SessionOptions {
            endpoint: Some("ws://127.0.0.1:9222".to_owned()),
            ..SessionOptions::default()
        },
    )
    .await;
    assert_eq!(
        attached,
        "browser unavailable: CDP connection failed: refused"
    );
}

#[tokio::test]
async fn sessions_are_capped_and_unknown_ones_are_named() {
    let fake = Fake::new();
    let browser = Browser::with_scratch(Arc::new(fake), scratch("cap"));
    for _ in 0..MAX_SESSIONS {
        browser
            .open_session(SessionOptions {
                endpoint: Some("ws://127.0.0.1:9222".to_owned()),
                ..SessionOptions::default()
            })
            .await
            .unwrap();
    }
    assert!(matches!(
        browser.open_session(SessionOptions::default()).await,
        Err(Error::LimitExceeded { .. })
    ));
    let missing = SessionId::new("s-missing");
    assert!(matches!(
        browser
            .navigate(&missing, NavigateRequest::new("https://x.test"))
            .await,
        Err(Error::NoSuchSession { .. })
    ));
    assert!(matches!(
        browser.list_downloads(&missing).await,
        Err(Error::NoSuchSession { .. })
    ));
}

#[test]
fn the_default_scratch_space_is_private_to_the_process() {
    let browser = Browser::new(Arc::new(Fake::new()));
    assert!(format!("{browser:?}").contains(&std::process::id().to_string()));
}

/// An engine that yields before every reply, so concurrent launches
/// interleave the way real ones do.
#[derive(Debug)]
struct Slow;

impl crate::engine::Engine for Slow {
    fn execute(&mut self, command: serde_json::Value) -> crate::engine::Reply<'_> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            crate::fake::default_reply(&command)
        })
    }
}

impl crate::engine::Launcher for Slow {
    fn open(&self, _session: &str) -> Box<dyn crate::engine::Engine> {
        Box::new(Slow)
    }
}

#[tokio::test]
async fn concurrent_opens_never_exceed_the_cap() {
    let browser = Arc::new(Browser::with_scratch(Arc::new(Slow), scratch("race")));
    let mut opens = tokio::task::JoinSet::new();
    for _ in 0..MAX_SESSIONS + 4 {
        let browser = browser.clone();
        opens.spawn(async move {
            browser
                .open_session(SessionOptions {
                    endpoint: Some("ws://127.0.0.1:9222".to_owned()),
                    ..SessionOptions::default()
                })
                .await
        });
    }
    let mut refused = 0;
    while let Some(opened) = opens.join_next().await {
        if matches!(opened.unwrap(), Err(Error::LimitExceeded { .. })) {
            refused += 1;
        }
    }
    assert_eq!(refused, 4);
    assert_eq!(browser.list_sessions().await.unwrap().len(), MAX_SESSIONS);
}

#[tokio::test]
async fn a_failed_launch_gives_its_slot_back() {
    let fake =
        Fake::scripted(|command| (command["action"] == "launch").then(|| failure("Chrome exited")));
    let browser = Browser::with_scratch(Arc::new(fake), scratch("slot"));
    // Every launch fails; none may be refused for want of a slot, which is
    // what leaked reservations would cause after MAX_SESSIONS attempts.
    for _ in 0..MAX_SESSIONS + 2 {
        let error = browser
            .open_session(SessionOptions::default())
            .await
            .unwrap_err();
        assert!(!matches!(error, Error::LimitExceeded { .. }), "{error:?}");
    }
}
