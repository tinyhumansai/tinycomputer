//! The members that manage applications, windows, and displays.

use agent_desktop_core::{
    commands::{
        close_app, focus_window, launch, list_apps, list_displays, list_surfaces, list_windows,
        maximize, minimize, move_window, resize_window, restore,
    },
    launch_options::LaunchOptions,
};
use tinycomputer_bus::{
    CloseAppRequest, DesktopResponse, FocusWindowRequest, LaunchRequest, ListAppsRequest,
    ListSurfacesRequest, ListWindowsRequest, MoveWindowRequest, ResizeWindowRequest, WindowRequest,
};

use super::{Desktop, Need, convert};

/// Declares a member that operates on a window as a whole.
macro_rules! window_op {
    ($(#[$meta:meta])* $name:ident => $command:literal, $execute:path) => {
        $(#[$meta])*
        #[must_use]
        pub fn $name(&self, request: WindowRequest) -> DesktopResponse {
            self.run($command, Need::Accessibility, |adapter, _context| {
                $execute(convert::window_args(request), adapter)
            })
        }
    };
}

impl Desktop {
    /// Starts an application, or attaches to it if it is already running.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, LaunchRequest};
    /// let reply = Desktop::new().launch(LaunchRequest::new("Safari"));
    /// assert_eq!(reply.command, "launch");
    /// ```
    #[must_use]
    pub fn launch(&self, request: LaunchRequest) -> DesktopResponse {
        self.run("launch", Need::Accessibility, |adapter, _context| {
            let defaults = LaunchOptions::default();
            launch::execute(
                launch::LaunchArgs {
                    app: request.app,
                    options: LaunchOptions {
                        args: request.args,
                        env: request.env,
                        cwd: convert::path(request.cwd),
                        timeout_ms: request.timeout_ms.unwrap_or(defaults.timeout_ms),
                        attach_if_running: request
                            .attach_if_running
                            .unwrap_or(defaults.attach_if_running),
                        activate: request.activate,
                        cdp_port: request.cdp_port,
                    },
                },
                adapter,
            )
        })
    }

    /// Quits or terminates an application.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{CloseAppRequest, Desktop};
    /// let reply = Desktop::new().close_app(CloseAppRequest {
    ///     app: "Calculator".to_owned(),
    ///     force: false,
    /// });
    ///
    /// assert_eq!(reply.command, "close-app");
    /// ```
    #[must_use]
    pub fn close_app(&self, request: CloseAppRequest) -> DesktopResponse {
        self.run("close-app", Need::Accessibility, |adapter, _context| {
            close_app::execute(
                close_app::CloseAppArgs {
                    app: request.app,
                    force: request.force,
                },
                adapter,
            )
        })
    }

    /// Lists running applications.
    ///
    /// Needs no permission: this is the window server's own list, which every
    /// process may read. It is the right first call from an unconfigured
    /// machine, because it works before accessibility access is granted.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ListAppsRequest};
    /// let reply = Desktop::new().list_apps(ListAppsRequest::default());
    /// assert_eq!(reply.command, "list-apps");
    /// ```
    #[must_use]
    pub fn list_apps(&self, request: ListAppsRequest) -> DesktopResponse {
        self.run("list-apps", Need::Nothing, |adapter, _context| {
            list_apps::execute(
                list_apps::ListAppsArgs {
                    app: request.app,
                    timeout_ms: None,
                },
                adapter,
            )
        })
    }

    /// Lists windows, optionally narrowed to one application.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ListWindowsRequest};
    /// let reply = Desktop::new().list_windows(ListWindowsRequest::default());
    /// assert_eq!(reply.command, "list-windows");
    /// ```
    #[must_use]
    pub fn list_windows(&self, request: ListWindowsRequest) -> DesktopResponse {
        self.run("list-windows", Need::Nothing, |adapter, _context| {
            list_windows::execute(
                list_windows::ListWindowsArgs {
                    app: request.app,
                    timeout_ms: None,
                },
                adapter,
            )
        })
    }

    /// Lists displays, with their geometry and scale factors.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::Desktop;
    /// let reply = Desktop::new().list_displays();
    /// assert_eq!(reply.command, "list-displays");
    /// ```
    #[must_use]
    pub fn list_displays(&self) -> DesktopResponse {
        self.run("list-displays", Need::Nothing, |adapter, _context| {
            list_displays::execute(adapter)
        })
    }

    /// Lists the surfaces an application currently exposes.
    ///
    /// This is how a caller finds out whether a menu or a popover is open and
    /// reachable, before asking to snapshot one.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ListSurfacesRequest};
    /// let reply = Desktop::new().list_surfaces(ListSurfacesRequest::default());
    /// assert_eq!(reply.command, "list-surfaces");
    /// ```
    #[must_use]
    pub fn list_surfaces(&self, request: ListSurfacesRequest) -> DesktopResponse {
        self.run("list-surfaces", Need::Accessibility, |adapter, _context| {
            list_surfaces::execute(
                list_surfaces::ListSurfacesArgs { app: request.app },
                adapter,
            )
        })
    }

    /// Brings a window forward.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, FocusWindowRequest};
    /// let reply = Desktop::new().focus_window(FocusWindowRequest {
    ///     app: Some("Safari".to_owned()),
    ///     ..FocusWindowRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "focus-window");
    /// ```
    #[must_use]
    pub fn focus_window(&self, request: FocusWindowRequest) -> DesktopResponse {
        self.run("focus-window", Need::Accessibility, |adapter, _context| {
            focus_window::execute(
                focus_window::FocusWindowArgs {
                    window_id: request.window_id,
                    app: request.app,
                    title: request.title,
                },
                adapter,
            )
        })
    }

    /// Resizes a window.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, ResizeWindowRequest};
    /// let reply = Desktop::new().resize_window(ResizeWindowRequest {
    ///     width: 1_280.0,
    ///     height: 720.0,
    ///     ..ResizeWindowRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "resize-window");
    /// ```
    #[must_use]
    pub fn resize_window(&self, request: ResizeWindowRequest) -> DesktopResponse {
        self.run("resize-window", Need::Accessibility, |adapter, _context| {
            resize_window::execute(
                resize_window::ResizeWindowArgs {
                    app: request.app,
                    window_id: request.window_id,
                    width: request.width,
                    height: request.height,
                },
                adapter,
            )
        })
    }

    /// Moves a window's origin.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, MoveWindowRequest};
    /// let reply = Desktop::new().move_window(MoveWindowRequest {
    ///     x: 0.0,
    ///     y: 0.0,
    ///     ..MoveWindowRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "move-window");
    /// ```
    #[must_use]
    pub fn move_window(&self, request: MoveWindowRequest) -> DesktopResponse {
        self.run("move-window", Need::Accessibility, |adapter, _context| {
            move_window::execute(
                move_window::MoveWindowArgs {
                    app: request.app,
                    window_id: request.window_id,
                    x: request.x,
                    y: request.y,
                },
                adapter,
            )
        })
    }

    window_op! {
        /// Minimizes a window.
        ///
        /// # Examples
        ///
        /// ```
        /// # use tinycomputer_desktop::{Desktop, WindowRequest};
        /// let reply = Desktop::new().minimize(WindowRequest::default());
        /// assert_eq!(reply.command, "minimize");
        /// ```
        minimize => "minimize", minimize::execute
    }

    window_op! {
        /// Maximizes a window.
        maximize => "maximize", maximize::execute
    }

    window_op! {
        /// Restores a minimized or maximized window to its previous geometry.
        restore => "restore", restore::execute
    }
}
