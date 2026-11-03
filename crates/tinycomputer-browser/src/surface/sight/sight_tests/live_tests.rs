//! Live tests that read fixture pages by sight in a real browser, gated on
//! `TINYCOMPUTER_LIVE_BROWSER=1`.

#[cfg(feature = "agent-browser")]
use serde_json::json;

#[cfg(feature = "agent-browser")]
use crate::surface::sight::{script, script_without_form_values};

/// Reads `html` by sight in a real browser: `None` unless
/// `TINYCOMPUTER_LIVE_BROWSER=1`, since CI has no browser to launch. The
/// page is written into a blank tab, so nothing is fetched but what the
/// fixture itself asks for.
#[cfg(feature = "agent-browser")]
pub(super) async fn live_reading(html: &str) -> Option<serde_json::Value> {
    live_results(html, &[script(None)])
        .await
        .map(|mut results| results.remove(0))
}

/// A blank tab of a real browser with `html` written into it: `None`
/// unless `TINYCOMPUTER_LIVE_BROWSER=1`, since CI has no browser to launch.
#[cfg(feature = "agent-browser")]
pub(super) async fn live_page(
    html: &str,
) -> Option<(
    crate::sessions::Browser,
    tinycomputer_bus::browser::SessionInfo,
)> {
    use std::sync::Arc;

    use tinycomputer_bus::browser::SessionOptions;

    use crate::sessions::Browser;

    if std::env::var("TINYCOMPUTER_LIVE_BROWSER").as_deref() != Ok("1") {
        return None;
    }
    let browser = Browser::new(Arc::new(crate::AgentBrowser));
    let info = browser
        .open_session(SessionOptions::default())
        .await
        .expect("a browser launches when live runs are asked for");
    let write = format!(
        "document.open(); document.write({}); document.close(); \
         Promise.all([...document.images].map((image) => image.decode().catch(() => null)))\
         .then(() => true)",
        serde_json::Value::String(html.to_owned())
    );
    browser
        .command(&info.id, json!({"action": "evaluate", "script": write}))
        .await
        .expect("the fixture is written");
    Some((browser, info))
}

/// Writes `html` into a blank tab of a real browser, as [`live_reading`]
/// does, and runs each of `scripts` on it in turn: their results, or `None`
/// unless `TINYCOMPUTER_LIVE_BROWSER=1`.
#[cfg(feature = "agent-browser")]
async fn live_results(html: &str, scripts: &[String]) -> Option<Vec<serde_json::Value>> {
    let (browser, info) = live_page(html).await?;
    let mut replies = Vec::new();
    for script in scripts {
        replies.push(
            browser
                .command(&info.id, json!({"action": "evaluate", "script": script}))
                .await,
        );
    }
    browser.close_session(&info.id).await.unwrap();
    Some(
        replies
            .into_iter()
            .map(|reply| reply.expect("the script runs on the fixture")["result"].clone())
            .collect(),
    )
}

