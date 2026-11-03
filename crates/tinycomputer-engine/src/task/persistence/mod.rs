//! Versioned snapshots for tasks paused for input, approval, or human help.

const VERSION: u32 = 2;
const OLDEST_VERSION: u32 = 1;

mod error;
mod export;
mod import;
mod types;

pub use error::PausedTaskError;
