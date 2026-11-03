//! The members that act on a ref through its accessibility action.

use agent_desktop_core::commands::{
    check, clear, click, collapse, double_click, expand, focus, right_click, scroll, scroll_to,
    select, set_value, toggle, triple_click, type_text, uncheck,
};
use tinycomputer_bus::{
    DesktopResponse, RefRequest, ScrollRequest, SelectRequest, SetValueRequest, TypeRequest,
};

use super::{Desktop, Need, convert};

/// Declares a member that needs nothing but a ref.
///
/// A dozen members differ only in the engine function they call and the name
/// they report, and writing each one out would be a dozen chances to paste the
/// wrong name into the envelope — the kind of mistake that produces a correct
/// action attributed to the wrong command in a trace.
macro_rules! ref_action {
    ($(#[$meta:meta])* $name:ident => $command:literal, $execute:path) => {
        $(#[$meta])*
        #[must_use]
        pub fn $name(&self, request: RefRequest) -> DesktopResponse {
            self.run($command, Need::Accessibility, |adapter, context| {
                $execute(convert::ref_args(request), adapter, context)
            })
        }
    };
}

impl Desktop {
    ref_action! {
        /// Clicks a ref.
        ///
        /// # Examples
        ///
        /// ```
        /// # use tinycomputer_desktop::{Desktop, RefRequest};
        /// let reply = Desktop::new().click(RefRequest::new("@s1:e2"));
        /// assert_eq!(reply.command, "click");
        /// ```
        click => "click", click::execute
    }

    ref_action! {
        /// Double-clicks a ref.
        double_click => "double-click", double_click::execute
    }

    ref_action! {
        /// Triple-clicks a ref, which is how a text field is selected whole.
        triple_click => "triple-click", triple_click::execute
    }

    ref_action! {
        /// Right-clicks a ref, opening its context menu.
        ///
        /// The menu it opens is a separate surface: snapshot with
        /// [`tinycomputer_bus::Surface::Menu`] to see inside it.
        right_click => "right-click", right_click::execute
    }

    ref_action! {
        /// Empties a ref's value.
        clear => "clear", clear::execute
    }

    ref_action! {
        /// Gives a ref keyboard focus.
        focus => "focus", focus::execute
    }

    ref_action! {
        /// Toggles a checkable ref, whichever state it is in.
        toggle => "toggle", toggle::execute
    }

    ref_action! {
        /// Checks a checkable ref. Checking one already checked is a no-op
        /// rather than a toggle.
        check => "check", check::execute
    }

    ref_action! {
        /// Unchecks a checkable ref.
        uncheck => "uncheck", uncheck::execute
    }

    ref_action! {
        /// Expands an expandable ref — a disclosure triangle, a tree node.
        expand => "expand", expand::execute
    }

    ref_action! {
        /// Collapses an expandable ref.
        collapse => "collapse", collapse::execute
    }

    ref_action! {
        /// Scrolls a ref into view without acting on it.
        ///
        /// Worth doing before a coordinate-based interaction, which cannot
        /// reach an element that is scrolled out of the viewport.
        scroll_to => "scroll-to", scroll_to::execute
    }

    /// Types text into a ref.
    ///
    /// The keystrokes are delivered to the element rather than to whatever
    /// holds focus, so a window that steals focus mid-run does not receive
    /// them.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, TypeRequest};
    /// let reply = Desktop::new().type_text(TypeRequest {
    ///     ref_id: "@s1:e2".to_owned(),
    ///     text: "hello".to_owned(),
    ///     ..TypeRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "type");
    /// ```
    #[must_use]
    pub fn type_text(&self, request: TypeRequest) -> DesktopResponse {
        self.run("type", Need::Accessibility, |adapter, context| {
            type_text::execute(
                type_text::TypeArgs {
                    ref_id: convert::qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
                    text: request.text,
                    timeout_ms: request.timeout_ms,
                },
                adapter,
                context,
            )
        })
    }

    /// Replaces a ref's value in one step.
    ///
    /// Faster than typing and safe for a long value, but it does not fire the
    /// per-character handlers some forms validate with. Prefer
    /// [`Desktop::type_text`] when the typing itself matters.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, SetValueRequest};
    /// let reply = Desktop::new().set_value(SetValueRequest {
    ///     ref_id: "@s1:e2".to_owned(),
    ///     value: "42".to_owned(),
    ///     ..SetValueRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "set-value");
    /// ```
    #[must_use]
    pub fn set_value(&self, request: SetValueRequest) -> DesktopResponse {
        self.run("set-value", Need::Accessibility, |adapter, context| {
            set_value::execute(
                set_value::SetValueArgs {
                    ref_id: convert::qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
                    value: request.value,
                    timeout_ms: request.timeout_ms,
                },
                adapter,
                context,
            )
        })
    }

    /// Selects an option within a list, menu, or picker.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, SelectRequest};
    /// let reply = Desktop::new().select(SelectRequest {
    ///     ref_id: "@s1:e3".to_owned(),
    ///     value: "Monday".to_owned(),
    ///     ..SelectRequest::default()
    /// });
    ///
    /// assert_eq!(reply.command, "select");
    /// ```
    #[must_use]
    pub fn select(&self, request: SelectRequest) -> DesktopResponse {
        self.run("select", Need::Accessibility, |adapter, context| {
            select::execute(
                select::SelectArgs {
                    ref_id: convert::qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
                    value: request.value,
                    timeout_ms: request.timeout_ms,
                },
                adapter,
                context,
            )
        })
    }

    /// Scrolls a scrollable ref.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinycomputer_desktop::{Desktop, Direction, ScrollRequest};
    /// let reply = Desktop::new().scroll(ScrollRequest::new("@s1:e4", Direction::Down, 3));
    /// assert_eq!(reply.command, "scroll");
    /// ```
    #[must_use]
    pub fn scroll(&self, request: ScrollRequest) -> DesktopResponse {
        self.run("scroll", Need::Accessibility, |adapter, context| {
            scroll::execute(
                scroll::ScrollArgs {
                    ref_id: convert::qualified_ref(request.ref_id, request.snapshot_id.as_deref()),
                    direction: convert::direction(request.direction),
                    amount: request.amount,
                    timeout_ms: request.timeout_ms,
                },
                adapter,
                context,
            )
        })
    }
}
