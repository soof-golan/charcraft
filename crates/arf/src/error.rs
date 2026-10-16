//! Error type.

use thiserror::Error;

/// Everything that can go wrong reading an `.arf` file.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The data ended in the middle of a chunk.
    #[error("offset {offset}: truncated chunk")]
    Truncated {
        /// Byte offset of the chunk header.
        offset: usize,
    },
    /// A chunk header or trailer is not what the format has.
    #[error("offset {offset}: bad chunk: {reason}")]
    BadChunk {
        /// Byte offset of the chunk header.
        offset: usize,
        /// What was wrong.
        reason: &'static str,
    },
    /// Chunks nest deeper than any real file does.
    #[error("offset {offset}: chunks nested too deeply")]
    TooDeep {
        /// Byte offset of the chunk header.
        offset: usize,
    },
    /// The chunks are well formed but do not make an event graph.
    #[error("event graph: {0}")]
    Structure(String),
    /// A number spelled as text could not be read.
    #[error("event graph: {0:?} is not a number")]
    BadNumber(String),
    /// A limit was exceeded.
    #[error("event graph: limit exceeded: {0}")]
    Limit(&'static str),
}

impl Error {
    pub(crate) fn structure(reason: impl Into<String>) -> Self {
        Self::Structure(reason.into())
    }
}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;
