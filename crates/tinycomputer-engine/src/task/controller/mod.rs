//! [`Tasks`], the controller, and the calls it answers: planning, starting,
//! awaiting, continuing, cancelling, and reporting a task.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use tinycomputer_bus::agent::{LanguageModelConfiguration, SecretHandle, TaskId};

use super::FlowRunner;
use super::store::Cell;
use crate::rescue::Rescuer;
use crate::shape::Shaper;

mod continuation;
mod plan;
mod start;
mod status;

/// Worker-only authorization to continue a payment checkpoint after the
/// host has secured the card and verified the live checkout.
#[derive(Debug, Clone)]
pub struct ContinuePaymentRequest {
    /// The task that stopped at its payment checkpoint.
    pub id: TaskId,
    /// Opaque host handles for payment-card fields; values never enter the
    /// task's facts or planner request.
    pub fact_handles: BTreeMap<String, SecretHandle>,
    /// Sanitized, untrusted text extracted from the retained checkout page.
    pub checkout_context: String,
}

/// Worker-only continuation after the payer has approved a sealed at-shop
/// QR payment. It retains the non-destructive payment stop policy and can
/// only continue the same task from its unused payment checkpoint.
#[derive(Debug, Clone)]
pub struct ContinuePaidAtShopRequest {
    /// The task stopped at its payment checkpoint.
    pub id: TaskId,
    /// Sanitized, untrusted text from the retained checkout page.
    pub checkout_context: String,
}

/// The task controller.
pub struct Tasks {
    pub(super) runner: Arc<dyn FlowRunner>,
    pub(super) planner: Option<crate::planner::Planner>,
    pub(super) rescuer: Option<Rescuer>,
    pub(super) shaper: Option<Shaper>,
    pub(super) cells: Mutex<BTreeMap<u64, Arc<Cell>>>,
    pub(super) counter: AtomicU64,
}

impl std::fmt::Debug for Tasks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Tasks").finish_non_exhaustive()
    }
}

impl Tasks {
    /// A controller that runs flows with `runner`.
    #[must_use]
    pub fn new(runner: Arc<dyn FlowRunner>) -> Self {
        Self {
            runner,
            planner: None,
            rescuer: None,
            shaper: None,
            cells: Mutex::new(BTreeMap::new()),
            counter: AtomicU64::new(0),
        }
    }

    /// This controller, turning plain-language tasks into flows with
    /// `planner`.
    #[must_use]
    pub fn with_planner(mut self, planner: crate::planner::Planner) -> Self {
        self.planner = Some(planner);
        self
    }

    /// This controller, handing a failed step to `rescuer` for guidance
    /// before the task fails.
    #[must_use]
    pub fn with_rescuer(mut self, rescuer: Rescuer) -> Self {
        self.rescuer = Some(rescuer);
        self
    }

    /// This controller, turning what a finished task read into the shape
    /// its `output` asks for with `shaper`.
    #[must_use]
    pub fn with_shaper(mut self, shaper: Shaper) -> Self {
        self.shaper = Some(shaper);
        self
    }

    /// Whether a task may ask for its answer in a shape.
    #[must_use]
    pub fn output_configured(&self) -> bool {
        self.shaper.is_some()
    }

    /// Whether a rescuer is configured.
    #[must_use]
    pub fn rescue_configured(&self) -> bool {
        self.rescuer.is_some()
    }

    /// Whether a planner is configured.
    #[must_use]
    pub fn planner_configured(&self) -> bool {
        self.planner.is_some()
    }

    /// The planner's route and model, when a planner is configured and says.
    #[must_use]
    pub fn planner_model(&self) -> Option<&LanguageModelConfiguration> {
        self.planner.as_ref()?.configuration()
    }

    /// The rescuer's route and model, when a rescuer is configured and says.
    #[must_use]
    pub fn rescue_model(&self) -> Option<&LanguageModelConfiguration> {
        self.rescuer.as_ref()?.configuration()
    }

    /// The shaper's route and model, when a shaper is configured and says.
    #[must_use]
    pub fn output_model(&self) -> Option<&LanguageModelConfiguration> {
        self.shaper.as_ref()?.configuration()
    }
}
