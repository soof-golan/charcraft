//! Error type.

use thiserror::Error;

use crate::oid::Oid;

/// Everything that can go wrong reading, writing or walking a project store.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A record did not start with the rope magic.
    #[error("offset {offset}: not a rope record (bad magic)")]
    BadMagic {
        /// Byte offset of the record.
        offset: usize,
    },
    /// The data ended in the middle of a record.
    #[error("offset {offset}: truncated record")]
    Truncated {
        /// Byte offset of the record.
        offset: usize,
    },
    /// A record is internally inconsistent.
    #[error("offset {offset}: malformed record: {reason}")]
    BadRecord {
        /// Byte offset of the record.
        offset: usize,
        /// What was wrong.
        reason: &'static str,
    },
    /// A configured limit was exceeded.
    #[error("offset {offset}: limit exceeded: {what}")]
    LimitExceeded {
        /// Byte offset of the record (0 when not tied to a record).
        offset: usize,
        /// Which limit.
        what: &'static str,
    },
    /// An object's declared id does not match the SHA-1 of its content.
    #[error("offset {offset}: object id mismatch: declared {declared}, content hashes to {computed}")]
    IdMismatch {
        /// Byte offset of the record.
        offset: usize,
        /// The id in the record.
        declared: Oid,
        /// The id the content actually has.
        computed: Oid,
    },
    /// An object referenced by a ref, commit or tree is not in the store.
    #[error("object {0} is not in the store")]
    MissingObject(Oid),
    /// An object exists but is not the type, or not well formed.
    #[error("object {id}: {reason}")]
    BadObject {
        /// The object.
        id: Oid,
        /// What was wrong.
        reason: &'static str,
    },
    /// A ref could not be resolved to an object.
    #[error("ref `{0}` does not resolve to an object")]
    UnresolvedRef(String),
    /// Text that should be a 40 digit hex id was not.
    #[error("`{0}` is not a 40 digit hex object id")]
    BadHex(String),
    /// No `projectdata.rope`/repository was found at the given location.
    #[error("no project store (repo.noindex/projectdata.rope) found in `{0}`")]
    NoStore(String),
    /// A file or folder could not be read.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The zip or folder container could not be read.
    #[error(transparent)]
    Archive(#[from] anim_archive::ArchiveError),
    /// An ltab blob could not be parsed.
    #[error("ltab: {0}")]
    Ltab(#[from] charcraft_ltab::Error),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io { context: context.into(), source }
    }
}
