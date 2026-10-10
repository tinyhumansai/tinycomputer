//! Making an option show when a `choose` step cannot find it: opening the
//! control, paging a calendar, or typing the option to filter a list.

use tinycomputer_bus::JevOperation;

use crate::agentic::flow::{
    FlowRun, Halt, StepLog,
    backend::{AgentBackend, deliver_text},
    view::{Candidate, Screen, element_kind, label},
};

use super::{
    REVEAL_TURNS,
    date::{
        BARE_DAYS, DAYS_REACH, HEADING_REACH, MAX_MONTHS, heads_month_of, is_next_month,
        looks_like_date, names_a_month,
    },
    matching::{clickable, lists_more_than, mentions, search_text},
};

impl<B: AgentBackend + Sync> FlowRun<'_, B> {
    /// The next way to make `option` show after attempt `attempt` found
    /// nothing: open `what`, page a calendar, or type the option to filter.
    pub(super) async fn another_way(
        &mut self,
        log: &mut StepLog,
        attempt: usize,
        screen: &Screen,
        what: &str,
        option: &str,
        into_focus: bool,
    ) -> Result<(), Halt> {
        match attempt {
            0 => {
                // Revealing is one way among several; when it fails the
                // next attempt tries another rather than giving up.
                let revealed = self
                    .accomplish(
                        log,
                        &format!("open {what} so its options show"),
                        REVEAL_TURNS,
                    )
                    .await;
                match revealed {
                    Err(Halt::Failed(note)) => self.history.push(format!(
                        "could not open {what} ({note}); trying another way"
                    )),
                    other => {
                        other?;
                    }
                }
            }
            1 if looks_like_date(option) => self.page_to(log, option).await?,
            // An opened autocomplete holds the focus in its search input,
            // often unnamed; type there before anything moves the focus. A
            // widget that leaves the focus outside any field refuses the
            // text (the browser answers `INVALID_TARGET`), so look for its
            // search box at once rather than spend the attempt (IndiGo's
            // city pickers, tinycomputer#62).
            1 if into_focus => {
                if !self.type_into_focus(log, option).await? {
                    self.type_to_filter(log, screen, what, option).await?;
                }
            }
            2 if !looks_like_date(option) => {
                self.type_to_filter(log, screen, what, option).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Pages a calendar forward, one month at a time, until a control shows
    /// `date` or the calendar heads its month; stops at [`MAX_MONTHS`] or
    /// where there is no next month.
    async fn page_to(&mut self, log: &mut StepLog, date: &str) -> Result<(), Halt> {
        for _ in 0..MAX_MONTHS {
            let screen = self.look().await?;
            // Restricted to the same clickable, non-aggregating pool
            // `pick_option` selects from: a non-clickable calendar container
            // whose label lists every date in the month, or an unrelated
            // result elsewhere on the page, both "mention" the date without
            // being an actionable day, and stopping on either leaves the
            // step with nothing to press.
            if clickable(&screen.candidates)
                .iter()
                .any(|candidate| mentions(candidate, date) && !lists_more_than(candidate, date))
            {
                return Ok(());
            }
            let Some(next) = clickable(&screen.candidates)
                .into_iter()
                .find(|candidate| candidate.name.as_deref().is_some_and(is_next_month))
            else {
                return Ok(());
            };
            // A calendar whose days show bare numbers (a fare beside each)
            // names no day's month: its heading does, and the day is on
            // screen however it reads. Live, a hotel site's open days
            // showed a number and a fare, and its calendar was paged a year
            // past the month it wanted.
            if heads_its_month(&screen, &next, date) {
                return Ok(());
            }
            let clicked = next.clone();
            let reply = self
                .act(log, "click", Some(&next), move |backend| {
                    backend.execute(JevOperation::Click, Some(clicked), None)
                })
                .await?;
            if !reply.ok {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Types `option` wherever the focus is; whether the focus took it. A
    /// refusal leaves no text anywhere, so nothing is recorded as typed.
    async fn type_into_focus(&mut self, log: &mut StepLog, option: &str) -> Result<bool, Halt> {
        let text = search_text(option);
        let reply = self
            .act(log, "type to filter", None, move |backend| {
                backend.execute(JevOperation::TypeText, None, Some(text))
            })
            .await?;
        if !reply.ok {
            self.history
                .push("the focus was not in a field that takes text".to_owned());
            return Ok(false);
        }
        log.filtered = true;
        self.history
            .push("typed into the focused field to filter it".to_owned());
        Ok(true)
    }

    /// Types `option` into the search box of `what`, so an autocomplete
    /// lists it; nothing happens when no field takes text.
    async fn type_to_filter(
        &mut self,
        log: &mut StepLog,
        screen: &Screen,
        what: &str,
        option: &str,
    ) -> Result<(), Halt> {
        let fields = screen
            .candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .available_actions
                    .iter()
                    .any(|action| action == "SetValue")
                    && !self.refused.contains(&element_kind(candidate))
            })
            .cloned()
            .collect::<Vec<_>>();
        let purpose = format!("the search box that filters the options of {what}");
        if let Some(grounded) = self.ground(log, screen, &purpose, &purpose, fields).await? {
            let app = self.app.clone();
            let target = grounded.candidate;
            let field = target.clone();
            let text = search_text(option);
            let reply = self
                .act(log, "type to filter", Some(&target), move |backend| {
                    deliver_text(&backend, &app, &field, &text)
                })
                .await?;
            self.typed.insert(element_kind(&target));
            log.filtered = true;
            if reply.ok {
                self.history
                    .push(format!("typed into {} to filter it", label(&target)));
            } else {
                self.refused.insert(element_kind(&target));
            }
        }
        Ok(())
    }
}

/// Roles of a control that offers a month to choose rather than heading
/// the one shown: a month menu's options, a strip of month tabs.
const CHOICE_ROLES: &[&str] = &[
    "option",
    "tab",
    "radio",
    "menuitem",
    "menuitemradio",
    "menuitemcheckbox",
];

/// Whether the calendar `next` pages heads the month of `date` while its
/// days name none: [`BARE_DAYS`] days or more within [`DAYS_REACH`] nodes of
/// that arrow show only their number, and a heading ([`heads_month_of`])
/// that is no choice of month sits within [`HEADING_REACH`] nodes of it, in
/// document order. A calendar whose days name their month is paged by them
/// alone, and numbers or the same words elsewhere on the page (a results
/// list's pages, months to fly in) say nothing of the month it shows.
pub(in crate::agentic::flow) fn heads_its_month(
    screen: &Screen,
    next: &Candidate,
    date: &str,
) -> bool {
    let bare = clickable(&screen.candidates)
        .iter()
        .filter(|candidate| {
            candidate.order.abs_diff(next.order) <= DAYS_REACH && bare_day(candidate)
        })
        .count();
    bare >= BARE_DAYS
        && screen
            .candidates
            .iter()
            .chain(&screen.text_nodes)
            .filter(|node| {
                node.order.abs_diff(next.order) <= HEADING_REACH
                    && !CHOICE_ROLES
                        .iter()
                        .any(|role| node.role.eq_ignore_ascii_case(role))
            })
            .filter_map(|node| node.name.as_deref())
            .any(|text| heads_month_of(text, date))
}

/// Whether `candidate` is a calendar day that names no month: its label
/// starts with a day's number ("23", "23 6,085"), and neither the label nor
/// its description names a month.
fn bare_day(candidate: &Candidate) -> bool {
    let name = candidate.name.as_deref().unwrap_or_default();
    name.split_whitespace()
        .next()
        .and_then(|word| word.parse::<u8>().ok())
        .is_some_and(|day| (1..=31).contains(&day))
        && !names_a_month(name)
        && !candidate.description.as_deref().is_some_and(names_a_month)
}
