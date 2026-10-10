//! Tests for the allowed origins a session checks its pages against: a
//! navigation refused before the browser is asked, a page any call lands on
//! outside them left and reported, and no call ever sent to a refused page.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tinycomputer_bus::browser::{
    Action, EvaluateRequest, NavigateRequest, ScreenshotRequest, SessionId, SessionOptions,
    SnapshotRequest, Target,
};

use super::{Browser, scratch};
use crate::error::Error;
use crate::fake::{Fake, ok};

async fn open_within(fake: &Fake, name: &str, origins: &[&str]) -> (Browser, SessionId) {
    let browser = Browser::with_scratch(Arc::new(fake.clone()), scratch(name));
    let info = browser
        .open_session(SessionOptions {
            allowed_origins: origins.iter().map(|origin| (*origin).to_owned()).collect(),
            ..SessionOptions::default()
        })
        .await
        .unwrap();
    (browser, info.id)
}

/// A page that starts at `start` and moves where `moves` takes it after each
/// command, reporting where it is as the engine would.
fn moving(
    start: &str,
    moves: impl Fn(&Value) -> Option<&'static str> + Send + Sync + 'static,
) -> Fake {
    let at = Arc::new(Mutex::new(start.to_owned()));
    Fake::scripted(move |command| {
        let mut at = at.lock().unwrap();
        if let Some(next) = moves(command) {
            next.clone_into(&mut at);
        }
        match command["action"].as_str().unwrap() {
            "url" => Some(ok(&json!({"url": *at}))),
            "navigate" => Some(ok(&json!({"url": *at, "title": "Loaded"}))),
            _ => None,
        }
    })
}

/// A page that opens at `https://flights.test/` and moves to `drift` on its
/// own (a timer, a redirect) once the session has opened: from the second
/// time its address is read, until the session goes back.
fn drifting(drift: &'static str) -> Fake {
    let state = Arc::new(Mutex::new((0_u32, "https://flights.test/".to_owned())));
    Fake::scripted(move |command| {
        let mut state = state.lock().unwrap();
        match command["action"].as_str().unwrap() {
            "url" => {
                state.0 += 1;
                if state.0 == 2 {
                    drift.clone_into(&mut state.1);
                }
                Some(ok(&json!({"url": state.1})))
            }
            "back" => {
                "https://flights.test/".clone_into(&mut state.1);
                None
            }
            _ => None,
        }
    })
}

fn navigated_to(fake: &Fake, url: &str) -> bool {
    fake.sent()
        .iter()
        .any(|command| command["action"] == "navigate" && command["url"] == url)
}

/// `result` refused `refused` before the call was sent.
fn blocked(result: Result<impl std::fmt::Debug, Error>, refused: &str) {
    match result {
        Err(Error::BlockedByPolicy { url }) => assert_eq!(url, refused),
        other => panic!("expected {refused} to be refused, got {other:?}"),
    }
}

/// `result` reached `refused` by the call's own work, and left it.
fn left(result: Result<impl std::fmt::Debug, Error>, refused: &str) {
    match result {
        Err(Error::LeftRefusedPage { url }) => assert_eq!(url, refused),
        other => panic!("expected {refused} to be reached and left, got {other:?}"),
    }
}

#[tokio::test]
async fn a_navigation_outside_the_origins_is_refused_before_the_browser_is_asked() {
    let fake = moving("about:blank", |command| {
        (command["action"] == "navigate").then_some("https://www.flights.test/search")
    });
    let (browser, id) = open_within(&fake, "origins-navigate", &[".flights.test"]).await;
    blocked(
        browser
            .navigate(&id, NavigateRequest::new("https://evil.test/"))
            .await,
        "https://evil.test/",
    );
    assert!(!navigated_to(&fake, "https://evil.test/"));
    let page = browser
        .navigate(&id, NavigateRequest::new("https://www.flights.test/search"))
        .await
        .unwrap();
    assert_eq!(page.url, "https://www.flights.test/search");
}

