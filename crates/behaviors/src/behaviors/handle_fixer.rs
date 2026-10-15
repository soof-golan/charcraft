//! Handle Fixer (`Adobe.HandleFixer.js`): pins `Fixed` handles.
//!
//! The pinning happens inside the puppet's warp, so the behavior itself has nothing to add; it exists so projects
//! that use it load without being reported as unsupported.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common};

/// Handle Fixer has no parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HandleFixerParams {}

const DOCS: &[ParamDoc] = &[];

/// Pins fixed handles (done by the puppet's warp).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HandleFixer {
    /// The parameters (none).
    pub params: HandleFixerParams,
}

impl Behavior for HandleFixer {
    behavior_common!("Adobe.HandleFixer.js", "Handle Fixer", DOCS);

    fn step(&mut self, _cx: &Cx<'_>, _pose: &mut PuppetPose) {}

    fn reset(&mut self, _seed: u64) {}
}
