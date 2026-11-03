//! Tests for the contract-to-engine conversions.

use std::path::Path;

use serde_json::json;
use tinycomputer_bus as bus;

use crate::desktop::convert;

#[test]
fn every_contract_surface_maps_onto_an_engine_surface() {
    // The `match` in `convert` is exhaustive, so this loop is really asserting
    // that the mapping is one-to-one by name rather than merely total.
    for (contract, engine) in [
        (bus::Surface::Window, "window"),
        (bus::Surface::Focused, "focused"),
        (bus::Surface::Menu, "menu"),
        (bus::Surface::Menubar, "menubar"),
        (bus::Surface::Sheet, "sheet"),
        (bus::Surface::Popover, "popover"),
        (bus::Surface::Alert, "alert"),
        (bus::Surface::Desktop, "desktop"),
        (bus::Surface::Taskbar, "taskbar"),
        (bus::Surface::SystemTray, "system-tray"),
        (bus::Surface::QuickSettings, "quick-settings"),
        (bus::Surface::NotificationCenter, "notification-center"),
        (bus::Surface::Toolbar, "toolbar"),
        (bus::Surface::Dock, "dock"),
        (bus::Surface::Spotlight, "spotlight"),
        (bus::Surface::MenuBarExtras, "menu-bar-extras"),
        (bus::Surface::SystemTrayOverflow, "system-tray-overflow"),
        (bus::Surface::StartMenu, "start-menu"),
        (bus::Surface::ActionCenter, "action-center"),
    ] {
        assert_eq!(convert::surface(contract).as_str(), engine);
    }
}

#[test]
fn every_contract_clipboard_format_maps_onto_an_engine_format() {
    for (contract, engine) in [
        (bus::ClipboardFormat::Auto, "auto"),
        (bus::ClipboardFormat::Text, "text"),
        (bus::ClipboardFormat::Image, "image"),
        (bus::ClipboardFormat::FileUrls, "file_urls"),
    ] {
        assert_eq!(convert::clipboard_format(contract).as_str(), engine);
    }
}

#[test]
fn modifiers_convert_in_order_and_without_loss() {
    let converted = convert::modifiers(vec![
        bus::Modifier::Meta,
        bus::Modifier::Ctrl,
        bus::Modifier::Alt,
        bus::Modifier::Shift,
    ]);

    assert_eq!(
        serde_json::to_value(converted).unwrap(),
        json!(["Meta", "Ctrl", "Alt", "Shift"])
    );
}

#[test]
fn directions_and_buttons_convert_without_loss() {
    for (contract, expected) in [
        (bus::Direction::Up, "Up"),
        (bus::Direction::Down, "Down"),
        (bus::Direction::Left, "Left"),
        (bus::Direction::Right, "Right"),
    ] {
        assert_eq!(
            serde_json::to_value(convert::direction(contract)).unwrap(),
            json!(expected)
        );
    }
    for (contract, expected) in [
        (bus::MouseButton::Left, "Left"),
        (bus::MouseButton::Right, "Right"),
        (bus::MouseButton::Middle, "Middle"),
    ] {
        assert_eq!(
            serde_json::to_value(convert::mouse_button(contract)).unwrap(),
            json!(expected)
        );
    }
}

#[test]
fn a_half_specified_point_becomes_no_point_at_all() {
    // Filling the missing half with zero would drag to the corner of the
    // screen; the engine instead reports the missing input by name.
    assert_eq!(convert::point(Some(1.0), None), None);
    assert_eq!(convert::point(None, Some(2.0)), None);
    assert_eq!(convert::point(Some(1.0), Some(2.0)), Some((1.0, 2.0)));
}

#[test]
fn a_drag_endpoint_carries_a_ref_or_a_point_but_never_half_of_one() {
    let by_ref = convert::drag_endpoint(bus::DragEndpoint::at_ref("@s1:e2"), None);
    assert_eq!(by_ref.ref_id.as_deref(), Some("@s1:e2"));
    assert_eq!(by_ref.xy, None);

    let half = convert::drag_endpoint(
        bus::DragEndpoint {
            ref_id: None,
            x: Some(1.0),
            y: None,
        },
        None,
    );
    assert_eq!(half.xy, None);
}

