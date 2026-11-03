//! Typed configuration passed when the `TinyComputer` module is loaded.

mod browser;
mod computer;
mod planner;

pub use browser::{BrowserConfig, BrowserPerception, BrowserSettle};
pub use computer::ComputerConfig;
pub use planner::{ModelRoute, PlannerConfig};

#[cfg(test)]
mod config_tests;
