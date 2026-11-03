//! The shared domain of tinycomputer's surfaces.
//!
//! Everything here is deterministic, engine-free, and the same whether a task
//! is driving a desktop application or a web page:
//!
//! - [`Key`] and [`Platform`] — shortcuts named by what they do, spelled per
//!   platform for agent-desktop ([`Key::desktop`]) and agent-browser
//!   ([`Key::browser`]).
//! - [`consequence`] and [`payment_evidence`] — what pressing a control
//!   commits to, and whether a page is asking for payment. These are the
//!   checks that stop a run before anything irreversible or paid happens,
//!   whatever a model decided.
//! - [`Record`], the value parsers, and [`rank`] — result cards as named
//!   fields, and picking "the cheapest" or "the earliest" by arithmetic.
//! - [`parse_date`] and [`reformat_date`] — a caller's date, typed the way a
//!   form's field asks for it.
//! - [`Facts`] — the caller's values: shared ones briefed to a model, secret
//!   ones (cards, passports, passwords) only ever named as `${name}`.
//! - [`surface`] — the [`Surface`](surface::Surface) trait every decision loop
//!   runs against, the [`Screen`](surface::Screen) it observes, verified text
//!   delivery, and the [`digest`](surface::digest()) that parses a screen into
//!   regions a decision model reads at a glance.
//!
//! The crate holds no engine, no bus, no model, and no runtime; it speaks the
//! contract crate's closed operations and envelope. The surface
//! adapters and `tinycomputer-engine` build on it
//! (`docs/technical/specs/unified-agent.md`).
//!
//! ```
//! use tinycomputer_core::{Consequence, Criterion, Key, Platform, Record, consequence, rank};
//!
//! assert_eq!(Key::Paste.desktop(Platform::Windows).as_deref(), Some("ctrl+v"));
//! assert_eq!(consequence("Pay now"), Consequence::Payment);
//!
//! let fares = [
//!     Record::from_pairs([("fare", "₹7,210")]),
//!     Record::from_pairs([("fare", "₹6,840")]),
//! ];
//! assert_eq!(rank(&fares, Criterion::LowestPrice), Some(vec![1, 0]));
//! ```

mod dates;
mod error;
mod facts;
mod keymap;
mod records;
mod safety;
pub mod sites;
pub mod surface;

pub use dates::{Date, date_pattern, parse_date, reformat_date};
pub use error::{Error, Result};
pub use facts::{Facts, SecretFactHandle, is_sensitive_name};
pub use keymap::{Key, Platform};
pub use records::{
    Criterion, Price, Record, closest_to, parse_clock, parse_duration, parse_price, parse_stops,
    rank, rank_closest,
};
pub use safety::{
    Consequence, FieldHint, PaymentEvidence, adjusts_a_count, consequence, human_needed,
    payment_evidence, screen_payment_evidence,
};
