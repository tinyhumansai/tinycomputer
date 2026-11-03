//! Mapping contract payloads onto the engine's argument types.
//!
//! The contract crate mirrors the engine's enumerations rather than importing
//! them, so that a host can name a surface without linking an accessibility
//! backend. This file is the price of that separation, and paying it in one
//! place is the point: every `match` here is exhaustive over both sides, so a
//! variant added upstream fails this build rather than reaching a host as an
//! `INVALID_ARGS` reply nobody can explain.

use std::path::PathBuf;

use agent_desktop_core as core;
use tinycomputer_bus as bus;

/// Converts a contract surface into the engine's.
pub(super) fn surface(surface: bus::Surface) -> core::SnapshotSurface {
    match surface {
        bus::Surface::Window => core::SnapshotSurface::Window,
        bus::Surface::Focused => core::SnapshotSurface::Focused,
        bus::Surface::Menu => core::SnapshotSurface::Menu,
        bus::Surface::Menubar => core::SnapshotSurface::Menubar,
        bus::Surface::Sheet => core::SnapshotSurface::Sheet,
        bus::Surface::Popover => core::SnapshotSurface::Popover,
        bus::Surface::Alert => core::SnapshotSurface::Alert,
        bus::Surface::Desktop => core::SnapshotSurface::Desktop,
        bus::Surface::Taskbar => core::SnapshotSurface::Taskbar,
        bus::Surface::SystemTray => core::SnapshotSurface::SystemTray,
        bus::Surface::QuickSettings => core::SnapshotSurface::QuickSettings,
        bus::Surface::NotificationCenter => core::SnapshotSurface::NotificationCenter,
        bus::Surface::Toolbar => core::SnapshotSurface::Toolbar,
        bus::Surface::Dock => core::SnapshotSurface::Dock,
        bus::Surface::Spotlight => core::SnapshotSurface::Spotlight,
        bus::Surface::MenuBarExtras => core::SnapshotSurface::MenuBarExtras,
        bus::Surface::SystemTrayOverflow => core::SnapshotSurface::SystemTrayOverflow,
        bus::Surface::StartMenu => core::SnapshotSurface::StartMenu,
        bus::Surface::ActionCenter => core::SnapshotSurface::ActionCenter,
    }
}

/// Converts a contract direction into the engine's.
pub(super) fn direction(direction: bus::Direction) -> core::Direction {
    match direction {
        bus::Direction::Up => core::Direction::Up,
        bus::Direction::Down => core::Direction::Down,
        bus::Direction::Left => core::Direction::Left,
        bus::Direction::Right => core::Direction::Right,
    }
}

/// Converts a contract mouse button into the engine's.
pub(super) fn mouse_button(button: bus::MouseButton) -> core::MouseButton {
    match button {
        bus::MouseButton::Left => core::MouseButton::Left,
        bus::MouseButton::Right => core::MouseButton::Right,
        bus::MouseButton::Middle => core::MouseButton::Middle,
    }
}

/// Converts one contract modifier into the engine's.
pub(super) fn modifier(modifier: bus::Modifier) -> core::Modifier {
    match modifier {
        bus::Modifier::Meta => core::Modifier::Meta,
        bus::Modifier::Ctrl => core::Modifier::Ctrl,
        bus::Modifier::Alt => core::Modifier::Alt,
        bus::Modifier::Shift => core::Modifier::Shift,
    }
}

/// Converts a list of contract modifiers into the engine's.
pub(super) fn modifiers(list: Vec<bus::Modifier>) -> Vec<core::Modifier> {
    list.into_iter().map(modifier).collect()
}

/// Converts a contract clipboard format into the engine's.
pub(super) fn clipboard_format(format: bus::ClipboardFormat) -> core::ClipboardFormat {
    match format {
        bus::ClipboardFormat::Auto => core::ClipboardFormat::Auto,
        bus::ClipboardFormat::Text => core::ClipboardFormat::Text,
        bus::ClipboardFormat::Image => core::ClipboardFormat::Image,
        bus::ClipboardFormat::FileUrls => core::ClipboardFormat::FileUrls,
    }
}

