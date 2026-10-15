//! The part of a puppet the behaviors need to know: the stage, where the eyes are and which handles exist.
//!
//! A [`Rig`] is plain data. [`Rig::from_puppet`] reads it off a built puppet; tests and tools can also build one
//! by hand, so behaviors never need the artwork itself.

use std::collections::BTreeSet;

use anim_core::{Affine, Point, Rect, Size, Vec2};
use charcraft_puppet::tags::{self, ids};
use charcraft_puppet::{HandleKind, LayerId, Puppet, PuppetPose, Xform};
use serde::{Deserialize, Serialize};

/// Handles are grabbed within this many stage pixels when the artwork gives no better idea.
pub const DEFAULT_GRAB_RADIUS: f64 = 40.0;

/// One independent group on the way from the puppet's root down to a handle.
///
/// It holds enough to recompute the transform the pose gives the group, with the same lookup the puppet's
/// evaluator uses: the root, the head, and the `pose.groups` entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainLink {
    /// This is the puppet's root, which `pose.root` moves.
    pub is_root: bool,
    /// This is the group tagged `Head`, which `pose.head` moves.
    pub is_head: bool,
    /// Tag ids of the group, matched exactly against `pose.groups` keys.
    pub tags: BTreeSet<String>,
    /// Layer path of the group, matched exactly.
    pub path: String,
    /// The group's label in normalised form ([`charcraft_puppet::tags::normalize`]).
    pub label: String,
    /// The group's origin, the fixed point of its rotation and scale.
    pub origin: Point,
}

impl ChainLink {
    /// A link that only answers to a `pose.groups` key.
    #[must_use]
    pub fn named(key: &str, origin: Point) -> Self {
        Self { is_root: false, is_head: false, tags: BTreeSet::from([key.to_owned()]), path: key.to_owned(), label: tags::normalize(key), origin }
    }

    /// The change the pose makes to this group.
    #[must_use]
    pub fn xform(&self, pose: &PuppetPose) -> Xform {
        let mut x = Xform::IDENTITY;
        if self.is_root {
            x = x.then(&pose.root);
        }
        if self.is_head {
            x = x.then(&pose.head);
        }
        for (key, xf) in &pose.groups {
            if self.tags.contains(key) || self.path == *key || (!self.label.is_empty() && tags::normalize(key) == self.label) {
                x = x.then(xf);
            }
        }
        x
    }
}

/// A handle that behaviors can pull on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigHandle {
    /// `<group label>/<handle name>`, the key of [`PuppetPose::drag`].
    pub key: String,
    /// What the handle is for.
    pub kind: HandleKind,
    /// Rest position in stage pixels.
    pub position: Point,
    /// Farthest sensible displacement in stage pixels (the length of the dangling part, or the group's size).
    pub reach: f64,
    /// How close a pointer has to be to grab it.
    pub grab_radius: f64,
    /// The independent groups from the root down to the handle's own group, outermost first.
    pub chain: Vec<ChainLink>,
}

impl RigHandle {
    /// A handle with no ancestors (it only moves when the pose moves its own group).
    #[must_use]
    pub fn new(key: &str, kind: HandleKind, position: Point) -> Self {
        Self { key: key.to_owned(), kind, position, reach: 100.0, grab_radius: DEFAULT_GRAB_RADIUS, chain: Vec::new() }
    }

    /// The matrix the pose gives the handle's group, including every enclosing group.
    #[must_use]
    pub fn matrix(&self, pose: &PuppetPose) -> Affine {
        self.chain.iter().fold(Affine::IDENTITY, |acc, link| {
            let x = link.xform(pose);
            if x.is_identity() { acc } else { acc * x.about(link.origin) }
        })
    }

    /// Where the handle is in the pose before any drag: its rest position through the group matrices.
    #[must_use]
    pub fn driven_position(&self, pose: &PuppetPose) -> Point {
        self.matrix(pose) * self.position
    }

    /// Turns a displacement on stage into the displacement in the group's own rest space, which is what
    /// [`PuppetPose::drag`] holds. Zero when the group matrix cannot be inverted.
    #[must_use]
    pub fn to_group_space(&self, pose: &PuppetPose, stage_delta: Vec2) -> Vec2 {
        let m = self.matrix(pose);
        let det = m.determinant();
        if !det.is_finite() || det.abs() < 1e-9 {
            return Vec2::ZERO;
        }
        let inv = m.inverse();
        let base = self.driven_position(pose);
        let v = inv * (base + stage_delta) - inv * base;
        if v.x.is_finite() && v.y.is_finite() { v } else { Vec2::ZERO }
    }
}

/// What the behaviors need to know about a puppet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rig {
    /// The artwork size in stage pixels.
    pub stage: Size,
    /// Where the eyes are (the point eye gaze looks out from).
    pub eyes_center: Point,
    /// Handles that dragging and physics can move.
    pub handles: Vec<RigHandle>,
    /// Every tag id any layer has.
    pub tags: BTreeSet<String>,
}

impl Default for Rig {
    fn default() -> Self {
        Self::new(Size::new(1920.0, 1080.0))
    }
}

impl Rig {
    /// A rig with a stage and nothing on it. The eyes sit at the middle of the stage.
    #[must_use]
    pub fn new(stage: Size) -> Self {
        Self { stage, eyes_center: Point::new(stage.width / 2.0, stage.height / 2.0), handles: Vec::new(), tags: BTreeSet::new() }
    }

