//! Tests over a real in-memory bus: round trips, fail-closed members, the wire
//! sweep of every member, and confidential delivery.

use super::service;
use crate::tinybus_module::{DesktopService, setup};
use serde_json::json;
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Interface};
use tinycomputer_bus::browser::names::methods as browser_methods;
use tinycomputer_bus::{DesktopResponse, PermissionsRequest, names};

#[tokio::test]
async fn the_module_answers_a_command_over_a_real_bus() -> tinybus::Result<()> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone(), json!({})).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: DesktopResponse = proxy.call(names::methods::VERSION, ()).await?;

    assert!(reply.ok, "version needs no permission and no display");
    assert_eq!(reply.command, "version");
    assert!(reply.data.is_some());
    Ok(())
}

#[tokio::test]
async fn a_member_taking_a_payload_round_trips_it() -> tinybus::Result<()> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone(), json!({})).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: DesktopResponse = proxy
        .call(
            names::methods::PERMISSIONS,
            (PermissionsRequest { request: false },),
        )
        .await?;

    // Reporting permissions works everywhere; whether they are granted is the
    // machine's business, not this test's.
    assert_eq!(reply.command, "permissions");
    Ok(())
}

#[tokio::test]
async fn a_reserved_hold_member_fails_closed_over_the_bus() -> tinybus::Result<()> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone(), json!({})).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: DesktopResponse = proxy
        .call(
            names::methods::KEY_DOWN,
            (json!({ "combo": "shift", "force": false }),),
        )
        .await?;

    // It answers rather than erroring at the transport, and it says no.
    assert!(!reply.ok);
    assert!(reply.error.is_some());
    Ok(())
}

#[tokio::test]
async fn an_unknown_member_is_a_transport_error_not_an_envelope() -> tinybus::Result<()> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone(), json!({})).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let result = proxy.call::<DesktopResponse>("NoSuchMember", ()).await;

    // The envelope carries command failures; a member that does not exist is a
    // different kind of problem and belongs on the other channel.
    assert!(result.is_err());
    Ok(())
}

