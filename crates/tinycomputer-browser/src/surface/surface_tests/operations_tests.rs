//! Tests for every surface call turning into the right engine command over a
//! scripted engine.

use serde_json::json;
use tinycomputer_bus::JevOperation;
use tinycomputer_core::Platform;
use tinycomputer_core::surface::{Depth, Surface};

use tinycomputer_bus::browser::SessionOptions;
use tinycomputer_core::surface::Candidate;

use super::{Drawn, Harness, PAGE, harness, node, page_fake, shown_harness};
use crate::fake::{Fake, failure, ok};
use crate::surface::operations::browser_key;

#[test]
fn keys_are_spelled_the_way_agent_browser_reads_them() {
    assert_eq!(browser_key("cmd+a", Platform::MacOs), "Meta+a");
    assert_eq!(browser_key("cmd+a", Platform::Linux), "Control+a");
    assert_eq!(
        browser_key("ctrl+shift+Z", Platform::MacOs),
        "Control+Shift+z"
    );
    assert_eq!(browser_key("alt+left", Platform::Windows), "Alt+ArrowLeft");
    assert_eq!(browser_key("option+Down", Platform::MacOs), "Alt+ArrowDown");
    for (combo, key) in [
        ("return", "Enter"),
        ("esc", "Escape"),
        ("tab", "Tab"),
        ("space", "Space"),
        ("backspace", "Backspace"),
        ("delete", "Delete"),
        ("f5", "F5"),
        ("meta+command+k", "Meta+Meta+k"),
    ] {
        assert_eq!(browser_key(combo, Platform::MacOs), key);
    }
    assert_eq!(browser_key(" + ", Platform::MacOs), "");
}

#[test]
fn observing_opens_the_session_once_and_reads_the_page() {
    let Harness { fake, surface, .. } = harness("observe", page_fake());
    assert!(surface.session().is_none());
    let screen = surface.observe("flights", None, Depth::Skeleton).unwrap();
    assert_eq!(screen.app, "flights");
    assert_eq!(screen.window.as_deref(), Some("Flights"));
    assert_eq!(fake.last("snapshot")["maxDepth"], 6);
    let scoped = surface.observe("", Some("@e7"), Depth::Skeleton).unwrap();
    assert_eq!(scoped.app, "browser");
    let sent = fake.last("snapshot");
    assert_eq!(sent["selector"], "@e7");
    assert!(sent.get("maxDepth").is_none());
    assert_eq!(
        fake.actions()
            .iter()
            .filter(|action| *action == "launch")
            .count(),
        1
    );
    assert!(surface.session().is_some());
    assert!(format!("{surface:?}").contains("BrowserSurface"));
}

#[test]
fn a_failed_observation_is_an_envelope_with_the_wire_code() {
    let fake = Fake::scripted(|command| {
        (command["action"] == "snapshot").then(|| failure("Unknown ref: e9"))
    });
    let Harness { surface, .. } = harness("observe-failed", fake);
    let error = surface.observe("", Some("e9"), Depth::Full).unwrap_err();
    assert_eq!(error.error.unwrap().code, "STALE_REF");

    let unlaunchable = Fake::scripted(|command| {
        (command["action"] == "launch").then(|| failure("Auto-launch failed: no chrome"))
    });
    let Harness { surface, .. } = harness("unlaunchable", unlaunchable);
    assert_eq!(
        surface.launch("browser").error.unwrap().code,
        "BROWSER_UNAVAILABLE"
    );
}