#[tokio::test]
async fn a_redirect_out_of_the_origins_is_left_by_going_back() {
    let fake = moving("https://flights.test/", |command| {
        match command["action"].as_str().unwrap() {
            "navigate" if command["url"] == "https://flights.test/go" => {
                Some("https://evil.test/landing")
            }
            "back" => Some("https://flights.test/"),
            _ => None,
        }
    });
    let (browser, id) = open_within(&fake, "origins-redirect", &[".flights.test"]).await;
    left(
        browser
            .navigate(&id, NavigateRequest::new("https://flights.test/go"))
            .await,
        "https://evil.test/landing",
    );
    assert!(fake.actions().contains(&"back".to_owned()));
    assert!(!navigated_to(&fake, "about:blank"), "going back was enough");
    assert_eq!(
        browser.list_sessions().await.unwrap()[0].url,
        "https://flights.test/"
    );
}

#[tokio::test]
async fn a_click_into_a_refused_page_with_no_way_back_is_left_for_a_blank_page() {
    let fake = moving("https://flights.test/", |command| {
        match command["action"].as_str().unwrap() {
            "click" => Some("https://evil.test/offer"),
            "navigate" if command["url"] == "about:blank" => Some("about:blank"),
            // A new tab has no page to go back to.
            _ => None,
        }
    });
    let (browser, id) = open_within(&fake, "origins-click", &[".flights.test"]).await;
    left(
        browser
            .perform(
                &id,
                Action::Click {
                    target: Target::reference("e1"),
                    new_tab: false,
                },
            )
            .await,
        "https://evil.test/offer",
    );
    assert!(navigated_to(&fake, "about:blank"));
    assert_eq!(browser.list_sessions().await.unwrap()[0].url, "about:blank");
}

#[tokio::test]
async fn a_page_that_turns_refused_while_it_is_read_is_never_returned() {
    let fake = moving("https://flights.test/", |command| {
        match command["action"].as_str().unwrap() {
            "snapshot" => Some("https://evil.test/"),
            "back" => Some("https://flights.test/"),
            _ => None,
        }
    });
    let (browser, id) = open_within(&fake, "origins-read", &[".flights.test"]).await;
    left(
        browser.snapshot(&id, SnapshotRequest::default()).await,
        "https://evil.test/",
    );
    assert!(fake.actions().contains(&"back".to_owned()));
}

#[tokio::test]
async fn a_first_page_outside_the_origins_is_left_as_the_session_opens() {
    // A browser attached to, or a profile that restores its last pages.
    let fake = moving("https://evil.test/", |command| {
        (command["action"] == "back").then_some("https://flights.test/")
    });
    let (browser, _id) = open_within(&fake, "origins-first", &[".flights.test"]).await;
    assert!(fake.actions().contains(&"back".to_owned()));
    assert_eq!(
        browser.list_sessions().await.unwrap()[0].url,
        "https://flights.test/"
    );
}

#[tokio::test]
async fn no_call_reaches_a_page_that_moved_out_of_the_origins_on_its_own() {
    let reached = |fake: &Fake, action: &str| fake.actions().iter().any(|sent| sent == action);

    let fake = drifting("https://evil.test/");
    let (browser, id) = open_within(&fake, "origins-drift-click", &[".flights.test"]).await;
    let click = Action::Click {
        target: Target::reference("e1"),
        new_tab: false,
    };
    blocked(browser.perform(&id, click).await, "https://evil.test/");
    assert!(!reached(&fake, "click"), "the action was never sent");

    let fake = drifting("https://evil.test/");
    let (browser, id) = open_within(&fake, "origins-drift-evaluate", &[".flights.test"]).await;
    blocked(
        browser
            .evaluate(&id, EvaluateRequest::new("document.cookie"))
            .await,
        "https://evil.test/",
    );
    assert!(!reached(&fake, "evaluate"), "the script never ran");

    let fake = drifting("https://evil.test/");
    let (browser, id) = open_within(&fake, "origins-drift-raw", &[".flights.test"]).await;
    blocked(
        browser
            .command(&id, json!({"action": "inputvalue", "selector": "@e1"}))
            .await,
        "https://evil.test/",
    );
    assert!(
        !reached(&fake, "inputvalue"),
        "the raw command was never sent"
    );
}

