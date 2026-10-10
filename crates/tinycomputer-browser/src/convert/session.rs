//! Starting a session: the explicit launch and the viewport.
//!
//! The allowed origins are not handed to the engine: agent-browser would
//! refuse every request outside them, a page's own files and APIs included,
//! and refuse a profile beside them. The session checks pages itself
//! (`crate::origins`).

use serde_json::{Value, json};
use tinycomputer_bus::browser::SessionOptions;

/// The explicit `launch` every session starts with.
///
/// Sending it explicitly matters: without one, agent-browser auto-launches
/// from the `AGENT_BROWSER_*` environment of whatever process hosts the
/// module, which is not this session's configuration.
#[must_use]
pub(crate) fn launch(options: &SessionOptions) -> Value {
    let mut command = json!({
        "action": "launch",
        "headless": options.headless,
        "args": options.args,
    });
    let fields = [
        ("cdpUrl", options.endpoint.as_ref()),
        ("executablePath", options.executable.as_ref()),
        ("userAgent", options.user_agent.as_ref()),
        ("profile", options.user_data_dir.as_ref()),
        ("downloadPath", options.download_dir.as_ref()),
    ];
    for (key, value) in fields {
        if let Some(value) = value {
            command[key] = json!(value);
        }
    }
    command
}

/// The viewport a session applies straight after launching.
#[must_use]
pub(crate) fn viewport(options: &SessionOptions) -> Value {
    json!({
        "action": "viewport",
        "width": options.viewport.width,
        "height": options.viewport.height,
        "deviceScaleFactor": options.viewport.device_scale_factor,
        "mobile": options.viewport.mobile,
    })
}