#[test]
fn every_operation_becomes_its_engine_command() {
    let Harness { fake, surface, .. } = harness("execute", page_fake());
    let button = node("e5", &["Click"]);
    for (operation, action) in [
        (JevOperation::Click, "click"),
        (JevOperation::Expand, "click"),
        (JevOperation::Collapse, "click"),
        (JevOperation::TypeText, "fill"),
        (JevOperation::Check, "check"),
        (JevOperation::Uncheck, "uncheck"),
        (JevOperation::Scroll, "scroll"),
        (JevOperation::Wait, "wait"),
    ] {
        let reply = surface.execute(operation, Some(button.clone()), Some("SXR".to_owned()));
        assert!(reply.ok, "{operation:?}");
        assert_eq!(
            fake.actions().iter().rev().nth(2).unwrap(),
            action,
            "{operation:?}"
        );
    }
    assert_eq!(fake.last("fill")["value"], "SXR");
    assert_eq!(fake.last("click")["selector"], "@e5");
    for operation in [
        JevOperation::Drill,
        JevOperation::Widen,
        JevOperation::Done,
        JevOperation::Blocked,
    ] {
        assert!(surface.execute(operation, None, None).ok);
    }
    let untargeted = surface.execute(JevOperation::Click, Some(node("", &[])), None);
    assert_eq!(untargeted.error.unwrap().code, "INVALID_TARGET");
    let focused = surface.execute(JevOperation::TypeText, None, Some("Srinagar".to_owned()));
    assert!(
        focused.ok,
        "text without a target goes to the focused element"
    );
    assert_eq!(
        fake.last("inserttext"),
        json!({"action": "inserttext", "text": "Srinagar"})
    );
    assert!(surface.execute(JevOperation::Scroll, None, None).ok);
}

#[test]
fn typing_without_a_target_refuses_when_nothing_editable_is_focused() {
    // No `evaluate` script here: `default_reply` answers `{"result": 42}`,
    // which is not `true`, so the focused element is read as not editable.
    let fake = Fake::scripted(|command| {
        (command["action"] == "snapshot").then(|| ok(&json!({"snapshot": PAGE})))
    });
    let Harness { fake, surface, .. } = harness("focus-not-editable", fake);
    let refused = surface.execute(JevOperation::TypeText, None, Some("secret".to_owned()));
    assert_eq!(refused.error.unwrap().code, "INVALID_TARGET");
    assert!(
        !fake.actions().iter().any(|action| action == "inserttext"),
        "an unverified focus must never receive the text"
    );
}

#[test]
fn values_are_read_from_the_field_then_its_text() {
    let Harness { surface, .. } = harness("read", page_fake());
    assert_eq!(
        surface.read_value(&node("e1", &[])).as_deref(),
        Some("Delhi")
    );
    assert_eq!(
        surface.read_value(&node("e2", &[])).as_deref(),
        Some("Srinagar")
    );
    assert_eq!(surface.read_value(&node("e3", &[])), None);
    assert_eq!(surface.read_value(&node("", &[])), None);
}

#[test]
fn pasting_focuses_selects_and_inserts_without_a_clipboard() {
    let Harness { fake, surface, .. } = harness("paste", page_fake());
    assert!(
        surface
            .paste("", &node("e2", &["Click", "SetValue"]), "Srinagar")
            .ok
    );
    let actions = fake
        .actions()
        .into_iter()
        .filter(|action| ["focus", "evaluate", "press", "inserttext"].contains(&action.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        &actions[actions.len() - 4..],
        ["focus", "evaluate", "press", "inserttext"],
        "focus, check it takes text, select, insert"
    );
    assert_eq!(fake.last("inserttext")["text"], "Srinagar");
    let before = fake.actions().len();
    assert!(surface.paste("", &node("e9", &["Click"]), "note").ok);
    assert!(
        !fake.actions()[before..]
            .iter()
            .any(|action| action == "press"),
        "a field that cannot be set is typed into at the caret"
    );
    assert_eq!(
        surface.paste("", &node("", &[]), "x").error.unwrap().code,
        "INVALID_TARGET"
    );
}

#[test]
fn a_paste_stops_at_the_first_failed_step() {
    let unfocusable = Fake::scripted(|command| {
        (command["action"] == "focus").then(|| failure("Element not found: @e2"))
    });
    let Harness { surface, .. } = harness("paste-focus", unfocusable);
    assert_eq!(
        surface
            .paste("", &node("e2", &["SetValue"]), "x")
            .error
            .unwrap()
            .code,
        "NO_SUCH_ELEMENT"
    );
    let unselectable = Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "press" => Some(failure("Operation timed out")),
        "evaluate" => Some(ok(&json!({"result": true}))),
        _ => None,
    });
    let Harness { surface, .. } = harness("paste-select", unselectable);
    assert_eq!(
        surface
            .paste("", &node("e2", &["SetValue"]), "x")
            .error
            .unwrap()
            .code,
        "TIMEOUT"
    );
}

