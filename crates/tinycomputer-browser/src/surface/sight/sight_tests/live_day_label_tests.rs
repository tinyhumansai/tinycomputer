//! Live tests of calendar days that carry their date in a label of their
//! own, gated on `TINYCOMPUTER_LIVE_BROWSER=1`: a react-calendar's priced
//! days under one bar of arrows above both months, a day picker's priced
//! days, and a month title whose block pages the month but no wizard
//! around it.

#[cfg(feature = "agent-browser")]
use super::live_tests::{live_reading, shown_names};

/// Two months as a flight site's react-calendar drew them: a bar of "‹",
/// "October 2026 – November 2026", and "›" above both month views, each
/// month six weeks of tiles, every tile its number over a fare with its
/// date in an `abbr`'s label, the month before's and after's days at its
/// ends.
#[cfg(feature = "agent-browser")]
fn react_calendar_page() -> String {
    use std::fmt::Write as _;
    const MONTHS: [&str; 4] = ["September", "October", "November", "December"];
    // (month index, day) for each tile: a month's view starts on the Sunday
    // on or before its first day and holds six weeks.
    let view = |month: usize, before: u32, days: u32, fares: u32| {
        let lead = (0..before).map(|back| (month - 1, 30 - before + 1 + back));
        let own = (1..=days).map(|day| (month, day));
        let tail = (1..).map(|day| (month + 1, day));
        let tiles = lead.chain(own).chain(tail).take(42).fold(String::new(), |mut tiles, (at, day)| {
            let _ = write!(
                tiles,
                "<button class=\"tile\"><abbr aria-label=\"{} {day}, 2026\">{day}</abbr><div>{}</div></button>",
                MONTHS[at],
                fares + day * 7
            );
            tiles
        });
        let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
            .map(|name| format!("<div><abbr aria-label=\"{name}day\">{name}</abbr></div>"))
            .concat();
        format!(
            "<div class=\"month-view\"><div><div><div class=\"weekdays\" style=\"display: flex\">{names}</div>\
             <div class=\"days\" style=\"display: grid; grid-template-columns: repeat(7, 44px)\">{tiles}</div>\
             </div></div></div>"
        )
    };
    // 1 October 2026 is a Thursday (four September days lead), and
    // 1 November a Sunday.
    format!(
        "<style>.tile {{ width: 44px; height: 44px }}</style>\
         <div class=\"react-calendar\" style=\"width: 700px\">\
         <div class=\"navigation\"><button class=\"prev\">‹</button>\
         <button class=\"label\"><span>October 2026</span><span> – </span><span>November 2026</span></button>\
         <button class=\"next\">›</button></div>\
         <div class=\"view-container\" style=\"display: flex; gap: 20px\">{}{}</div></div>",
        view(1, 4, 31, 6_000),
        view(2, 0, 30, 7_000)
    )
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_react_calendars_priced_days_and_its_bar_arrows_are_read() {
    let Some(reading) = live_reading(&react_calendar_page()).await else {
        return;
    };
    let nodes = reading["nodes"].as_array().unwrap();
    let described = |name: &str| {
        nodes.iter().find(|node| node["name"] == name).map_or_else(
            || panic!("{name} not offered: {:?}", shown_names(&reading)),
            |node| node["description"].as_str().unwrap_or_default().to_owned(),
        )
    };
    // Its own date, though a fare follows its number.
    assert_eq!(described("23 6161"), "October 23, 2026");
    assert_eq!(described("23 7161"), "November 23, 2026");
    let names = shown_names(&reading);
    assert!(
        names.iter().any(|name| name == "next month"),
        "the bar's arrow pages both months: {names:?}"
    );
}

/// A month as a hotel site's day picker drew it: weeks of grid cells, each
/// day's button labelled with its date, its number over a fare. Its label
/// sits on the pressable element itself, so it was read before inner labels
/// were: kept as the guard for that layout.
#[cfg(feature = "agent-browser")]
const DAY_PICKER: &str = r#"<div class="DayPicker-Month" role="grid" style="width: 340px">
  <div class="DayPicker-Caption">October 2026</div>
  <div class="DayPicker-Body" role="rowgroup" id="body"></div>
</div>
<script>
  const body = document.getElementById('body');
  const names = ['Thu', 'Fri', 'Sat', 'Sun', 'Mon', 'Tue', 'Wed'];
  const cells = [...Array(3).fill('<div role="gridcell"></div>'),
    ...Array.from({ length: 31 }, (_, index) => {
      const day = index + 1;
      const label = `${names[index % 7]} Oct ${String(day).padStart(2, '0')} 2026`;
      return `<div role="gridcell" style="width: 44px; height: 44px"><div role="button" aria-label="${label}">`
        + `<div>${day}</div><div>${(5000 + day * 37).toLocaleString('en-IN')}</div></div></div>`;
    })];
  for (let at = 0; at < cells.length; at += 7) {
    body.insertAdjacentHTML('beforeend', `<div role="row" style="display: flex">${cells.slice(at, at + 7).join('')}</div>`);
  }
</script>"#;

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_day_pickers_priced_days_are_read_by_their_own_date_labels() {
    let Some(reading) = live_reading(DAY_PICKER).await else {
        return;
    };
    let day = reading["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| {
            node["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("23 "))
        })
        .unwrap_or_else(|| panic!("the 23rd not offered: {:?}", shown_names(&reading)));
    assert_eq!(day["description"], "Fri Oct 23 2026", "{day}");
}

/// A month under a title above a wizard's step: its grid of days, then the
/// wizard's own "Next" beside the grid, outside it.
#[cfg(feature = "agent-browser")]
const WIZARD_PAGE: &str = r#"<section style="width: 600px">
  <h3>October 2026</h3>
  <div class="step">
    <div class="month"><div class="days" id="days" style="display: grid; grid-template-columns: repeat(7, 44px)"></div></div>
    <button>Next</button>
  </div>
</section>
<script>
  const days = document.getElementById('days');
  for (let day = 1; day <= 31; day += 1) days.insertAdjacentHTML('beforeend', `<button>${day}</button>`);
</script>"#;

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_month_title_pages_its_month_but_not_the_wizard_around_it() {
    let Some(reading) = live_reading(WIZARD_PAGE).await else {
        return;
    };
    let nodes = reading["nodes"].as_array().unwrap();
    let day = nodes
        .iter()
        .find(|node| node["name"] == "23")
        .unwrap_or_else(|| panic!("the 23rd not offered: {:?}", shown_names(&reading)));
    assert_eq!(day["description"], "23 October 2026", "{day}");
    let names = shown_names(&reading);
    assert!(
        names.iter().any(|name| name == "Next") && !names.iter().any(|name| name == "next month"),
        "the wizard's own Next stays its own: {names:?}"
    );
}
