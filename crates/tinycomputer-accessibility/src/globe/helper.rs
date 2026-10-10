//! Preserved private Swift helper build/source, with cancellable owned compiler work.
use super::LOG_PREFIX;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;

pub(super) fn ensure_globe_helper_binary(
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<PathBuf, String> {
    let cache_dir = super::super::helper::private_cache_dir("openhuman-globe-listener")?;

    let source = globe_swift_source();
    let mut source_hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut source_hasher);
    let source_id = format!("{:016x}", source_hasher.finish());
    let source_path = cache_dir.join(format!("globe_listener_{source_id}.swift"));
    let binary_path = cache_dir.join(format!("globe_listener_{source_id}"));

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, &source)
            .map_err(|e| format!("failed to write globe helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        let temporary_binary = cache_dir.join(format!(
            "globe_listener_{source_id}.tmp-{}",
            std::process::id()
        ));
        log::debug!("{LOG_PREFIX} compiling Swift helper");
        let compiler = |executable: &str, xcrun: bool| {
            let mut command = Command::new(executable);
            if xcrun {
                command.arg("swiftc");
            }
            command
                .args(["-O", "-framework", "Cocoa"])
                .arg(&source_path)
                .arg("-o")
                .arg(&temporary_binary);
            super::compiler::run(&mut command, cancel)
        };
        let result = compiler("xcrun", true).or_else(|reason| {
            if reason == "native_compiler_missing" {
                compiler("swiftc", false)
            } else {
                Err(reason)
            }
        });
        if result.is_err() {
            let _ = fs::remove_file(&temporary_binary);
            return Err(result.err().unwrap_or("native_compiler_failed").into());
        }
        fs::rename(&temporary_binary, &binary_path)
            .map_err(|e| format!("failed to install compiled globe listener helper: {e}"))?;
        log::debug!("{LOG_PREFIX} Swift helper compiled successfully");
    }

    Ok(binary_path)
}

fn globe_swift_source() -> String {
    r#"import Cocoa
import Darwin

var fnIsDown = false
func emit(_ message: String) {
    FileHandle.standardOutput.write((message + "\n").data(using: .utf8)!)
    fflush(stdout)
}

guard let monitor = NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged, handler: { event in
    let flags = event.modifierFlags
    let containsFn = flags.contains(.function)

    if containsFn && !fnIsDown {
        fnIsDown = true
        emit("FN_DOWN")
    } else if !containsFn && fnIsDown {
        fnIsDown = false
        emit("FN_UP")
    }

}) else {
    FileHandle.standardError.write("Failed to create event monitor\n".data(using: .utf8)!)
    exit(1)
}

let signalSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
signal(SIGTERM, SIG_IGN)
signalSource.setEventHandler {
    NSEvent.removeMonitor(monitor)
    exit(0)
}
signalSource.resume()

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
app.run()
"#
    .to_string()
}
