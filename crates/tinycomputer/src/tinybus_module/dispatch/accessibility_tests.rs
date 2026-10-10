//! Native bridge behavior using an injected platform fixture, never real devices.
#![allow(
    clippy::expect_used,
    reason = "test assertions require fixture operations to succeed"
)]
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tinycomputer_bus::accessibility::PermissionState;

#[derive(Debug, Default)]
struct Fixture {
    reject_target: AtomicBool,
    calls: Mutex<Vec<&'static str>>,
}
impl Fixture {
    fn called(&self, operation: &'static str) {
        self.calls.lock().expect("calls").push(operation);
    }
    fn status() -> GlobeHotkeyStatus {
        GlobeHotkeyStatus {
            supported: true,
            running: true,
            input_monitoring_permission: PermissionState::Granted,
            last_error: None,
            events_pending: 0,
        }
    }
}
impl Backend for Fixture {
    fn permissions(&self) -> PermissionStatus {
        self.called("permissions");
        PermissionStatus {
            accessibility: PermissionState::Granted,
            input_monitoring: PermissionState::Denied,
            microphone: PermissionState::Unknown,
        }
    }
    fn request_permission(&self, _: PermissionKind) -> PermissionStatus {
        self.called("request");
        self.permissions()
    }
    fn focus(&self, _: FocusQuery) -> Result<FocusedTextContext> {
        self.called("focus");
        Ok(FocusedTextContext {
            app_name: Some("fixture".into()),
            role: None,
            text: "captured".into(),
            selected_text: None,
            raw_error: None,
            bounds: None,
        })
    }
    fn validate(&self, _: &FocusTarget) -> Result<()> {
        self.called("validate");
        if self.reject_target.load(Ordering::SeqCst) {
            Err(Error::FocusTargetChanged)
        } else {
            Ok(())
        }
    }
    fn paste(&self, _: PasteRequest) -> Result<()> {
        self.called("paste");
        Ok(())
    }
    fn start(&self) -> Result<GlobeHotkeyStatus> {
        self.called("start");
        Ok(Self::status())
    }
    fn poll(&self) -> Result<GlobeHotkeyPollResult> {
        self.called("poll");
        Ok(GlobeHotkeyPollResult {
            status: Self::status(),
            events: vec!["FN_DOWN".into(), "FN_UP".into()],
        })
    }
    fn stop(&self) -> Result<GlobeHotkeyStatus> {
        self.called("stop");
        let mut status = Self::status();
        status.running = false;
        Ok(status)
    }
}

#[test]
fn insertion_validates_the_captured_target_before_touching_the_platform() {
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture.clone());
    let request = PasteRequest {
        text: "authorized".into(),
        target: FocusTarget::default(),
    };
    access.paste(request.clone()).expect("insert");
    fixture.reject_target.store(true, Ordering::SeqCst);
    assert!(matches!(
        access.paste(request),
        Err(Error::FocusTargetChanged)
    ));
    assert_eq!(
        *fixture.calls.lock().expect("calls"),
        ["validate", "paste", "validate"]
    );
}

#[test]
fn listener_leases_preserve_order_and_reject_calls_after_release() {
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture.clone());
    let first = access.start().expect("start");
    let second = access.start().expect("idempotent start");
    assert_eq!(first.handle, second.handle);
    let invalid = GlobeHandle("unknown".into());
    assert!(matches!(access.poll(&invalid), Err(Error::UnknownListener)));
    assert!(matches!(access.stop(&invalid), Err(Error::UnknownListener)));
    assert_eq!(
        access.poll(&first.handle).expect("poll").events,
        ["FN_DOWN", "FN_UP"]
    );
    assert!(!access.stop(&first.handle).expect("stop").running);
    assert!(matches!(
        access.poll(&first.handle),
        Err(Error::UnknownListener)
    ));
    assert!(matches!(
        access.stop(&first.handle),
        Err(Error::UnknownListener)
    ));
    drop(access);
    assert_eq!(
        *fixture.calls.lock().expect("calls"),
        ["start", "start", "poll", "stop"]
    );
}

#[test]
fn dropping_the_last_owner_stops_an_abandoned_listener() {
    let fixture = Arc::new(Fixture::default());
    let access = Access::new(fixture.clone());
    access.start().expect("start");
    drop(access);
    assert_eq!(*fixture.calls.lock().expect("calls"), ["start", "stop"]);
}

#[tokio::test]
async fn module_threads_project_native_results_and_structured_target_errors() {
    let fixture = Arc::new(Fixture::default());
    let access = Arc::new(Access::new(fixture.clone()));
    let permissions = run(access.clone(), "permissions", |a| {
        Ok(a.request_permission(PermissionKind::Microphone))
    })
    .await
    .expect("reply");
    assert!(permissions.ok);
    let state: PermissionStatus =
        serde_json::from_value(permissions.data.expect("data")).expect("permissions");
    assert_eq!(state.microphone, PermissionState::Unknown);
    let focus = run(access.clone(), "focus", |a| a.focus(FocusQuery::default()))
        .await
        .expect("reply");
    assert!(focus.ok);
    assert_eq!(focus.data.expect("data")["text"], "captured");
    fixture.reject_target.store(true, Ordering::SeqCst);
    let failed = run(access, "paste", |a| {
        a.paste(PasteRequest {
            text: "authorized".into(),
            target: FocusTarget::default(),
        })
    })
    .await
    .expect("reply");
    assert!(!failed.ok);
    assert_eq!(failed.error.expect("error").code, "FOCUS_CHANGED");
    assert!(!fixture.calls.lock().expect("calls").contains(&"paste"));
}

