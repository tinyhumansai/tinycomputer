//! Host policy for actions sent to a browser page.

use std::sync::Arc;

use serde_json::Value;
use tinycomputer_bus::browser::{Action, ElementRef, Target};

/// Per-task facts supplied by the host when it creates a browser surface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionContext {
    /// The task that is driving the page, when there is one.
    pub task_id: Option<String>,
    /// The retained browser session driving this request, when it is open.
    pub session_id: Option<String>,
    /// Whether payment handling is in play for the task.
    pub payment_in_play: bool,
}

/// A target resolved as far as the latest accessibility snapshot allows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionTarget {
    /// The element's accessibility role, when known.
    pub role: Option<String>,
    /// The element's accessible name, when known.
    pub name: Option<String>,
    /// The snapshot ref used to identify the element, when known.
    pub reference: Option<String>,
    /// Whether exactly one element in the latest snapshot has this role and
    /// accessible name. Locators without a snapshot ref are never unique.
    pub unique: bool,
}

/// Page content an engine caller is about to receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationKind {
    /// The page's accessibility tree and controls.
    Snapshot,
    /// The page's extracted text, Markdown, or HTML.
    ReadPage,
    /// A screenshot of the page.
    Screenshot,
    /// A fixed browser-surface page-stability watcher. This exposes only
    /// whether the page changed, not its content.
    PageStability,
    /// A fixed-purpose hit-test used by the browser surface before it decides
    /// whether a target's pointer action is covered.
    TargetCheck,
}

/// Information a trusted host may inspect before the browser acts or returns
/// page content.
///
/// `payload` contains the requested action and its value, including typed
/// text. It is intentionally omitted from `Debug`; implementations must treat
/// it as confidential and must not log or persist it.
pub struct ActionRequest {
    /// The typed or raw browser action, including values sent to the page.
    pub payload: Value,
    /// The resolved semantic target, if the request names one.
    pub target: Option<ActionTarget>,
    /// The page's actual current URL immediately before the request.
    /// The actual address the browser reported immediately before the request,
    /// or `None` when the page is between documents and has no readable URL.
    /// A host that makes page-specific decisions should refuse an unknown URL.
    pub current_url: Option<String>,
    /// The requested destination for a navigation, when applicable.
    pub destination_url: Option<String>,
    /// Whether this request observes page content rather than acting on it.
    pub observation: Option<ObservationKind>,
    /// Host-supplied task context.
    pub context: ActionContext,
}

impl ActionRequest {
    pub(crate) fn typed(
        action: &Action,
        current_url: Option<String>,
        destination_url: Option<String>,
        context: &ActionContext,
        refs: &[ElementRef],
    ) -> Self {
        let payload = serde_json::to_value(action).unwrap_or(Value::Null);
        let target = action_target(action, refs);
        Self {
            payload,
            target,
            current_url,
            destination_url,
            observation: None,
            context: context.clone(),
        }
    }

    pub(crate) fn raw(
        payload: Value,
        current_url: Option<String>,
        destination_url: Option<String>,
        context: &ActionContext,
        refs: &[ElementRef],
    ) -> Self {
        let target = payload
            .get("selector")
            .and_then(Value::as_str)
            .and_then(|selector| selector.strip_prefix('@'))
            .and_then(|reference| resolve_reference(reference, refs));
        Self {
            payload,
            target,
            current_url,
            destination_url,
            observation: None,
            context: context.clone(),
        }
    }

    pub(crate) fn observation(
        kind: ObservationKind,
        current_url: Option<String>,
        context: &ActionContext,
    ) -> Self {
        Self {
            payload: serde_json::json!({ "action": "observe" }),
            target: None,
            current_url,
            destination_url: None,
            observation: Some(kind),
            context: context.clone(),
        }
    }
}

impl std::fmt::Debug for ActionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionRequest")
            .field(
                "action",
                &self.payload.get("action").and_then(Value::as_str),
            )
            .field("target", &self.target)
            .field("current_url", &self.current_url)
            .field("destination_url", &self.destination_url)
            .field("observation", &self.observation)
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

/// The host's decision for one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionVerdict {
    /// Let the browser perform the request.
    Allow,
    /// Refuse it before any page interaction occurs.
    Refuse {
        /// A short explanation safe to show in task reports. Do not include
        /// the request value in this message.
        reason: String,
    },
}

/// A synchronous host policy consulted before every typed or raw action and
/// before a snapshot, page read, or screenshot can expose content.
pub trait ActionGate: Send + Sync {
    /// Decide whether this request may reach the page.
    fn check(&self, request: &ActionRequest) -> ActionVerdict;
}

/// A gate that leaves browser behavior unchanged.
#[derive(Debug, Default)]
pub struct AllowAllActions;

impl ActionGate for AllowAllActions {
    fn check(&self, _request: &ActionRequest) -> ActionVerdict {
        ActionVerdict::Allow
    }
}

pub(crate) fn default_gate() -> Arc<dyn ActionGate> {
    Arc::new(AllowAllActions)
}

fn action_target(action: &Action, refs: &[ElementRef]) -> Option<ActionTarget> {
    let target = match action {
        Action::Click { target, .. }
        | Action::DoubleClick { target }
        | Action::Hover { target }
        | Action::Focus { target }
        | Action::Fill { target, .. }
        | Action::Select { target, .. }
        | Action::Check { target, .. }
        | Action::GetText { target }
        | Action::GetAttribute { target, .. }
        | Action::IsVisible { target } => Some(target),
        Action::Type { target, .. }
        | Action::Press { target, .. }
        | Action::Scroll { target, .. }
        | Action::WaitFor { target, .. } => target.as_ref(),
        Action::Back | Action::Forward | Action::Reload => None,
    }?;
    target_from_target(target, refs)
}

fn target_from_target(target: &Target, refs: &[ElementRef]) -> Option<ActionTarget> {
    match target {
        Target::Ref { value } => resolve_reference(value, refs),
        Target::Selector { .. } => None,
        Target::Locator { value } if value.by == tinycomputer_bus::browser::LocateBy::Role => {
            Some(ActionTarget {
                role: Some(value.value.clone()),
                name: value.name.clone(),
                reference: None,
                unique: false,
            })
        }
        Target::Locator { value } => Some(ActionTarget {
            role: None,
            name: Some(value.value.clone()),
            reference: None,
            unique: false,
        }),
    }
}

fn resolve_reference(value: &str, refs: &[ElementRef]) -> Option<ActionTarget> {
    let reference = value.strip_prefix('@').unwrap_or(value);
    refs.iter()
        .find(|item| item.id == reference)
        .map(|item| ActionTarget {
            role: Some(item.role.clone()),
            name: Some(item.name.clone()),
            reference: Some(item.id.clone()),
            unique: refs
                .iter()
                .filter(|candidate| candidate.role == item.role && candidate.name == item.name)
                .count()
                == 1,
        })
}

#[cfg(test)]
#[path = "action_gate_tests.rs"]
mod tests;