/// Every member paired with the positional argument array a caller would send,
/// in the order of [`names::METHODS`].
///
/// The payloads are the same rejected-before-anything-happens ones the engine
/// sweep in `desktop/desktop_tests/members_tests.rs` uses, and safe for the
/// same reasons — see the note there. `ClipboardClear` is absent for that
/// note's reason: there is no invalid input to hand it. The browser members
/// name a session or an output that was never opened, so none reaches a
/// browser; `BrowserOpenSession` is absent because every input to it launches
/// one.
fn wire_sweep() -> Vec<(&'static str, serde_json::Value)> {
    let empty_ref = json!([{ "ref_id": "" }]);
    let no_app = json!([{ "app": "" }]);
    let nothing = json!([]);
    let empty = json!([{}]);
    let no_session = json!([{ "session": "s-0" }]);
    let no_output = json!([{ "output": "o-0" }]);

    with_native_cases(vec![
        (
            names::methods::VALIDATE_FLOW,
            json!([{ "flow": { "app": "Mail", "steps": ["start a new note"] } }]),
        ),
        (names::methods::FLOW_GUIDE, nothing.clone()),
        (names::methods::SNAPSHOT, empty.clone()),
        (names::methods::FIND, empty.clone()),
        (
            names::methods::GET,
            json!([{ "ref_id": "", "property": "text" }]),
        ),
        (
            names::methods::IS,
            json!([{ "ref_id": "", "property": "visible" }]),
        ),
        (names::methods::SCREENSHOT, no_app.clone()),
        (names::methods::CLICK, empty_ref.clone()),
        (names::methods::DOUBLE_CLICK, empty_ref.clone()),
        (names::methods::TRIPLE_CLICK, empty_ref.clone()),
        (names::methods::RIGHT_CLICK, empty_ref.clone()),
        (names::methods::TYPE, empty.clone()),
        (names::methods::SET_VALUE, empty.clone()),
        (names::methods::CLEAR, empty_ref.clone()),
        (names::methods::FOCUS, empty_ref.clone()),
        (names::methods::SELECT, empty.clone()),
        (names::methods::TOGGLE, empty_ref.clone()),
        (names::methods::CHECK, empty_ref.clone()),
        (names::methods::UNCHECK, empty_ref.clone()),
        (names::methods::EXPAND, empty_ref.clone()),
        (names::methods::COLLAPSE, empty_ref.clone()),
        (
            names::methods::SCROLL,
            json!([{ "ref_id": "", "direction": "Down", "amount": 1 }]),
        ),
        (names::methods::SCROLL_TO, empty_ref),
        (names::methods::PRESS, empty.clone()),
        (names::methods::KEY_DOWN, empty.clone()),
        (names::methods::KEY_UP, empty.clone()),
        (names::methods::HOVER, empty.clone()),
        (names::methods::DRAG, empty.clone()),
        (names::methods::MOUSE_MOVE, empty.clone()),
        (names::methods::MOUSE_CLICK, empty.clone()),
        (names::methods::MOUSE_DOWN, empty.clone()),
        (names::methods::MOUSE_UP, empty.clone()),
        (names::methods::MOUSE_WHEEL, empty.clone()),
        (names::methods::LAUNCH, no_app.clone()),
        (names::methods::CLOSE_APP, no_app.clone()),
        (names::methods::LIST_APPS, empty.clone()),
        (names::methods::LIST_WINDOWS, empty.clone()),
        (names::methods::LIST_DISPLAYS, nothing.clone()),
        (names::methods::LIST_SURFACES, no_app.clone()),
        (names::methods::FOCUS_WINDOW, no_app.clone()),
        (
            names::methods::RESIZE_WINDOW,
            json!([{ "app": "", "width": 100.0, "height": 100.0 }]),
        ),
        (names::methods::MOVE_WINDOW, no_app.clone()),
        (names::methods::MINIMIZE, no_app.clone()),
        (names::methods::MAXIMIZE, no_app.clone()),
        (names::methods::RESTORE, no_app),
        (names::methods::CLIPBOARD_GET, empty.clone()),
        (names::methods::CLIPBOARD_SET, empty.clone()),
        (names::methods::LIST_NOTIFICATIONS, empty.clone()),
        (names::methods::NOTIFICATION_ACTION, empty.clone()),
        (names::methods::DISMISS_NOTIFICATION, empty.clone()),
        (names::methods::DISMISS_ALL_NOTIFICATIONS, empty.clone()),
        (names::methods::WAIT, json!([{ "ms": 1 }])),
        (names::methods::VERSION, nothing.clone()),
        (names::methods::STATUS, nothing.clone()),
        (names::methods::PERMISSIONS, empty),
        (browser_methods::CLOSE_SESSION, no_session.clone()),
        (browser_methods::LIST_SESSIONS, nothing),
        (
            browser_methods::NAVIGATE,
            json!([{ "session": "s-0", "url": "https://example.com" }]),
        ),
        (browser_methods::SNAPSHOT, no_session.clone()),
        (
            browser_methods::PERFORM,
            json!([{ "session": "s-0", "action": "press", "key": "Tab" }]),
        ),
        (browser_methods::READ_PAGE, no_session.clone()),
        (
            browser_methods::EVALUATE,
            json!([{ "session": "s-0", "expression": "1" }]),
        ),
        (browser_methods::SCREENSHOT, no_session.clone()),
        (browser_methods::READ_OUTPUT, no_output.clone()),
        (browser_methods::RELEASE_OUTPUT, no_output),
        (browser_methods::LIST_DOWNLOADS, no_session.clone()),
        (browser_methods::WAIT_DOWNLOAD, no_session),
    ])
}

