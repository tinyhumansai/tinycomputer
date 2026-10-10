//! `Sim`, a tiny stateful application, and `App`, the `AgentBackend` that
//! serves it: it holds field values, follows pages, and records every press.

use super::*;

/// Fixed behaviours a test gives the simulated app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Quirk {
    /// Set-value on the body is silently ignored, as in a rich-text editor.
    BodyIgnoresSetValue,
    /// Every extra row sits in one list instead of three.
    OneRegion,
    /// No action changes anything.
    Frozen,
    /// Observation fails.
    FailObserve,
    /// Launching fails.
    FailLaunch,
    /// The surface has no addresses, as a desktop application has none.
    NoAddresses,
    /// The compose fields sit in a subtree the budgeted snapshot cut short.
    HiddenEditor,
    /// A side drawer lies over the page: every click is refused as covered
    /// until Escape closes it.
    Drawer,
    /// Every click is refused: the element is not visible.
    Unclickable,
    /// A list of cities whose rows carry a `combobox` role but take no
    /// text, above the one real search field.
    CityRows,
    /// The shop's history is stuck: going back or loading an address
    /// reports success and changes nothing.
    StuckHistory,
    /// The Archive button shows disabled.
    DisabledArchive,
    /// A promo toast with a Close button sits over the page until closed.
    PromoToast,
    /// A cookie bar drawn without a dialog's role, its "Accept all" button
    /// the way out, shows until accepted; like the promo toast, it covers
    /// no control.
    CookieBar,
    /// The inbox's search, once open, fills the window as a sheet.
    SearchSheet,
    /// A consent banner lies over the page as a popover: every other click
    /// is refused as covered, Escape leaves it, and its "Allow Selection"
    /// or "Allow all" closes it.
    ConsentBanner,
    /// Text typed with no target lands at the end of the field typed into
    /// last, as a browser keeps the focus there.
    FocusStays,
    /// Something Escape does not close covers the New Message button.
    Covered,
    /// The opened booking widget leaves the focus outside any text field:
    /// text with no target is refused, as the browser surface refuses it.
    NoFocus,
    /// The obstacle sheet is a dialog whose own bar covers "Keep Editing".
    BarOverSheet,
    /// The obstacle sheet also asks to confirm, with a "Yes" button.
    YesOnSheet,
    /// The inbox shows its search only once its "Search mail" link is
    /// pressed, beside a "Contact us" link.
    SearchBehindLink,
    /// The search behind "Search mail" shows.
    SearchOpen,
    /// The booking calendar is a dialog that stays open once a day is
    /// picked, covering a "Find flights" button, until Escape closes it.
    CalendarStaysOpen,
    /// The booking calendar draws each day as a grid cell holding its
    /// number and a fare, under a heading naming its month, as a hotel
    /// site's does: no day names its month.
    BareCalendarDays,
    /// A box's list of suggestions stays open with an empty backdrop over
    /// the rest of the page, as a store's search box left it: a click there
    /// is refused as covered by that empty layer, Escape leaves it, and only
    /// a press on the backdrop itself closes it.
    Backdrop,
    /// The press the backdrop refused scrolled the page's header into view,
    /// under the backdrop: its three controls show as covered too.
    HeaderUnderBackdrop,
    /// A location button of the page's own is drawn over its controls: a
    /// click there is refused as covered by it, and neither Escape nor a
    /// press outside moves it.
    ControlOver,
    /// An empty layer over the page that nothing closes: a click there is
    /// refused as covered by it, and pressing it is refused too.
    StubbornBackdrop,
    /// An empty layer over the page that goes by itself after refusing two
    /// clicks, before anything presses it.
    FleetingBackdrop,
    /// The fleeting layer has refused one click.
    FleetingOnce,
}

