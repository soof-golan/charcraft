//! Character Animator projects and puppets as a typed model, saved back losslessly.
//!
//! A `.puppet` zip and a project folder hold the same three things: a git-shaped store (`Ch Data/repo.noindex`,
//! read through [`charcraft_rope`]), the media next to it (`Ch Media/`) and, for a `.puppet`, an `_archive_info`
//! manifest. [`Project::open`] reads any of them (the zip, the folder, or the `.chproj` marker inside a folder).
//!
//! ```no_run
//! use charcraft_project::Project;
//!
//! # fn main() -> Result<(), charcraft_project::Error> {
//! let project = Project::open("Boy.puppet")?;
//! let model = project.model()?;
//! for puppet in model.root_puppets() {
//!     println!("{}: {} layers", puppet.display_name(), puppet.bindings.len());
//!     for behavior in &puppet.behaviors {
//!         println!("  {}", behavior.behavior);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Lossless saving
//!
//! The typed [`Model`] is a read-only view. What is saved is the rope log (the bytes read, plus new records
//! appended the way Character Animator appends them), the media (copied entry by entry, names, order and
//! timestamps kept) and the manifest (verbatim unless edited). Anything this crate does not understand therefore
//! survives a save untouched. Edits go through [`Project::commit`] and the helpers built on it
//! ([`Project::rename_puppet`], [`Project::set_behavior_param`], [`Project::set_layer_tag`]); they rewrite only the
//! blobs they change, with [`ClassedDoc`] keeping every class name.
//!
//! # For tools
//!
//! Everything in [`Model`] and [`ProjectSummary`] is `Serialize`, so a tool server (MCP or otherwise) can return
//! them as JSON as they are.

mod classed;
mod error;
mod manifest;
mod model;
mod package;
mod project;
mod scaffold;

pub use classed::{ClassedDoc, class_of};
pub use error::{Error, Result};
pub use manifest::{ArchiveInfo, BehaviorDefaults};
pub use model::{
    BehaviorDef, BehaviorInstance, Binding, BindingKind, BoundBehavior, Bounds, Handle, HotTake, KeyTrigger, LayerKind, LayerNode, Location, Marker, MediaFile,
    MediaItem, MediaKind, Model, ParamDef, PuppetItem, RecordingFile, RecordingRef, Scene, Tag, TagDef, Take, TimeItem, Track, TrackItem, Transform, Trigger,
    TriggerGroup,
};
pub use model::{behavior, media, puppet, scene};
pub use package::{Package, PackageEntry};
pub use project::{Author, Change, MediaSummary, Project, ProjectKind, ProjectSummary, PuppetSummary, SceneSummary};
pub use scaffold::{DEFAULT_BUILD, build_branch, make_guid, minimal_puppet};