#[tokio::test]
async fn what_a_call_leads_to_outside_the_origins_is_never_returned() {
    // A raw command, a script, and a picture whose page moved out of the
    // origins while they ran.
    let leads_out = |action: &'static str| {
        moving(
            "https://flights.test/",
            move |command| match command["action"].as_str().unwrap() {
                sent if sent == action => Some("https://evil.test/"),
                "back" => Some("https://flights.test/"),
                _ => None,
            },
        )
    };
    let fake = leads_out("press");
    let (browser, id) = open_within(&fake, "origins-leads-raw", &[".flights.test"]).await;
    left(
        browser
            .command(&id, json!({"action": "press", "key": "Enter"}))
            .await,
        "https://evil.test/",
    );

    let fake = leads_out("evaluate");
    let (browser, id) = open_within(&fake, "origins-leads-script", &[".flights.test"]).await;
    left(
        browser
            .evaluate(
                &id,
                EvaluateRequest::new("location.assign('https://evil.test/')"),
            )
            .await,
        "https://evil.test/",
    );

    let fake = leads_out("screenshot");
    let (browser, id) = open_within(&fake, "origins-leads-shot", &[".flights.test"]).await;
    left(
        browser.screenshot(&id, ScreenshotRequest::default()).await,
        "https://evil.test/",
    );
    assert!(
        fake.actions().contains(&"back".to_owned()),
        "each refused page was left"
    );
}

