//! Transform (`Adobe.Transform.js`): a fixed move, turn and scale of the group tagged `Transform`.

use anim_core::Vec2;
use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{add_group, finite_or, percent};

/// Parameters of Transform, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "mirrors the stored parameter table")]
pub struct TransformParams {
    /// Pivot offset from the group's origin, x.
    #[serde(rename = "anchorPtX")]
    pub anchor_pt_x: f64,
    /// Pivot offset from the group's origin, y.
    #[serde(rename = "anchorPtY")]
    pub anchor_pt_y: f64,
    /// Apply the position.
    #[serde(rename = "applyPosition")]
    pub apply_position: bool,
    /// Apply the rotation.
    #[serde(rename = "applyRotation")]
    pub apply_rotation: bool,
    /// Apply the scale.
    #[serde(rename = "applyScale")]
    pub apply_scale: bool,
    /// Apply the skew (stored; skew is not representable in a pose).
    #[serde(rename = "applySkew")]
    pub apply_skew: bool,
    /// Opacity affects the whole group (stored, opacity is not part of a pose).
    #[serde(rename = "groupOpacity")]
    pub group_opacity: bool,
    /// Opacity, percent (stored, no effect).
    pub opacity: f64,
    /// Horizontal move, stage pixels.
    #[serde(rename = "positionX")]
    pub position_x: f64,
    /// Vertical move, stage pixels.
    #[serde(rename = "positionY")]
    pub position_y: f64,
    /// Rotation, degrees clockwise.
    pub rotation: f64,
    /// Horizontal scale, percent.
    #[serde(rename = "scaleX")]
    pub scale_x: f64,
    /// Vertical scale, percent.
    #[serde(rename = "scaleY")]
    pub scale_y: f64,
    /// Skew, degrees (stored, no effect).
    pub skew: f64,
    /// Skew axis, degrees (stored, no effect).
    #[serde(rename = "skewAxis")]
    pub skew_axis: f64,
    /// How much of the transform applies, percent.
    pub strength: f64,
    /// Scale of both axes, percent.
    #[serde(rename = "uniformScale")]
    pub uniform_scale: f64,
}

impl Default for TransformParams {
    fn default() -> Self {
        Self {
            anchor_pt_x: 0.0,
            anchor_pt_y: 0.0,
            apply_position: true,
            apply_rotation: true,
            apply_scale: true,
            apply_skew: true,
            group_opacity: false,
            opacity: 100.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
            skew: 0.0,
            skew_axis: 0.0,
            strength: 100.0,
            uniform_scale: 100.0,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "positionX", "px", "Horizontal move of the group tagged Transform.";
    "positionY", "px", "Vertical move.";
    "rotation", "deg", "Clockwise rotation about the anchor.";
    "scaleX", "%", "Horizontal scale about the anchor.";
    "scaleY", "%", "Vertical scale about the anchor.";
    "uniformScale", "%", "Scale of both axes, multiplied with scaleX and scaleY.";
    "anchorPtX", "px", "Pivot offset from the group's origin.";
    "anchorPtY", "px", "Pivot offset from the group's origin.";
    "strength", "%", "Blend between no change and the full transform.";
    "applyPosition", "", "Switch for the position.";
    "applyRotation", "", "Switch for the rotation.";
    "applyScale", "", "Switch for the scale.";
};

/// Moves, turns and scales the group tagged [`ids::TRANSFORM`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transform {
    /// The parameters.
    pub params: TransformParams,
}

impl Transform {
    /// The change this behavior makes to the group (about the group's own origin).
    #[must_use]
    pub fn xform(&self) -> Xform {
        let p = &self.params;
        let s = percent(p.strength);
        let lerp = |one: f64, v: f64| one + (finite_or(v, one) - one) * s;
        let translation = if p.apply_position { Vec2::new(finite_or(p.position_x, 0.0), finite_or(p.position_y, 0.0)) * s } else { Vec2::ZERO };
        let rotation = if p.apply_rotation { lerp(0.0, p.rotation).to_radians() } else { 0.0 };
        let scale = if p.apply_scale {
            let u = percent(p.uniform_scale);
            Vec2::new(lerp(1.0, percent(p.scale_x) * u), lerp(1.0, percent(p.scale_y) * u))
        } else {
            Vec2::new(1.0, 1.0)
        };
        let base = Xform { translation, rotation, scale }.sanitized();
        // Rotating and scaling about an anchor away from the origin is the same turn about the origin plus a shift.
        let anchor = Vec2::new(finite_or(p.anchor_pt_x, 0.0), finite_or(p.anchor_pt_y, 0.0));
        if anchor == Vec2::ZERO {
            return base;
        }
        let (sin, cos) = base.rotation.sin_cos();
        let scaled = Vec2::new(anchor.x * base.scale.x, anchor.y * base.scale.y);
        let turned = Vec2::new(scaled.x * cos - scaled.y * sin, scaled.x * sin + scaled.y * cos);
        Xform { translation: base.translation + anchor - turned, ..base }
    }
}

impl Behavior for Transform {
    behavior_common!("Adobe.Transform.js", "Transform", DOCS);

    fn step(&mut self, _cx: &Cx<'_>, pose: &mut PuppetPose) {
        let x = self.xform();
        if !x.is_identity() {
            add_group(pose, ids::TRANSFORM, &x);
        }
    }

    fn reset(&mut self, _seed: u64) {}
}
