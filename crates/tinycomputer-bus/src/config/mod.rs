//! Typed configuration passed when the `TinyComputer` module is loaded.

mod browser;
mod captcha;
mod computer;
mod planner;

pub use browser::{BrowserConfig, BrowserPerception, BrowserSettle};
pub use captcha::{CaptchaConfig, DEFAULT_CAPTCHA_MODEL};
pub use computer::ComputerConfig;
pub use planner::{ModelRoute, PlannerConfig};

#[cfg(test)]
mod config_tests;
