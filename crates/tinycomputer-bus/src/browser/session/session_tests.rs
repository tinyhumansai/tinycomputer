//! Tests for the session payload types.
//!
//! The assertions on field names are the wire form: a host and a module that
//! disagree about `default_timeout_ms` fail at runtime with a decode error, and
//! nothing else in the build would catch it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    Screen, SessionId, SessionInfo, SessionOptions, SessionRef, SessionRequest, UserAgentBrand,
    UserAgentMetadata, Viewport,
};
use crate::browser::{Action, NavigateRequest, Target};
use serde_json::json;

#[test]
fn session_id_is_a_bare_string_on_the_wire() {
    let encoded = serde_json::to_value(SessionId::new("s-7")).expect("serializes");
    assert_eq!(encoded, json!("s-7"));

    let decoded: SessionId = serde_json::from_value(json!("s-7")).expect("deserializes");
    assert_eq!(decoded.as_str(), "s-7");
}

#[test]
fn session_id_displays_as_its_identity() {
    assert_eq!(SessionId::from("s-7").to_string(), "s-7");
}

#[test]
fn default_options_are_a_headless_desktop_browser() {
    let options = SessionOptions::default();

    assert!(options.headless);
    assert_eq!(options.viewport, Viewport::desktop(1280, 800));
    assert_eq!(options.default_timeout_ms, 30_000);
    assert!(options.endpoint.is_none());
    assert!(options.download_dir.is_none());
    assert_eq!(options.allowed_origins.len(), 0);
}

#[test]
fn options_fill_every_absent_field_from_the_default() {
    // `#[serde(default)]` on the struct is what lets a host send `{}` and a
    // later contract version add a field without breaking it.
    let options: SessionOptions = serde_json::from_value(json!({})).expect("deserializes");
    assert_eq!(options, SessionOptions::default());

    let partial: SessionOptions =
        serde_json::from_value(json!({ "headless": false })).expect("deserializes");
    assert!(!partial.headless);
    assert_eq!(partial.default_timeout_ms, 30_000);
}

#[test]
fn options_serialize_with_the_documented_field_names() {
    let encoded = serde_json::to_value(SessionOptions::default()).expect("serializes");
    let object = encoded.as_object().expect("an object");

    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "accept_language",
            "allowed_origins",
            "args",
            "default_timeout_ms",
            "download_dir",
            "endpoint",
            "executable",
            "headless",
            "init_scripts",
            "locale",
            "screen",
            "stealth",
            "timezone",
            "user_agent",
            "user_agent_metadata",
            "user_data_dir",
            "viewport",
        ]
    );
}

#[test]
fn a_host_that_predates_the_identity_fields_still_opens_a_session() {
    // The wire form of `SessionOptions` before the identity fields: every
    // field a host could send, and none of the new ones.
    let old: SessionOptions = serde_json::from_value(json!({
        "endpoint": null,
        "executable": "/usr/bin/chromium",
        "headless": false,
        "viewport": {"width": 800, "height": 600, "device_scale_factor": 2.0, "mobile": false},
        "user_agent": "agent",
        "user_data_dir": "/profiles/trip",
        "download_dir": "/downloads",
        "args": ["--lang=en"],
        "allowed_origins": ["https://example.com"],
        "default_timeout_ms": 5000,
    }))
    .expect("an old client's options deserialize");

    assert_eq!(old.timezone, None);
    assert_eq!(old.locale, None);
    assert_eq!(old.accept_language, None);
    assert_eq!(old.user_agent_metadata, None);
    assert_eq!(old.screen, None);
    assert_eq!(old.init_scripts, Vec::<String>::new());
    assert!(!old.stealth);
    assert_eq!(old.user_agent.as_deref(), Some("agent"));
}

#[test]
fn identity_fields_round_trip_in_snake_case() {
    let options = SessionOptions {
        timezone: Some("Asia/Tokyo".to_owned()),
        locale: Some("ja-JP".to_owned()),
        accept_language: Some("ja-JP,ja".to_owned()),
        user_agent_metadata: Some(UserAgentMetadata {
            brands: Some(vec![UserAgentBrand {
                brand: "Chromium".to_owned(),
                version: "140".to_owned(),
            }]),
            full_version_list: None,
            full_version: Some("140.0.0.0".to_owned()),
            platform: "macOS".to_owned(),
            platform_version: "14.5.0".to_owned(),
            architecture: "arm".to_owned(),
            model: String::new(),
            mobile: false,
            bitness: Some("64".to_owned()),
            wow64: None,
        }),
        screen: Some(Screen {
            width: 1920,
            height: 1080,
            device_scale_factor: 2.0,
        }),
        init_scripts: vec!["window.marker = 1;".to_owned()],
        stealth: true,
        ..SessionOptions::default()
    };

    let encoded = serde_json::to_value(&options).expect("serializes");
    assert_eq!(encoded["timezone"], json!("Asia/Tokyo"));
    assert_eq!(encoded["accept_language"], json!("ja-JP,ja"));
    assert_eq!(
        encoded["user_agent_metadata"],
        json!({
            "brands": [{"brand": "Chromium", "version": "140"}],
            "full_version": "140.0.0.0",
            "platform": "macOS",
            "platform_version": "14.5.0",
            "architecture": "arm",
            "model": "",
            "mobile": false,
            "bitness": "64",
        })
    );
    assert_eq!(
        encoded["screen"],
        json!({"width": 1920, "height": 1080, "device_scale_factor": 2.0})
    );
    assert_eq!(encoded["init_scripts"], json!(["window.marker = 1;"]));
    assert_eq!(encoded["stealth"], json!(true));
    assert_eq!(
        serde_json::from_value::<SessionOptions>(encoded).expect("deserializes"),
        options
    );
}