#[derive(Debug, Default)]
pub(super) struct Sim {
    pub(super) compose_open: bool,
    pub(super) sent: bool,
    pub(super) obstacle: bool,
    pub(super) fields: BTreeMap<String, String>,
    pub(super) presses: Vec<String>,
    pub(super) clicks: Vec<String>,
    pub(super) launched: Vec<String>,
    pub(super) navigated: Vec<String>,
    /// Result cards: (airline, price, departure), shown as a list.
    pub(super) results: Vec<(&'static str, &'static str, &'static str)>,
    /// Refs of the result cards' "Select" buttons clicked, in order.
    pub(super) picked: Vec<String>,
    /// The result card, by index, whose "Select" the page shows selected.
    pub(super) selected_result: Option<usize>,
    /// Days in a date strip above the results, a longer list than they are.
    pub(super) date_strip: usize,
    /// Bare fares in a list above the results, cheaper than every flight,
    /// with nothing to open.
    pub(super) fare_chips: usize,
    pub(super) extra_buttons: usize,
    /// A booking form with an autocomplete destination and a calendar.
    pub(super) booking: Option<Booking>,
    /// A ride form whose two boxes keep a place only once it is picked.
    pub(super) places: Option<Places>,
    /// A fare radio shown already checked, as a fare page preselects one.
    pub(super) checked_fare: Option<&'static str>,
    /// A line of guidance shown on the page, such as a date layout.
    pub(super) hint: Option<&'static str>,
    /// A passengers box with adult steppers, holding this many adults.
    pub(super) adults: Option<u8>,
    /// Trip-type tabs: the selected one, and how many clicks on the others
    /// the page ignores first, as a page still loading its scripts does.
    pub(super) trip: Option<(&'static str, u8)>,
    /// A web shop's page stack, newest last; empty for the mail app.
    pub(super) pages: Vec<&'static str>,
    /// The shop's extras checked: "Travel insurance", "Seat protection".
    pub(super) checked: BTreeSet<&'static str>,
    /// Times the shop went back a page.
    pub(super) backs: u32,
    /// The field typed or pasted into last: where the focus stays.
    pub(super) focused: Option<String>,
    pub(super) quirks: BTreeSet<Quirk>,
    /// The calls that touch no element, in order: each `settle` and `settle
    /// briefly`, and each `await_change` as `changed` or `still`.
    pub(super) trail: Vec<&'static str>,
}

impl Sim {
    pub(super) fn has(&self, quirk: Quirk) -> bool {
        self.quirks.contains(&quirk)
    }

    /// The shop page showing, or `None` for the mail app.
    pub(super) fn page(&self) -> Option<&'static str> {
        self.pages.last().copied()
    }

    /// A reply carrying the shop's address, as a browser's does.
    pub(super) fn located(&self, command: &str) -> DesktopResponse {
        DesktopResponse::ok(
            command,
            self.page()
                .map_or_else(|| json!({}), |page| json!({"url": page})),
        )
    }
}

#[derive(Clone, Default)]
pub(super) struct App(Arc<Mutex<Sim>>);

impl App {
    pub(super) fn with(configure: impl FnOnce(&mut Sim)) -> Self {
        let app = Self::default();
        configure(&mut app.0.lock().unwrap());
        app
    }

    pub(super) fn quirky(quirk: Quirk) -> Self {
        Self::with(|sim| {
            sim.quirks.insert(quirk);
        })
    }

    pub(super) fn sim(&self) -> std::sync::MutexGuard<'_, Sim> {
        self.0.lock().unwrap()
    }
}

pub(super) fn node(name: &str, role: &str, actions: &[&str], path: &[&str], y: f64) -> Candidate {
    Candidate {
        ref_id: format!("@s:{name}"),
        role: role.to_owned(),
        name: Some(name.to_owned()),
        available_actions: actions.iter().map(|action| (*action).to_owned()).collect(),
        bounds: Some(json!({"x": 10.0, "y": y})),
        path: path.iter().map(|label| (*label).to_owned()).collect(),
        ..Candidate::default()
    }
}

