//! Host-resolved secret facts. Secret material is typed by the embedding
//! host and can be redacted from observations without returning it to engine.

/// Metadata needed to resolve and type one host-side secret.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretTypeRequest {
    /// Opaque host identifier. Never pass this to a model or report.
    pub handle: String,
    /// Safe mask associated with the secret fact.
    pub mask: String,
    /// Host-defined secret category.
    pub kind: String,
    /// Accessibility role of the selected field.
    pub field_role: String,
    /// Accessibility name of the selected field, when present.
    pub field_name: Option<String>,
    /// Snapshot reference identifying the selected field.
    pub field_ref: String,
    /// Current page URL. Secret typing is refused when this is unknown.
    pub current_url: String,
    /// Task whose browser session owns the field.
    pub task_id: String,
    /// Task actions available before this host typing operation is charged.
    pub actions_remaining: u32,
}

impl std::fmt::Debug for SecretTypeRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretTypeRequest")
            .field("handle", &"[opaque]")
            .field("mask", &self.mask)
            .field("kind", &self.kind)
            .field("field_role", &self.field_role)
            .field("field_name", &self.field_name)
            .field("field_ref", &self.field_ref)
            .field("current_url", &self.current_url)
            .field("task_id", &self.task_id)
            .field("actions_remaining", &self.actions_remaining)
            .finish()
    }
}

/// A host that resolves opaque handles and types secrets without returning
/// their values to the engine.
pub trait SecretHost: Send + Sync + 'static {
    /// Resolve and type into the requested field. A refusal must type nothing.
    ///
    /// # Errors
    ///
    /// Returns a safe reason when the handle, task, page, field, site, or
    /// host policy refuses the type.
    fn type_secret(&self, request: &SecretTypeRequest) -> Result<(), String>;

    /// Replace any host-held secret values appearing in outbound text,
    /// including their textual representations as JSON numbers and keys.
    fn redact(&self, task_id: &str, text: &str) -> String;
}

#[cfg(test)]
#[path = "secret_host_tests.rs"]
mod tests;
