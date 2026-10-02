//! Tests for the shared screen cursor: glides reach the sink, one cursor
//! continues across targets, and a missing or failing sink costs nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use super::{OverlaySink, ScreenCursor};
use crate::geometry::Rect;
use crate::pace::CursorPace;
use crate::protocol::OverlayCommand;

#[derive(Clone, Default)]
struct Recorder {
    sent: Arc<Mutex<Vec<OverlayCommand>>>,
    fail: bool,
}

impl OverlaySink for Recorder {
    fn send(&mut self, command: &OverlayCommand) -> std::io::Result<()> {
        if self.fail {
            return Err(std::io::Error::other("gone"));
        }
        self.sent.lock().unwrap().push(command.clone());
        Ok(())
    }
}

const MAIL_BUTTON: Rect = Rect::new(40.0, 700.0, 80.0, 24.0);
const WEB_BUTTON: Rect = Rect::new(900.0, 300.0, 120.0, 32.0);

fn path(command: &OverlayCommand) -> (&[[f64; 3]], bool) {
    match command {
        OverlayCommand::Glide { path, appears } => (path, *appears),
        OverlayCommand::Hide => panic!("expected a glide"),
    }
}

#[test]
fn one_cursor_glides_from_target_to_target_across_surfaces() {
    let recorder = Recorder::default();
    let cursor = ScreenCursor::with_sink(CursorPace::Natural, Box::new(recorder.clone()));
    assert_eq!(cursor.pace(), CursorPace::Natural);
    cursor.show(MAIL_BUTTON);
    cursor.show(WEB_BUTTON);
    let sent = recorder.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    let (first, appears) = path(&sent[0]);
    assert!(appears);
    let landed = *first.last().unwrap();
    assert!(MAIL_BUTTON.contains(crate::Point::new(landed[1], landed[2])));
    let (second, appears) = path(&sent[1]);
    assert!(!appears);
    assert_eq!(
        second[0].map(f64::to_bits),
        [0.0, landed[1], landed[2]].map(f64::to_bits),
        "no jump between surfaces"
    );
}

#[test]
fn hiding_fades_out_once_and_the_next_glide_fades_in() {
    let recorder = Recorder::default();
    let cursor = ScreenCursor::with_sink(CursorPace::Brisk, Box::new(recorder.clone()));
    cursor.hide();
    assert!(
        recorder.sent.lock().unwrap().is_empty(),
        "nothing on screen to hide"
    );
    cursor.show(MAIL_BUTTON);
    cursor.hide();
    cursor.hide();
    cursor.show(WEB_BUTTON);
    let sent = recorder.sent.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[1], OverlayCommand::Hide);
    assert!(path(&sent[2]).1);
}

#[test]
fn an_off_cursor_or_an_unusable_target_sends_nothing() {
    let recorder = Recorder::default();
    let off = ScreenCursor::with_sink(CursorPace::Off, Box::new(recorder.clone()));
    off.show(MAIL_BUTTON);
    off.hide();
    let on = ScreenCursor::with_sink(CursorPace::Natural, Box::new(recorder.clone()));
    on.show(Rect::new(0.0, 0.0, 0.0, 10.0));
    on.show(Rect::new(f64::NAN, 0.0, 10.0, 10.0));
    assert_eq!(*recorder.sent.lock().unwrap(), []);
    assert!(ScreenCursor::off().pace().is_off());
    ScreenCursor::off().show(MAIL_BUTTON);
}

/// Records how long `arrive` waited, per test thread.
fn waited() -> std::time::Duration {
    WAITED.with(std::cell::Cell::get)
}

thread_local! {
    static WAITED: std::cell::Cell<std::time::Duration> =
        const { std::cell::Cell::new(std::time::Duration::ZERO) };
}

fn record(pause: std::time::Duration) {
    WAITED.with(|total| total.set(total.get() + pause));
}