impl App {
    pub(super) fn screen(&self) -> Screen {
        let sim = self.sim();
        if sim.page().is_some() {
            return shop_screen(&sim);
        }
        let window = if sim.compose_open {
            "New Message"
        } else {
            "Inbox"
        };
        let root = format!("window {window:?}");
        let mut candidates = Vec::new();
        if sim.compose_open {
            for (index, field) in ["To", "Subject"].iter().enumerate() {
                let mut field_node = node(
                    field,
                    "textfield",
                    &["SetValue"],
                    &[&root, "group \"Header\""],
                    100.0 + 30.0 * f64::from(u8::try_from(index).unwrap()),
                );
                field_node.value = sim.fields.get(*field).map(|value| json!(value));
                candidates.push(field_node);
            }
            let mut body = node("Body", "textarea", &["SetValue"], &[&root], 300.0);
            body.value = sim.fields.get("Body").map(|value| json!(value));
            candidates.push(body);
            candidates.push(node(
                "Send",
                "button",
                &["Click"],
                &[&root, "toolbar"],
                40.0,
            ));
        } else {
            checked_fare(&sim, &root, &mut candidates);
            candidates.push(node(
                "New Message",
                "button",
                &["Click"],
                &[&root, "toolbar"],
                40.0,
            ));
            search_behind_link(&sim, &root, &mut candidates);
            let mut archive = node("Archive", "button", &["Click"], &[&root, "toolbar"], 40.0);
            if sim.has(Quirk::DisabledArchive) {
                archive.states = vec!["disabled".to_owned()];
            }
            candidates.push(archive);
            for index in 0..sim.extra_buttons {
                let region = if sim.has(Quirk::OneRegion) {
                    "list \"Messages\"".to_owned()
                } else {
                    format!("list \"Region {}\"", index % 3)
                };
                let name = format!("Message {index}");
                candidates.push(node(
                    &name,
                    "row",
                    &["Click"],
                    &[&root, &region],
                    60.0 + f64::from(u32::try_from(index).unwrap()),
                ));
            }
        }
        if let Some(booking) = &sim.booking {
            booking_widget(&sim, booking, &root, &mut candidates);
        }
        if let Some(places) = &sim.places {
            places_widget(&sim, places, &root, &mut candidates);
        }
        if let Some(adults) = sim.adults {
            passenger_steppers(adults, &root, &mut candidates);
        }
        if let Some((selected, _)) = sim.trip {
            trip_tabs(selected, &root, &mut candidates);
        }
        if sim.has(Quirk::CityRows) {
            city_rows(&root, &mut candidates);
        }
        overlays(&sim, &root, &mut candidates);
        let mut text_nodes = result_cards(&sim, &root, &mut candidates);
        text_nodes.extend(heading_node(&sim, &root));
        let surface = surface_of(&sim);
        if sim.obstacle {
            obstacle_sheet(&sim, &mut candidates);
        }
        Screen {
            app: "Mail".to_owned(),
            window: Some(window.to_owned()),
            surface,
            candidates,
            context: std::iter::once(format!("{window} heading"))
                .chain(sim.hint.map(str::to_owned))
                .chain(calendar_heading(&sim))
                .collect(),
            unexplored: Vec::new(),
            text_nodes,
        }
    }
}

/// What the simulated page shows in front: the obstacle's sheet, a
/// calendar that stays open as a dialog, or the window.
fn surface_of(sim: &Sim) -> String {
    if sim.obstacle || (sim.has(Quirk::SearchSheet) && sim.has(Quirk::SearchOpen)) {
        "sheet".to_owned()
    } else if sim.has(Quirk::CalendarStaysOpen)
        && sim
            .booking
            .as_ref()
            .is_some_and(|booking| booking.calendar.is_some())
    {
        "dialog".to_owned()
    } else {
        "window".to_owned()
    }
}

