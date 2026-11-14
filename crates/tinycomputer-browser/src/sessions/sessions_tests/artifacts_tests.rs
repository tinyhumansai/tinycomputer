//! Tests for screenshots collected into held outputs, and downloads.

use serde_json::json;
use tinycomputer_bus::browser::{DownloadWaitRequest, ScreenshotRequest};

use super::open;
use crate::error::Error;
use crate::fake::{Fake, ok};

#[tokio::test]
async fn a_screenshot_is_collected_into_a_held_output_and_its_file_removed() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "screenshot").await;
    let held = browser
        .screenshot(&id, ScreenshotRequest::default())
        .await
        .unwrap();
    assert_eq!((held.width, held.height), (1280, 800));
    assert_eq!(held.media_type, "image/png");
    let written = fake.last("screenshot")["path"].as_str().unwrap().to_owned();
    assert!(
        !std::path::Path::new(&written).exists(),
        "the staged file is removed"
    );
    let chunk = browser.read_output(&held.id, 0, 1_024).unwrap();
    assert!(chunk.eof);
    browser.sweep_outputs().unwrap();
    assert!(
        browser.read_output(&held.id, 0, 1).is_ok(),
        "fresh outputs survive a sweep"
    );
    browser.release_output(&held.id).unwrap();
    assert!(matches!(
        browser.read_output(&held.id, 0, 1),
        Err(Error::NoSuchOutput { .. })
    ));
}

#[tokio::test]
async fn a_screenshot_the_engine_never_wrote_is_a_failure() {
    let fake = Fake::scripted(|command| {
        (command["action"] == "screenshot").then(|| ok(&json!({"path": "/nonexistent/shot.png"})))
    });
    let (browser, id) = open(&fake, "unwritten").await;
    assert!(matches!(
        browser.screenshot(&id, ScreenshotRequest::default()).await,
        Err(Error::ModuleFailed { .. })
    ));
}

#[tokio::test]
async fn downloads_are_recorded_as_they_finish() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "download").await;
    assert_eq!(
        browser.list_downloads(&id).await.unwrap(),
        [] as [tinycomputer_bus::browser::DownloadInfo; 0]
    );
    let finished = browser
        .wait_download(
            &id,
            DownloadWaitRequest {
                timeout_ms: Some(1_000),
            },
        )
        .await
        .unwrap();
    assert_eq!(finished.received_bytes, 9);
    assert!(finished.suggested_filename.starts_with("download-"));
    assert_eq!(fake.last("waitfordownload")["timeout"], 1_000);
    assert_eq!(browser.list_downloads(&id).await.unwrap(), [finished]);
    browser.close_session(&id).await.unwrap();
}

#[tokio::test]
async fn screenshot_bytes_hands_the_image_back_and_holds_nothing() {
    let fake = Fake::new();
    let (browser, id) = open(&fake, "screenshot-bytes").await;
    let bytes = browser
        .screenshot_bytes(&id, ScreenshotRequest::default())
        .await
        .unwrap();
    assert_eq!(bytes, crate::fake::png(1280, 800));
    let held = browser
        .screenshot(&id, ScreenshotRequest::default())
        .await
        .unwrap();
    assert_eq!(held.id.0, "out-2", "the first image was released, not kept");
}