#[tokio::test]
async fn a_raw_command_that_opens_a_refused_page_is_never_sent() {
    let fake = Fake::new();
    let (browser, id) = open_within(&fake, "origins-command", &[".flights.test"]).await;
    blocked(
        browser
            .command(
                &id,
                json!({"action": "tab_new", "url": "https://evil.test/"}),
            )
            .await,
        "https://evil.test/",
    );
    assert!(!fake.actions().contains(&"tab_new".to_owned()));
    browser
        .command(
            &id,
            json!({"action": "tab_new", "url": "https://flights.test/deals"}),
        )
        .await
        .unwrap();
    browser
        .command(&id, json!({"action": "evaluate", "script": "1"}))
        .await
        .unwrap();
    // A command that fetches or opens the address it names, whatever the
    // engine calls it, is checked like a navigation.
    for action in ["read", "a11y", "vitals", "auth_login", "recording_start"] {
        blocked(
            browser
                .command(&id, json!({"action": action, "url": "https://evil.test/"}))
                .await,
            "https://evil.test/",
        );
        assert!(!fake.actions().contains(&action.to_owned()), "{action}");
    }
    // A `url` that is a pattern to wait for names no page to open.
    browser
        .command(
            &id,
            json!({"action": "waitforurl", "url": "**/confirmation"}),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn checking_the_page_is_free_with_no_list_and_leaves_a_refused_page_with_one() {
    let fake = Fake::new();
    let (browser, id) = open_within(&fake, "origins-free", &[]).await;
    let sent = fake.sent().len();
    browser.check_page(&id).await.unwrap();
    assert_eq!(
        fake.sent().len(),
        sent,
        "no list, nothing asked of the engine"
    );

    let fake = drifting("https://evil.test/");
    let (browser, id) = open_within(&fake, "origins-check", &["https://flights.test"]).await;
    blocked(browser.check_page(&id).await, "https://evil.test/");
    assert!(fake.actions().contains(&"back".to_owned()));
    browser.check_page(&id).await.unwrap();
}

#[tokio::test]
async fn the_engine_is_never_handed_the_origins_so_a_page_loads_its_own_files() {
    let fake = Fake::new();
    let (_browser, _id) = open_within(&fake, "origins-launch", &[".flights.test"]).await;
    let launch = fake.last("launch");
    assert!(
        launch.get("allowedDomains").is_none(),
        "the engine would refuse the page's CDN, and a profile beside it: {launch}"
    );
}

#[tokio::test]
async fn an_attached_browser_on_a_refused_page_gets_a_tab_of_the_sessions_own() {
    // The person's own tab is never sent back or blanked.
    let fake = moving("https://mail.test/inbox", |command| {
        (command["action"] == "tab_new").then_some("about:blank")
    });
    let browser = Browser::with_scratch(Arc::new(fake.clone()), scratch("origins-attached"));
    let info = browser
        .open_session(SessionOptions {
            endpoint: Some("ws://127.0.0.1:9222/devtools/browser".to_owned()),
            allowed_origins: vec![".flights.test".to_owned()],
            ..SessionOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(info.url, "about:blank");
    let actions = fake.actions();
    assert!(actions.contains(&"tab_new".to_owned()), "{actions:?}");
    assert!(
        !actions
            .iter()
            .any(|action| action == "back" || action == "navigate"),
        "{actions:?}"
    );
}

#[tokio::test]
async fn every_address_a_raw_command_names_is_checked_and_a_fetch_is_refused() {
    let fake = Fake::new();
    let (browser, id) = open_within(&fake, "origins-fields", &[".flights.test"]).await;
    blocked(
        browser
            .command(
                &id,
                json!({"action": "diff_url", "url1": "https://evil.test/", "url2": "https://flights.test/"}),
            )
            .await,
        "https://evil.test/",
    );
    // A fetch follows redirects where no page is checked, so it is refused
    // under a list even for a listed site.
    blocked(
        browser
            .command(
                &id,
                json!({"action": "read", "url": "https://flights.test/"}),
            )
            .await,
        "https://flights.test/",
    );
    // A script or style the caller puts into the page comes from the list.
    for action in ["addscript", "addstyle"] {
        blocked(
            browser
                .command(
                    &id,
                    json!({"action": action, "url": "https://evil.test/x.js"}),
                )
                .await,
            "https://evil.test/x.js",
        );
    }
    assert!(!fake.actions().iter().any(|action| {
        ["diff_url", "read", "addscript", "addstyle"].contains(&action.as_str())
    }));
}

#[tokio::test]
async fn a_page_that_cannot_say_where_it_is_still_gets_its_wait_and_its_action() {
    // A page committing a navigation has no address for a moment: the wait
    // for it to load, a script, and an action go ahead; the next check that
    // can read the page catches a refused one.
    let reads = Arc::new(Mutex::new(0_u32));
    let fake = Fake::scripted(move |command| {
        let mut reads = reads.lock().unwrap();
        match command["action"].as_str().unwrap() {
            // The address reads as the session opens, then fails.
            "url" => {
                *reads += 1;
                Some(if *reads == 1 {
                    ok(&json!({"url": "https://flights.test/"}))
                } else {
                    crate::fake::failure("Execution context was destroyed")
                })
            }
            _ => None,
        }
    });
    let (browser, id) = open_within(&fake, "origins-unreadable", &[".flights.test"]).await;
    browser
        .command(&id, json!({"action": "waitforloadstate", "state": "load"}))
        .await
        .unwrap();
    browser
        .command(&id, json!({"action": "evaluate", "script": "1"}))
        .await
        .unwrap();
    let reported = browser
        .perform(
            &id,
            Action::Click {
                target: Target::reference("e1"),
                new_tab: false,
            },
        )
        .await;
    let actions = fake.actions();
    for sent in ["waitforloadstate", "evaluate", "click"] {
        assert!(actions.contains(&sent.to_owned()), "{sent}: {actions:?}");
    }
    // The click went ahead; the page it left could not be read to report.
    assert!(
        reported.as_ref().is_err_and(|error| error
            .to_string()
            .contains("Execution context was destroyed")),
        "{reported:?}"
    );
    // The observation's own check reads twice, then reports it cannot.
    assert!(browser.check_page(&id).await.is_err());
}

#[tokio::test]
async fn a_page_that_cannot_be_read_for_another_reason_gets_nothing() {
    // Only a navigation's passing failure lets work go ahead.
    let reads = Arc::new(Mutex::new(0_u32));
    let fake = Fake::scripted(move |command| {
        let mut reads = reads.lock().unwrap();
        match command["action"].as_str().unwrap() {
            "url" => {
                *reads += 1;
                Some(if *reads == 1 {
                    ok(&json!({"url": "https://flights.test/"}))
                } else {
                    crate::fake::failure("CDP response could not be parsed")
                })
            }
            _ => None,
        }
    });
    let (browser, id) = open_within(&fake, "origins-unreadable-other", &[".flights.test"]).await;
    assert!(
        browser
            .command(&id, json!({"action": "evaluate", "script": "1"}))
            .await
            .is_err()
    );
    assert!(
        !fake.actions().contains(&"evaluate".to_owned()),
        "nothing ran on a page that could not be checked"
    );
}

#[tokio::test]
async fn a_page_read_once_more_after_a_moment_passes_the_check() {
    let reads = Arc::new(Mutex::new(0_u32));
    let fake = Fake::scripted(move |command| {
        let mut reads = reads.lock().unwrap();
        match command["action"].as_str().unwrap() {
            "url" => {
                *reads += 1;
                Some(if *reads == 2 {
                    crate::fake::failure("Cannot find default execution context")
                } else {
                    ok(&json!({"url": "https://flights.test/"}))
                })
            }
            _ => None,
        }
    });
    let (browser, id) = open_within(&fake, "origins-retry", &[".flights.test"]).await;
    browser.check_page(&id).await.unwrap();
}

#[tokio::test]
async fn a_session_is_never_handed_out_on_a_refused_page_it_cannot_leave() {
    // Neither going back nor a blank page leaves it.
    let fake = Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "url" => Some(ok(&json!({"url": "https://evil.test/"}))),
        _ => None,
    });
    let browser = Browser::with_scratch(Arc::new(fake.clone()), scratch("origins-stuck"));
    let opened = browser
        .open_session(SessionOptions {
            allowed_origins: vec![".flights.test".to_owned()],
            ..SessionOptions::default()
        })
        .await;
    blocked(opened, "https://evil.test/");
    assert!(fake.actions().contains(&"close".to_owned()));
    assert_eq!(
        browser.list_sessions().await.unwrap(),
        [] as [tinycomputer_bus::browser::SessionInfo; 0]
    );

    // An attached browser that cannot open a tab of the session's own.
    let fake = Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "url" => Some(ok(&json!({"url": "https://mail.test/inbox"}))),
        "tab_new" => Some(crate::fake::failure("Target.createTarget failed")),
        _ => None,
    });
    let browser = Browser::with_scratch(Arc::new(fake.clone()), scratch("origins-no-tab"));
    let opened = browser
        .open_session(SessionOptions {
            endpoint: Some("ws://127.0.0.1:9222/devtools/browser".to_owned()),
            allowed_origins: vec![".flights.test".to_owned()],
            ..SessionOptions::default()
        })
        .await;
    assert!(
        opened
            .as_ref()
            .is_err_and(|error| error.to_string().contains("Target.createTarget failed")),
        "the tab that could not be opened fails the open: {opened:?}"
    );
    assert!(
        !fake
            .actions()
            .iter()
            .any(|action| action == "back" || action == "navigate"),
        "the person's tab was never moved"
    );
}