/// The reply to a click on `name` that something on the simulated page
/// refuses or takes over, or `None` when the click goes through: an
/// unclickable page, a consent banner whose own buttons close it, or a
/// drawer over everything.
fn refused_click(sim: &mut Sim, name: &str) -> Option<DesktopResponse> {
    let covered = |by: &str| {
        DesktopResponse::err(
            "click",
            tinycomputer_bus::DesktopError::new(
                "NOT_ACTIONABLE",
                format!("Element '@s:{name}' is covered by <div.{by}> at its click point"),
            ),
        )
    };
    if sim.has(Quirk::Unclickable) {
        return Some(DesktopResponse::err(
            "click",
            tinycomputer_bus::DesktopError::new(
                "NOT_ACTIONABLE",
                "Element exists but is not visible.",
            ),
        ));
    }
    if sim.has(Quirk::ConsentBanner) {
        if name.starts_with("Allow ") {
            sim.clicks.push(name.to_owned());
            sim.quirks.remove(&Quirk::ConsentBanner);
            return Some(DesktopResponse::ok("click", json!({})));
        }
        return Some(covered("consent"));
    }
    if name == "Find flights"
        && sim.has(Quirk::CalendarStaysOpen)
        && sim
            .booking
            .as_ref()
            .is_some_and(|booking| booking.calendar.is_some())
    {
        return Some(covered("calendar"));
    }
    // What covers the click as the browser surface names it, and whether
    // that is an empty layer a press outside closes.
    let named = |by: &str, cover: &str, empty: bool| {
        let mut reply = covered(by);
        if let Some(error) = reply.error.as_mut() {
            error.details = Some(json!({"cover": cover, "empty_layer": empty}));
        }
        reply
    };
    if sim.has(Quirk::Backdrop) {
        sim.quirks.insert(Quirk::HeaderUnderBackdrop);
        return Some(named("backdrop", "an empty layer", true));
    }
    if sim.has(Quirk::ControlOver) {
        return Some(named("location", "button \"Select Location\"", false));
    }
    if sim.has(Quirk::StubbornBackdrop) {
        return Some(named("backdrop", "an empty layer", true));
    }
    if sim.has(Quirk::FleetingBackdrop) {
        if sim.has(Quirk::FleetingOnce) {
            sim.quirks.remove(&Quirk::FleetingBackdrop);
            sim.quirks.remove(&Quirk::FleetingOnce);
        } else {
            sim.quirks.insert(Quirk::FleetingOnce);
        }
        return Some(named("backdrop", "an empty layer", true));
    }
    sim.has(Quirk::Drawer).then(|| covered("drawer"))
}