/// Converts a contract element property into the engine's.
pub(super) fn element_property(property: bus::ElementProperty) -> core::commands::get::GetProperty {
    use core::commands::get::GetProperty;

    match property {
        bus::ElementProperty::Text => GetProperty::Text,
        bus::ElementProperty::Value => GetProperty::Value,
        bus::ElementProperty::Title => GetProperty::Title,
        bus::ElementProperty::Bounds => GetProperty::Bounds,
        bus::ElementProperty::Role => GetProperty::Role,
        bus::ElementProperty::States => GetProperty::States,
    }
}

/// Converts a contract element state into the engine's.
pub(super) fn state_property(
    property: bus::ElementStateProperty,
) -> core::commands::is_check::IsProperty {
    use core::commands::is_check::IsProperty;

    match property {
        bus::ElementStateProperty::Visible => IsProperty::Visible,
        bus::ElementStateProperty::Enabled => IsProperty::Enabled,
        bus::ElementStateProperty::Checked => IsProperty::Checked,
        bus::ElementStateProperty::Focused => IsProperty::Focused,
        bus::ElementStateProperty::Expanded => IsProperty::Expanded,
        bus::ElementStateProperty::Selected => IsProperty::Selected,
    }
}

/// Converts a contract state predicate into the engine's.
pub(super) fn state_predicate(predicate: bus::StatePredicate) -> core::StatePredicate {
    core::StatePredicate {
        token: predicate.token,
        expected: predicate.expected,
    }
}

/// Converts an optional path carried as a string.
///
/// The contract carries paths as `String` rather than `PathBuf` because the
/// wire form is JSON and a non-UTF-8 path could not survive the trip anyway.
pub(super) fn path(value: Option<String>) -> Option<PathBuf> {
    value.map(PathBuf::from)
}

/// Converts a contract ref action payload into the engine's shared arguments.
pub(super) fn ref_args(request: bus::RefRequest) -> core::commands::helpers::RefArgs {
    core::commands::helpers::RefArgs {
        ref_id: qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
        timeout_ms: request.timeout_ms,
    }
}

/// Adds a separate snapshot id to a bare local ref while preserving refs that
/// already carry their snapshot in the engine's canonical wire form.
pub(super) fn qualified_ref(ref_id: String, snapshot_id: Option<&str>) -> String {
    let Some(snapshot_id) = snapshot_id else {
        return ref_id;
    };
    let Some(element_number) = ref_id.strip_prefix("@e") else {
        return ref_id;
    };
    if element_number.is_empty() || !element_number.chars().all(|digit| digit.is_ascii_digit()) {
        return ref_id;
    }
    format!("@{snapshot_id}:e{element_number}")
}

/// Converts a contract window target into the engine's.
pub(super) fn window_args(request: bus::WindowRequest) -> core::commands::helpers::AppArgs {
    core::commands::helpers::AppArgs {
        app: request.app,
        window_id: request.window_id,
    }
}

/// Converts a contract drag endpoint into the engine's, folding the two
/// coordinate fields into the pair the engine expects.
///
/// A half-specified point — an `x` with no `y` — becomes no point at all, and
/// the engine reports the missing input by name. Guessing a zero for the other
/// half would drag to the corner of the screen.
pub(super) fn drag_endpoint(
    endpoint: bus::DragEndpoint,
    snapshot_id: Option<&str>,
) -> core::commands::drag::DragEndpoint {
    core::commands::drag::DragEndpoint {
        ref_id: endpoint
            .ref_id
            .map(|ref_id| qualified_ref(ref_id, snapshot_id)),
        xy: point(endpoint.x, endpoint.y),
    }
}

/// Folds an optional `x` and `y` into an optional point.
pub(super) fn point(x: Option<f64>, y: Option<f64>) -> Option<(f64, f64)> {
    x.zip(y)
}
