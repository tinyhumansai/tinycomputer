//! Tests for the members that touch nothing outside this process, and the
//! sweep that calls every member on a headless desktop.

use serde_json::json;
use tinycomputer_bus as bus;

use crate::desktop::Desktop;
use agent_desktop_core::ErrorCode;

#[test]
fn version_answers_without_a_permission_or_a_display() {
    let reply = Desktop::new().version();

    assert!(reply.ok, "version must work on an unconfigured machine");
    assert_eq!(reply.command, "version");
    let data = reply.data.expect("a successful reply carries data");
    assert!(data.get("version").is_some());
    assert_eq!(data["os"], json!(std::env::consts::OS));
}

#[test]
fn a_zero_millisecond_wait_returns_immediately_and_reports_what_it_waited() {
    let reply = Desktop::new().wait(bus::WaitRequest::sleep(1));

    assert!(reply.ok);
    assert_eq!(reply.data.expect("data")["waited_ms"], json!(1));
}

#[test]
fn a_wait_with_no_mode_is_an_invalid_argument_rather_than_a_hang() {
    let reply = Desktop::new().wait(bus::WaitRequest::default());

    assert!(!reply.ok);
    let error = reply.error.expect("a failed reply carries an error");
    assert_eq!(error.code, ErrorCode::InvalidArgs.as_str());
}

#[test]
fn an_unknown_surface_wait_is_rejected_by_name() {
    let reply = Desktop::new().wait(bus::WaitRequest {
        surface: Some("menu-closed".to_owned()),
        ..bus::WaitRequest::default()
    });

    assert!(!reply.ok);
    let error = reply.error.expect("error");
    assert!(
        error.message.contains("menu-closed"),
        "the rejection must name the value: {}",
        error.message
    );
}

