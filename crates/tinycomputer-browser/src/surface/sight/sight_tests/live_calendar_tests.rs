//! Live tests of calendars sight reads as dates, gated on
//! `TINYCOMPUTER_LIVE_BROWSER=1`: two months that share their arrows,
//! months drawn as grids of buttons rather than tables, months drawn as
//! rows of weeks whose days show a fare and no month, and a grid only a
//! hidden element titles, which is none.

#[cfg(feature = "agent-browser")]
use super::live_tests::{live_reading, shown_names};

/// Two months drawn as grids of buttons, not tables, under one header that
/// names both: each day shows its fare after its number ("22 6529"). Each
/// grid sits in a box of its month's own when `boxed`, or both side by
/// side in one.
#[cfg(feature = "agent-browser")]
fn grid_calendar_page(boxed: bool) -> String {
    let grid = |blanks: u32, days: u32, fares: u32| {
        let cells = (0..blanks)
            .map(|_| "<span></span>".to_owned())
            .chain((1..=days).map(|day| format!("<button>{day} {}</button>", fares + day * 7)))
            .collect::<String>();
        let grid = format!(
            "<div style=\"display: grid; grid-template-columns: repeat(7, 44px)\">{cells}</div>"
        );
        if boxed {
            format!("<div><div>Su Mo Tu We Th Fr Sa</div>{grid}</div>")
        } else {
            grid
        }
    };
    format!(
        "<div style=\"width: 700px\"><div><span>October 2026</span> <span>November 2026</span></div>\
         <div style=\"display: flex; gap: 20px\">{}{}</div></div>",
        grid(4, 31, 6_000),
        grid(0, 30, 7_000)
    )
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_calendar_drawn_as_grids_of_buttons_reads_its_days_as_dates() {
    // Two grids side by side in one block are two months too, not one
    // calendar that hides the second.
    for boxed in [true, false] {
        let Some(reading) = live_reading(&grid_calendar_page(boxed)).await else {
            return;
        };
        let described = |name: &str| {
            reading["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["name"] == name)
                .map_or_else(
                    || panic!("{name} not offered: {:?}", shown_names(&reading)),
                    |node| node["description"].as_str().unwrap_or_default().to_owned(),
                )
        };
        assert_eq!(described("22 6154"), "22 October 2026", "boxed: {boxed}");
        assert_eq!(described("5 7035"), "5 November 2026", "boxed: {boxed}");
    }
}

/// Two months as grids of bare day numbers under one header naming both:
/// October's first day reads "Today 1" when `today_first`, so October's
/// grid is no run of days from 1.
#[cfg(feature = "agent-browser")]
fn shared_header_page(today_first: bool) -> String {
    let grid = |blanks: u32, days: u32, first: &str| {
        let cells = (0..blanks)
            .map(|_| "<span></span>".to_owned())
            .chain((1..=days).map(|day| {
                if day == 1 {
                    format!("<button>{first}</button>")
                } else {
                    format!("<button>{day}</button>")
                }
            }))
            .collect::<String>();
        format!(
            "<div style=\"display: grid; grid-template-columns: repeat(7, 44px)\">{cells}</div>"
        )
    };
    let october_first = if today_first { "Today 1" } else { "1" };
    format!(
        "<div style=\"width: 700px\"><div><span>October 2026</span> <span>November 2026</span></div>\
         <div style=\"display: flex; gap: 20px\">{}{}</div></div>",
        grid(4, 31, october_first),
        grid(0, 30, "1")
    )
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_bare_grids_under_one_header_take_its_months_in_order_or_none() {
    for today_first in [false, true] {
        let Some(reading) = live_reading(&shared_header_page(today_first)).await else {
            return;
        };
        let descriptions = |name: &str| {
            reading["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["name"] == name)
                .map(|node| node["description"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        if today_first {
            // October's grid is missed, so November's must not be read as
            // October: no day of either is dated.
            assert_eq!(descriptions("18"), ["", ""], "{:?}", shown_names(&reading));
        } else {
            assert_eq!(
                descriptions("18"),
                ["18 October 2026", "18 November 2026"],
                "{:?}",
                shown_names(&reading)
            );
        }
    }
}

/// Two months drawn as a hotel site's were: each month's days in rows of
/// a week, each open day its number over a fare and no month, each past
/// day labelled by the page with its date. October's 24th is labelled "Sold
/// out" and its 25th only by its number. Each month sits under a heading
/// of its own when `boxed`; otherwise one header names both, and each
/// month's rows start with a row of day names (November's six weeks make
/// seven rows).
#[cfg(feature = "agent-browser")]
fn week_row_calendar_page(boxed: bool) -> String {
    use std::fmt::Write as _;
    let names = if boxed {
        String::new()
    } else {
        let cells = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
            .map(|name| format!("<div class=\"day\">{name}</div>"))
            .concat();
        format!("<div class=\"week\" role=\"row\">{cells}</div>")
    };
    let body = |blanks: u32, days: u32, fares: u32, past: u32| {
        let cells = (0..blanks)
            .map(|_| "<div class=\"day\"></div>".to_owned())
            .chain((1..=days).map(|day| {
                if day <= past {
                    return format!(
                        "<div class=\"day\" role=\"gridcell\" aria-label=\"Oct {day:02} 2026\">{day}</div>"
                    );
                }
                let label = match day {
                    24 if past > 0 => " aria-label=\"Sold out\"",
                    25 if past > 0 => " aria-label=\"25\"",
                    _ => "",
                };
                let fare = fares + day * 7;
                format!(
                    "<div class=\"day\" role=\"gridcell\" tabindex=\"-1\"{label}><div>{day}</div><div>{},{:03}</div></div>",
                    fare / 1000,
                    fare % 1000
                )
            }))
            .collect::<Vec<_>>();
        let weeks = cells.chunks(7).fold(names.clone(), |mut weeks, week| {
            let _ = write!(
                weeks,
                "<div class=\"week\" role=\"row\">{}</div>",
                week.concat()
            );
            weeks
        });
        format!("<div class=\"body\" role=\"rowgroup\">{weeks}</div>")
    };
    // 1 October 2026 is a Thursday, 1 November a Sunday; weeks start on
    // Monday.
    let (october, november) = (body(3, 31, 5_000, 8), body(6, 30, 7_000, 0));
    let weekdays = "<div>Mo Tu We Th Fr Sa Su</div>";
    let months = if boxed {
        format!(
            "<div style=\"display: flex; gap: 20px\">\
             <div><div>October 2026</div>{weekdays}{october}</div>\
             <div><div>November 2026</div>{weekdays}{november}</div></div>"
        )
    } else {
        format!(
            "<div role=\"button\" tabindex=\"0\" style=\"display: flex; gap: 20px\">\
             <div>October 2026 {weekdays}</div><div>November 2026 {weekdays}</div></div>\
             <div style=\"display: flex; gap: 20px\">{october}{november}</div>"
        )
    };
    format!(
        "<style>.week {{ display: flex }} .day {{ width: 44px; height: 40px; cursor: pointer }}</style>\
         <div style=\"width: 700px\">{months}</div>"
    )
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_calendar_drawn_in_rows_of_weeks_reads_its_days_as_dates() {
    // Live, a hotel site's open days named no month, none read as a date,
    // and the step paged the calendar a year past the month it wanted.
    for boxed in [true, false] {
        let Some(reading) = live_reading(&week_row_calendar_page(boxed)).await else {
            return;
        };
        let described = |name: &str| {
            reading["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["name"] == name)
                .map_or_else(
                    || panic!("{name} not offered: {:?}", shown_names(&reading)),
                    |node| node["description"].as_str().unwrap_or_default().to_owned(),
                )
        };
        assert_eq!(described("23 5,161"), "23 October 2026", "boxed: {boxed}");
        assert_eq!(described("23 7,161"), "23 November 2026", "boxed: {boxed}");
        // A label of the page's own that names the month stays, and one
        // that names none follows the date.
        assert_eq!(described("1"), "Oct 01 2026", "boxed: {boxed}");
        assert_eq!(
            described("24 5,168"),
            "24 October 2026, Sold out",
            "boxed: {boxed}"
        );
        assert_eq!(described("25 5,175"), "25 October 2026", "boxed: {boxed}");
    }
}

/// A picker showing two months, drawn as a hotel site's was: each month a
/// table under a header that holds a hidden month menu, and one pair of icon
/// arrows beside both months, inside neither.
#[cfg(feature = "agent-browser")]
const TWO_MONTH_PICKER: &str = r#"<style>
  .range td span { cursor: pointer }
  .range select { position: absolute; opacity: 0; width: 1px }
  .range__arrow { cursor: pointer; width: 20px; height: 20px }
</style>
<div class="range" style="display: flex; position: absolute; top: 10px; left: 10px">
  <div class="range__arrow range__arrow--previous"></div>
  <div class="range__month">
    <div class="range__header"><span>October<select class="months"></select></span><span> 2026<select><option>1970</option></select></span></div>
    <table><thead><tr><th>Mon</th><th>Tue</th><th>Wed</th><th>Thu</th><th>Fri</th><th>Sat</th><th>Sun</th></tr></thead>
      <tbody id="october"></tbody></table>
  </div>
  <div class="range__month">
    <div class="range__header"><span>November<select class="months"></select></span><span> 2026<select><option>1970</option></select></span></div>
    <table><thead><tr><th>Mon</th><th>Tue</th><th>Wed</th><th>Thu</th><th>Fri</th><th>Sat</th><th>Sun</th></tr></thead>
      <tbody id="november"></tbody></table>
  </div>
  <div class="range__arrow range__arrow--next"></div>
</div>
<script>
  for (const menu of document.querySelectorAll('select.months')) {
    for (const month of ['January', 'February', 'March', 'April', 'May', 'June', 'July',
      'August', 'September', 'October', 'November', 'December']) menu.add(new Option(month));
  }
  const fill = (body, blanks, days) => {
    const cells = [...Array(blanks).fill('<td></td>'),
      ...Array.from({ length: days }, (_, index) => `<td><span>${index + 1}</span></td>`)];
    for (let at = 0; at < cells.length; at += 7) {
      const row = body.insertRow();
      for (const html of cells.slice(at, at + 7)) row.insertCell().outerHTML = html;
    }
  };
  // 1 October 2026 is a Thursday, 1 November a Sunday.
  fill(document.getElementById('october'), 3, 31);
  fill(document.getElementById('november'), 6, 30);
</script>"#;

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_two_month_pickers_days_and_shared_arrows_are_read_as_dates() {
    let Some(reading) = live_reading(TWO_MONTH_PICKER).await else {
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
    // Each month by its header as shown, not by its hidden menu's names.
    assert_eq!(days.len(), 61, "{days:?}");
    for day in [
        "22 (22 October 2026)",
        "31 (31 October 2026)",
        "22 (22 November 2026)",
    ] {
        assert!(days.iter().any(|seen| seen == day), "{day} in {days:?}");
    }
    let names = shown_names(&reading);
    assert_eq!(
        names
            .iter()
            .filter(|name| name.ends_with(" month"))
            .collect::<Vec<_>>(),
        ["previous month", "next month"],
        "the arrows beside both months page them"
    );
}

/// A grid of 31 numbered buttons whose only month and year sit in a hidden
/// element before it, such as a template the page never shows.
#[cfg(feature = "agent-browser")]
fn hidden_title_grid_page() -> String {
    use std::fmt::Write as _;
    let cells = (1..=31).fold(String::new(), |mut cells, day| {
        let _ = write!(cells, "<button>{day}</button>");
        cells
    });
    format!(
        "<div><div style=\"display: none\">October 2026</div>\
         <div style=\"display: grid; grid-template-columns: repeat(7, 44px)\">{cells}</div></div>"
    )
}

#[cfg(feature = "agent-browser")]
#[tokio::test]
async fn live_a_grid_titled_only_by_hidden_text_is_no_calendar() {
    let Some(reading) = live_reading(&hidden_title_grid_page()).await else {
        return;
    };
    let nodes = reading["nodes"].as_array().unwrap();
    assert!(
        !nodes.iter().any(|node| node["role"] == "gridcell"),
        "{:?}",
        shown_names(&reading)
    );
    let day = nodes
        .iter()
        .find(|node| node["name"] == "22")
        .unwrap_or_else(|| panic!("22 not offered: {:?}", shown_names(&reading)));
    assert_eq!(day["description"].as_str().unwrap_or_default(), "");
}
