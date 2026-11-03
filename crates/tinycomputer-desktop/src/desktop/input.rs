//! The members that synthesize keyboard and mouse input.

use agent_desktop_core::commands::{
    drag, hover, key_down, key_up, mouse_click, mouse_down, mouse_move, mouse_up, mouse_wheel,
    press,
};
use tinycomputer_bus::{
    DesktopResponse, DragRequest, HoldKeyRequest, HoldMouseRequest, HoverRequest,
    MouseClickRequest, MouseMoveRequest, MouseWheelRequest, PressRequest,
};

use super::{Desktop, Need, convert};

impl Desktop {
    /// Presses a key combination.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, PressRequest};
    /// let reply = Desktop::new().press(PressRequest::new("cmd+shift+p"));
    /// assert_eq!(reply.command, "press");
    /// ```
    #[must_use]
    pub fn press(&self, request: PressRequest) -> DesktopResponse {
        self.run("press", Need::Accessibility, |adapter, context| {
            press::execute(
                press::PressArgs {
                    combo: request.combo,
                    app: request.app,
                    window_id: None,
                    force: request.force,
                },
                adapter,
                context,
            )
        })
    }

    /// Holds a key down. Always fails.
    ///
    /// Validates the combination and then reports that a stateless module
    /// cannot hold a key across calls, naming [`Desktop::press`] instead. See
    /// the [`tinycomputer_bus::input`] documentation for why this is served
    /// rather than omitted.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, HoldKeyRequest};
    /// let reply = Desktop::new().key_down(HoldKeyRequest {
    ///     combo: "shift".to_owned(),
    ///     force: false,
    /// });
    ///
    /// assert!(!reply.ok);
    /// ```
    #[must_use]
    pub fn key_down(&self, request: HoldKeyRequest) -> DesktopResponse {
        self.run("key-down", Need::Accessibility, |adapter, _context| {
            key_down::execute(
                key_down::KeyDownArgs {
                    combo: request.combo,
                    force: request.force,
                },
                adapter,
            )
        })
    }

    /// Releases a held key. Always fails, for the same reason
    /// [`Desktop::key_down`] does.
    #[must_use]
    pub fn key_up(&self, request: HoldKeyRequest) -> DesktopResponse {
        self.run("key-up", Need::Accessibility, |adapter, _context| {
            key_up::execute(
                key_up::KeyUpArgs {
                    combo: request.combo,
                    force: request.force,
                },
                adapter,
            )
        })
    }

    /// Moves the cursor over a ref or a point.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, HoverRequest};
    /// let reply = Desktop::new().hover(HoverRequest {
    ///     ref_id: Some("@s1:e2".to_owned()),
    ///     ..HoverRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "hover");
    /// ```
    #[must_use]
    pub fn hover(&self, request: HoverRequest) -> DesktopResponse {
        self.run("hover", Need::Accessibility, |adapter, context| {
            hover::execute(
                hover::HoverArgs {
                    ref_id: request.ref_id.map(|ref_id| {
                        convert::qualified_ref(ref_id, request.snapshot_id.as_deref())
                    }),
                    xy: convert::point(request.x, request.y),
                    duration_ms: request.duration_ms,
                    timeout_ms: request.timeout_ms,
                },
                adapter,
                context,
            )
        })
    }

    /// Drags from one endpoint to another.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, DragEndpoint, DragRequest};
    /// let reply = Desktop::new().drag(DragRequest {
    ///     from: DragEndpoint::at_ref("@s1:e2"),
    ///     to: DragEndpoint::at_point(400.0, 300.0),
    ///     ..DragRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "drag");
    /// ```
    #[must_use]
    pub fn drag(&self, request: DragRequest) -> DesktopResponse {
        self.run("drag", Need::Accessibility, |adapter, context| {
            drag::execute(
                drag::DragArgs {
                    from: convert::drag_endpoint(request.from, request.snapshot_id.as_deref()),
                    to: convert::drag_endpoint(request.to, request.snapshot_id.as_deref()),
                    duration_ms: request.duration_ms,
                    drop_delay_ms: request.drop_delay_ms,
                    timeout_ms: request.timeout_ms,
                    wait_for_scope: drag::WaitForScope::default(),
                },
                adapter,
                context,
            )
        })
    }

