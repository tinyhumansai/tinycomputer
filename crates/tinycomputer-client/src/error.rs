//! Errors returned by typed `TinyComputer` host calls.

use thiserror::Error;
use tinybus::{Error as TinyBusError, module::ModuleCallError};
use tinycomputer_bus::{DesktopResponseError, agent::AgentResponseError, catalogue::Family};

/// A rejected configuration, transport call, or typed response.
#[derive(Debug, Error)]
pub enum ClientError {
    /// `TinyBus` could not prepare or reconfigure the module.
    #[error("TinyComputer module is unavailable: {0}")]
    Module(#[from] ModuleCallError),
    /// `TinyBus` rejected or could not deliver a member call.
    #[error("TinyComputer bus call failed: {0}")]
    Bus(#[from] TinyBusError),
    /// A desktop, browser, or flow member returned a malformed or failed envelope.
    #[error("TinyComputer desktop response failed: {0}")]
    DesktopResponse(#[from] DesktopResponseError),
    /// A task member returned a malformed or failed envelope.
    #[error("TinyComputer task response failed: {0}")]
    AgentResponse(#[from] AgentResponseError),
    /// The requested name is not in this `TinyComputer` contract catalogue.
    #[error("TinyComputer does not catalogue member `{0}`")]
    UnknownMember(String),
    /// The member belongs to a different response family than this call method.
    #[error("TinyComputer member `{member}` belongs to {actual:?}, expected {expected}")]
    WrongFamily {
        /// The member that was requested.
        member: String,
        /// Its catalogue family.
        actual: Family,
        /// The family accepted by the selected client method.
        expected: &'static str,
    },
}
