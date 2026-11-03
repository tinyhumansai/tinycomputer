//! Crate-wide error and result types.

/// Errors returned by this crate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A name was marked secret, but no fact carries it.
    ///
    /// Refused rather than ignored: a misspelt secret name would otherwise
    /// leave the value it meant to protect shared with the decision model.
    #[error("`{name}` is marked secret, but no fact is called that")]
    UnknownSecret {
        /// The name marked secret.
        name: String,
    },
    /// A fact name was supplied both with a value and as a host handle.
    #[error("`{name}` has both a value and a host secret handle")]
    DuplicateFact {
        /// The conflicting fact name.
        name: String,
    },
    /// A host-side handle is empty and cannot resolve a secret.
    #[error("`{name}` has an empty host secret handle")]
    EmptyFactHandle {
        /// The fact name.
        name: String,
    },
}

/// The crate's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod error_tests;
