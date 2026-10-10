//! The bus identity of the tinycomputer module: interface name, object path,
//! and one constant per member.
//!
//! Nothing here is a string literal at a call site. A host names a member
//! through [`methods`] and the object through [`OBJECT_PATH`], so a rename is a
//! compile error in every consumer rather than a runtime "unknown method".
//!
//! [`METHODS`] lists every member in the order the interface dispatches them.
//! `crates/tinycomputer` asserts its dispatch table and its embedded manifest
//! against that list, so the three cannot drift.

/// The well-known interface name the module claims on the bus.
pub const INTERFACE: &str = "ai.tinyhumans.tinycomputer.Desktop";

/// The object path the module serves its interface at.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/tinycomputer/Desktop";

/// One constant per member of [`INTERFACE`].
///
/// Every member returns a [`crate::DesktopResponse`], including on failure;
/// see [`crate::envelope`] for why.
pub mod methods {
    pub use crate::agent::names::methods::{
        AWAIT_TASK, CANCEL_TASK, CONTINUE_TASK, DESCRIBE, LIST_TASKS, PLAN_TASK, START_TASK,
        TASK_REPORT,
    };

    /// Resolves one natural-language intent. Requires confidential delivery.
    /// Takes a [`crate::ResolveIntentRequest`].
    pub const RESOLVE_INTENT: &str = "ResolveIntent";
    /// Runs a bounded Jev desktop-control loop. Requires confidential delivery.
    /// Takes a [`crate::RunGoalRequest`].
    pub const RUN_GOAL: &str = "RunGoal";
    /// Runs a high-level intent flow with Jev decision loops. Requires
    /// confidential delivery. Takes a [`crate::RunFlowRequest`].
    pub const RUN_FLOW: &str = "RunFlow";
    /// Checks a flow without touching the desktop or Jev.
    /// Takes a [`crate::ValidateFlowRequest`].
    pub const VALIDATE_FLOW: &str = "ValidateFlow";
    /// Returns the flow authoring guide as prompt text. Takes no argument.
    pub const FLOW_GUIDE: &str = "FlowGuide";

    /// Walks an accessibility tree and allocates a ref per element.
    /// Takes a [`crate::SnapshotRequest`].
    pub const SNAPSHOT: &str = "Snapshot";
    /// Returns the elements matching a query.
    /// Takes a [`crate::FindRequest`].
    pub const FIND: &str = "Find";
    /// Reads one property of one ref.
    /// Takes a [`crate::GetRequest`].
    pub const GET: &str = "Get";
    /// Tests one boolean state of one ref.
    /// Takes a [`crate::IsRequest`].
    pub const IS: &str = "Is";
    /// Captures an application, window, or display as an image.
    /// Takes a [`crate::ScreenshotRequest`].
    pub const SCREENSHOT: &str = "Screenshot";

    /// Clicks a ref through its accessibility action.
    /// Takes a [`crate::RefRequest`].
    pub const CLICK: &str = "Click";
    /// Double-clicks a ref.
    /// Takes a [`crate::RefRequest`].
    pub const DOUBLE_CLICK: &str = "DoubleClick";
    /// Triple-clicks a ref.
    /// Takes a [`crate::RefRequest`].
    pub const TRIPLE_CLICK: &str = "TripleClick";
    /// Right-clicks a ref.
    /// Takes a [`crate::RefRequest`].
    pub const RIGHT_CLICK: &str = "RightClick";
    /// Types text into a ref.
    /// Takes a [`crate::TypeRequest`].
    pub const TYPE: &str = "Type";
    /// Replaces a ref's value in one step.
    /// Takes a [`crate::SetValueRequest`].
    pub const SET_VALUE: &str = "SetValue";
    /// Empties a ref's value.
    /// Takes a [`crate::RefRequest`].
    pub const CLEAR: &str = "Clear";
    /// Gives a ref keyboard focus.
    /// Takes a [`crate::RefRequest`].
    pub const FOCUS: &str = "Focus";
    /// Selects an option within a ref.
    /// Takes a [`crate::SelectRequest`].
    pub const SELECT: &str = "Select";
    /// Toggles a checkable ref.
    /// Takes a [`crate::RefRequest`].
    pub const TOGGLE: &str = "Toggle";
    /// Checks a checkable ref.
    /// Takes a [`crate::RefRequest`].
    pub const CHECK: &str = "Check";
    /// Unchecks a checkable ref.
    /// Takes a [`crate::RefRequest`].
    pub const UNCHECK: &str = "Uncheck";
    /// Expands an expandable ref.
    /// Takes a [`crate::RefRequest`].
    pub const EXPAND: &str = "Expand";
    /// Collapses an expandable ref.
    /// Takes a [`crate::RefRequest`].
    pub const COLLAPSE: &str = "Collapse";
    /// Scrolls a scrollable ref.
    /// Takes a [`crate::ScrollRequest`].
    pub const SCROLL: &str = "Scroll";
    /// Scrolls a ref into view.
    /// Takes a [`crate::RefRequest`].
    pub const SCROLL_TO: &str = "ScrollTo";

