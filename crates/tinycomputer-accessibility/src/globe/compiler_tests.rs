//! Local compiler-command fixtures exercise cancellation and joined cleanup.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "controlled native fixtures assert completion and worker panic handling"
)]
use super::*;
#[cfg(unix)]
#[test]
fn bounded_compiler_command_drains_output_and_preserves_failure_classification() {
    let cancel = AtomicBool::new(false);
    assert!(
        run(
            std::process::Command::new("/bin/sh").args(["-c", "head -c 40000 /dev/zero >&2"]),
            &cancel
        )
        .is_ok()
    );
    assert_eq!(
        run(
            std::process::Command::new("/bin/sh").args(["-c", "exit 7"]),
            &cancel
        ),
        Err("native_compiler_failed")
    );
    assert_eq!(
        run(
            &mut std::process::Command::new("/nonexistent-compiler"),
            &cancel
        ),
        Err("native_compiler_missing")
    );
    stop().expect("no native compiler ownership remaining");
}
#[cfg(unix)]
#[test]
fn terminal_cancellation_prevents_compiler_side_effects() {
    let cancel = AtomicBool::new(true);
    assert_eq!(
        run(
            std::process::Command::new("/bin/sh").args(["-c", "exit 0"]),
            &cancel
        ),
        Err("native_start_canceled")
    );
    stop().expect("cleanup");
}

#[cfg(unix)]
#[test]
fn terminal_cancellation_during_native_compilation_kills_and_joins_owned_group() {
    let scratch = tempfile::tempdir().expect("fixture directory");
    let mark = scratch.path().join("pid");
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker_mark = mark.clone();
    let worker = std::thread::spawn(move || {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "echo $$ > \"$1\"; exec sleep 600", "fixture"])
            .arg(worker_mark);
        run(&mut command, &worker_cancel)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(pid) = std::fs::read_to_string(&mark)
            && let Ok(pid) = pid.trim().parse::<u32>()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "fixture did not start");
        std::thread::yield_now();
    };
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(
        worker.join().expect("compiler owner"),
        Err("native_start_canceled")
    );
    #[cfg(target_os = "linux")]
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "direct child was not reaped before cancellation reply"
    );
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
    stop().expect("idempotent joined cleanup");
}