impl AgentBackend for App {
    fn observe(
        &self,
        _app: &str,
        root: Option<&str>,
        _depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        if self.sim().has(Quirk::FailObserve) {
            return Err(Box::new(DesktopResponse::err(
                "snapshot",
                tinycomputer_bus::DesktopError::new("APP_NOT_FOUND", "no such app"),
            )));
        }
        let mut screen = self.screen();
        if self.sim().has(Quirk::HiddenEditor) && self.sim().compose_open {
            let (fields, rest): (Vec<_>, Vec<_>) =
                screen.candidates.into_iter().partition(|candidate| {
                    candidate
                        .available_actions
                        .iter()
                        .any(|action| action == "SetValue")
                });
            if root == Some("@s:editor") {
                screen.candidates = fields;
            } else {
                screen.candidates = rest;
                screen.unexplored = vec!["@s:editor".to_owned()];
            }
        }
        Ok(screen)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        let mut sim = self.sim();
        if sim.has(Quirk::Frozen) {
            return DesktopResponse::ok("fake", json!({}));
        }
        let name = target
            .as_ref()
            .and_then(|target| target.name.clone())
            .unwrap_or_default();
        if operation == JevOperation::Click
            && let Some(reply) = refused_click(&mut sim, &name)
        {
            return reply;
        }
        if is_city_row(target.as_ref()) && operation == JevOperation::TypeText {
            return not_a_text_field();
        }
        match operation {
            JevOperation::Click => {
                note_pick(&mut sim, target.as_ref());
                sim.clicks.push(name.clone());
                if name == "Close" && sim.has(Quirk::PromoToast) {
                    sim.quirks.remove(&Quirk::PromoToast);
                    return DesktopResponse::ok("click", json!({}));
                }
                if name == "Accept all" && sim.has(Quirk::CookieBar) {
                    sim.quirks.remove(&Quirk::CookieBar);
                    return DesktopResponse::ok("click", json!({}));
                }
                if sim.page().is_some() {
                    press_shop(&mut sim, &name);
                    return sim.located("click");
                }
                match name.as_str() {
                    "Search mail" => {
                        sim.quirks.insert(Quirk::SearchOpen);
                    }
                    "New Message" => sim.compose_open = true,
                    "Send" => sim.sent = true,
                    "Keep Editing" => sim.obstacle = false,
                    "Archive" => sim.compose_open = false,
                    "Return" | "One way" | "Multi-city" => {
                        if let Some((selected, ignored)) = sim.trip.as_mut() {
                            if *ignored > 0 {
                                *ignored -= 1;
                            } else {
                                *selected = match name.as_str() {
                                    "Return" => "Return",
                                    "One way" => "One way",
                                    _ => "Multi-city",
                                };
                            }
                        }
                    }
                    _ if name.starts_with("Increase number of Adult") => {
                        sim.adults = sim.adults.map(|adults| adults + 1);
                    }
                    _ if name.starts_with("Decrease number of Adult") => {
                        sim.adults = sim.adults.map(|adults| adults.saturating_sub(1));
                    }
                    _ if press_place(&mut sim, &name) => {}
                    _ if sim.booking.is_some() => press_booking(&mut sim, &name),
                    _ => {}
                }
            }
            JevOperation::TypeText if name == "Mumbai, BOM" => {}
            JevOperation::TypeText if target.is_none() && sim.has(Quirk::NoFocus) => {
                return no_focus();
            }
            JevOperation::TypeText if target.is_none() && sim.has(Quirk::FocusStays) => {
                if let Some(field) = sim.focused.clone() {
                    sim.fields
                        .entry(field)
                        .or_default()
                        .push_str(&text.unwrap_or_default());
                }
            }
            // Text with no target goes to the focused field: the booking
            // form's search box once it is open.
            JevOperation::TypeText if target.is_none() => {
                sim.fields
                    .insert("Search city".to_owned(), text.unwrap_or_default());
            }
            JevOperation::TypeText if !(name == "Body" && sim.has(Quirk::BodyIgnoresSetValue)) => {
                type_into(&mut sim, &name, text.unwrap_or_default());
            }
            _ => {}
        }
        DesktopResponse::ok("fake", json!({}))
    }

    fn read_value(&self, target: &Candidate) -> Option<String> {
        let name = target.name.clone().unwrap_or_default();
        Some(self.sim().fields.get(&name).cloned().unwrap_or_default())
    }

    fn await_change(&self, _ms: u64) -> bool {
        let mut sim = self.sim();
        let changed = await_place_rows(&mut sim);
        sim.trail.push(if changed { "changed" } else { "still" });
        changed
    }

    fn settle(&self) {
        self.sim().trail.push("settle");
    }

    fn settle_briefly(&self) {
        self.sim().trail.push("settle briefly");
    }

    fn paste(&self, _app: &str, target: &Candidate, text: &str) -> DesktopResponse {
        if is_city_row(Some(target)) {
            return not_a_text_field();
        }
        let mut sim = self.sim();
        let name = target.name.clone().unwrap_or_default();
        type_into(&mut sim, &name, text.to_owned());
        DesktopResponse::ok("paste", json!({}))
    }

    fn press(&self, _app: &str, combo: &str) -> DesktopResponse {
        let mut sim = self.sim();
        sim.presses.push(combo.to_owned());
        if sim.has(Quirk::Frozen) {
            return DesktopResponse::ok("press", json!({}));
        }
        match combo {
            "cmd+n" => sim.compose_open = true,
            "escape" => {
                sim.obstacle = false;
                sim.quirks.remove(&Quirk::Drawer);
                drop_unpicked(&mut sim);
                if sim.has(Quirk::CalendarStaysOpen)
                    && let Some(booking) = sim.booking.as_mut()
                {
                    booking.calendar = None;
                }
            }
            _ => {}
        }
        DesktopResponse::ok("press", json!({}))
    }