    /// Presses a key combination.
    /// Takes a [`crate::PressRequest`].
    pub const PRESS: &str = "Press";
    /// Reserved for a stateful daemon; fails closed.
    /// Takes a [`crate::HoldKeyRequest`].
    pub const KEY_DOWN: &str = "KeyDown";
    /// Reserved for a stateful daemon; fails closed.
    /// Takes a [`crate::HoldKeyRequest`].
    pub const KEY_UP: &str = "KeyUp";
    /// Moves the cursor over a ref or a point.
    /// Takes a [`crate::HoverRequest`].
    pub const HOVER: &str = "Hover";
    /// Drags between two endpoints.
    /// Takes a [`crate::DragRequest`].
    pub const DRAG: &str = "Drag";
    /// Moves the cursor to a point.
    /// Takes a [`crate::MouseMoveRequest`].
    pub const MOUSE_MOVE: &str = "MouseMove";
    /// Clicks at a point.
    /// Takes a [`crate::MouseClickRequest`].
    pub const MOUSE_CLICK: &str = "MouseClick";
    /// Reserved for a stateful daemon; fails closed.
    /// Takes a [`crate::HoldMouseRequest`].
    pub const MOUSE_DOWN: &str = "MouseDown";
    /// Reserved for a stateful daemon; fails closed.
    /// Takes a [`crate::HoldMouseRequest`].
    pub const MOUSE_UP: &str = "MouseUp";
    /// Scrolls the wheel at a point.
    /// Takes a [`crate::MouseWheelRequest`].
    pub const MOUSE_WHEEL: &str = "MouseWheel";

    /// Starts or attaches to an application.
    /// Takes a [`crate::LaunchRequest`].
    pub const LAUNCH: &str = "Launch";
    /// Quits or terminates an application.
    /// Takes a [`crate::CloseAppRequest`].
    pub const CLOSE_APP: &str = "CloseApp";
    /// Lists running applications.
    /// Takes a [`crate::ListAppsRequest`].
    pub const LIST_APPS: &str = "ListApps";
    /// Lists windows.
    /// Takes a [`crate::ListWindowsRequest`].
    pub const LIST_WINDOWS: &str = "ListWindows";
    /// Lists displays. Takes no argument.
    pub const LIST_DISPLAYS: &str = "ListDisplays";
    /// Lists the surfaces an application currently exposes.
    /// Takes a [`crate::ListSurfacesRequest`].
    pub const LIST_SURFACES: &str = "ListSurfaces";
    /// Brings a window forward.
    /// Takes a [`crate::FocusWindowRequest`].
    pub const FOCUS_WINDOW: &str = "FocusWindow";
    /// Resizes a window.
    /// Takes a [`crate::ResizeWindowRequest`].
    pub const RESIZE_WINDOW: &str = "ResizeWindow";
    /// Moves a window.
    /// Takes a [`crate::MoveWindowRequest`].
    pub const MOVE_WINDOW: &str = "MoveWindow";
    /// Minimizes a window.
    /// Takes a [`crate::WindowRequest`].
    pub const MINIMIZE: &str = "Minimize";
    /// Maximizes a window.
    /// Takes a [`crate::WindowRequest`].
    pub const MAXIMIZE: &str = "Maximize";
    /// Restores a minimized or maximized window.
    /// Takes a [`crate::WindowRequest`].
    pub const RESTORE: &str = "Restore";

