//! The pose: everything the behaviors decide for one frame.

use std::collections::{BTreeMap, BTreeSet};

use anim_core::{Affine, Point, Vec2};
use serde::{Deserialize, Serialize};

use crate::tags::Viseme;

/// The smallest scale magnitude kept, so a collapsed group still has an invertible matrix.
const MIN_SCALE: f64 = 1e-6;

/// A 2D transform about a group's origin: move, then rotate and scale about the origin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xform {
    /// Movement in the group's parent space (stage pixels at rest, y down).
    pub translation: Vec2,
    /// Rotation in radians, clockwise on screen (y down).
    pub rotation: f64,
    /// Scale factors.
    pub scale: Vec2,
}

impl Default for Xform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Xform {
    /// No change.
    pub const IDENTITY: Self = Self { translation: Vec2::ZERO, rotation: 0.0, scale: Vec2::new(1.0, 1.0) };

    /// A pure move.
    #[must_use]
    pub const fn moved(dx: f64, dy: f64) -> Self {
        Self { translation: Vec2::new(dx, dy), ..Self::IDENTITY }
    }

    /// A pure rotation in degrees.
    #[must_use]
    pub const fn rotated_degrees(degrees: f64) -> Self {
        Self { rotation: degrees.to_radians(), ..Self::IDENTITY }
    }

    /// True when applying it changes nothing (non-finite parts count as no change).
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.sanitized() == Self::IDENTITY
    }

    /// Replaces non-finite parts with the identity's and keeps scales away from zero: a scale of 0 (or nearly) is
    /// a collapsed group, so it becomes the smallest scale of the same sign, never "no scale".
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let ok = |v: f64, fallback: f64| if v.is_finite() { v } else { fallback };
        let scale = |v: f64| match v {
            v if !v.is_finite() => 1.0,
            v if v.abs() > MIN_SCALE => v,
            v => MIN_SCALE.copysign(v),
        };
        Self {
            translation: Vec2::new(ok(self.translation.x, 0.0), ok(self.translation.y, 0.0)),
            rotation: ok(self.rotation, 0.0),
            scale: Vec2::new(scale(self.scale.x), scale(self.scale.y)),
        }
    }

    /// Combines two changes: translations add, rotations add, scales multiply.
    #[must_use]
    pub fn then(&self, other: &Self) -> Self {
        let (a, b) = (self.sanitized(), other.sanitized());
        Self { translation: a.translation + b.translation, rotation: a.rotation + b.rotation, scale: Vec2::new(a.scale.x * b.scale.x, a.scale.y * b.scale.y) }
    }

    /// The matrix that turns rest-pose points of a group into posed ones, with `origin` (in the same space as the
    /// points) as the fixed point of the rotation and scale.
    #[must_use]
    pub fn about(&self, origin: Point) -> Affine {
        let s = self.sanitized();
        let to_origin = Affine::translate(origin.to_vec2());
        let from_origin = Affine::translate(-origin.to_vec2());
        Affine::translate(s.translation) * to_origin * Affine::rotate(s.rotation) * Affine::scale_non_uniform(s.scale.x, s.scale.y) * from_origin
    }
}

/// One frame of a puppet: face, body, dragging and visibility switches. The default is the neutral pose, which
/// renders exactly like the artwork.
///
/// All lengths are stage pixels of the puppet's artwork (y down). Amounts such as blink and eyebrows are
/// normalised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PuppetPose {
    /// Moves the whole puppet.
    pub root: Xform,
    /// The group tagged `Head`.
    pub head: Xform,
    /// Extra transforms for any independent group, keyed by one of its tag ids, its label (`left hand`, matched
    /// like tags are) or its layer path.
    pub groups: BTreeMap<String, Xform>,
    /// Left eyebrow raise, -1 (down) to 1 (up).
    pub eyebrow_left: f64,
    /// Right eyebrow raise, -1 (down) to 1 (up).
    pub eyebrow_right: f64,
    /// Where both eyes look, -1 to 1 on each axis (x right, y down); the pupils move within their eyes.
    pub gaze: Vec2,
    /// Overrides the left pupil's gaze.
    pub pupil_left: Option<Vec2>,
    /// Overrides the right pupil's gaze.
    pub pupil_right: Option<Vec2>,
    /// Left eye closed amount, 0 to 1; the blink layer shows from 0.5.
    pub blink_left: f64,
    /// Right eye closed amount, 0 to 1; the blink layer shows from 0.5.
    pub blink_right: f64,
    /// The mouth shape; `None` leaves the artwork's own (usually neutral).
    pub viseme: Option<Viseme>,
    /// Breathing, 0 to 1: grows layers tagged `Breathe`.
    pub breathe: f64,
    /// Where draggable, dangling and other handles are pulled to, as a displacement from their rest position in
    /// the handle's group space. Keys are [`crate::HandleDef::key`].
    pub drag: BTreeMap<String, Vec2>,
    /// Keys of the triggers that are on; layers with a key trigger show only while theirs is on.
    pub triggers: BTreeSet<String>,
    /// Chosen member of each swap set: set name to the member's layer label.
    pub swaps: BTreeMap<String, String>,
    /// Forces layers on or off, keyed by layer path or label.
    pub visibility: BTreeMap<String, bool>,
    /// Milliseconds since the performance began: what layer cycles run on. It is not part of what makes a pose
    /// neutral.
    #[serde(default)]
    pub time_millis: u64,
    /// Multiplies layers' opacity (0 to 1), keyed by layer path or label.
    #[serde(default)]
    pub opacity: BTreeMap<String, f64>,
    /// Extra copies of layers drawn on top of the puppet: particles, motion trails, shadows.
    #[serde(default)]
    pub sprites: Vec<Sprite>,
}

