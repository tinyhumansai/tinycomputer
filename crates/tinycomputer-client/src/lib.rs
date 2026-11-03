//! Typed host calls to the `TinyComputer` `TinyBus` module.
//!
//! [`ComputerClient`] owns no desktop or browser implementation. It takes the
//! contract's request and response types, coordinates each call with the
//! module's current [`ComputerConfig`](tinycomputer_bus::config::ComputerConfig), and applies the contract catalogue's
//! confidential-delivery rules before sending a frame.
//!
//! # Example
//!
//! ```no_run
//! use tinybus::module::{ModuleAccess, ReconfigurableModule};
//! use tinycomputer_bus::{agentic::RunGoalRequest, config::ComputerConfig, names};
//! use tinycomputer_client::ComputerClient;
//!
//! # async fn example(access: impl ModuleAccess + 'static) -> Result<(), Box<dyn std::error::Error>> {
//! let module = ReconfigurableModule::new(access, "tinycomputer");
//! let client = ComputerClient::new(module);
//! let config = ComputerConfig::default();
//! let _result = client
//!     .call_desktop::<_, serde_json::Value>(&config, names::methods::RUN_GOAL, RunGoalRequest::default())
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! The client deliberately does not hold browser policy, Chrome hints, task
//! constraints, or the `TinyComputer` engine. Hosts decide those values and pass
//! the resulting contract request types to the client.

mod client;
mod error;

pub use client::{ComputerClient, RUN_GOAL_TIMEOUT, TASK_CALL_TIMEOUT};
pub use error::ClientError;
