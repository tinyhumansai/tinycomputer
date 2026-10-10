//! Local mock process and pipe worker cleanup, with no devices or display.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "controlled native fixtures assert completion and worker panic handling"
)]
use super::*;
#[cfg(unix)]
use command_group::CommandGroup;
#[cfg(unix)]
#[test]
fn cleanup_failure_retains_child_and_pipe_workers_until_successful_retry() {
    use std::process::{Command, Stdio};
    let child = Command::new("/bin/sh")
        .args(["-c", "exec sleep 600"])
        .stdout(Stdio::piped())
        .group_spawn()
        .expect("fixture");
    let mut owned = Owned::new(child);
    let (release, wait) = std::sync::mpsc::channel();
    let (joined, observed) = std::sync::mpsc::channel();
    owned.reader(std::thread::spawn(move || {
        wait.recv().expect("release");
        joined.send(()).expect("join notification");
    }));
    owned.fail_reap = true;
    assert_eq!(owned.cleanup(), Err("native_reap_failed"));
    assert!(observed.try_recv().is_err());
    release.send(()).expect("allow worker completion");
    owned.cleanup().expect("retry cleanup");
    observed
        .recv()
        .expect("native worker completed before cleanup reply");
    assert!(owned.try_wait().expect("status").is_some());
}
#[cfg(unix)]
#[test]
fn reader_panic_is_reported_after_all_native_workers_finish_and_retry_is_safe() {
    let child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .group_spawn()
        .expect("fixture");
    let mut owned = Owned::new(child);
    owned.reader(std::thread::spawn(|| panic!("controlled worker failure")));
    let (done, observed) = std::sync::mpsc::channel();
    owned.reader(std::thread::spawn(move || {
        done.send(()).expect("completion");
    }));
    assert_eq!(owned.cleanup(), Err("native_reader_failed"));
    observed.recv().expect("all workers completed");
    owned
        .cleanup()
        .expect("idempotent retry after completed reader fault");
}

#[cfg(target_os = "linux")]
#[test]
fn cleanup_terminates_descendants_even_when_the_direct_child_already_exited() {
    let scratch = tempfile::tempdir().expect("fixture directory");
    let mark = scratch.path().join("descendant");
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", "sleep 600 & echo $! > \"$1\"; exit 0", "fixture"])
        .arg(&mark)
        .stdout(std::process::Stdio::piped());
    let child = command.group_spawn().expect("native group");
    let mut owned = Owned::new(child);
    let stdout = owned.child.inner().stdout.take().expect("pipe");
    owned.reader(std::thread::spawn(move || {
        super::super::reader::errors(stdout, || {}).expect("pipe reader");
    }));
    owned.child.inner().wait().expect("direct parent exited");
    let pid = std::fs::read_to_string(&mark)
        .expect("descendant pid")
        .trim()
        .parse::<u32>()
        .expect("pid");
    let (done, observed) = std::sync::mpsc::channel();
    let cleanup = std::thread::spawn(move || done.send(owned.cleanup()).expect("cleanup reply"));
    let completed = observed
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok();
    if !completed {
        // Ensure the failing regression never leaves its controlled descendant alive.
        std::process::Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .expect("fixture cleanup");
    }
    cleanup.join().expect("owner thread");
    assert!(
        completed,
        "already-exited parent must not bypass descendant termination and pipe join"
    );
}