#[test]
fn pressing_launching_settling_and_navigating() {
    let Harness { fake, surface, .. } = harness("misc", page_fake());
    assert!(surface.press("", "return").ok);
    assert_eq!(fake.last("press")["key"], "Enter");
    assert_eq!(surface.press("", "").error.unwrap().code, "INVALID_KEY");
    let launched = surface.launch("browser");
    assert_eq!(launched.data.unwrap()["running"], true);
    // Settling steadily, as a host may still ask for.
    surface.clone().with_settle(crate::Settle::Steady).settle();
    let idle = fake.last("waitforloadstate");
    assert_eq!(
        (idle["state"].as_str(), idle["timeout"].as_u64()),
        (Some("networkidle"), Some(2_000))
    );
    assert_eq!(fake.last("wait")["timeout"], 400);
    let loaded = surface.navigate("https://flights.test/search");
    assert_eq!(loaded.data.unwrap()["url"], "https://flights.test/search");
    let back = surface.back("browser");
    assert!(back.ok, "{:?}", back.error);
    assert_eq!(fake.last("back")["action"], "back");
    assert!(back.data.unwrap().get("url").is_some());
    let refused = Fake::scripted(|command| {
        (command["action"] == "navigate")
            .then(|| failure("Domain 'evil.test' is not in the allowed domains list"))
    });
    let Harness { surface, .. } = harness("refused", refused);
    assert_eq!(
        surface.navigate("https://evil.test").error.unwrap().code,
        "BLOCKED_BY_POLICY"
    );
}