    fn dismiss_cover(&self, _target: &Candidate) -> DesktopResponse {
        let mut sim = self.sim();
        if sim.has(Quirk::StubbornBackdrop) {
            return DesktopResponse::err(
                "dismiss-cover",
                tinycomputer_bus::DesktopError::new(
                    "NOT_ACTIONABLE",
                    "an empty layer lies over the element, and every spot on it lies over something pressable, so it was not pressed",
                ),
            );
        }
        if !sim.has(Quirk::Backdrop) {
            return DesktopResponse::ok("dismiss-cover", json!({"dismissed": null}));
        }
        sim.quirks.remove(&Quirk::Backdrop);
        sim.quirks.remove(&Quirk::HeaderUnderBackdrop);
        sim.clicks.push("the backdrop".to_owned());
        DesktopResponse::ok("dismiss-cover", json!({"dismissed": "an empty layer"}))
    }

    fn back(&self, _app: &str) -> DesktopResponse {
        let mut sim = self.sim();
        if sim.page().is_none() {
            return DesktopResponse::err(
                "back",
                tinycomputer_bus::DesktopError::new("ACTION_NOT_SUPPORTED", "no history"),
            );
        }
        sim.backs += 1;
        if !sim.has(Quirk::StuckHistory) && sim.pages.len() > 1 {
            sim.pages.pop();
        }
        sim.located("back")
    }

    fn launch(&self, app: &str) -> DesktopResponse {
        let mut sim = self.sim();
        sim.launched.push(app.to_owned());
        if sim.page().is_some() {
            return sim.located("launch");
        }
        if sim.has(Quirk::FailLaunch) {
            return DesktopResponse::err(
                "launch",
                tinycomputer_bus::DesktopError::new("APP_NOT_FOUND", "no such app"),
            );
        }
        DesktopResponse::ok("launch", json!({}))
    }

    fn navigate(&self, url: &str) -> DesktopResponse {
        let mut sim = self.sim();
        if sim.has(Quirk::NoAddresses) {
            return DesktopResponse::err(
                "navigate",
                tinycomputer_bus::DesktopError::new("ACTION_NOT_SUPPORTED", "no addresses"),
            );
        }
        sim.navigated.push(url.to_owned());
        if !sim.has(Quirk::StuckHistory)
            && let Some(at) = sim.pages.iter().position(|page| *page == url)
        {
            sim.pages.truncate(at + 1);
        }
        DesktopResponse::ok("navigate", json!({"url": url, "title": "Flights"}))
    }
}

pub(super) fn is_city_row(target: Option<&Candidate>) -> bool {
    target.is_some_and(|target| target.ref_id.starts_with("@s:city-"))
}

pub(super) fn not_a_text_field() -> DesktopResponse {
    DesktopResponse::err(
        "type-text",
        tinycomputer_bus::DesktopError::new("NOT_A_TEXT_FIELD", "no input takes the text"),
    )
}

/// The browser surface's refusal of text with no target when the focus is
/// in no field that takes text.
pub(super) fn no_focus() -> DesktopResponse {
    DesktopResponse::err(
        "type-text",
        tinycomputer_bus::DesktopError::new("INVALID_TARGET", "no editable field has focus"),
    )
}

/// A press of a result card's "Select": recorded, and the card shows itself
/// chosen, as a store's card turns its "Add" into a stepper.
fn note_pick(sim: &mut Sim, target: Option<&Candidate>) {
    let Some(reference) = target
        .map(|target| target.ref_id.clone())
        .filter(|reference| reference.starts_with("@s:select-"))
    else {
        return;
    };
    sim.selected_result = reference
        .trim_start_matches("@s:select-")
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1));
    sim.picked.push(reference);
}
