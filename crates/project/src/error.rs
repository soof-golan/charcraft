//! Error type.

use thiserror::Error;

/// Everything that can go wrong opening, decoding, editing or saving a project.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The zip or folder container could not be read or written.
    #[error(transparent)]
    Archive(#[from] anim_archive::ArchiveError),
    /// The project store could not be read.
    #[error(transparent)]
    Rope(#[from] charcraft_rope::Error),
    /// A metadata table could not be parsed.
    #[error("ltab: {0}")]
    Ltab(#[from] charcraft_ltab::Error),
    /// A file could not be read or written.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The input is not a Character Animator project or puppet.
    #[error("not a Character Animator project or puppet: {0}")]
    NotAProject(String),
    /// A file or item the operation needs is not in the project.
    #[error("`{0}` is not in the project")]
    Missing(String),
    /// A file is present but not shaped the way the operation needs.
    #[error("{path}: {reason}")]
    Malformed {
        /// The project file.
        path: String,
        /// What was wrong.
        reason: String,
    },
    /// A classed table could not be printed back with its class names.
    #[error("{path}: class names do not line up with the tables ({classes} names, {tables} tables)")]
    ClassMismatch {
        /// The project file.
        path: String,
        /// Class slots found in the text.
        classes: usize,
        /// Tables in the parsed value.
        tables: usize,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io { context: context.into(), source }
    }

    pub(crate) fn malformed(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Malformed { path: path.into(), reason: reason.into() }
    }
}
