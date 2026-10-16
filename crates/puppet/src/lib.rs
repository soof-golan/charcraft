//! The `CharCraft` puppet: Character Animator's puppet model and its evaluation.
//!
//! A [`Puppet`] is built from imported artwork ([`Puppet::from_artwork`]), optionally together with a project's
//! data ([`Puppet::from_project`]): the layer tree with independent groups (names starting with `+`), the tags
//! names give (`Head`, `Left Eye`, the mouth shapes, `Draggable`, ...), handles, swap sets and triggers.
//! [`Puppet::evaluate`] turns a [`PuppetPose`] into an [`anim_display::DisplayList`]: independent groups are
//! transformed about their origins, groups with pinning handles are warped with ARAP meshes
//! ([`anim_rig`]), and mouth shapes, blinks and swaps are chosen by visibility. The neutral pose draws the
//! artwork unchanged.
//!
//! For tools, [`PuppetOp`] (edits), [`LayerInfo`], [`Puppet::summary`] and [`PuppetPose`] are plain serde values
//! addressed by layer path; the [`api`] module has the tool table an MCP server maps them to.
//!
//! ```no_run
//! use anim_artwork::{import_path, Limits};
//! use charcraft_puppet::{Puppet, PuppetPose, Viseme};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let tree = import_path("boy.ai".as_ref(), &Limits::default())?;
//! let puppet = Puppet::from_artwork(&tree)?;
//! let list = puppet.evaluate(&PuppetPose::neutral().with_viseme(Viseme::Ah).with_blink(1.0));
//! // anim_render::render(&list, puppet.images(), &options) draws it.
//! # let _ = list;
//! # Ok(())
//! # }
//! ```

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: poses must be bit-identical on every CPU")]
#![cfg_attr(test, allow(clippy::arithmetic_side_effects, clippy::assert_is_empty, clippy::many_single_char_names, reason = "test code"))]

pub mod api;
mod build;
mod error;
mod eval;
mod model;
mod pose;
mod project;
mod silhouette;
mod summary;
pub mod tags;
mod warp;

pub use api::{LayerInfo, OpOutcome, PuppetOp};
pub use build::MAX_LAYERS;
pub use error::{Error, Result};
pub use model::{AttachStyle, CycleDef, CycleMode, HandleDef, HandleKind, Layer, LayerId, LayerKind, MESH_DENSITY, Puppet, STICK_PINS, SwapSet};
pub use pose::{PuppetPose, Xform};
pub use summary::{GroupSummary, PuppetSummary};
pub use tags::{TagKind, Viseme};