#[tokio::test]
async fn a_read_is_never_sent_to_a_page_that_moved_out_of_the_origins() {
    // The engine actions each read would send; none goes to the refused page.
    for (read, actions) in [
        ("snapshot", &["snapshot"][..]),
        ("read", &["read", "gettext", "content", "innerhtml"][..]),
        ("screenshot", &["screenshot"][..]),
    ] {
        let fake = drifting("https://evil.test/");
        let (browser, id) = open_within(&fake, "origins-drift-read", &[".flights.test"]).await;
        let refused = match read {
            "snapshot" => browser
                .snapshot(&id, SnapshotRequest::default())
                .await
                .map(|_| ()),
            "read" => browser
                .read_page(&id, tinycomputer_bus::browser::ReadRequest::default())
                .await
                .map(|_| ()),
            _ => browser
                .screenshot(&id, ScreenshotRequest::default())
                .await
                .map(|_| ()),
        };
        blocked(refused, "https://evil.test/");
        let sent = fake.actions();
        assert!(
            !sent.iter().any(|action| actions.contains(&action.as_str())),
            "{read}: {sent:?}"
        );
    }
    // A raw command for the page's title reads what the page shows, a box
    // lookup can test whether a text shows, and a permission grant changes
    // what pages may do.
    for command in [
        json!({"action": "title"}),
        json!({"action": "boundingbox", "selector": "text=Account balance"}),
        json!({"action": "permissions", "permissions": ["geolocation"]}),
    ] {
        let fake = drifting("https://evil.test/");
        let (browser, id) = open_within(&fake, "origins-drift-raw-read", &[".flights.test"]).await;
        let opened = fake.actions().len();
        blocked(
            browser.command(&id, command.clone()).await,
            "https://evil.test/",
        );
        // Leaving the refused page reads the page it goes back to, title and
        // all; neither a box lookup nor a grant is sent.
        let sent = fake.actions().split_off(opened);
        assert!(
            !sent
                .iter()
                .any(|action| action == "boundingbox" || action == "permissions"),
            "{sent:?}"
        );
    }
    // Its address, and a wait for it to load, read nothing of what it shows.
    for command in [
        json!({"action": "url"}),
        json!({"action": "waitforloadstate", "state": "load"}),
    ] {
        let fake = drifting("https://evil.test/");
        let (browser, id) = open_within(&fake, "origins-drift-raw-wait", &[".flights.test"]).await;
        browser.command(&id, command).await.unwrap();
    }
}
