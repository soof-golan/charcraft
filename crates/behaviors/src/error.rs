//! Errors of the behavior layer.

/// Result alias of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why a behavior or stack operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No behavior has this id.
    #[error("unknown behavior {0:?}")]
    UnknownBehavior(String),
    /// The parameters do not fit the behavior.
    #[error("bad parameters for {behavior}: {message}")]
    BadParams {
        /// The behavior id.
        behavior: String,
        /// What serde said.
        message: String,
    },
    /// The behavior has no parameter with this name.
    #[error("{behavior} has no parameter {param:?}")]
    NoSuchParam {
        /// The behavior id.
        behavior: String,
        /// The parameter name.
        param: String,
    },
    /// A stack position that does not exist.
    #[error("no behavior at position {0}")]
    NoSuchSlot(usize),
    /// A script that cannot be played.
    #[error("bad script: {0}")]
    BadScript(String),
}