/// The names of the controls and the words of the text a reading returned.
#[cfg(feature = "agent-browser")]
pub(super) fn shown_names(reading: &serde_json::Value) -> Vec<String> {
    reading["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            node.get("text")
                .or_else(|| node.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_ad_iframes_and_sponsored_blocks_are_removed() {
    let Some(reading) = live_reading(
        r#"<main>
          <h1>Flights to Srinagar</h1>
          <button>Search</button>
          <iframe src="https://securepubads.g.doubleclick.net/slot" width="900" height="300"></iframe>
          <div class="ad-slot"><a href="https://shop.example/deal">Cheap watches</a></div>
          <div id="div-gpt-ad-1234-0"><button>Ad choices</button></div>
          <div class="box"><p>Advertisement</p>
            <iframe src="https://tpc.googlesyndication.com/x" width="300" height="100"></iframe></div>
          <ul>
            <li><span>Sponsored</span> <a href="https://hotel.example/grand">Grand Hotel</a></li>
            <li><a href="https://inn.example/lake">Lake Inn</a></li>
          </ul>
          <a href="https://ad.doubleclick.net/click?x=1">Buy now</a>
          <img src="https://sb.scorecardresearch.com/p?c1=2" width="1" height="1" alt="">
          <img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" alt="">
          <header class="header shadow"><a href="/download">Download app</a></header>
          <div class="badge adults-picker"><button>2 adults</button></div>
          <p class="address">Address: 1 Lake Road</p>
          <div class="css-1ad4k9 sc-hAdSfq"><button>Continue</button></div>
          <div class="gb_2d gb_Ad"><button>Main menu</button></div>
          <div class="gb_ad"><button>Apps</button></div>
          <div class="AdSlot_wrapper__x1y2"><a href="/deal">Watch deal</a></div>
        </main>"#,
    )
    .await
    else {
        return;
    };
    let names = shown_names(&reading);
    for kept in [
        "Flights to Srinagar",
        "Search",
        "Lake Inn",
        "Download app",
        "2 adults",
        "Address: 1 Lake Road",
        "Continue",
        "Main menu",
        "Apps",
    ] {
        assert!(names.iter().any(|name| name == kept), "{kept} in {names:?}");
    }
    for dropped in [
        "Cheap watches",
        "Ad choices",
        "Advertisement",
        "Sponsored",
        "Grand Hotel",
        "Buy now",
        "Watch deal",
    ] {
        assert!(
            !names.iter().any(|name| name.contains(dropped)),
            "{dropped} in {names:?}"
        );
    }
    assert_eq!(
        reading["unreachable"], 0,
        "an ad frame never hides the page"
    );
    assert_eq!(
        reading["denoised"],
        json!({"ads": 9, "empty": 0, "hidden": 0}),
        "{names:?}"
    );
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_private_sight_omits_editable_descendant_content() {
    const UNKNOWN: &str = "profile-autofill-unknown-7Qz9";
    let html = format!(
        r#"<main>
          <div contenteditable="true" aria-label="Message">
            <span>{UNKNOWN}</span>
            <button aria-label="{UNKNOWN} action">Send</button>
          </div>
          <p>Order total is $12.50</p>
        </main>"#
    );
    let Some(mut readings) =
        live_results(&html, &[script_without_form_values(None), script(None)]).await
    else {
        return;
    };
    let private = readings.remove(0);
    let public = readings.remove(0);
    let private_text = private.to_string();
    let public_text = public.to_string();

    assert!(!private_text.contains(UNKNOWN));
    assert!(private_text.contains("Message"));
    assert!(private_text.contains("Order total is $12.50"));
    assert!(public_text.contains(UNKNOWN));
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_blank_containers_are_dropped() {
    let Some(reading) = live_reading(
        r#"<main>
          <div style="cursor: pointer; width: 200px; height: 40px"></div>
          <div tabindex="0" style="width: 100px; height: 30px"><span></span></div>
          <div style="cursor: pointer; width: 40px; height: 40px">
            <svg width="20" height="20"><circle r="5" cx="10" cy="10"></circle></svg></div>
          <div style="cursor: pointer"><span>Show more</span></div>
          <button style="width: 50px; height: 30px"></button>
        </main>"#,
    )
    .await
    else {
        return;
    };
    let controls = reading["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            (
                node["role"].as_str().unwrap(),
                node["name"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        controls,
        [("button", ""), ("button", "Show more"), ("button", "")],
        "a picture's box and a real button stay; blank boxes go"
    );
    assert_eq!(
        reading["denoised"],
        json!({"ads": 0, "empty": 2, "hidden": 0})
    );
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_panels_and_regions_leave_their_rows_to_be_read() {
    // Live, a store's delivery-area popover took a tab stop and was read as
    // one button whose name strung its rows together, so its rows could not
    // be pressed; a tab panel's tab stop hid its fare rows the same way.
    let Some(reading) = live_reading(
        r#"<header>
          <div tabindex="0" style="width: 340px">
            <h3>Select a location for delivery</h3>
            <input type="text" placeholder="Search for area or street name" value="560001">
            <div style="cursor: pointer">560001, Bengaluru, Karnataka</div>
            <div style="cursor: pointer">MG Road, Bengaluru 560001</div>
          </div>
        </header>
        <main>
          <div role="tabpanel" tabindex="0" style="width: 300px">
            <div style="cursor: pointer">Economy</div>
            <div style="cursor: pointer">Business</div>
          </div>
          <div role="group" style="cursor: pointer; width: 300px">Weekend deals</div>
        </main>"#,
    )
    .await
    else {
        return;
    };
    let nodes = reading["nodes"].as_array().unwrap();
    let named = |role: &str| {
        nodes
            .iter()
            .filter(|node| node["role"] == role)
            .map(|node| node["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        named("button"),
        [
            "560001, Bengaluru, Karnataka",
            "MG Road, Bengaluru 560001",
            "Economy",
            "Business",
            "Weekend deals"
        ],
        "each row is its own control, no panel strings them together, and a \
         region the page makes pressable itself (a carousel's slide) stays one"
    );
    assert_eq!(named("textbox").len(), 1, "the panel's search box is read");
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_consent_banners_are_kept() {
    let Some(reading) = live_reading(
        r#"<main><button>Search</button></main>
        <div id="cookie-banner" class="consent-banner ad-consent" role="dialog"
             aria-label="Cookie consent">
          <p>We and our advertising partners use cookies</p>
          <button>Accept all</button><button>Reject</button>
        </div>
        <div class="newsletter sponsor-newsletter">
          <label>Email <input type="email"></label><button>Subscribe</button>
        </div>
        <div id="onetrust-banner-sdk" class="ads-consent-bar">
          <p>Personalised ads and cookies</p><button>Allow</button>
        </div>
        <div class="bar"><span>Ad</span> <p>Your privacy choices</p><button>Manage</button></div>"#,
    )
    .await
    else {
        return;
    };
    let names = shown_names(&reading);
    for kept in [
        "Accept all",
        "Reject",
        "Email",
        "Subscribe",
        "Allow",
        "Manage",
        "We and our advertising partners use cookies",
    ] {
        assert!(names.iter().any(|name| name == kept), "{kept} in {names:?}");
    }
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_shadow_root_that_shows_controls_is_handed_to_the_tree_under_its_layer() {
    // Live, a consent banner's host was drawn as `display: contents`, with
    // no box of its own, and its buttons went unread while the banner lay
    // over the page. A block host whose banner is fixed draws no box either.
    // Its shadow root holds a stylesheet beside the banner, as live: the tree
    // read under the host then finds the banner only through the shadow
    // root.
    let page = |host: &str, banner: &str| {
        format!(
            r#"<main><button>Add To Cart</button></main>
            <div id="host" style="{host}"></div>
            <script>
              document.getElementById('host').attachShadow({{ mode: 'open' }}).innerHTML =
                '<style>p {{ margin: 0 }}</style>'
                + '<div style="position: fixed; right: 0; bottom: 0; width: 400px; height: 200px; {banner}">'
                + '<p>We value your privacy</p><button>Allow Selection</button><button>Allow all</button></div>';
            </script>"#
        )
    };
    for (host, banner, shown) in [
        ("display: contents", "", true),
        ("display: block", "", true),
        ("display: contents", "display: none", false),
    ] {
        let Some(reading) = live_reading(&page(host, banner)).await else {
            return;
        };
        assert_eq!(reading["unreachable"], 0, "{reading}");
        let shadows = reading["shadows"].as_array().unwrap();
        assert_eq!(
            shadows.len(),
            usize::from(shown),
            "host {host:?}, banner {banner:?}: {reading}"
        );
        if shown {
            assert_eq!(
                shadows[0]["label"], "popover \"We value your privacy\"",
                "{reading}"
            );
        }
    }
    // The tree read under the host offers the banner's buttons, and only
    // them.
    let (browser, info) = live_page(&page("display: contents", ""))
        .await
        .expect("a live run opens the page");
    let reading = browser
        .command(
            &info.id,
            json!({"action": "evaluate", "script": script(None)}),
        )
        .await
        .unwrap()["result"]
        .clone();
    let host = reading["shadows"][0]["id"].as_str().unwrap().to_owned();
    let subtree = browser
        .snapshot(
            &info.id,
            tinycomputer_bus::browser::SnapshotRequest {
                selector: Some(format!("[data-tc-seen=\"{host}\"]")),
                ..tinycomputer_bus::browser::SnapshotRequest::default()
            },
        )
        .await
        .unwrap();
    browser.close_session(&info.id).await.unwrap();
    assert!(subtree.tree.contains("Allow Selection"), "{}", subtree.tree);
    assert!(!subtree.tree.contains("Add To Cart"), "{}", subtree.tree);
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_what_a_shadow_host_slots_is_left_to_the_tree() {
    // The tree read under a host holds the host and what the page puts in
    // its slots: sight reads neither, or each would be offered twice. A
    // hidden dialog in the shadow root names no layer, and the banner's
    // `aria-labelledby` resolves inside the shadow root.
    let page = r#"<main><button>Add To Cart</button></main>
        <div id="host"><button>Slotted choice</button></div>
        <script>
          document.getElementById('host').attachShadow({ mode: 'open' }).innerHTML =
            '<div role="dialog" style="display: none">Old template</div>'
            + '<div aria-labelledby="title" style="position: fixed; right: 0; bottom: 0; width: 400px; height: 200px">'
            + '<p id="title">Cookie choices</p><button>Allow Selection</button><slot></slot></div>';
        </script>"#;
    let Some((browser, info)) = live_page(page).await else {
        return;
    };
    let reading = browser
        .command(
            &info.id,
            json!({"action": "evaluate", "script": script(None)}),
        )
        .await
        .unwrap()["result"]
        .clone();
    let names = shown_names(&reading);
    assert!(names.iter().any(|name| name == "Add To Cart"), "{names:?}");
    assert!(
        !names.iter().any(|name| name == "Slotted choice"),
        "{names:?}"
    );
    let shadows = reading["shadows"].as_array().unwrap();
    assert_eq!(shadows.len(), 1, "{reading}");
    assert_eq!(
        shadows[0]["label"], "popover \"Cookie choices\"",
        "{reading}"
    );
    let host = shadows[0]["id"].as_str().unwrap().to_owned();
    let subtree = browser
        .snapshot(
            &info.id,
            tinycomputer_bus::browser::SnapshotRequest {
                selector: Some(format!("[data-tc-seen=\"{host}\"]")),
                ..tinycomputer_bus::browser::SnapshotRequest::default()
            },
        )
        .await
        .unwrap();
    browser.close_session(&info.id).await.unwrap();
    for read in ["Allow Selection", "Slotted choice"] {
        assert!(subtree.tree.contains(read), "{read}: {}", subtree.tree);
    }
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_hidden_elements_are_dropped() {
    let Some(reading) = live_reading(
        r#"<main>
          <button>Visible</button>
          <div aria-hidden="true" style="position: absolute; left: 1400px; top: 100px">
            <button>Clone slide</button><p>Hidden words</p></div>
          <div inert><a href="/x">Inert link</a></div>
          <span style="position: absolute; top: 300px; clip: rect(0 0 0 0)">Screen reader only</span>
          <label><input type="checkbox" style="position: absolute; opacity: 0; width: 1px; height: 1px">
            Keep me signed in</label>
          <label><input type="checkbox"
            style="position: absolute; clip: rect(0 0 0 0); width: 20px; height: 20px"> Send offers</label>
          <p><span aria-hidden="true">Sort by:</span></p>
          <div aria-hidden="true" style="margin-top: 1200px"><button>Explore destinations</button></div>
        </main>"#,
    )
    .await
    else {
        return;
    };
    let names = shown_names(&reading);
    for kept in ["Visible", "Sort by:", "Explore destinations"] {
        assert!(names.iter().any(|name| name == kept), "{kept} in {names:?}");
    }
    for dropped in [
        "Clone slide",
        "Hidden words",
        "Inert link",
        "Screen reader only",
    ] {
        assert!(
            !names.contains(&dropped.to_owned()),
            "{dropped} in {names:?}"
        );
    }
    let checkboxes = reading["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["role"] == "checkbox")
        .map(|node| node["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        checkboxes,
        ["Keep me signed in", "Send offers"],
        "a label stands in for its hidden box"
    );
    assert_eq!(
        reading["denoised"],
        json!({"ads": 0, "empty": 0, "hidden": 3})
    );

    // The page a dialog hides is left out; the dialog is what is read.
    let Some(behind) = live_reading(
        r#"<main aria-hidden="true"><button>Behind the dialog</button><p>Page words</p></main>
        <div role="dialog" aria-modal="true" style="position: fixed; inset: 0; background: white">
          <button>Close</button></div>"#,
    )
    .await
    else {
        return;
    };
    assert_eq!(shown_names(&behind), ["Close"]);
    assert_eq!(behind["denoised"]["hidden"], 1);

    // A page left marked hidden with nothing in front of it — a modal
    // library that forgot to undo its marking — is still what a person sees.
    let Some(stale) = live_reading(
        r#"<div id="app" aria-hidden="true" style="min-height: 600px">
          <button>Book now</button></div>"#,
    )
    .await
    else {
        return;
    };
    assert_eq!(shown_names(&stale), ["Book now"]);
}

/// A page with a native dropdown wrapped in its label, a text box with its
/// own suggestions, and two boxes sharing one list of suggestions; it
/// records every `change` in `window.changes`.
#[cfg(feature = "agent-browser")]
const CHOICES_PAGE: &str = r#"<main>
  <label>Dropdown (select)
    <select id="count"><option>Open this select menu</option><option>One</option>
      <option value="2">Two</option><option disabled>Three</option></select></label>
  <label for="city">Dropdown (datalist)</label>
  <input id="city" list="cities" placeholder="Type to search...">
  <datalist id="cities"><option value="San Francisco">
    <option value="Seattle" label="Washington"></datalist>
  <input id="from" list="airports" aria-label="From">
  <input id="to" list="airports" aria-label="To">
  <datalist id="airports"><option value="BOS"><option value="LHR"></datalist>
</main>
<script>
  window.changes = [];
  document.addEventListener('change', (event) => window.changes.push(event.target.id));
</script>"#;

/// A popover's list scrolled so its last rows are out of its view, beside
/// a button a modal covers.
#[cfg(feature = "agent-browser")]
const SCROLLED_LIST_PAGE: &str = r#"<div style="position: absolute; top: 10px; left: 10px; width: 300px">
  <ul id="list" role="listbox" aria-label="Airports" style="height: 100px; overflow: auto; margin: 0; padding: 0">
    <li role="option">DEL Delhi</li><li role="option">BLR Bengaluru</li><li role="option">MAA Chennai</li>
    <li role="option">HYD Hyderabad</li><li role="option">CCU Kolkata</li><li role="option">BOM Mumbai</li>
    <li role="option">GOI Goa</li><li role="option">PNQ Pune</li>
  </ul>
  <div style="height: 40px; background: white">Planning a holiday?</div>
</div>
<button id="behind" style="position: absolute; top: 200px; left: 10px">Search</button>
<div role="dialog" aria-label="Offer" style="position: absolute; top: 180px; left: 0; width: 400px; height: 80px; background: white">
  Sign up for offers</div>
<style> #list li { height: 24px; cursor: pointer } </style>"#;

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_row_scrolled_out_of_its_list_is_offscreen_not_covered() {
    let Some(reading) = live_reading(SCROLLED_LIST_PAGE).await else {
        return;
    };
    let states = |name: &str| {
        reading["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["name"] == name)
            .map_or_else(
                || panic!("{name} not offered"),
                |node| node["states"].to_string(),
            )
    };
    assert!(
        !states("DEL Delhi").contains("offscreen"),
        "{}",
        states("DEL Delhi")
    );
    for row in ["BOM Mumbai", "PNQ Pune"] {
        let read = states(row);
        assert!(
            read.contains("offscreen") && !read.contains("covered"),
            "{row}: {read}"
        );
    }
    assert!(states("Search").contains("covered"), "a modal still covers");
}

/// The options a reading offers, each as "<name> in <container> <states>".
#[cfg(feature = "agent-browser")]
fn offered(reading: &serde_json::Value) -> Vec<String> {
    reading["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["role"] == "option")
        .map(|node| {
            let inside = node["path"].as_array().unwrap().last().unwrap();
            format!(
                "{} in {} {}",
                node["name"].as_str().unwrap(),
                inside.as_str().unwrap(),
                node["states"]
            )
        })
        .collect()
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_native_choices_are_offered_as_options_and_chosen_by_value() {
    use crate::surface::native_select::CHOOSE_JS;

    let choose = |selector: &str| {
        format!(
            "{CHOOSE_JS}(document.querySelector({}))",
            serde_json::Value::String(selector.to_owned())
        )
    };
    let Some(results) = live_results(
        CHOICES_PAGE,
        &[
            script(None),
            choose("#cities option[value=Seattle]"),
            choose("#count option[value='2']"),
            choose("#count option:disabled"),
            choose("#airports option"),
            "[document.querySelector('#city').value, document.querySelector('#count').value, \
             window.changes]"
                .to_owned(),
            "document.querySelector('#from').focus()".to_owned(),
            script(None),
        ],
    )
    .await
    else {
        return;
    };
    let before = &results[0];
    assert_eq!(
        offered(before),
        [
            r#"Open this select menu in listbox "Dropdown (select)" ["selected"]"#,
            r#"One in listbox "Dropdown (select)" []"#,
            r#"Two in listbox "Dropdown (select)" []"#,
            r#"San Francisco in listbox "Dropdown (datalist)" []"#,
            r#"Seattle in listbox "Dropdown (datalist)" []"#,
        ],
        "a disabled choice is left out, and suggestions two boxes share wait for one to be typed in"
    );
    let names = shown_names(before);
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == "Dropdown (select)")
            .count(),
        2,
        "the label and the dropdown read without its choices: {names:?}"
    );
    let seattle = before["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "Seattle")
        .unwrap();
    assert_eq!(seattle["description"], "Washington");

    assert_eq!(results[1], json!(true), "the suggestion fills its box");
    assert_eq!(results[2], json!(true), "the dropdown takes its choice");
    assert_eq!(results[3], json!(false), "a disabled choice is refused");
    assert_eq!(
        results[4],
        json!(false),
        "a shared suggestion names no box until one is typed in"
    );
    assert_eq!(results[5], json!(["Seattle", "2", ["city", "count"]]));
    assert_eq!(
        offered(&results[7]),
        [
            r#"Open this select menu in listbox "Dropdown (select)" []"#,
            r#"One in listbox "Dropdown (select)" []"#,
            r#"Two in listbox "Dropdown (select)" ["selected"]"#,
            r#"San Francisco in listbox "Dropdown (datalist)" []"#,
            r#"Seattle in listbox "Dropdown (datalist)" ["selected"]"#,
            r#"BOS in listbox "From" []"#,
            r#"LHR in listbox "From" []"#,
        ]
    );
}

/// A page with two calendars, as date pickers draw them, and a table of
/// numbers that is no calendar:
///
/// - a Bootstrap-style picker whose days are plain cells that show a pointer
///   only under the mouse, with "«" and "»" arrows, the days around the
///   month from the months either side, and the 16th disabled;
/// - a jQuery-UI-style one whose month is named in a header before the
///   table, paged by a "Next" link;
/// - a "Season scores" table holding 1 to 28.
#[cfg(feature = "agent-browser")]
const CALENDARS_PAGE: &str = r##"<style>
  .picker th.prev, .picker th.next { cursor: pointer }
  .picker td.day:hover { cursor: pointer }
</style>
<div class="picker" style="position: absolute; top: 10px; left: 10px">
  <table>
    <thead>
      <tr><th class="prev">«</th><th colspan="5">November 2026</th><th class="next">»</th></tr>
      <tr><th>Su</th><th>Mo</th><th>Tu</th><th>We</th><th>Th</th><th>Fr</th><th>Sa</th></tr>
    </thead>
    <tbody id="days"></tbody>
  </table>
</div>
<div class="ui-datepicker" style="position: absolute; top: 260px; left: 10px">
  <div class="ui-datepicker-header"><a class="ui-datepicker-next" style="cursor: pointer">Next</a>
    <div class="ui-datepicker-title">December 2026</div></div>
  <table class="ui-datepicker-calendar"><tbody id="december"></tbody></table>
</div>
<table id="scores" style="position: absolute; top: 520px; left: 10px">
  <caption>Season scores</caption><tbody id="scores-body"></tbody>
</table>
<script>
  const fill = (body, cells, perRow) => {
    for (let at = 0; at < cells.length; at += perRow) {
      const row = body.insertRow();
      for (const html of cells.slice(at, at + perRow)) row.insertCell().outerHTML = html;
    }
  };
  const range = (from, to) => Array.from({ length: to - from + 1 }, (_, index) => from + index);
  fill(document.getElementById('days'), [
    ...range(25, 31).map((day) => `<td class="day old">${day}</td>`),
    ...range(1, 30).map((day) => `<td class="day${day === 16 ? ' disabled' : ''}">${day}</td>`),
    ...range(1, 5).map((day) => `<td class="day new">${day}</td>`),
  ], 7);
  fill(document.getElementById('december'),
    range(1, 31).map((day) => `<td><a href="#">${day}</a></td>`), 7);
  fill(document.getElementById('scores-body'), range(1, 28).map((score) => `<td>${score}</td>`), 7);
</script>"##;

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_date_pickers_days_are_offered_with_the_dates_they_stand_for() {
    let Some(reading) = live_reading(CALENDARS_PAGE).await else {
        return;
    };
    let nodes = reading["nodes"].as_array().unwrap();
    let days = nodes
        .iter()
        .filter(|node| node["role"] == "gridcell")
        .map(|node| {
            format!(
                "{} ({})",
                node["name"].as_str().unwrap(),
                node["description"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>();
    // 41 November days (the disabled 16th left out) and 31 in December;
    // none of the scores.
    assert_eq!(days.len(), 72, "{days:?}");
    for day in [
        "25 (25 October 2026)",
        "1 (1 November 2026)",
        "15 (15 November 2026)",
        "30 (30 November 2026)",
        "5 (5 December 2026)",
        "15 (15 December 2026)",
    ] {
        assert!(days.iter().any(|seen| seen == day), "{day} in {days:?}");
    }
    assert!(!days.iter().any(|seen| seen.starts_with("16 (16 November")));
    let names = shown_names(&reading);
    assert_eq!(
        names
            .iter()
            .filter(|name| name.ends_with(" month"))
            .collect::<Vec<_>>(),
        ["previous month", "next month", "next month"],
        "arrows and a bare Next inside a calendar page it"
    );
}
