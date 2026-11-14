#![allow(
    clippy::expect_used,
    reason = "the test's fixture setup must succeed before its assertions are meaningful"
)]

use serde_json::json;
use tinycomputer_bus::config::{BrowserConfig, BrowserPerception, BrowserSettle, ComputerConfig};

use super::DesktopService;

#[test]
fn module_service_consumes_shared_config_and_preserves_its_extensions() {
    let shared = ComputerConfig {
        browser: Some(BrowserConfig {
            executable: Some("/opt/chromium".into()),
            profile: None,
            user_agent: None,
            args: vec!["--disable-gpu".into()],
            perception: BrowserPerception::Tree,
            settle: BrowserSettle::Steady,
            prelaunch: false,
        }),
        trace_path: Some("/tmp/computer.jsonl".into()),
        ..ComputerConfig::default()
    };
    let mut config = serde_json::to_value(shared).expect("shared config serializes");
    let object = config.as_object_mut().expect("object config");
    object.insert("session_id".into(), json!("run-42"));
    object.insert("trace_strict".into(), json!(true));
    object.insert("headed".into(), json!(true));
    object.insert("cursor".into(), json!("off"));

    let service = DesktopService::from_config(&config).expect("typed config is consumed");

    assert_eq!(service.desktop.session_id(), Some("run-42"));
    assert_eq!(
        service.desktop.trace_path(),
        Some(std::path::Path::new("/tmp/computer.jsonl"))
    );
    assert!(service.desktop.is_tracing());
    assert!(service.desktop.is_headed());
    assert_eq!(
        service.browser_defaults.executable.as_deref(),
        Some("/opt/chromium")
    );
    assert_eq!(service.browser_defaults.args, ["--disable-gpu"]);
    assert!(!service.browser_defaults.prelaunch);
}

#[test]
fn module_rejects_null_shared_sections_but_preserves_nullable_trace_path() {
    for field in ["jev", "planner", "browser"] {
        assert!(
            DesktopService::from_config(&json!({field: null})).is_err(),
            "explicit null must not silently disable {field}"
        );
    }

    let service = DesktopService::from_config(&json!({"trace_path": null}))
        .expect("null trace path keeps the desktop's unset behavior");
    assert!(!service.desktop.is_tracing());
}

#[tokio::test]
async fn a_host_executable_beside_egress_loads_the_enforced_service() {
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.expect("transport"))
        .await
        .expect("module connection");
    let service = DesktopService::from_config_with_connection(
        &json!({"browser": {
            "executable": "/opt/host/chrome",
            "egress": {"policy_id": "host-policy", "generation": 7}
        }}),
        connection,
    )
    .expect("host executable and egress load together");
    assert_eq!(
        service.browser_defaults.executable.as_deref(),
        Some("/opt/host/chrome")
    );
    assert!(service.browser_defaults.egress.is_some());
}
