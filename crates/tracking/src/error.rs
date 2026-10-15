//! Errors of the tracking crate.

use std::path::PathBuf;

/// Everything that can go wrong while tracking a face.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The pixel buffer of a frame does not match its size.
    #[error("frame {width}x{height} needs {expected} bytes of RGB, got {got}")]
    BadFrame {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// Bytes an RGB frame of that size takes.
        expected: usize,
        /// Bytes supplied.
        got: usize,
    },
    /// A frame is larger than [`crate::frame::MAX_FRAME_PIXELS`].
    #[error("frame {width}x{height} is larger than the {max} pixel limit")]
    FrameTooLarge {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// The cap, in pixels.
        max: usize,
    },
    /// A model file does not match its registry entry.
    #[error("{name}: SHA-256 is {got}, the registry says {expected}")]
    ChecksumMismatch {
        /// Model name.
        name: &'static str,
        /// Hash of the bytes.
        got: String,
        /// Hash the registry lists.
        expected: &'static str,
    },
    /// A model's licence is not on the allow list.
    #[error("{name}: licence {licence} is not allowed")]
    LicenceNotAllowed {
        /// Model name.
        name: &'static str,
        /// SPDX identifier.
        licence: &'static str,
    },
    /// A model is not in the cache and downloading is unavailable or failed.
    #[error("{name} is not installed and could not be fetched: {reason}")]
    ModelUnavailable {
        /// Model name.
        name: &'static str,
        /// Why.
        reason: String,
    },
    /// No platform cache directory exists.
    #[error("this platform has no cache directory; set CHARCRAFT_MODELS_DIR")]
    NoCacheDir,
    /// Reading or writing a file failed.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The camera failed.
    #[error("camera: {0}")]
    Camera(String),
    /// The inference runtime failed or a model has an unexpected shape.
    #[error("inference: {0}")]
    Inference(String),
    /// The tracking actor has stopped.
    #[error("the tracking actor has stopped")]
    ActorGone,
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
