//! The members that read the screen without touching it.

use agent_desktop_core::commands::{find, get, is_check, screenshot, snapshot};
use tinycomputer_bus::{
    DesktopResponse, FindRequest, GetRequest, IsRequest, ScreenshotRequest, SnapshotRequest,
};

use super::{Desktop, Need, convert};

/// The engine's own default tree depth, applied when a request leaves
/// `max_depth` absent. Spelled here rather than defaulted in the contract so
/// "unspecified" stays distinguishable from "ten" on the wire.
const DEFAULT_MAX_DEPTH: u8 = 10;

impl Desktop {
    /// Walks an application's accessibility tree, allocating a ref per element.
    ///
    /// The reply carries a snapshot id and the tree. Every ref in it is
    /// qualified with that snapshot, so a later call can name one without
    /// repeating the id.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, SnapshotRequest};
    /// let reply = Desktop::new().snapshot(SnapshotRequest {
    ///     app: Some("Safari".to_owned()),
    ///     skeleton: true,
    ///     ..SnapshotRequest::default()
    /// });
    ///
    /// // Succeeds where the platform has an accessibility backend and the
    /// // permission is granted; a structured error otherwise.
    /// assert_eq!(reply.command, "snapshot");
    /// ```
    #[must_use]
    pub fn snapshot(&self, request: SnapshotRequest) -> DesktopResponse {
        self.run("snapshot", Need::Accessibility, |adapter, context| {
            snapshot::execute(
                snapshot::SnapshotArgs {
                    app: request.app,
                    window_id: request.window_id,
                    max_depth: request.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
                    include_bounds: request.include_bounds,
                    interactive_only: request.interactive_only,
                    compact: request.compact,
                    surface: convert::surface(request.surface),
                    skeleton: request.skeleton,
                    root_ref: request.root_ref.map(|ref_id| {
                        convert::qualified_ref(ref_id, request.snapshot_id.as_deref())
                    }),
                    timeout_ms: None,
                    force_electron_a11y: false,
                },
                adapter,
                context,
            )
        })
    }

    /// Returns the elements matching a query.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, FindRequest};
    /// let reply = Desktop::new().find(FindRequest {
    ///     role: Some("button".to_owned()),
    ///     name: Some("Save".to_owned()),
    ///     first: true,
    ///     ..FindRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "find");
    /// ```
    #[must_use]
    pub fn find(&self, request: FindRequest) -> DesktopResponse {
        self.run("find", Need::Accessibility, |adapter, context| {
            find::execute(
                find::FindArgs {
                    app: request.app,
                    window_id: request.window_id,
                    root: request
                        .root
                        .map(|ref_id| convert::qualified_ref(ref_id, request.snapshot.as_deref())),
                    surface: convert::surface(request.surface),
                    filter: find::FindFilterArgs {
                        role: request.role,
                        name: request.name,
                        description: request.description,
                        native_id: request.native_id,
                        value: request.value,
                        text: request.text,
                        exact: request.exact,
                    },
                    states: request
                        .states
                        .into_iter()
                        .map(convert::state_predicate)
                        .collect(),
                    selection: find::FindSelectionArgs {
                        count: request.count,
                        first: request.first,
                        last: request.last,
                        nth: request.nth,
                        limit: request.limit,
                    },
                    timeout_ms: None,
                },
                adapter,
                context,
            )
        })
    }

    /// Reads one property of one ref.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ElementProperty, GetRequest};
    /// let reply = Desktop::new().get(GetRequest::new("@s1:e2", ElementProperty::Value));
    /// assert_eq!(reply.command, "get");
    /// ```
    #[must_use]
    pub fn get(&self, request: GetRequest) -> DesktopResponse {
        self.run("get", Need::Accessibility, |adapter, context| {
            get::execute(
                get::GetArgs {
                    ref_id: convert::qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
                    property: convert::element_property(request.property),
                },
                adapter,
                context,
            )
        })
    }

    /// Tests one boolean state of one ref.
    ///
    /// A state that does not apply to the element's role is reported as
    /// inapplicable rather than as `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ElementStateProperty, IsRequest};
    /// let reply = Desktop::new().is(IsRequest::new("@s1:e2", ElementStateProperty::Enabled));
    /// assert_eq!(reply.command, "is");
    /// ```
    #[must_use]
    pub fn is(&self, request: IsRequest) -> DesktopResponse {
        self.run("is", Need::Accessibility, |adapter, context| {
            is_check::execute(
                is_check::IsArgs {
                    ref_id: convert::qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
                    property: convert::state_property(request.property),
                },
                adapter,
                context,
            )
        })
    }

    /// Captures an application, a window, or a display as an image.
    ///
    /// Capturing a whole display needs only screen recording; capturing a named
    /// application or window also needs accessibility access, because the target
    /// has to be resolved through the tree before it can be framed.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ScreenshotRequest};
    /// let reply = Desktop::new().screenshot(ScreenshotRequest::default());
    /// assert_eq!(reply.command, "screenshot");
    /// ```
    #[must_use]
    pub fn screenshot(&self, request: ScreenshotRequest) -> DesktopResponse {
        let need = if request.app.is_some() || request.window_id.is_some() {
            Need::AccessibilityAndScreenRecording
        } else {
            Need::ScreenRecording
        };

        self.run("screenshot", need, |adapter, _context| {
            screenshot::execute(
                screenshot::ScreenshotArgs {
                    app: request.app,
                    window_id: request.window_id,
                    screen: request.screen,
                    output_path: convert::path(request.output_path),
                },
                adapter,
            )
        })
    }
}
