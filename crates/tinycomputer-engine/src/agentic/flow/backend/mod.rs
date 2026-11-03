//! The surface the flow runtime drives, and the async wrappers around it.
//!
//! The [`AgentBackend`] trait is `tinycomputer_core::surface::Surface`;
//! `tinycomputer-desktop` implements it for `Desktop`. Surface calls block, so
//! the runtime makes them off the async executor through [`observe_async`]
//! and [`blocking`].

use tinycomputer_bus::DesktopResponse;
pub(in crate::agentic) use tinycomputer_core::surface::{Surface as AgentBackend, deliver_text};

use super::{
    super::internal_error,
    view::{Depth, Screen},
};

pub(in crate::agentic) async fn observe_async<B: AgentBackend>(
    backend: B,
    app: String,
    root: Option<String>,
    depth: Depth,
) -> Result<Screen, Box<DesktopResponse>> {
    tokio::task::spawn_blocking(move || backend.observe(&app, root.as_deref(), depth))
        .await
        .map_err(|error| {
            Box::new(internal_error(&format!(
                "desktop observation task failed: {error}"
            )))
        })?
}

/// Runs a blocking backend call off the async executor.
pub(in crate::agentic) async fn blocking<B, F, T>(backend: B, call: F) -> T
where
    B: AgentBackend,
    F: FnOnce(B) -> T + Send + 'static,
    T: Send + 'static + From<DesktopResponse>,
{
    tokio::task::spawn_blocking(move || call(backend))
        .await
        .unwrap_or_else(|error| {
            T::from(internal_error(&format!(
                "desktop action task failed: {error}"
            )))
        })
}

#[cfg(all(test, feature = "desktop"))]
mod backend_tests;
