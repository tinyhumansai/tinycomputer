//! The reply every member of the interface returns.
//!
//! # Why every member replies with the same type
//!
//! A desktop command fails in ways a caller has to act on rather than merely
//! report: a ref went stale and the caller must re-snapshot, a permission is
//! missing and the caller must prompt for it, an app was not running. Folding
//! those into a `TinyBus` error would reduce each one to a string, and a string
//! is exactly what an agent driving this module over tool calls cannot act on.
//!
//! So a member returns `Ok(DesktopResponse)` for both outcomes, and the
//! envelope carries either [`DesktopResponse::data`] or a structured
//! [`DesktopError`] with its code, its suggestion, and its recovery hint. A
//! `TinyBus` error is reserved for a genuine transport or dispatch failure —
//! an unknown member, an undecodable frame — which is a different kind of
//! problem and deserves a different channel.
//!
//! # The wire form is `agent-desktop`'s own
//!
//! [`ENVELOPE_VERSION`] and the field names match the JSON the `agent-desktop`
//! CLI writes to stdout, byte for byte, so a host that already parses that
//! output does not need a second parser, and a reply can be forwarded to an
//! agent unchanged.

mod types;

pub use types::{
    Delivery, DeliveryDisposition, DesktopError, DesktopResponse, DesktopResponseError,
    ENVELOPE_VERSION, RecoveryHint, RetryDisposition,
};

#[cfg(test)]
mod envelope_tests;