#[test]
fn closing_ends_the_session_and_is_harmless_twice() {
    let Harness {
        fake,
        surface,
        _runtime: runtime,
    } = harness("close", page_fake());
    surface.close();
    assert!(surface.launch("browser").ok);
    assert!(surface.session().is_some());
    surface.close();
    assert!(surface.session().is_none());
    // The close runs on the runtime; wait for it to land.
    runtime.block_on(async {
        for _ in 0..100 {
            if fake.actions().iter().any(|action| action == "close") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("the session was never closed");
    });
}

#[test]
fn text_is_never_filled_or_pasted_into_an_element_that_does_not_take_it() {
    // A `div` with a `combobox` role — a city in a list of suggestions —
    // focuses, but what takes focus is no input: the page's check says no.
    let rows = Fake::scripted(|command| match command["action"].as_str().unwrap() {
        "evaluate" => Some(ok(&json!({"result": false}))),
        _ => None,
    });
    let Harness { fake, surface, .. } = harness("not-a-field", rows);
    let row = node("e216", &["Click", "SetValue"]);
    let filled = surface.execute(
        JevOperation::TypeText,
        Some(row.clone()),
        Some("Srinagar".to_owned()),
    );
    assert_eq!(filled.error.unwrap().code, "NOT_A_TEXT_FIELD");
    let pasted = surface.paste("", &row, "Srinagar");
    assert_eq!(pasted.error.unwrap().code, "NOT_A_TEXT_FIELD");
    let actions = fake.actions();
    assert!(
        !actions
            .iter()
            .any(|action| action == "fill" || action == "inserttext" || action == "press"),
        "nothing is typed: {actions:?}"
    );
    assert_eq!(fake.last("focus")["selector"], "@e216");
}

#[test]
fn only_a_control_asking_for_the_place_is_read_as_one() {
    use crate::surface::location::asks_where;
    let button = |name: &str| Candidate {
        name: Some(name.to_owned()),
        role: "button".to_owned(),
        ..Candidate::default()
    };
    for asks in ["Use my current location", "Detect my location", "Locate me"] {
        assert!(asks_where(&button(asks)), "{asks}");
    }
    for other in ["Auto detect language", "Search", "Change city"] {
        assert!(!asks_where(&button(other)), "{other}");
    }
}

#[test]
fn two_addresses_of_one_page_are_one_place() {
    use crate::surface::tabs::place;
    assert_eq!(
        place("https://www.shop.test/Cart/#top"),
        place("http://shop.test/cart")
    );
    assert_ne!(
        place("https://shop.test/cart"),
        place("https://shop.test/bag")
    );
    assert_ne!(
        place("https://shop.test/search?q=boots"),
        place("https://shop.test/search?q=shoes"),
        "another search is another page"
    );
}

#[test]
fn location_is_granted_only_in_a_browser_the_module_launched() {
    // The grant covers every page of the browser for the session: a
    // person's own browser keeps its own say in the bubble they answer.
    let location = Candidate {
        ref_id: "e5".to_owned(),
        role: "button".to_owned(),
        name: Some("Use my current location".to_owned()),
        available_actions: vec!["Click".to_owned()],
        ..Candidate::default()
    };
    let granted = |harness: &Harness| {
        harness
            .fake
            .sent()
            .iter()
            .any(|command| command["action"] == "permissions")
    };
    let own = harness("location-own", page_fake());
    let _pressed = own
        .surface
        .execute(JevOperation::Click, Some(location.clone()), None);
    assert!(granted(&own));
    // A task's own profile, kept between its runs, is still a browser the
    // module launched: headless, no one could answer the bubble.
    let kept = shown_harness(
        "location-kept-profile",
        page_fake(),
        SessionOptions {
            user_data_dir: Some(
                std::env::temp_dir()
                    .join("tinycomputer-location-profile")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..SessionOptions::default()
        },
        &Drawn::default(),
    );
    let _pressed = kept
        .surface
        .execute(JevOperation::Click, Some(location.clone()), None);
    assert!(granted(&kept), "{:?}", kept.fake.sent());
    let attached = shown_harness(
        "location-attached",
        page_fake(),
        SessionOptions {
            endpoint: Some("ws://127.0.0.1:9222/devtools/browser/test".to_owned()),
            ..SessionOptions::default()
        },
        &Drawn::default(),
    );
    let _pressed = attached
        .surface
        .execute(JevOperation::Click, Some(location), None);
    assert!(!granted(&attached), "{:?}", attached.fake.sent());
}

/// A page read by sight holding one control, `role` and `name`, whose
/// commands `answer` scripts first; everything else answers as the engine
/// would.
fn seen_page(
    role: &'static str,
    name: &'static str,
    answer: impl Fn(&serde_json::Value) -> Option<serde_json::Value> + Send + Sync + 'static,
) -> Fake {
    Fake::scripted(move |command| {
        if let Some(reply) = answer(command) {
            return Some(reply);
        }
        let script = command["script"].as_str().unwrap_or_default();
        match command["action"].as_str().unwrap() {
            "evaluate" if script.contains("__tinycomputerSeen") => Some(ok(&json!({"result": {
                "ok": true,
                "title": "Shop",
                "surface": "window",
                "unreachable": 0,
                "denoised": {"ads": 0, "empty": 0, "hidden": 0},
                "nodes": [{"id": "1", "role": role, "name": name, "states": [], "path": []}]
            }}))),
            "boundingbox" => Some(ok(
                &json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 40.0}),
            )),
            "evaluate" => Some(ok(&json!({"result": true}))),
            _ => None,
        }
    })
}

#[test]
fn a_press_refused_as_covered_is_tried_again_centred_with_the_pointer_moved_off() {
    // Live, a product photo's hover zoom covered "Add to cart" twelve times
    // while the pointer rested on the photo.
    let clicks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = clicks.clone();
    let fake = seen_page("button", "Add to cart", move |command| {
        let script = command["script"].as_str().unwrap_or_default();
        match command["action"].as_str().unwrap() {
            "click" if counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => {
                Some(failure(
                    "Element is covered by <div.zoom> at its click point, so the input would land on that element instead.",
                ))
            }
            // Not the card's own layer: the click-through declines.
            "evaluate" if script.contains("elementsFromPoint") => {
                Some(ok(&json!({"result": false})))
            }
            _ => None,
        }
    });
    let Harness { fake, surface, .. } = harness("covered-retry", fake);
    let screen = surface.observe("shop", None, Depth::Skeleton).unwrap();
    let reply = surface.execute(
        JevOperation::Click,
        Some(screen.candidates[0].clone()),
        None,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        fake.sent()
            .iter()
            .any(|command| command["action"] == "mouse" && command["eventType"] == "mouseMoved"),
        "{:?}",
        fake.actions()
    );
    assert_eq!(
        fake.actions()
            .iter()
            .filter(|action| *action == "click")
            .count(),
        2
    );
}