    /// Adds a handle.
    #[must_use]
    pub fn with_handle(mut self, handle: RigHandle) -> Self {
        self.handles.push(handle);
        self
    }

    /// Reads the rig off a puppet.
    #[must_use]
    pub fn from_puppet(puppet: &Puppet) -> Self {
        let mut rig = Self::new(puppet.size);
        for layer in puppet.layers() {
            rig.tags.extend(layer.tags.iter().cloned());
        }
        let eyes: Vec<Rect> = [ids::LEFT_EYE, ids::RIGHT_EYE].iter().flat_map(|t| puppet.with_tag(t)).filter_map(|id| puppet.layer(id)?.bounds).collect();
        let mut union = eyes.iter().copied();
        if let Some(first) = union.next() {
            rig.eyes_center = union.fold(first, |a, b| a.union(b)).center();
        } else if let Some(b) = puppet.with_tag(ids::HEAD).into_iter().find_map(|id| puppet.layer(id)?.bounds) {
            rig.eyes_center = Point::new(b.center().x, b.y0 + b.height() * 0.4);
        }
        for layer in puppet.layers().iter().filter(|l| !l.handles.is_empty()) {
            let chain = chain_to(puppet, layer.id);
            let size = layer.bounds.map_or(100.0, |b| b.width().max(b.height()));
            for h in &layer.handles {
                let reach = match h.kind {
                    HandleKind::Dangle => (h.position - layer.origin).hypot().max(size * 0.25),
                    _ => size,
                };
                rig.handles.push(RigHandle {
                    key: h.key.clone(),
                    kind: h.kind,
                    position: h.position,
                    reach: if reach.is_finite() && reach > 0.0 { reach } else { 100.0 },
                    grab_radius: DEFAULT_GRAB_RADIUS,
                    chain: chain.clone(),
                });
            }
        }
        rig
    }

    /// Whether any layer has the tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// The handles of one kind.
    pub fn handles_of(&self, kind: HandleKind) -> impl Iterator<Item = &RigHandle> {
        self.handles.iter().filter(move |h| h.kind == kind)
    }

    /// The shorter side of the stage, the unit several behaviors scale by.
    #[must_use]
    pub fn unit(&self) -> f64 {
        let u = self.stage.width.min(self.stage.height);
        if u.is_finite() && u > 1.0 { u } else { 1000.0 }
    }
}

fn chain_to(puppet: &Puppet, id: LayerId) -> Vec<ChainLink> {
    let mut links = Vec::new();
    let mut at = Some(id);
    let mut guard = 0_u32;
    while let Some(current) = at {
        guard = guard.saturating_add(1);
        let Some(layer) = puppet.layer(current) else { break };
        if guard > 256 {
            break;
        }
        if layer.independent || current == LayerId::ROOT {
            links.push(ChainLink {
                is_root: current == LayerId::ROOT,
                is_head: layer.has_tag(ids::HEAD),
                tags: layer.tags.clone(),
                path: layer.path.clone(),
                label: tags::normalize(&layer.label),
                origin: layer.origin,
            });
        }
        at = layer.parent;
    }
    links.reverse();
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle_in_rotating_head() -> RigHandle {
        let head =
            ChainLink { is_root: false, is_head: true, tags: BTreeSet::new(), path: "Head".into(), label: "head".into(), origin: Point::new(100.0, 100.0) };
        RigHandle { chain: vec![head], position: Point::new(200.0, 100.0), ..RigHandle::new("head/Tip", HandleKind::Dangle, Point::new(200.0, 100.0)) }
    }

    #[test]
    fn neutral_pose_leaves_handles_in_place() {
        let h = handle_in_rotating_head();
        assert_eq!(h.driven_position(&PuppetPose::neutral()), h.position);
    }

    #[test]
    fn a_rotated_head_carries_its_handle() {
        let h = handle_in_rotating_head();
        let pose = PuppetPose::neutral().with_head_rotation(90.0);
        let p = h.driven_position(&pose);
        assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 200.0).abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn stage_displacements_are_turned_into_group_space() {
        let h = handle_in_rotating_head();
        let pose = PuppetPose::neutral().with_head_rotation(90.0);
        // The head is turned a quarter clockwise: a downward move on stage is a move to the right in the group.
        let local = h.to_group_space(&pose, Vec2::new(0.0, 10.0));
        assert!((local.x - 10.0).abs() < 1e-9 && local.y.abs() < 1e-9, "{local:?}");
    }

    #[test]
    fn groups_are_matched_by_tag_label_and_path() {
        let link = ChainLink {
            is_root: false,
            is_head: false,
            tags: BTreeSet::from(["Adobe.Face.LeftEyebrow".to_owned()]),
            path: "Face/brow".into(),
            label: "leftbrow".into(),
            origin: Point::ORIGIN,
        };
        for key in ["Adobe.Face.LeftEyebrow", "Face/brow", "Left Brow"] {
            let mut pose = PuppetPose::neutral();
            pose.groups.insert(key.to_owned(), Xform::moved(1.0, 2.0));
            assert_eq!(link.xform(&pose), Xform::moved(1.0, 2.0), "{key}");
        }
    }
}