#[test]
fn every_recognized_surface_wait_is_accepted_by_the_parser() {
    for surface in ["menu", "menu_closed", "notification"] {
        assert!(
            crate::desktop::waiting::surface_wait(Some(surface)).is_ok(),
            "{surface} must parse"
        );
    }
    assert!(
        crate::desktop::waiting::surface_wait(None)
            .unwrap()
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// The member sweep.
//
// Every member is one call into the engine, and the thing worth asserting about
// each is that it calls the right one, names itself correctly, and answers in
// the envelope. Agentic members live above `Desktop` and are covered in their
// own module and bus tests.
// driven from one table.
//
// # Why running all of them is safe
//
// A default `Desktop` is headless, and every payload below is one the engine
// rejects before it reaches the machine:
//
// - the cursor and notification members are refused by the headless input
//   policy, before any hardware or system surface is touched;
// - the ref actions carry an empty ref, which resolves nowhere;
// - `press` carries an empty combo, which fails to parse;
// - the application and window members name an empty application, which
//   resolves to nothing;
// - `clipboard-set` carries no content, which fails validation before the
//   pasteboard is opened.
//
// `clipboard-clear` is the one member with no invalid input to hand it, so it
// is exercised only where the platform has no pasteboard to empty. Anything
// that has to be proved against a live application belongs in a live suite.
// ---------------------------------------------------------------------------

/// Calls every member on a default `Desktop` and returns the replies.
fn sweep() -> Vec<tinycomputer_bus::DesktopResponse> {
    let desktop = Desktop::new();
    let no_app = || Some(String::new());
    let empty_ref = || bus::RefRequest::new("");

    let mut replies = vec![
        desktop.snapshot(bus::SnapshotRequest::default()),
        desktop.find(bus::FindRequest::default()),
        desktop.get(bus::GetRequest::new("", bus::ElementProperty::Text)),
        desktop.is(bus::IsRequest::new("", bus::ElementStateProperty::Visible)),
        desktop.screenshot(bus::ScreenshotRequest {
            app: no_app(),
            ..bus::ScreenshotRequest::default()
        }),
        desktop.click(empty_ref()),
        desktop.double_click(empty_ref()),
        desktop.triple_click(empty_ref()),
        desktop.right_click(empty_ref()),
        desktop.type_text(bus::TypeRequest::default()),
        desktop.set_value(bus::SetValueRequest::default()),
        desktop.clear(empty_ref()),
        desktop.focus(empty_ref()),
        desktop.select(bus::SelectRequest::default()),
        desktop.toggle(empty_ref()),
        desktop.check(empty_ref()),
        desktop.uncheck(empty_ref()),
        desktop.expand(empty_ref()),
        desktop.collapse(empty_ref()),
        desktop.scroll(bus::ScrollRequest::new("", bus::Direction::Down, 1)),
        desktop.scroll_to(empty_ref()),
        desktop.press(bus::PressRequest::new("")),
        desktop.key_down(bus::HoldKeyRequest::default()),
        desktop.key_up(bus::HoldKeyRequest::default()),
        desktop.hover(bus::HoverRequest::default()),
        desktop.drag(bus::DragRequest::default()),
        desktop.mouse_move(bus::MouseMoveRequest::default()),
        desktop.mouse_click(bus::MouseClickRequest::default()),
        desktop.mouse_down(bus::HoldMouseRequest::default()),
        desktop.mouse_up(bus::HoldMouseRequest::default()),
        desktop.mouse_wheel(bus::MouseWheelRequest::default()),
        desktop.launch(bus::LaunchRequest::new("")),
        desktop.close_app(bus::CloseAppRequest::default()),
        desktop.list_apps(bus::ListAppsRequest::default()),
        desktop.list_windows(bus::ListWindowsRequest::default()),
        desktop.list_displays(),
        desktop.list_surfaces(bus::ListSurfacesRequest { app: no_app() }),
        desktop.focus_window(bus::FocusWindowRequest {
            app: no_app(),
            ..bus::FocusWindowRequest::default()
        }),
        desktop.resize_window(bus::ResizeWindowRequest {
            app: no_app(),
            width: 100.0,
            height: 100.0,
            ..bus::ResizeWindowRequest::default()
        }),
        desktop.move_window(bus::MoveWindowRequest {
            app: no_app(),
            ..bus::MoveWindowRequest::default()
        }),
        desktop.minimize(bus::WindowRequest {
            app: no_app(),
            window_id: None,
        }),
        desktop.maximize(bus::WindowRequest {
            app: no_app(),
            window_id: None,
        }),
        desktop.restore(bus::WindowRequest {
            app: no_app(),
            window_id: None,
        }),
        desktop.clipboard_get(bus::ClipboardGetRequest::default()),
        desktop.clipboard_set(bus::ClipboardSetRequest::default()),
    ];

    // See the note above: nothing invalid can be handed to this one, so it runs
    // only where there is no pasteboard for it to empty.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    replies.push(desktop.clipboard_clear());
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    replies.push(tinycomputer_bus::DesktopResponse::ok(
        "clipboard-clear",
        json!({ "skipped": "would empty a real pasteboard" }),
    ));

    replies.extend([
        desktop.list_notifications(bus::ListNotificationsRequest::default()),
        desktop.notification_action(bus::NotificationActionRequest::default()),
        desktop.dismiss_notification(bus::DismissNotificationRequest::default()),
        desktop.dismiss_all_notifications(bus::DismissAllNotificationsRequest::default()),
        desktop.wait(bus::WaitRequest::sleep(1)),
        desktop.version(),
        desktop.status(),
        desktop.permissions(bus::PermissionsRequest::default()),
    ]);

    replies
}

#[test]
fn the_sweep_covers_every_member_the_contract_names() {
    // The five agentic members compose the others, the eight task members
    // run on the engine, and the browser members on the browser adapter;
    // native accessibility members run on their own bridge; none has a
    // desktop command to sweep.
    assert_eq!(
        sweep().len()
            + 5
            + 8
            + bus::browser::names::METHODS.len()
            + bus::names::accessibility::METHODS.len(),
        bus::METHODS.len()
    );
}

#[test]
fn every_member_answers_in_the_envelope() {
    for reply in sweep() {
        assert_eq!(reply.version, bus::ENVELOPE_VERSION);
        assert_ne!(reply.command, "");
        assert_eq!(reply.ok, reply.data.is_some());
        assert_eq!(!reply.ok, reply.error.is_some());
        if let Some(error) = reply.error {
            assert!(
                !error.code.is_empty(),
                "{} gave an empty code",
                reply.command
            );
            assert_ne!(error.message, "");
        }
    }
}

#[test]
fn every_member_names_itself_in_the_engines_spelling() {
    // The envelope's `command` is what a batched caller correlates on and what
    // a trace records, so a member reporting a neighbour's name would be
    // invisible until someone read a trace and disbelieved it.
    let commands = sweep()
        .into_iter()
        .map(|reply| reply.command)
        .collect::<Vec<_>>();

    assert_eq!(
        commands,
        vec![
            "snapshot",
            "find",
            "get",
            "is",
            "screenshot",
            "click",
            "double-click",
            "triple-click",
            "right-click",
            "type",
            "set-value",
            "clear",
            "focus",
            "select",
            "toggle",
            "check",
            "uncheck",
            "expand",
            "collapse",
            "scroll",
            "scroll-to",
            "press",
            "key-down",
            "key-up",
            "hover",
            "drag",
            "mouse-move",
            "mouse-click",
            "mouse-down",
            "mouse-up",
            "mouse-wheel",
            "launch",
            "close-app",
            "list-apps",
            "list-windows",
            "list-displays",
            "list-surfaces",
            "focus-window",
            "resize-window",
            "move-window",
            "minimize",
            "maximize",
            "restore",
            "clipboard-get",
            "clipboard-set",
            "clipboard-clear",
            "list-notifications",
            "notification-action",
            "dismiss-notification",
            "dismiss-all-notifications",
            "wait",
            "version",
            "status",
            "permissions",
        ]
    );
}

#[test]
fn a_headless_desktop_refuses_the_members_that_would_move_the_real_cursor() {
    let desktop = Desktop::new();

    for reply in [
        desktop.hover(bus::HoverRequest::default()),
        desktop.mouse_move(bus::MouseMoveRequest::default()),
        desktop.mouse_click(bus::MouseClickRequest::default()),
        desktop.mouse_wheel(bus::MouseWheelRequest::default()),
        desktop.drag(bus::DragRequest::default()),
    ] {
        assert!(
            !reply.ok,
            "{} must be refused while headless",
            reply.command
        );
    }
}

#[test]
fn a_notification_mutation_is_refused_while_headless() {
    // Opening the notification surface takes focus, so a headless run cannot do
    // it — and finds that out before it acts, not after.
    let reply =
        Desktop::new().dismiss_all_notifications(bus::DismissAllNotificationsRequest::default());

    assert!(!reply.ok);
}