#[test]
fn a_link_whose_press_went_nowhere_is_followed_unless_the_page_moved() {
    // Live, a product link pressed six times never opened its product.
    let follow = |moves: bool| {
        let pressed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        seen_page("link", "boAt Airdopes 141 Gen 2 earbuds", move |command| {
            let script = command["script"].as_str().unwrap_or_default();
            match command["action"].as_str().unwrap() {
                "click" => {
                    pressed.store(true, std::sync::atomic::Ordering::SeqCst);
                    None
                }
                "url" => {
                    let went = moves && pressed.load(std::sync::atomic::Ordering::SeqCst);
                    let url = if went {
                        "https://shop.test/product/141"
                    } else {
                        "https://shop.test/results"
                    };
                    Some(ok(&json!({"url": url})))
                }
                "evaluate" if script.contains("__tcLeaving") && script.contains("download") => {
                    Some(ok(&json!({"result": "https://shop.test/product/141"})))
                }
                "navigate" => Some(ok(&json!({
                    "url": "https://shop.test/product/141",
                    "title": "boAt Airdopes 141"
                }))),
                _ => None,
            }
        })
    };
    let followed = harness("link-follow", follow(false));
    let screen = followed
        .surface
        .observe("shop", None, Depth::Skeleton)
        .unwrap();
    let reply = followed.surface.execute(
        JevOperation::Click,
        Some(screen.candidates[0].clone()),
        None,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(
        followed.fake.last("navigate")["url"],
        "https://shop.test/product/141"
    );

    let moved = harness("link-moved", follow(true));
    let screen = moved
        .surface
        .observe("shop", None, Depth::Skeleton)
        .unwrap();
    let reply = moved.surface.execute(
        JevOperation::Click,
        Some(screen.candidates[0].clone()),
        None,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        !moved
            .fake
            .actions()
            .iter()
            .any(|action| action == "navigate"),
        "the page moved by itself: {:?}",
        moved.fake.actions()
    );
}

#[test]
fn a_navigation_that_timed_out_on_a_drawn_page_is_taken_as_open() {
    // A heavy results page can be read long before its `load` fires.
    let opening = |drawn: bool| {
        Fake::scripted(move |command| {
            let script = command["script"].as_str().unwrap_or_default();
            match command["action"].as_str().unwrap() {
                "navigate" => Some(failure("navigation timed out after 30000ms")),
                "evaluate" if script.contains("readyState") => Some(ok(&json!({"result": {
                    "url": "https://www.shop.test/search?q=milk",
                    "title": "milk - Shop",
                    "drawn": drawn,
                }}))),
                _ => None,
            }
        })
    };
    let drawn = harness("drawn-page", opening(true));
    let reply = drawn.surface.navigate("https://shop.test/search?q=milk");
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.data.unwrap()["title"], "milk - Shop");

    let blank = harness("blank-page", opening(false));
    let reply = blank.surface.navigate("https://shop.test/search?q=milk");
    assert!(!reply.ok, "nothing drawn yet: the timeout stands");
}

