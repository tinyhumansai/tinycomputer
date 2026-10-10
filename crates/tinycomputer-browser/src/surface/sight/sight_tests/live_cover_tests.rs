//! Live tests of what lies over a press, gated on
//! `TINYCOMPUTER_LIVE_BROWSER=1`: the cover named as a person would say it,
//! an empty backdrop pressed where nothing lies beneath it, a control (also
//! inside a component's shadow root), a dialog, or a backdrop with nowhere
//! clear never pressed for it, and a target centred at once on a page that
//! scrolls smoothly.

#[cfg(feature = "agent-browser")]
use serde_json::{Value, json};

#[cfg(feature = "agent-browser")]
use super::live_tests::live_page;
#[cfg(feature = "agent-browser")]
use crate::surface::{CENTRE_JS, COVER_JS, SPOT_GRID};

/// A store's header under the backdrop its search box's open suggestions
/// leave over the page, as a store's did; `BACKDROP` is the backdrop's
/// markup. The backdrop closes when pressed, Escape leaves it, and every
/// control records its press.
#[cfg(feature = "agent-browser")]
const SUGGESTIONS_PAGE: &str = r##"<style>
  body { margin: 0; }
  header { display: flex; gap: 16px; align-items: center; height: 56px; padding: 0 16px; background: #eee; }
  .search { position: relative; z-index: 11; width: 320px; }
  .search ul, .backdrop ul { position: absolute; top: 40px; left: 16px; width: 320px; margin: 0; padding: 0;
    list-style: none; background: white; }
  .backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.4); z-index: 10; }
  main { padding: 16px; }
</style>
<header>
  <div class="search">
    <input aria-label="Search for Products..." value="Maggi">
    <ul><li>Maggi 2-Minute Noodles 70 g <button>Add</button></li></ul>
  </div>
  <button id="location">Select Location</button>
  <button id="basket" aria-label="Basket">2</button>
</header>
<main>
  <div>Maggi Masala Noodles 280 g <button id="add">Add</button></div>
  <a id="deals" href="#deals">Today's deals</a>
</main>
BACKDROP
<script>
  window.pressed = [];
  document.querySelector('.backdrop').addEventListener('click', (event) => event.currentTarget.remove());
  for (const control of document.querySelectorAll('button, a')) {
    control.addEventListener('click', () => window.pressed.push(control.id || 'listed'));
  }
</script>"##;

/// [`SUGGESTIONS_PAGE`] with `backdrop` as its backdrop.
#[cfg(feature = "agent-browser")]
fn suggestions_page(backdrop: &str) -> String {
    SUGGESTIONS_PAGE.replace("BACKDROP", backdrop)
}

/// What [`COVER_JS`] says covers the middle of what `selector` selects on
/// the live page, with a spot to press when `press`.
#[cfg(feature = "agent-browser")]
async fn cover_of(
    browser: &crate::sessions::Browser,
    info: &tinycomputer_bus::browser::SessionInfo,
    selector: &str,
    press: bool,
) -> Value {
    let bounds = browser
        .command(
            &info.id,
            json!({"action": "boundingbox", "selector": selector}),
        )
        .await
        .unwrap();
    let middle = |start: &str, size: &str| {
        bounds[start].as_f64().unwrap() + bounds[size].as_f64().unwrap() / 2.0
    };
    let script = format!(
        "{COVER_JS}({}, {}, {}, {press}, {SPOT_GRID})",
        middle("x", "width"),
        middle("y", "height"),
        Value::String(selector.to_owned())
    );
    browser
        .command(&info.id, json!({"action": "evaluate", "script": script}))
        .await
        .unwrap()["result"]
        .clone()
}

