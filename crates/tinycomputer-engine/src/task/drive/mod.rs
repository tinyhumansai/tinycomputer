//! Running a task in the background: planning it when asked to, running its
//! flows in order, interpreting where each stopped, and finishing it.

use std::collections::BTreeMap;

type TaskRecords = BTreeMap<String, Vec<BTreeMap<String, String>>>;

mod finish;
mod handoff;
mod plan;
mod run;

pub(super) use plan::{plan_payment_then_drive, plan_then_drive};
pub(super) use run::drive;