#[test]
fn arriving_waits_for_the_glide_to_land_and_showing_does_not() {
    let recorder = Recorder::default();
    let mut cursor = ScreenCursor::with_sink(CursorPace::Calm, Box::new(recorder));
    cursor.wait = record;
    let travel = cursor.show(MAIL_BUTTON).unwrap();
    assert_eq!(waited(), std::time::Duration::ZERO, "show never waits");
    assert!(travel > std::time::Duration::from_millis(100));
    cursor.arrive(WEB_BUTTON);
    let landed = waited();
    assert!(landed > std::time::Duration::from_millis(100));
    assert!(landed < std::time::Duration::from_secs(3));
}

#[test]
fn a_freshly_started_helper_gets_time_to_appear() {
    let mut cursor = ScreenCursor::with_connect(
        CursorPace::Brisk,
        Box::new(|| Some(Box::new(Recorder::default()) as Box<dyn OverlaySink>)),
    );
    cursor.wait = record;
    let first = cursor.show(MAIL_BUTTON).unwrap();
    let again = cursor.show(MAIL_BUTTON).unwrap();
    assert!(first >= super::HELPER_STARTUP, "{first:?}");
    assert!(
        again < first,
        "only the first glide waits for the helper to start"
    );
}

/// A sink whose queue is always full.
struct Busy;

impl OverlaySink for Busy {
    fn send(&mut self, _: &OverlayCommand) -> std::io::Result<()> {
        Err(std::io::ErrorKind::WouldBlock.into())
    }
}

#[test]
fn a_glide_the_sink_could_not_take_is_never_waited_for() {
    fn never(_: std::time::Duration) {
        panic!("an undrawn glide must not delay the action");
    }
    let mut busy = ScreenCursor::with_sink(CursorPace::Calm, Box::new(Busy));
    busy.wait = never;
    busy.arrive(MAIL_BUTTON);
    busy.arrive(WEB_BUTTON);
    let mut failing = ScreenCursor::with_sink(
        CursorPace::Calm,
        Box::new(Recorder {
            fail: true,
            ..Recorder::default()
        }),
    );
    failing.wait = never;
    failing.arrive(MAIL_BUTTON);
    ScreenCursor::off().arrive(MAIL_BUTTON);
    let _instant = ScreenCursor::with_sink(CursorPace::Natural, Box::new(Recorder::default()))
        .without_waiting();
}

#[test]
fn a_failing_sink_is_dropped_and_the_cursor_starts_afresh() {
    let failing = ScreenCursor::with_sink(
        CursorPace::Calm,
        Box::new(Recorder {
            fail: true,
            ..Recorder::default()
        }),
    );
    failing.show(MAIL_BUTTON);
    failing.show(WEB_BUTTON);
    failing.hide();
    assert!(format!("{failing:?}").contains("Calm"));
}

#[test]
fn a_missing_helper_leaves_the_cursor_off() {
    let cursor = ScreenCursor::new(
        CursorPace::Natural,
        Some("/nonexistent/tinycomputer-cursor-overlay".into()),
    );
    assert!(cursor.show(MAIL_BUTTON).is_none());
    assert!(cursor.show(WEB_BUTTON).is_none());
}

#[cfg(unix)]
#[test]
fn the_helper_process_receives_one_line_per_command() {
    use super::ProcessOverlay;
    let mut overlay = ProcessOverlay::spawn(Some(std::path::Path::new("/bin/cat"))).unwrap();
    // A burst faster than the helper reads is delivered in part and dropped
    // in part — never blocked on.
    let delivered = (0..100)
        .map(|_| overlay.send(&OverlayCommand::Hide))
        .filter(|sent| match sent {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("the helper is alive: {error}"),
        })
        .count();
    assert!(delivered > 0);
    drop(overlay);

    // A helper that exits at once is noticed, and the cursor stops sending.
    let mut gone = ProcessOverlay::spawn(Some(std::path::Path::new("/usr/bin/true"))).unwrap();
    let noticed = (0..200).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(5));
        gone.send(&OverlayCommand::Hide).is_err()
    });
    assert!(noticed);
    let _located = ProcessOverlay::locate();
}
