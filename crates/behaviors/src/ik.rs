//! Two-bone inverse kinematics in the plane: an upper and a lower bone from a root towards a target.

use anim_core::Vec2;

/// The pose of a two-bone limb as the absolute directions of its bones, radians, clockwise on a y-down screen
/// (the direction of the vector `(1, 0)` is 0, straight down is a quarter turn).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limb {
    /// Direction of the first bone (shoulder to elbow, hip to knee).
    pub upper: f64,
    /// Direction of the second bone (elbow to wrist, knee to ankle).
    pub lower: f64,
}

impl Limb {
    /// The elbow (knee) and wrist (ankle) positions for a root and the bone lengths.
    #[must_use]
    pub fn joints(&self, root: Vec2, upper_len: f64, lower_len: f64) -> (Vec2, Vec2) {
        let mid = root + Vec2::new(self.upper.cos(), self.upper.sin()) * upper_len;
        (mid, mid + Vec2::new(self.lower.cos(), self.lower.sin()) * lower_len)
    }
}

/// The direction of a vector, radians, clockwise on a y-down screen.
#[must_use]
pub fn direction(v: Vec2) -> f64 {
    v.y.atan2(v.x)
}

/// An angle wrapped to `(-pi, pi]`.
#[must_use]
pub fn wrap(angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    if !angle.is_finite() {
        return 0.0;
    }
    let a = angle.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

/// Which way the joint between two bones bends, +1 or -1: the side the middle joint lies on, seen from the root
/// towards the end. `fallback` is returned when the limb is straight.
#[must_use]
pub fn bend_side(root: Vec2, mid: Vec2, end: Vec2, fallback: f64) -> f64 {
    let cross = (mid.x - root.x) * (end.y - root.y) - (mid.y - root.y) * (end.x - root.x);
    if cross.is_finite() && cross.abs() > 1e-6 { -cross.signum() } else { fallback }
}

/// Solves a limb of two bones, `upper_len` and `lower_len` long, that reaches from `root` towards `target`.
///
/// * `bend` is +1 or -1: which way the joint folds (see [`bend_side`]).
/// * `min_bend` is the least the joint stays folded, radians: 0 lets the limb lock straight, which looks like a
///   hyperextended elbow, so rigs use a few degrees.
///
/// A target out of reach is approached along the straight line (the bones do not stretch); a target too close
/// folds the limb as far as the geometry allows.
#[must_use]
pub fn solve(root: Vec2, target: Vec2, upper_len: f64, lower_len: f64, bend: f64, min_bend: f64) -> Limb {
    let (l1, l2) = (upper_len.max(1e-6), lower_len.max(1e-6));
    let to_target = target - root;
    let finite = to_target.x.is_finite() && to_target.y.is_finite();
    let to_target = if finite { to_target } else { Vec2::new(0.0, 1.0) };
    let base = direction(to_target);
    // The farthest the wrist gets while the joint stays folded by `min_bend`, and the nearest (bones folded back).
    let farthest = (l1 * l1 + l2 * l2 + 2.0 * l1 * l2 * min_bend.clamp(0.0, 1.5).cos()).sqrt();
    let nearest = (l1 - l2).abs() + 1e-6;
    let reach = if finite { to_target.hypot().clamp(nearest, farthest.max(nearest)) } else { farthest.max(nearest) };
    // The angle at the root between the target direction and the upper bone.
    let cos_root = ((l1 * l1 + reach * reach - l2 * l2) / (2.0 * l1 * reach)).clamp(-1.0, 1.0);
    let upper = base + bend.signum() * cos_root.acos();
    let mid = root + Vec2::new(upper.cos(), upper.sin()) * l1;
    let end = root + Vec2::new(base.cos(), base.sin()) * reach;
    let lower = direction(end - mid);
    Limb { upper, lower: if lower.is_finite() { lower } else { upper } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).hypot() < 1e-6
    }

    #[test]
    fn the_wrist_lands_on_a_reachable_target() {
        let root = Vec2::new(1.0, 2.0);
        for (tx, ty) in [(0.6, 0.6), (0.0, 1.0), (-0.9, 0.2), (0.3, -0.8)] {
            let target = root + Vec2::new(tx, ty);
            for bend in [1.0, -1.0] {
                let limb = solve(root, target, 0.6, 0.6, bend, 0.0);
                let (_, end) = limb.joints(root, 0.6, 0.6);
                assert!(near(end, target), "{target:?} bend {bend}: {end:?}");
            }
        }
    }

    #[test]
    fn the_elbow_folds_to_the_side_it_was_seen_on() {
        // Shoulder at the origin, wrist straight below, elbow bulging to the right (positive x).
        let (root, mid, end) = (Vec2::new(0.0, 0.0), Vec2::new(0.3, 0.5), Vec2::new(0.0, 0.8));
        let bend = bend_side(root, mid, end, 1.0);
        let limb = solve(root, end, 0.583_095, 0.583_095, bend, 0.0);
        let (elbow, wrist) = limb.joints(root, 0.583_095, 0.583_095);
        assert!(elbow.x > 0.0, "{elbow:?}");
        assert!(near(wrist, end));
        let other = solve(root, end, 0.583_095, 0.583_095, -bend, 0.0).joints(root, 0.583_095, 0.583_095).0;
        assert!(other.x < 0.0, "{other:?}");
    }

    #[test]
    fn a_target_out_of_reach_stretches_the_limb_straight() {
        let limb = solve(Vec2::ZERO, Vec2::new(0.0, 5.0), 0.5, 0.5, 1.0, 0.0);
        let (mid, end) = limb.joints(Vec2::ZERO, 0.5, 0.5);
        assert!(near(mid, Vec2::new(0.0, 0.5)) && near(end, Vec2::new(0.0, 1.0)), "{mid:?} {end:?}");
    }

    #[test]
    fn a_minimum_bend_keeps_the_limb_from_locking() {
        let limb = solve(Vec2::ZERO, Vec2::new(0.0, 5.0), 0.5, 0.5, 1.0, 0.2);
        let (_, end) = limb.joints(Vec2::ZERO, 0.5, 0.5);
        assert!(end.hypot() < 0.999, "{end:?}");
        assert!(wrap(limb.lower - limb.upper).abs() > 0.15);
    }

    #[test]
    fn hostile_targets_give_finite_angles() {
        for target in [Vec2::ZERO, Vec2::new(f64::NAN, 1.0), Vec2::new(f64::INFINITY, 0.0)] {
            let limb = solve(Vec2::ZERO, target, 0.5, 0.5, 1.0, 0.0);
            assert!(limb.upper.is_finite() && limb.lower.is_finite(), "{target:?}");
        }
        assert!((wrap(3.0 * std::f64::consts::PI) - std::f64::consts::PI).abs() < 1e-9);
    }
}