/// An extra copy of a layer, drawn at rest geometry and then moved by its own transform.
///
/// The layer is drawn even when the pose hides the original, so a behavior can hide the source artwork and show
/// only its copies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sprite {
    /// The layer to copy, by path or label.
    pub layer: String,
    /// Moves, rotates and scales the copy about the layer's origin.
    pub xform: Xform,
    /// 0 to 1.
    pub opacity: f64,
    /// 0 keeps the artwork's colours, 1 paints it black (shadows).
    #[serde(default)]
    pub shade: f64,
}

impl PuppetPose {
    /// The neutral pose.
    #[must_use]
    pub fn neutral() -> Self {
        Self::default()
    }

    /// A pose with a mouth shape.
    #[must_use]
    pub const fn with_viseme(mut self, viseme: Viseme) -> Self {
        self.viseme = Some(viseme);
        self
    }

    /// A pose with both eyes shut.
    #[must_use]
    pub const fn with_blink(mut self, amount: f64) -> Self {
        self.blink_left = amount;
        self.blink_right = amount;
        self
    }

    /// A pose with a head rotation in degrees.
    #[must_use]
    pub const fn with_head_rotation(mut self, degrees: f64) -> Self {
        self.head.rotation = degrees.to_radians();
        self
    }

    /// True when no field differs from [`Self::neutral`].
    #[must_use]
    pub fn is_neutral(&self) -> bool {
        Self { time_millis: 0, ..self.clone() } == Self::default()
    }

    /// The displacement for a handle key, zero when none is set or the value is not finite.
    #[must_use]
    pub fn drag_of(&self, key: &str) -> Vec2 {
        self.drag.get(key).copied().filter(|d| d.x.is_finite() && d.y.is_finite()).unwrap_or(Vec2::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_identity() {
        let p = PuppetPose::neutral();
        assert!(p.is_neutral());
        assert!(p.head.is_identity());
        assert_eq!(p.head.about(Point::new(5.0, 7.0)), Affine::IDENTITY);
    }

    #[test]
    fn rotation_keeps_the_origin_fixed() {
        let x = Xform::rotated_degrees(90.0);
        let origin = Point::new(10.0, 20.0);
        let m = x.about(origin);
        let moved = m * origin;
        assert!((moved.x - origin.x).abs() < 1e-9 && (moved.y - origin.y).abs() < 1e-9);
        // A point to the right of the origin ends up below it (clockwise on screen, y down).
        let p = m * Point::new(11.0, 20.0);
        assert!((p.x - 10.0).abs() < 1e-9 && (p.y - 21.0).abs() < 1e-9);
    }

    #[test]
    fn non_finite_parts_are_ignored() {
        let x = Xform { translation: Vec2::new(f64::NAN, 3.0), rotation: f64::INFINITY, scale: Vec2::new(f64::INFINITY, f64::NAN) };
        assert_eq!(x.sanitized(), Xform::moved(0.0, 3.0));
    }

    #[test]
    fn a_zero_scale_collapses_instead_of_vanishing() {
        let x = Xform { scale: Vec2::new(0.0, -0.0), ..Xform::IDENTITY }.sanitized();
        assert_eq!(x.scale, Vec2::new(MIN_SCALE, -MIN_SCALE));
        assert!(!Xform { scale: Vec2::new(0.0, 1.0), ..Xform::IDENTITY }.is_identity());
    }

    #[test]
    fn pose_survives_json() {
        let mut p = PuppetPose::neutral().with_viseme(Viseme::WOo).with_blink(1.0);
        p.drag.insert("hand/Dangle".into(), Vec2::new(3.0, -2.0));
        p.swaps.insert("hats".into(), "cap".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: PuppetPose = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
