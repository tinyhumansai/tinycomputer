//! The helper reply wait: only the line carrying the request's id answers it.

#![cfg(target_os = "macos")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex as StdMutex, mpsc};
use std::time::Duration;

use super::await_reply;

/// A reply channel holding `lines`. Keep the sender alive so an empty
/// channel times out instead of disconnecting.
fn channel(
    lines: &[&str],
) -> (
    mpsc::Sender<String>,
    StdMutex<Option<mpsc::Receiver<String>>>,
) {
    let (tx, rx) = mpsc::channel();
    for line in lines {
        tx.send((*line).to_owned()).unwrap();
    }
    (tx, StdMutex::new(Some(rx)))
}

#[test]
fn stale_blank_and_unnumbered_lines_are_skipped_until_the_id_matches() {
    let (_tx, rx) = channel(&[
        r#"{"id":"6","ok":"stale"}"#,
        "   ",
        r#"{"ok":"no id"}"#,
        r#"{"id":"7","ok":"mine"}"#,
    ]);
    let reply = await_reply(&rx, "7", Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(reply["ok"], "mine");
}

#[test]
fn no_matching_reply_before_the_timeout_is_none() {
    let (_tx, rx) = channel(&[r#"{"id":"6"}"#]);
    assert!(
        await_reply(&rx, "7", Duration::from_millis(50))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_reply_that_is_not_json_is_an_error() {
    let (_tx, rx) = channel(&["not json"]);
    assert!(await_reply(&rx, "7", Duration::from_secs(1)).is_err());
}
