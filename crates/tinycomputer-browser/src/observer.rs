//! Successful browser actions observed by a trusted host.

/// The kind of a successful browser action, without its input payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedAction {
    /// A page navigation completed.
    Navigate,
    /// A control was clicked.
    Click,
    /// A field was filled.
    Fill,
    /// Text was typed into a field. The text is not included.
    Type,
    /// One or more options were selected. Their values are not included.
    Select,
    /// A checkbox or radio was checked.
    Check,
    /// A checkbox or radio was unchecked.
    Uncheck,
    /// A control was double-clicked.
    DoubleClick,
    /// The pointer hovered over a control.
    Hover,
    /// A control received focus.
    Focus,
    /// The page or a control was scrolled.
    Scroll,
    /// A control was read or tested for visibility.
    Read,
    /// The browser waited for a condition.
    Wait,
    /// Another typed browser action completed.
    Other,
    /// Escape was pressed.
    PressEscape,
    /// Another key was pressed. The key text is not included.
    Press,
}

/// A semantic target resolved from the latest accessibility snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct ObservedTarget {
    /// The target's accessibility role.
    pub role: String,
    /// The target's accessible name, if present.
    pub name: Option<String>,
    /// The snapshot reference actually dispatched.
    pub reference: Option<String>,
    /// True only when the current snapshot contains exactly one target with
    /// this role and non-empty name, and the dispatched reference names it.
    pub unique: bool,
}

impl std::fmt::Debug for ObservedTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservedTarget")
            .field("role", &self.role)
            .field("reference", &self.reference)
            .field("unique", &self.unique)
            .finish_non_exhaustive()
    }
}

/// Metadata for a successful dispatch. It deliberately carries no request
/// payload, typed text, fill values, or locator source text.
#[derive(Clone, PartialEq, Eq)]
pub struct ActionObservation {
    /// Task that owns the browser session, when task context was supplied.
    pub task_id: Option<String>,
    /// Browser session that performed the action.
    pub session_id: String,
    /// The action that completed.
    pub action: ObservedAction,
    /// The page immediately before dispatch.
    pub before_url: String,
    /// The page after dispatch and origin checks.
    pub after_url: String,
    /// The resolved target, when the action named one.
    pub target: Option<ObservedTarget>,
}

impl std::fmt::Debug for ActionObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionObservation")
            .field("task_id", &self.task_id)
            .field("session_id", &self.session_id)
            .field("action", &self.action)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// A trusted host sink for successful browser action metadata.
pub trait ActionObserver: Send + Sync {
    /// Receives one successful dispatch. Implementations should return
    /// quickly and must not retain data they do not need.
    fn observed(&self, action: ActionObservation);
}

#[cfg(test)]
#[path = "observer_tests.rs"]
mod observer_tests;