#[test]
fn a_minimal_metadata_and_screen_fill_their_defaults() {
    let options: SessionOptions = serde_json::from_value(json!({
        "user_agent_metadata": {"platform": "Windows"},
        "screen": {"width": 1366, "height": 768},
    }))
    .expect("deserializes");

    let metadata = options.user_agent_metadata.expect("metadata");
    assert_eq!(metadata.platform, "Windows");
    assert_eq!(metadata.platform_version, "");
    assert!(!metadata.mobile);
    assert_eq!(
        options
            .screen
            .expect("screen")
            .device_scale_factor
            .to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn viewport_round_trips() {
    let viewport = Viewport {
        width: 390,
        height: 844,
        device_scale_factor: 3.0,
        mobile: true,
    };
    let encoded = serde_json::to_value(viewport).expect("serializes");

    assert_eq!(
        encoded,
        json!({
            "width": 390,
            "height": 844,
            "device_scale_factor": 3.0,
            "mobile": true,
        })
    );
    assert_eq!(
        serde_json::from_value::<Viewport>(encoded).expect("deserializes"),
        viewport
    );
}

#[test]
fn session_info_round_trips() {
    let info = SessionInfo {
        id: SessionId::new("s-1"),
        endpoint: "ws://127.0.0.1:9222/devtools/browser/abc".to_string(),
        launched: true,
        headless: true,
        viewport: Viewport::default(),
        url: "https://example.com/".to_string(),
        title: "Example Domain".to_string(),
    };

    let encoded = serde_json::to_value(&info).expect("serializes");
    assert_eq!(encoded["id"], json!("s-1"));
    assert_eq!(
        serde_json::from_value::<SessionInfo>(encoded).expect("deserializes"),
        info
    );
}

#[test]
fn a_session_id_is_reachable_from_both_string_forms() {
    assert_eq!(SessionId::from("s-1".to_string()), SessionId::new("s-1"));
    assert_eq!(SessionId::from("s-1"), SessionId::new("s-1"));
}

#[test]
fn session_ids_order_and_hash_so_a_host_can_key_by_them() {
    let mut ids = vec![SessionId::new("s-2"), SessionId::new("s-1")];
    ids.sort();

    assert_eq!(ids, vec![SessionId::new("s-1"), SessionId::new("s-2")]);
    assert_eq!(
        std::collections::HashSet::from([SessionId::new("s-1"), SessionId::new("s-1")]).len(),
        1
    );
}

#[test]
fn a_viewport_can_be_built_for_a_phone() {
    let phone = Viewport {
        mobile: true,
        device_scale_factor: 3.0,
        ..Viewport::desktop(390, 844)
    };

    assert!(phone.mobile);
    assert_ne!(phone, Viewport::default());
}

#[test]
fn a_session_ref_is_one_named_field() {
    let wire = serde_json::to_value(SessionRef {
        session: SessionId::new("s-1"),
    })
    .expect("serializes");
    assert_eq!(wire, json!({"session": "s-1"}));
}

#[test]
fn a_session_request_flattens_the_member_request_beside_the_session() {
    let request: SessionRequest<NavigateRequest> =
        serde_json::from_value(json!({"session": "s-1", "url": "https://example.com"}))
            .expect("deserializes");
    assert_eq!(request.session.as_str(), "s-1");
    assert_eq!(request.request.url, "https://example.com");
}

#[test]
fn a_session_request_carries_a_tagged_action_flat() {
    let wire = serde_json::to_value(SessionRequest::new(
        SessionId::new("s-1"),
        Action::Click {
            target: Target::parse("@e3"),
            new_tab: false,
        },
    ))
    .expect("serializes");
    assert_eq!(wire["session"], "s-1");
    assert_eq!(wire["action"], "click");

    let back: SessionRequest<Action> = serde_json::from_value(wire).expect("round-trips");
    assert!(matches!(back.request, Action::Click { .. }));
}

#[test]
fn a_session_request_without_a_session_is_refused() {
    let missing = serde_json::from_value::<SessionRequest<NavigateRequest>>(
        json!({"url": "https://example.com"}),
    );
    assert!(missing.is_err());
}
