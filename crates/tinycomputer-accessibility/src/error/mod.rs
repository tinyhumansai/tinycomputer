//! Compatibility exports of the shared accessibility error vocabulary.
pub use tinycomputer_bus::accessibility::{Error, Result};

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
