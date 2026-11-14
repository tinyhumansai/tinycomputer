//! The statically linked build is admitted by the in-process module host.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;

#[tokio::test]
async fn linked_module_is_admitted_by_the_module_host() {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus);
    let host = ModuleHost::new(broker);
    let module = super::linked_module().expect("manifest is valid");
    let bus_name = module.manifest.bus_name.clone();
    let info = host
        .attach_linked_with_config(module, serde_json::json!({}))
        .expect("linked tinycomputer is admitted");
    assert_eq!(info.manifest.bus_name, bus_name);
    assert_eq!(host.list().len(), 1);
    broker_task.abort();
}

/// Loads the linked module with an egress scope and the host's profile folder,
/// then opens a session asking for `user_data_dir`, returning the reply.
async fn open_with_profile(user_data_dir: &str) -> tinycomputer_bus::DesktopResponse {
    use tinybus::Connection;
    use tinycomputer_bus::{DesktopResponse, browser::names::methods, names};

    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    host.attach_linked_with_config(
        super::linked_module().expect("manifest is valid"),
        serde_json::json!({"browser": {
            // A binary that cannot start, so a permitted launch fails fast
            // instead of looking for or starting a real Chrome.
            "executable": "/nonexistent/host-chrome",
            "profile": "/srv/state/browser/profile",
            "egress": {"policy_id": "host-policy", "generation": 7}
        }}),
    )
    .expect("linked tinycomputer is admitted");
    let client = Connection::connect(bus.connect().await.expect("transport"))
        .await
        .expect("client connection");
    let proxy = client
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .expect("proxy");
    let reply: DesktopResponse = proxy
        .call(
            methods::OPEN_SESSION,
            (serde_json::json!({"user_data_dir": user_data_dir}),),
        )
        .await
        .expect("the call reaches the module");
    broker_task.abort();
    reply
}

#[tokio::test]
async fn a_session_may_not_name_a_profile_the_host_did_not_configure() {
    let reply = open_with_profile("/tmp/some-other-profile").await;
    assert!(!reply.ok, "another folder must be refused");
    let error = format!("{:?}", reply.error);
    assert!(
        error.contains("a browser profile the host did not configure"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn a_session_may_name_the_host_profile_through_the_enforced_launcher() {
    // Without the host's `with_profile` wiring this launch is refused by the
    // profile gate; with it the gate passes, and whatever happens next (no
    // Chrome here, or a launch) is not the profile refusal.
    let reply = open_with_profile("/srv/state/browser/profile").await;
    let error = format!("{:?}", reply.error);
    assert!(
        !error.contains("a browser profile the host did not configure"),
        "the host's own profile was refused: {error}"
    );
}
