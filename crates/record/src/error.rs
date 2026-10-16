//! Errors of the recording layer.

use std::path::PathBuf;

/// Result alias of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why a recording, scene or playback operation failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A scene (or a file claiming to be one) breaks a rule.
    #[error("invalid scene: {0}")]
    Invalid(String),
    /// There is no puppet track with this index.
    #[error("no puppet track {0}")]
    NoTrack(usize),
    /// There is no take with this index on the track.
    #[error("no take {take} on track {track}")]
    NoTake {
        /// The track.
        track: usize,
        /// The take.
        take: usize,
    },
    /// There is no audio track or clip with this index.
    #[error("no audio {what} {index}")]
    NoAudio {
        /// `track` or `clip`.
        what: &'static str,
        /// The index.
        index: usize,
    },
    /// Nothing is armed, so a recording would capture nothing.
    #[error("nothing is armed for recording")]
    NothingArmed,
    /// A recording is already running.
    #[error("a recording is already running")]
    AlreadyRecording,
    /// No recording is running.
    #[error("no recording is running")]
    NotRecording,
    /// Something is larger than the limit.
    #[error("{what} is too large (limit {limit})")]
    TooLarge {
        /// What was too large.
        what: &'static str,
        /// The limit.
        limit: usize,
    },
    /// A behavior edit failed.
    #[error(transparent)]
    Behaviors(#[from] charcraft_behaviors::Error),
    /// Sound could not be decoded or mixed.
    #[error(transparent)]
    Audio(#[from] anim_audio::AudioError),
    /// The project could not be read.
    #[error(transparent)]
    Project(#[from] charcraft_project::Error),
    /// A puppet could not be built.
    #[error(transparent)]
    Puppet(#[from] charcraft_puppet::Error),
    /// A file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// JSON that does not fit.
    #[error("bad JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    /// Shorthand for [`Error::Invalid`].
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// Wraps an I/O error with its path.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }
}