#[test]
fn a_page_opened_early_is_not_loaded_again_until_it_is_read() {
    // A shop whose page has drawn by the time it is asked.
    let shop = || {
        Fake::scripted(|command| {
            let script = command["script"].as_str().unwrap_or_default();
            (command["action"] == "evaluate" && script.contains("readyState")).then(|| {
                ok(&json!({"result": {
                    "url": "https://www.shop.test/",
                    "title": "Shop",
                    "drawn": true,
                }}))
            })
        })
    };
    let loads = |fake: &Fake| {
        fake.actions()
            .iter()
            .filter(|action| *action == "navigate")
            .count()
    };

    let early = harness("open-at", shop());
    assert!(early.surface.open_at("https://shop.test"));
    assert_eq!(loads(&early.fake), 1);
    assert_eq!(
        early.fake.last("navigate")["timeout"],
        10_000,
        "a plan drafted sooner waits for it no longer"
    );
    // The plan's first step browses there, by another of its addresses.
    let reply = early.surface.navigate("https://www.shop.test/");
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.data.unwrap()["title"], "Shop");
    assert_eq!(loads(&early.fake), 1, "loaded once");
    // Asked again, it loads again, with the session's own deadline.
    assert!(early.surface.navigate("https://shop.test").ok);
    assert_eq!(loads(&early.fake), 2);
    assert!(early.fake.last("navigate")["timeout"].is_null());

    // Once the page is read, or another page is asked for, it loads.
    let read = harness("open-at-read", shop());
    assert!(read.surface.open_at("https://shop.test"));
    assert!(read.surface.observe("browser", None, Depth::Full).is_ok());
    assert!(read.surface.navigate("https://shop.test").ok);
    assert_eq!(loads(&read.fake), 2);
    let elsewhere = harness("open-at-elsewhere", shop());
    assert!(elsewhere.surface.open_at("https://shop.test"));
    assert!(elsewhere.surface.navigate("https://shop.test/cart").ok);
    assert_eq!(loads(&elsewhere.fake), 2);

    // A page that has drawn nothing yet is loaded as asked.
    let blank = harness("open-at-blank", Fake::new());
    assert!(blank.surface.open_at("https://shop.test"));
    assert!(blank.surface.navigate("https://shop.test").ok);
    assert_eq!(loads(&blank.fake), 2);

    // A page that would not load is not kept, and a surface let go opens
    // none.
    let refused = harness(
        "open-at-refused",
        Fake::scripted(|command| {
            (command["action"] == "navigate")
                .then(|| failure("Domain 'shop.test' is not in the allowed domains list"))
        }),
    );
    assert!(!refused.surface.open_at("https://shop.test"));
    let closed = harness("open-at-closed", shop());
    closed.surface.close();
    assert!(!closed.surface.open_at("https://shop.test"));
    assert_eq!(
        closed.fake.actions().len(),
        0,
        "{:?}",
        closed.fake.actions()
    );
    // Let go after it opened early, it keeps no page for a later session.
    let let_go = harness("open-at-let-go", shop());
    assert!(let_go.surface.open_at("https://shop.test"));
    let_go.surface.close();
    // A new session shows the shop drawn: a page kept would skip the load.
    assert!(let_go.surface.launch("browser").ok);
    assert!(let_go.surface.navigate("https://shop.test").ok);
    assert_eq!(loads(&let_go.fake), 2);
}

#[test]
fn a_prompt_settle_counts_quiet_from_the_start_and_waits_only_while_the_page_changes() {
    // Prompt is how a surface settles unless told otherwise.
    let Harness { fake, surface, .. } = harness("prompt-settle", page_fake());
    surface.settle();
    let quiet = fake.last("waitforloadstate");
    assert_eq!(
        (quiet["state"].as_str(), quiet["timeout"].as_u64()),
        (Some("networkquiet"), Some(1_000))
    );
    let still = fake.last("evaluate");
    let script = still["script"].as_str().unwrap();
    assert!(script.contains("MutationObserver"), "{script}");
    assert!(
        script.contains("setTimeout(done, 400)"),
        "capped at SETTLE_MS: {script}"
    );
    assert!(script.contains(">= 120"), "still for STILL_MS: {script}");
    assert!(
        script.contains("getAnimations"),
        "waits out CSS animations: {script}"
    );
    assert!(
        script.contains("if (finished) return"),
        "stops looking at frames once settled: {script}"
    );
    assert!(
        script.contains("element.shadowRoot"),
        "watches shadow roots too: {script}"
    );
    assert!(
        !fake.actions().iter().any(|action| action == "wait"),
        "no fixed pause: {:?}",
        fake.actions()
    );
}

#[test]
fn a_brief_settle_waits_only_while_the_page_changes() {
    // A launch or Escape fetched nothing: no wait for the network.
    let brief = harness("brief-settle", page_fake());
    brief.surface.settle_briefly();
    assert!(
        !brief
            .fake
            .actions()
            .iter()
            .any(|action| action == "waitforloadstate" || action == "wait"),
        "{:?}",
        brief.fake.actions()
    );
    let still = brief.fake.last("evaluate");
    assert!(
        still["script"].as_str().unwrap().contains("getAnimations"),
        "the page is still watched until it stops changing"
    );
    // Settling steadily settles in full, as it always did.
    let steady = harness("brief-steady", page_fake());
    steady
        .surface
        .clone()
        .with_settle(crate::Settle::Steady)
        .settle_briefly();
    assert_eq!(steady.fake.last("waitforloadstate")["state"], "networkidle");
    assert_eq!(steady.fake.last("wait")["timeout"], 400);
}