    /// Moves the cursor to a screen point.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, MouseMoveRequest};
    /// let reply = Desktop::new().mouse_move(MouseMoveRequest { x: 10.0, y: 20.0 });
    /// assert_eq!(reply.command, "mouse-move");
    /// ```
    #[must_use]
    pub fn mouse_move(&self, request: MouseMoveRequest) -> DesktopResponse {
        self.run("mouse-move", Need::Accessibility, |adapter, context| {
            mouse_move::execute(
                mouse_move::MouseMoveArgs {
                    x: request.x,
                    y: request.y,
                },
                adapter,
                context,
            )
        })
    }

    /// Clicks at a screen point.
    ///
    /// The last resort for an element the accessibility tree cannot describe —
    /// a canvas, a custom-drawn control. It clicks whatever is at those
    /// coordinates, which is a different guarantee from clicking a named
    /// element.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, MouseClickRequest};
    /// let reply = Desktop::new().mouse_click(MouseClickRequest {
    ///     x: 10.0,
    ///     y: 20.0,
    ///     count: 1,
    ///     ..MouseClickRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "mouse-click");
    /// ```
    #[must_use]
    pub fn mouse_click(&self, request: MouseClickRequest) -> DesktopResponse {
        self.run("mouse-click", Need::Accessibility, |adapter, context| {
            mouse_click::execute(
                mouse_click::MouseClickArgs {
                    x: request.x,
                    y: request.y,
                    button: convert::mouse_button(request.button),
                    count: request.count,
                    modifiers: convert::modifiers(request.modifiers),
                },
                adapter,
                context,
            )
        })
    }

    /// Presses a mouse button and holds it. Always fails.
    ///
    /// Nothing keeps the button held once the call returns, so a success here
    /// would be a lie the caller only discovers through a broken drag. Use
    /// [`Desktop::drag`], which owns both ends of the gesture.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, HoldMouseRequest};
    /// let reply = Desktop::new().mouse_down(HoldMouseRequest::default());
    /// assert!(!reply.ok);
    /// ```
    #[must_use]
    pub fn mouse_down(&self, request: HoldMouseRequest) -> DesktopResponse {
        self.run("mouse-down", Need::Accessibility, |adapter, context| {
            mouse_down::execute(
                mouse_down::MouseDownArgs {
                    x: request.x,
                    y: request.y,
                    button: convert::mouse_button(request.button),
                    modifiers: convert::modifiers(request.modifiers),
                },
                adapter,
                context,
            )
        })
    }

    /// Releases a held mouse button. Always fails, for the same reason
    /// [`Desktop::mouse_down`] does.
    #[must_use]
    pub fn mouse_up(&self, request: HoldMouseRequest) -> DesktopResponse {
        self.run("mouse-up", Need::Accessibility, |adapter, context| {
            mouse_up::execute(
                mouse_up::MouseUpArgs {
                    x: request.x,
                    y: request.y,
                    button: convert::mouse_button(request.button),
                    modifiers: convert::modifiers(request.modifiers),
                },
                adapter,
                context,
            )
        })
    }

    /// Scrolls the wheel at a screen point.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, MouseWheelRequest};
    /// let reply = Desktop::new().mouse_wheel(MouseWheelRequest {
    ///     x: 10.0,
    ///     y: 20.0,
    ///     dy: -3.0,
    ///     ..MouseWheelRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "mouse-wheel");
    /// ```
    #[must_use]
    pub fn mouse_wheel(&self, request: MouseWheelRequest) -> DesktopResponse {
        self.run("mouse-wheel", Need::Accessibility, |adapter, context| {
            mouse_wheel::execute(
                mouse_wheel::MouseWheelArgs {
                    x: request.x,
                    y: request.y,
                    dy: request.dy,
                    dx: request.dx,
                    modifiers: convert::modifiers(request.modifiers),
                },
                adapter,
                context,
            )
        })
    }
}