fn with_native_cases(
    mut cases: Vec<(&'static str, serde_json::Value)>,
) -> Vec<(&'static str, serde_json::Value)> {
    let at = cases
        .iter()
        .position(|(member, _)| *member == browser_methods::CLOSE_SESSION)
        .expect("browser cases");
    cases.splice(at..at, native_wire_sweep());
    cases
}

fn native_wire_sweep() -> Vec<(&'static str, serde_json::Value)> {
    let nothing = json!([]);
    vec![
        (
            names::accessibility::ACCESSIBILITY_PERMISSIONS,
            nothing.clone(),
        ),
        (
            names::accessibility::ACCESSIBILITY_REQUEST_PERMISSION,
            json!(["microphone"]),
        ),
        (names::accessibility::GLOBE_START, nothing.clone()),
        (names::accessibility::GLOBE_POLL, json!(["unknown-lease"])),
        (names::accessibility::GLOBE_STOP, json!(["unknown-lease"])),
    ]
}

#[test]
fn the_wire_sweep_covers_every_member_except_the_one_with_no_safe_input() {
    let swept = wire_sweep()
        .into_iter()
        .map(|(member, _)| member)
        .collect::<Vec<_>>();
    let missing = names::METHODS
        .iter()
        .filter(|member| !swept.contains(member))
        .collect::<Vec<_>>();

    assert_eq!(
        missing,
        vec![
            &names::methods::RESOLVE_INTENT,
            &names::methods::RUN_GOAL,
            &names::methods::RUN_FLOW,
            &names::methods::DESCRIBE,
            &names::methods::PLAN_TASK,
            &names::methods::START_TASK,
            &names::methods::AWAIT_TASK,
            &names::methods::CONTINUE_TASK,
            &names::methods::CANCEL_TASK,
            &names::methods::TASK_REPORT,
            &names::methods::LIST_TASKS,
            &names::methods::CLIPBOARD_CLEAR,
            &names::accessibility::ACCESSIBILITY_FOCUS,
            &names::accessibility::ACCESSIBILITY_VALIDATE_TARGET,
            &names::accessibility::ACCESSIBILITY_PASTE,
            &browser_methods::OPEN_SESSION,
        ],
        "the task members answer in their own reply shape; see the agent tests below"
    );
}

#[test]
fn every_agentic_member_requires_confidential_delivery() {
    let service = service();
    for member in [
        names::methods::RESOLVE_INTENT,
        names::methods::RUN_GOAL,
        names::methods::RUN_FLOW,
        names::methods::START_TASK,
        names::methods::CONTINUE_TASK,
        names::methods::TASK_REPORT,
    ] {
        assert!(
            service.requires_confidential(&member.try_into().expect("valid member")),
            "{member} was ordinary"
        );
    }
    assert!(
        !service.requires_confidential(
            &names::methods::VERSION
                .try_into()
                .expect("valid ordinary member")
        )
    );
}

#[test]
fn the_catalogue_marks_exactly_the_confidential_members() {
    let service = service();
    for member in tinycomputer_bus::catalogue::MEMBERS {
        assert_eq!(
            service.requires_confidential(&member.name.try_into().expect("valid member")),
            member.confidential,
            "{} disagrees with the served interface",
            member.name
        );
    }
}

#[tokio::test]
async fn private_module_configuration_initializes_jev_without_exposing_the_key()
-> tinybus::Result<()> {
    // The provider's default endpoint: loopback is trusted only inside the
    // engine's own tests. The missing application fails observation before
    // any Jev request, so nothing here reaches the network.
    let configured_service = DesktopService::from_config(&json!({
        "jev": {
            "api_key": "test-secret",
            "provider": "open_router",
            "model": "jev-test",
            "max_retries": 0
        }
    }))
    .expect("private Jev configuration is valid");
    let resolved = configured_service
        .call(
            &names::methods::RESOLVE_INTENT.try_into()?,
            json!([{"app": "__tinycomputer_missing__", "intent": "click"}]),
        )
        .await?;
    let resolved: DesktopResponse = serde_json::from_value(resolved)?;
    assert!(!resolved.ok);
    assert!(!format!("{resolved:?}").contains("test-secret"));

    let service = service();
    for (member, body) in [
        (
            names::methods::RESOLVE_INTENT,
            json!([{"app": "App", "intent": "click something"}]),
        ),
        (
            names::methods::RUN_GOAL,
            json!([{"app": "App", "goal": "finish"}]),
        ),
        (
            names::methods::RUN_FLOW,
            json!([{"flow": {"app": "App", "steps": ["finish"]}}]),
        ),
    ] {
        let reply = service.call(&member.try_into()?, body).await?;
        let reply: DesktopResponse = serde_json::from_value(reply)?;
        assert_eq!(
            reply.error.expect("unconfigured reply").code,
            "JEV_NOT_CONFIGURED"
        );
    }
    assert!(DesktopService::from_config(&json!({"jev": {"api_key": 7}})).is_err());
    Ok(())
}

#[tokio::test]
async fn every_member_decodes_its_payload_and_answers_in_the_envelope() -> tinybus::Result<()> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone(), json!({})).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    for (member, body) in wire_sweep() {
        // A decode failure here would be a transport error rather than an
        // envelope, so reaching the envelope at all is half the assertion.
        let reply: DesktopResponse = proxy.call(member, body).await?;

        assert!(!reply.command.is_empty(), "{member} named nothing");
        assert_eq!(reply.ok, reply.data.is_some(), "{member}");
        assert_eq!(!reply.ok, reply.error.is_some(), "{member}");
    }
    Ok(())
}

#[tokio::test]
async fn confidential_native_payloads_decode_and_execute_on_the_injected_platform()
-> tinybus::Result<()> {
    let service = service();
    for (member, body) in [
        (
            names::accessibility::ACCESSIBILITY_FOCUS,
            json!([{"verbose":true}]),
        ),
        (
            names::accessibility::ACCESSIBILITY_VALIDATE_TARGET,
            json!([{}]),
        ),
        (
            names::accessibility::ACCESSIBILITY_PASTE,
            json!([{"text":"fixture", "target":{}}]),
        ),
    ] {
        assert!(service.requires_confidential(&member.try_into()?));
        let response: DesktopResponse =
            serde_json::from_value(service.call(&member.try_into()?, body).await?)
                .expect("native envelope");
        assert!(response.ok, "{member}: {:?}", response.error);
    }
    Ok(())
}