#[test]
fn a_wait_for_a_change_ends_at_the_pages_first_change_or_its_time() {
    let Harness { fake, surface, .. } = harness("await-change", page_fake());
    assert!(surface.open());
    assert!(surface.await_change(1_000), "the page changed");
    let watch = fake.last("evaluate");
    let script = watch["script"].as_str().unwrap();
    assert!(script.contains("MutationObserver"), "{script}");
    assert!(
        script.contains("element.shadowRoot"),
        "watches shadow roots too: {script}"
    );
    assert!(
        script.contains("done(false), 1000"),
        "still once the time given passes: {script}"
    );
    assert!(
        script.contains("'data-tc-'"),
        "sight's own marks are no change: {script}"
    );
    assert!(
        !fake.actions().iter().any(|action| action == "wait"),
        "no fixed pause: {:?}",
        fake.actions()
    );

    let still = harness(
        "await-still",
        Fake::scripted(|command| {
            (command["action"] == "evaluate").then(|| ok(&json!({"result": false})))
        }),
    );
    assert!(still.surface.open());
    assert!(!still.surface.await_change(1_000), "the page stayed still");

    // A watch that cannot run says the page may have changed, so a caller
    // looks again as it would after a pause.
    let unwatched = harness("await-unwatched", Fake::new());
    assert!(unwatched.surface.open());
    assert!(
        unwatched.surface.await_change(1_000),
        "no answer of its own"
    );
    // With no page open, there is nothing to watch and nothing is opened.
    let closed = harness("await-closed", Fake::new());
    assert!(closed.surface.await_change(1_000), "no session to watch");
    assert!(
        closed.fake.actions().is_empty(),
        "{:?}",
        closed.fake.actions()
    );
}

#[test]
fn a_watch_or_wait_the_page_never_answers_is_given_up_on() {
    // Live, an evaluate sent while a page was being replaced waited out the
    // browser's 30 s deadline. The harness is kept whole: its runtime runs
    // the deadlines.
    let watched = harness(
        "watch-stalled",
        page_fake().stalling(|command| command["action"] == "evaluate"),
    );
    assert!(watched.surface.open());
    let started = std::time::Instant::now();
    assert!(
        watched.surface.await_change(10),
        "a watch given up on may have seen a change"
    );
    watched.surface.settle();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );

    let waited = harness(
        "quiet-stalled",
        page_fake().stalling(|command| command["action"] == "waitforloadstate"),
    );
    assert!(waited.surface.open());
    let started = std::time::Instant::now();
    waited.surface.settle();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let still = waited.fake.last("evaluate");
    assert!(
        still["script"].as_str().unwrap().contains("getAnimations"),
        "the page is still watched once the wait is given up on"
    );
}

#[test]
fn a_surface_opens_its_session_when_asked_rather_than_at_first_use() {
    let Harness { fake, surface, .. } = harness("open-early", page_fake());
    assert!(surface.session().is_none());
    assert!(surface.open());
    assert!(surface.session().is_some());
    assert!(fake.actions().iter().any(|action| action == "launch"));
    let launches = fake.actions().len();
    assert!(surface.open(), "already open");
    assert_eq!(fake.actions().len(), launches, "nothing launched twice");

    let refused = Fake::scripted(|command| {
        (command["action"] == "launch").then(|| failure("Chrome not found"))
    });
    let Harness { surface, .. } = harness("open-refused", refused);
    assert!(!surface.open());

    // A surface let go before its early open began is not opened: a task
    // cancelled while planning holds no browser.
    let Harness { fake, surface, .. } = harness("open-closed", page_fake());
    surface.close();
    assert!(!surface.open());
    assert!(surface.session().is_none());
    assert!(fake.actions().is_empty(), "{:?}", fake.actions());
}
