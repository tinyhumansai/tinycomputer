//! [`Desktop`] as a [`Surface`]: agent-desktop snapshots parsed into what a
//! decision loop sees, and the closed operations it acts with.
//!
//! Observation keeps static text as context, records subtrees the engine cut
//! short, keeps field content out of that shared context, and resolves an
//! application with several windows to its front one. Paste goes through the
//! clipboard and puts back what it held.

use serde_json::Value;
use tinycomputer_bus::{
    ClipboardGetRequest, ClipboardSetRequest, DesktopResponse, ElementProperty, GetRequest,
    JevOperation, LaunchRequest, PressRequest, RefRequest, WaitRequest,
};
use tinycomputer_core::surface::{Candidate, Depth, Screen, Surface};

mod act;
mod observation;
mod paste;

use act::{execute_desktop, running_is_launched};
use observation::observe;
use paste::{PasteSteps, paste_through};

use crate::Desktop;

/// How long a field is given to commit text before it is read back again.
const SETTLE_MS: u64 = 200;

impl Surface for Desktop {
    fn observe(
        &self,
        app: &str,
        root: Option<&str>,
        depth: Depth,
    ) -> Result<Screen, Box<DesktopResponse>> {
        observe(self, app, root, depth)
    }

    fn execute(
        &self,
        operation: JevOperation,
        target: Option<Candidate>,
        text: Option<String>,
    ) -> DesktopResponse {
        execute_desktop(self, operation, target.as_ref(), text)
    }

    fn read_value(&self, target: &Candidate) -> Option<String> {
        // A plain field answers with its value; a rich-text area often has
        // none and answers with its text instead.
        [ElementProperty::Value, ElementProperty::Text]
            .into_iter()
            .find_map(|property| {
                let reply = self.get(GetRequest::new(target.ref_id.clone(), property));
                reply
                    .data
                    .as_ref()
                    .filter(|_| reply.ok)
                    .and_then(|data| data.get("value").and_then(Value::as_str))
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
            })
    }

    fn paste(&self, app: &str, target: &Candidate, text: &str) -> DesktopResponse {
        paste_through(self, app, target, text)
    }

    fn press(&self, app: &str, combo: &str) -> DesktopResponse {
        Desktop::press(self, act::press_at(app, combo))
    }

    fn launch(&self, app: &str) -> DesktopResponse {
        let mut request = LaunchRequest::new(app);
        request.activate = true;
        running_is_launched(Desktop::launch(self, request))
    }

    fn settle(&self) {
        let _settled = self.wait(WaitRequest::sleep(SETTLE_MS));
    }
}

/// The paste's steps, each answered by the engine.
impl PasteSteps for Desktop {
    fn clipboard_get(&self, request: ClipboardGetRequest) -> DesktopResponse {
        Desktop::clipboard_get(self, request)
    }
    fn clipboard_set(&self, request: ClipboardSetRequest) -> DesktopResponse {
        Desktop::clipboard_set(self, request)
    }
    fn clipboard_clear(&self) -> DesktopResponse {
        Desktop::clipboard_clear(self)
    }
    fn focus(&self, request: RefRequest) -> DesktopResponse {
        Desktop::focus(self, request)
    }
    fn click(&self, request: RefRequest) -> DesktopResponse {
        Desktop::click(self, request)
    }
    fn press(&self, request: PressRequest) -> DesktopResponse {
        Desktop::press(self, request)
    }
    fn wait(&self, request: WaitRequest) -> DesktopResponse {
        Desktop::wait(self, request)
    }
}

#[cfg(test)]
mod surface_tests;