#[test]
fn a_state_predicate_keeps_its_expectation_across_the_boundary() {
    let set = convert::state_predicate(bus::StatePredicate::set("enabled"));
    let negated = convert::state_predicate(bus::StatePredicate::expect("focused", false));

    assert_eq!(set.token, "enabled");
    assert_eq!(set.expected, None);
    assert_eq!(negated.expected, Some(false));
}

#[test]
fn a_ref_request_converts_field_for_field() {
    let args = convert::ref_args(bus::RefRequest {
        ref_id: "@s1:e2".to_owned(),
        snapshot_id: Some("s1".to_owned()),
        timeout_ms: Some(1_000),
    });

    assert_eq!(args.ref_id, "@s1:e2");
    assert_eq!(args.timeout_ms, Some(1_000));
}

#[test]
fn a_bare_ref_is_qualified_by_its_snapshot() {
    assert_eq!(
        convert::qualified_ref("@e2".to_owned(), Some("s1")),
        "@s1:e2"
    );
    assert_eq!(
        convert::qualified_ref("@s2:e2".to_owned(), Some("s1")),
        "@s2:e2"
    );
    assert_eq!(
        convert::qualified_ref("@@e2".to_owned(), Some("s1")),
        "@@e2"
    );
}

#[test]
fn a_window_request_converts_field_for_field() {
    let args = convert::window_args(bus::WindowRequest {
        app: Some("Safari".to_owned()),
        window_id: Some("w1".to_owned()),
    });

    assert_eq!(args.app.as_deref(), Some("Safari"));
    assert_eq!(args.window_id.as_deref(), Some("w1"));
}

#[test]
fn an_absent_path_stays_absent() {
    assert_eq!(convert::path(None), None);
    assert_eq!(
        convert::path(Some("/tmp/shot.png".to_owned())),
        Some(Path::new("/tmp/shot.png").to_path_buf())
    );
}

#[test]
fn every_contract_element_property_maps_onto_an_engine_property() {
    // `GetProperty` is not `PartialEq`, so the mapping is asserted through the
    // property name the engine puts in its reply.
    use agent_desktop_core::commands::get::GetProperty;

    for (contract, engine) in [
        (bus::ElementProperty::Text, GetProperty::Text),
        (bus::ElementProperty::Value, GetProperty::Value),
        (bus::ElementProperty::Title, GetProperty::Title),
        (bus::ElementProperty::Bounds, GetProperty::Bounds),
        (bus::ElementProperty::Role, GetProperty::Role),
        (bus::ElementProperty::States, GetProperty::States),
    ] {
        assert!(matches!(
            (convert::element_property(contract), engine),
            (GetProperty::Text, GetProperty::Text)
                | (GetProperty::Value, GetProperty::Value)
                | (GetProperty::Title, GetProperty::Title)
                | (GetProperty::Bounds, GetProperty::Bounds)
                | (GetProperty::Role, GetProperty::Role)
                | (GetProperty::States, GetProperty::States)
        ));
    }
}

#[test]
fn every_contract_element_state_maps_onto_an_engine_state() {
    use agent_desktop_core::commands::is_check::IsProperty;

    for (contract, engine) in [
        (bus::ElementStateProperty::Visible, IsProperty::Visible),
        (bus::ElementStateProperty::Enabled, IsProperty::Enabled),
        (bus::ElementStateProperty::Checked, IsProperty::Checked),
        (bus::ElementStateProperty::Focused, IsProperty::Focused),
        (bus::ElementStateProperty::Expanded, IsProperty::Expanded),
        (bus::ElementStateProperty::Selected, IsProperty::Selected),
    ] {
        assert!(matches!(
            (convert::state_property(contract), engine),
            (IsProperty::Visible, IsProperty::Visible)
                | (IsProperty::Enabled, IsProperty::Enabled)
                | (IsProperty::Checked, IsProperty::Checked)
                | (IsProperty::Focused, IsProperty::Focused)
                | (IsProperty::Expanded, IsProperty::Expanded)
                | (IsProperty::Selected, IsProperty::Selected)
        ));
    }
}