/// Presses the window at `spot` with raw mouse events, as the surface does.
#[cfg(feature = "agent-browser")]
async fn press_at(
    browser: &crate::sessions::Browser,
    info: &tinycomputer_bus::browser::SessionInfo,
    spot: &Value,
) {
    for event in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let pressed = event != "mouseMoved";
        browser
            .command(
                &info.id,
                json!({
                    "action": "mouse",
                    "eventType": event,
                    "x": spot[0],
                    "y": spot[1],
                    "button": if pressed { "left" } else { "none" },
                    "clickCount": i32::from(pressed),
                }),
            )
            .await
            .unwrap();
    }
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_an_empty_backdrop_over_a_press_is_pressed_where_nothing_lies_beneath() {
    for (backdrop, said) in [
        (r#"<div class="backdrop"></div>"#, "an empty layer"),
        (
            r#"<div class="backdrop"><ul><li>Maggi Masala 140 g <button>Add</button></li></ul></div>"#,
            r#"a layer holding "Maggi Masala 140 g Add""#,
        ),
    ] {
        let Some((browser, info)) = live_page(&suggestions_page(backdrop)).await else {
            return;
        };
        let evaluate = |script: &str| {
            browser.command(&info.id, json!({"action": "evaluate", "script": script}))
        };
        let refused = browser
            .command(&info.id, json!({"action": "click", "selector": "#basket"}))
            .await;
        assert!(
            refused.is_err_and(|error| error.to_string().contains("is covered by")),
            "the backdrop takes the press"
        );
        let named = cover_of(&browser, &info, "#basket", false).await;
        assert_eq!(named, json!({"said": said, "bare": true, "spot": null}));
        let pressing = cover_of(&browser, &info, "#basket", true).await;
        let spot = pressing["spot"].clone();
        assert!(spot.is_array(), "{pressing}");
        press_at(&browser, &info, &spot).await;
        let after = evaluate(&format!(
            "(() => {{ const under = document.elementFromPoint({}, {}); \
             return [document.querySelector('.backdrop') === null, window.pressed, \
             Boolean(under && under.closest('button, a, input'))]; }})()",
            spot[0], spot[1]
        ))
        .await
        .unwrap();
        assert_eq!(
            after["result"],
            json!([true, [], false]),
            "the backdrop closed, and nothing pressable lay where it was pressed"
        );
        browser
            .command(&info.id, json!({"action": "click", "selector": "#basket"}))
            .await
            .unwrap();
        let pressed = evaluate("window.pressed").await.unwrap();
        browser.close_session(&info.id).await.unwrap();
        assert_eq!(pressed["result"], json!(["basket"]));
    }
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_control_over_a_press_is_named_and_never_pressed_for_it() {
    // Live, a store drew a second, unseen location button over its basket.
    let ghost = r#"<div class="backdrop" style="display: none"></div>
<button id="ghost" style="position: fixed; top: 10px; width: 136px; height: 36px; opacity: 0; z-index: 5">Delivery in 10 mins Select Location</button>
<script>
  const basket = document.getElementById('basket').getBoundingClientRect();
  document.getElementById('ghost').style.left = (basket.right - 136) + 'px';
</script>"#;
    let Some((browser, info)) = live_page(&suggestions_page(ghost)).await else {
        return;
    };
    let pressing = cover_of(&browser, &info, "#basket", true).await;
    browser.close_session(&info.id).await.unwrap();
    assert_eq!(
        pressing,
        json!({"said": "button \"Delivery in 10 mins Select Location\"", "bare": false, "spot": null})
    );
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_component_a_dialog_or_a_backdrop_with_nowhere_clear_is_never_pressed() {
    for (cover, said) in [
        // A component drawn over the window, its button in its shadow root.
        (
            r#"<div class="backdrop" style="display: none"></div>
<x-sheet id="sheet" style="position: fixed; inset: 0; z-index: 5; display: block"></x-sheet>
<script>
  const root = document.getElementById('sheet').attachShadow({ mode: 'open' });
  root.innerHTML = '<button style="position: fixed; inset: 0; opacity: 0">Allow all</button>';
</script>"#,
            json!({"said": "button \"Allow all\"", "bare": false, "spot": null}),
        ),
        // A dialog that fills the window.
        (
            r#"<div class="backdrop" style="display: none"></div>
<div role="dialog" style="position: fixed; inset: 0; z-index: 5; background: white"><p>Sign in to continue</p></div>"#,
            json!({"said": "a dialog showing \"Sign in to continue\"", "bare": false, "spot": null}),
        ),
    ] {
        let Some((browser, info)) = live_page(&suggestions_page(cover)).await else {
            return;
        };
        let pressing = cover_of(&browser, &info, "#basket", true).await;
        browser.close_session(&info.id).await.unwrap();
        assert_eq!(pressing, said);
    }

    // A backdrop over a page of buttons has no spot clear of them, and is
    // left taking the pointer as it did.
    let crowded = r#"<div class="backdrop" style="pointer-events: auto"></div>
<style>main { display: grid; grid-template-columns: repeat(20, 1fr); position: fixed; inset: 0 }
main button { height: 60px }</style>
<script>
  const main = document.querySelector('main');
  main.innerHTML = '';
  for (let at = 0; at < 400; at += 1) main.insertAdjacentHTML('beforeend', '<button>Add</button>');
</script>"#;
    let Some((browser, info)) = live_page(&suggestions_page(crowded)).await else {
        return;
    };
    let pressing = cover_of(&browser, &info, "#basket", true).await;
    let kept = browser
        .command(
            &info.id,
            json!({"action": "evaluate", "script": "document.querySelector('.backdrop').getAttribute('style')"}),
        )
        .await
        .unwrap();
    browser.close_session(&info.id).await.unwrap();
    assert_eq!(
        pressing,
        json!({"said": "an empty layer", "bare": true, "spot": null})
    );
    assert_eq!(kept["result"], "pointer-events: auto;");
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_target_is_centred_at_once_on_a_page_that_scrolls_smoothly() {
    let Some((browser, info)) = live_page(
        r#"<style>html { scroll-behavior: smooth; }</style>
<div style="height: 2500px"></div><button id="far">Far</button><div style="height: 2500px"></div>"#,
    )
    .await
    else {
        return;
    };
    // Read in the same task as the scroll: a smooth one has not moved yet.
    let script = format!(
        "(() => {{ const found = {CENTRE_JS}(document.getElementById('far')); \
         const box = document.getElementById('far').getBoundingClientRect(); \
         return [found, Math.abs(box.top + box.height / 2 - innerHeight / 2) < 2]; }})()"
    );
    let centred = browser
        .command(&info.id, json!({"action": "evaluate", "script": script}))
        .await
        .unwrap();
    browser.close_session(&info.id).await.unwrap();
    assert_eq!(centred["result"], json!([true, true]));
}
