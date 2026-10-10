//! Loads a built module through the real `TinyBus` dynamic loader.
//!
//! The unit tests serve the interface in-process; this proves the compiled
//! `cdylib` exports the ABI, announces its manifest, claims its name, and
//! answers a call. A release archive is not accepted until this passes against
//! the artifact that would ship.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinycomputer_bus::{DesktopResponse, METHODS, names};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module = module_argument()?;
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let module_host = ModuleHost::new(broker);
    let info = module_host.load_file(&module)?;

    if info.name != "tinycomputer" {
        return Err(io::Error::other(format!(
            "loaded module `{}` instead of `{}`",
            info.name, "tinycomputer"
        ))
        .into());
    }

    let client = Connection::connect(bus.connect().await?).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let claimed = client.list_names().await?;
            if claimed.iter().any(|name| name.as_str() == names::INTERFACE) {
                return tinybus::Result::Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;

    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    // `Version` needs no permission and touches no other application, so it
    // proves the module answers without making the check depend on how the
    // build machine is configured.
    let reply: DesktopResponse = proxy.call(names::methods::VERSION, ()).await?;
    if !reply.ok {
        return Err(io::Error::other(format!(
            "module reported a failure for `{}`: {:?}",
            names::methods::VERSION,
            reply.error
        ))
        .into());
    }

    let declared: Vec<&str> = info
        .manifest
        .provides
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .map(tinybus::MemberName::as_str)
        .collect();
    if declared != METHODS {
        return Err(io::Error::other("manifest members differ from contract").into());
    }
    // Invalid handles exercise the new artifact paths without touching devices.
    for member in [
        names::accessibility::GLOBE_POLL,
        names::accessibility::GLOBE_STOP,
    ] {
        let reply: DesktopResponse = proxy
            .call(
                member,
                (tinycomputer_bus::accessibility::GlobeHandle(
                    "fixture-invalid".into(),
                ),),
            )
            .await?;
        if reply.ok
            || reply
                .error
                .as_ref()
                .is_none_or(|error| error.code != "UNKNOWN_LISTENER")
        {
            return Err(io::Error::other("module accepted an unknown listener lease").into());
        }
    }

    // Wrong-arity inputs stay safe even if a confidentiality guard regresses.
    for member in [
        names::accessibility::ACCESSIBILITY_FOCUS,
        names::accessibility::ACCESSIBILITY_VALIDATE_TARGET,
        names::accessibility::ACCESSIBILITY_PASTE,
    ] {
        let reply = proxy
            .call::<DesktopResponse>(member, serde_json::json!([]))
            .await;
        if !matches!(reply, Err(tinybus::Error::MethodFailed { name, .. }) if name == tinybus::Error::CONFIDENTIALITY_REQUIRED)
        {
            return Err(
                io::Error::other("native member failed to require confidential delivery").into(),
            );
        }
    }

    println!(
        "verified {} as TinyBus module `{}`, serving {} members",
        module.display(),
        info.name,
        METHODS.len()
    );
    broker_task.abort();
    Ok(())
}

/// The path to the module under test, from the first argument.
fn module_argument() -> Result<PathBuf, io::Error> {
    std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("usage: verify_module <path-to-module>"))
}
