//! Cancellable compiler process groups, with retained cleanup ownership.
use super::{owned::Owned, reader};
use command_group::CommandGroup;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static COMPILER: LazyLock<Mutex<Option<Owned>>> = LazyLock::new(|| Mutex::new(None));

pub(super) fn stop() -> Result<(), String> {
    let mut compiler = COMPILER.lock().map_err(|_| "native compiler lock failed")?;
    if let Some(owned) = compiler.as_mut() {
        owned.cleanup().map_err(str::to_owned)?;
    }
    *compiler = None;
    Ok(())
}

pub(super) fn run(command: &mut Command, cancel: &AtomicBool) -> Result<(), &'static str> {
    let mut slot = COMPILER.lock().map_err(|_| "native_compiler_lock_failed")?;
    if let Some(owned) = slot.as_mut() {
        owned.cleanup()?;
    }
    *slot = None;
    if cancel.load(Ordering::SeqCst) {
        return Err("native_start_canceled");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .group_spawn()
        .map_err(|_| "native_compiler_missing")?;
    *slot = Some(Owned::new(child));
    let owned = slot.as_mut().ok_or("native_compiler_closed")?;
    for pipe in [
        owned
            .child
            .inner()
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
        owned
            .child
            .inner()
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
    ] {
        let pipe = pipe.ok_or("native_compiler_pipe_failed")?;
        let worker = std::thread::Builder::new()
            .spawn(move || {
                let _ = reader::errors(pipe, || {});
            })
            .map_err(|_| "native_compiler_reader_failed")?;
        owned.reader(worker);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        if cancel.load(Ordering::SeqCst) {
            break Err("native_start_canceled");
        }
        if Instant::now() >= deadline {
            break Err("native_compiler_timeout");
        }
        match owned.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err("native_compiler_failed")
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break Err("native_compiler_status_failed"),
        }
    };
    owned.cleanup()?;
    *slot = None;
    result
}

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;
