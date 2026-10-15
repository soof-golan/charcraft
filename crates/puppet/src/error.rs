//! Errors of the puppet crate.

/// What can go wrong building a puppet.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The artwork has no layers.
    #[error("the artwork has no layers")]
    Empty,
    /// The artwork has more layers than the importer accepts.
    #[error("the artwork has more than {0} layers")]
    TooManyLayers(usize),
    /// The artwork could not be imported.
    #[error("artwork: {0}")]
    Artwork(#[from] anim_artwork::ArtworkError),
    /// The project could not be read.
    #[error("project: {0}")]
    Project(#[from] charcraft_project::Error),
    /// The project has no such puppet.
    #[error("the project has no puppet {0}")]
    NoPuppet(String),
    /// The puppet's artwork file is not in the project.
    #[error("the artwork file {0} is not in the project")]
    MissingArtwork(String),
}

/// Result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;