    /// Reads the pasteboard.
    /// Takes a [`crate::ClipboardGetRequest`].
    pub const CLIPBOARD_GET: &str = "ClipboardGet";
    /// Writes the pasteboard.
    /// Takes a [`crate::ClipboardSetRequest`].
    pub const CLIPBOARD_SET: &str = "ClipboardSet";
    /// Empties the pasteboard. Takes no argument.
    pub const CLIPBOARD_CLEAR: &str = "ClipboardClear";

    /// Lists notification-centre entries.
    /// Takes a [`crate::ListNotificationsRequest`].
    pub const LIST_NOTIFICATIONS: &str = "ListNotifications";
    /// Invokes an action on a notification.
    /// Takes a [`crate::NotificationActionRequest`].
    pub const NOTIFICATION_ACTION: &str = "NotificationAction";
    /// Dismisses one notification.
    /// Takes a [`crate::DismissNotificationRequest`].
    pub const DISMISS_NOTIFICATION: &str = "DismissNotification";
    /// Dismisses every notification, optionally scoped to one application.
    /// Takes a [`crate::DismissAllNotificationsRequest`].
    pub const DISMISS_ALL_NOTIFICATIONS: &str = "DismissAllNotifications";

    /// Blocks until a condition holds or the timeout expires.
    /// Takes a [`crate::WaitRequest`].
    pub const WAIT: &str = "Wait";

    /// Reports the engine version and target. Takes no argument.
    pub const VERSION: &str = "Version";
    /// Reports permissions, the active session, and the latest snapshot.
    /// Takes no argument.
    pub const STATUS: &str = "Status";
    /// Reports, and optionally prompts for, the permissions automation needs.
    /// Takes a [`crate::PermissionsRequest`].
    pub const PERMISSIONS: &str = "Permissions";
}

/// Native accessibility and listener member names.
pub mod accessibility {
    /// Reads native accessibility, input-monitoring and microphone permission states.
    pub const ACCESSIBILITY_PERMISSIONS: &str = "AccessibilityPermissions";
    /// Requests one native permission and returns current states.
    pub const ACCESSIBILITY_REQUEST_PERMISSION: &str = "AccessibilityRequestPermission";
    /// Reads the focused text context inside the computer module.
    pub const ACCESSIBILITY_FOCUS: &str = "AccessibilityFocus";
    /// Validates the captured application, role and bounds.
    pub const ACCESSIBILITY_VALIDATE_TARGET: &str = "AccessibilityValidateTarget";
    /// Validates captured focus and inserts authorized text.
    pub const ACCESSIBILITY_PASTE: &str = "AccessibilityPaste";
    /// Starts the module-owned Globe listener and returns its lease.
    pub const GLOBE_START: &str = "GlobeStart";
    /// Drains a bounded ordered batch for a Globe listener lease.
    pub const GLOBE_POLL: &str = "GlobePoll";
    /// Stops and releases a Globe listener lease.
    pub const GLOBE_STOP: &str = "GlobeStop";
    /// Native members served by the accessibility bridge, in dispatch order.
    pub const METHODS: &[&str] = &[
        ACCESSIBILITY_PERMISSIONS,
        ACCESSIBILITY_REQUEST_PERMISSION,
        ACCESSIBILITY_FOCUS,
        ACCESSIBILITY_VALIDATE_TARGET,
        ACCESSIBILITY_PASTE,
        GLOBE_START,
        GLOBE_POLL,
        GLOBE_STOP,
    ];
}