#[tokio::test]
async fn the_served_interface_routes_native_members_and_listener_handles() {
    use tinybus::{Connection, broker::Broker, transport::memory::MemoryBus};
    use tinycomputer_bus::names;
    let fixture = Arc::new(Fixture::default());
    let mut service =
        super::super::DesktopService::from_config(&serde_json::json!({})).expect("service");
    service.accessibility = Arc::new(Access::new(fixture.clone()));
    let bus = MemoryBus::new();
    let task = Broker::new().spawn(bus.clone());
    let server = Connection::connect(bus.connect().await.expect("transport"))
        .await
        .expect("server");
    server
        .serve_at(names::OBJECT_PATH.try_into().expect("path"), service)
        .await
        .expect("serve");
    server.request_name(names::INTERFACE).await.expect("name");
    let client = Connection::connect(bus.connect().await.expect("transport"))
        .await
        .expect("client");
    let proxy = client
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .expect("proxy");
    let permissions: DesktopResponse = proxy
        .call(names::accessibility::ACCESSIBILITY_PERMISSIONS, ())
        .await
        .expect("permissions");
    assert!(permissions.ok);
    let requested: DesktopResponse = proxy
        .call(
            names::accessibility::ACCESSIBILITY_REQUEST_PERMISSION,
            (PermissionKind::Microphone,),
        )
        .await
        .expect("request");
    assert!(requested.ok);
    let started: DesktopResponse = proxy
        .call(names::accessibility::GLOBE_START, ())
        .await
        .expect("start");
    let started: GlobeStarted =
        serde_json::from_value(started.data.expect("data")).expect("started");
    let polled: DesktopResponse = proxy
        .call(names::accessibility::GLOBE_POLL, (started.handle.clone(),))
        .await
        .expect("poll");
    let batch: GlobeHotkeyPollResult =
        serde_json::from_value(polled.data.expect("data")).expect("batch");
    assert_eq!(batch.events, ["FN_DOWN", "FN_UP"]);
    let stopped: DesktopResponse = proxy
        .call(names::accessibility::GLOBE_STOP, (started.handle.clone(),))
        .await
        .expect("stop");
    assert!(stopped.ok);
    let released: DesktopResponse = proxy
        .call(names::accessibility::GLOBE_POLL, (started.handle,))
        .await
        .expect("released reply");
    assert!(!released.ok);
    assert_eq!(released.error.expect("error").code, "UNKNOWN_LISTENER");
    task.abort();
}

pub(super) fn fixture_access() -> Arc<Access> {
    Arc::new(Access::new(Arc::new(Fixture::default())))
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_focus_and_globe_members_do_not_allocate_native_resources() {
    let platform = Platform;
    for verbose in [false, true] {
        assert!(matches!(
            platform.focus(FocusQuery { verbose }),
            Err(Error::UnsupportedPlatform)
        ));
    }
    platform
        .validate(&FocusTarget::default())
        .expect("empty target");
    assert!(!platform.start().expect("unsupported status").supported);
    assert!(
        platform
            .poll()
            .expect("unsupported batch")
            .events
            .is_empty()
    );
    assert!(!platform.stop().expect("unsupported status").running);
    let access = Access::default();
    assert!(matches!(access.start(), Err(Error::UnsupportedPlatform)));
    assert!(access.listener.lock().expect("listener").is_none());
}

#[derive(Debug)]
struct RefusedSerialization;
impl serde::Serialize for RefusedSerialization {
    fn serialize<S: serde::Serializer>(&self, _: S) -> std::result::Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("private fixture content"))
    }
}

#[tokio::test]
async fn module_faults_keep_safe_codes_and_transport_messages() {
    let access = Arc::new(Access::new(Arc::new(Fixture::default())));
    for (error, code) in [
        (Error::UnsupportedPlatform, "PLATFORM_NOT_SUPPORTED"),
        (
            Error::GlobeListener("fixture fault".into()),
            "ACCESSIBILITY_FAILED",
        ),
    ] {
        let response = run::<(), _>(access.clone(), "fixture", |_| Err(error))
            .await
            .expect("envelope");
        assert_eq!(response.error.expect("error").code, code);
    }
    let failed = run(access, "fixture", |_| Ok(RefusedSerialization))
        .await
        .expect_err("serialization refused");
    assert!(failed.to_string().contains("serialization failed"));
    assert!(!failed.to_string().contains("private fixture content"));
}