/// Every member of [`INTERFACE`], in the order the interface dispatches them.
///
/// `crates/tinycomputer` asserts both its dispatch table and its declared
/// manifest methods against this list, so the three cannot drift. The browser
/// members come last, from [`crate::browser::names::methods`]; their names
/// carry a `Browser` prefix, so they are not repeated in [`methods`].
/// [`crate::catalogue::MEMBERS`] says what each one is for.
pub const METHODS: &[&str] = &[
    methods::RESOLVE_INTENT,
    methods::RUN_GOAL,
    methods::RUN_FLOW,
    methods::VALIDATE_FLOW,
    methods::FLOW_GUIDE,
    methods::DESCRIBE,
    methods::PLAN_TASK,
    methods::START_TASK,
    methods::AWAIT_TASK,
    methods::CONTINUE_TASK,
    methods::CANCEL_TASK,
    methods::TASK_REPORT,
    methods::LIST_TASKS,
    methods::SNAPSHOT,
    methods::FIND,
    methods::GET,
    methods::IS,
    methods::SCREENSHOT,
    methods::CLICK,
    methods::DOUBLE_CLICK,
    methods::TRIPLE_CLICK,
    methods::RIGHT_CLICK,
    methods::TYPE,
    methods::SET_VALUE,
    methods::CLEAR,
    methods::FOCUS,
    methods::SELECT,
    methods::TOGGLE,
    methods::CHECK,
    methods::UNCHECK,
    methods::EXPAND,
    methods::COLLAPSE,
    methods::SCROLL,
    methods::SCROLL_TO,
    methods::PRESS,
    methods::KEY_DOWN,
    methods::KEY_UP,
    methods::HOVER,
    methods::DRAG,
    methods::MOUSE_MOVE,
    methods::MOUSE_CLICK,
    methods::MOUSE_DOWN,
    methods::MOUSE_UP,
    methods::MOUSE_WHEEL,
    methods::LAUNCH,
    methods::CLOSE_APP,
    methods::LIST_APPS,
    methods::LIST_WINDOWS,
    methods::LIST_DISPLAYS,
    methods::LIST_SURFACES,
    methods::FOCUS_WINDOW,
    methods::RESIZE_WINDOW,
    methods::MOVE_WINDOW,
    methods::MINIMIZE,
    methods::MAXIMIZE,
    methods::RESTORE,
    methods::CLIPBOARD_GET,
    methods::CLIPBOARD_SET,
    methods::CLIPBOARD_CLEAR,
    methods::LIST_NOTIFICATIONS,
    methods::NOTIFICATION_ACTION,
    methods::DISMISS_NOTIFICATION,
    methods::DISMISS_ALL_NOTIFICATIONS,
    methods::WAIT,
    methods::VERSION,
    methods::STATUS,
    methods::PERMISSIONS,
    accessibility::ACCESSIBILITY_PERMISSIONS,
    accessibility::ACCESSIBILITY_REQUEST_PERMISSION,
    accessibility::ACCESSIBILITY_FOCUS,
    accessibility::ACCESSIBILITY_VALIDATE_TARGET,
    accessibility::ACCESSIBILITY_PASTE,
    accessibility::GLOBE_START,
    accessibility::GLOBE_POLL,
    accessibility::GLOBE_STOP,
    crate::browser::names::methods::OPEN_SESSION,
    crate::browser::names::methods::CLOSE_SESSION,
    crate::browser::names::methods::LIST_SESSIONS,
    crate::browser::names::methods::NAVIGATE,
    crate::browser::names::methods::SNAPSHOT,
    crate::browser::names::methods::PERFORM,
    crate::browser::names::methods::READ_PAGE,
    crate::browser::names::methods::EVALUATE,
    crate::browser::names::methods::SCREENSHOT,
    crate::browser::names::methods::READ_OUTPUT,
    crate::browser::names::methods::RELEASE_OUTPUT,
    crate::browser::names::methods::LIST_DOWNLOADS,
    crate::browser::names::methods::WAIT_DOWNLOAD,
];

#[cfg(test)]
mod names_tests;
