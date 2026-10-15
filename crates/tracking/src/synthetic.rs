//! Synthetic landmarks: a parametric face built in `MediaPipe`'s landmark layout.
//!
//! This is what tests, demos and machines without a camera use: [`SyntheticFace`] places the landmarks the maths
//! reads (the other mesh points sit at the head's centre), poses and expresses them, and projects them into a
//! frame. [`demo_faces`] animates one into a short talking, blinking, head-turning loop.

use crate::cast::{f, fu};
use crate::landmarks::{Landmarks, MESH_POINTS_WITH_IRIS, V3, idx};

/// A face described by head pose and expression, ready to turn into [`Landmarks`].
///
/// Lengths are in eye distances (the distance between the outer eye corners is 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntheticFace {
    /// Head turn, radians, positive toward the image's right.
    pub yaw: f32,
    /// Head nod, radians, positive looking up.
    pub pitch: f32,
    /// Head tilt, radians, positive clockwise in the image.
    pub roll: f32,
    /// Where the eye midpoint is in the frame: x right and y up, both -1 to 1.
    pub center: [f32; 2],
    /// Distance between the outer eye corners, in pixels.
    pub eye_distance_px: f32,
    /// Frame size in pixels.
    pub frame: [u32; 2],
    /// How far each eyebrow (image-left, image-right) is raised.
    pub brow_raise: [f32; 2],
    /// How open each eye (image-left, image-right) is: 1 is the neutral open eye, 0 is shut.
    pub eye_open: [f32; 2],
    /// Where the pupils look: x toward image right, y up, each -1 to 1.
    pub gaze: [f32; 2],
    /// Gap between the inner lips.
    pub mouth_open: f32,
    /// How far the mouth corners are raised.
    pub smile: f32,
    /// Mouth width as a multiple of the neutral width.
    pub mouth_wide: f32,
    /// The score the landmarks report.
    pub score: f32,
}

impl Default for SyntheticFace {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            center: [0.0, 0.0],
            eye_distance_px: 120.0,
            frame: [640, 480],
            brow_raise: [0.0; 2],
            eye_open: [1.0; 2],
            gaze: [0.0; 2],
            mouth_open: 0.0,
            smile: 0.0,
            mouth_wide: 1.0,
            score: 1.0,
        }
    }
}

impl SyntheticFace {
    /// The landmarks of this face in its frame.
    #[must_use]
    pub fn landmarks(&self) -> Landmarks {
        let head_centre = V3([0.0, 0.4, 0.4]);
        let mut points = vec![head_centre; MESH_POINTS_WITH_IRIS];
        let mut set = |i: usize, p: [f32; 3]| {
            if let Some(slot) = points.get_mut(i) {
                *slot = V3(p);
            }
        };

        // Eyes: corners and lids. A lid pair is `height` apart when the eye is fully open.
        // `EYE_L_LIDS` runs outer to inner and `EYE_R_LIDS` inner to outer, so each list gets its own x offsets.
        let eyes = [
            (idx::EYE_L_OUTER, idx::EYE_L_INNER, idx::EYE_L_LIDS, -1.0_f32, self.eye_open[0], [0.42, 0.35, 0.28]),
            (idx::EYE_R_OUTER, idx::EYE_R_INNER, idx::EYE_R_LIDS, 1.0, self.eye_open[1], [0.28, 0.35, 0.42]),
        ];
        for (outer, inner, lids, sign, open, xs) in eyes {
            set(outer, [sign * 0.5, 0.0, 0.0]);
            set(inner, [sign * 0.2, 0.0, 0.03]);
            for (((up, low), x), half) in lids.into_iter().zip(xs).zip([0.035, 0.045, 0.035]) {
                set(up, [sign * x, -half * open, 0.0]);
                set(low, [sign * x, half * open, 0.0]);
            }
        }
        let iris_reach = 0.12;
        set(idx::IRIS_L, [-0.35 + self.gaze[0] * iris_reach, -self.gaze[1] * iris_reach, -0.02]);
        set(idx::IRIS_R, [0.35 + self.gaze[0] * iris_reach, -self.gaze[1] * iris_reach, -0.02]);

        // Eyebrows: `BROW_L` runs outer to inner and `BROW_R` inner to outer.
        let brows = [
            (idx::BROW_L, -1.0_f32, self.brow_raise[0], [0.55, 0.45, 0.35, 0.25, 0.17]),
            (idx::BROW_R, 1.0, self.brow_raise[1], [0.17, 0.25, 0.35, 0.45, 0.55]),
        ];
        for (brow, sign, raise, xs) in brows {
            for (i, x) in brow.into_iter().zip(xs) {
                set(i, [sign * x, -0.2 - raise, 0.03]);
            }
        }

        // Mouth.
        let corner_y = 0.95 - self.smile;
        set(idx::MOUTH_L, [-0.4 * self.mouth_wide, corner_y, 0.1]);
        set(idx::MOUTH_R, [0.4 * self.mouth_wide, corner_y, 0.1]);
        set(idx::LIP_UPPER_INNER, [0.0, 0.93, 0.12]);
        set(idx::LIP_LOWER_INNER, [0.0, 0.95 + self.mouth_open, 0.12]);

        // Face shape: nose, forehead, chin and the cheek edges that fix the head's axes.
        set(idx::NOSE_TIP, [0.0, 0.45, -0.25]);
        set(idx::FOREHEAD, [0.0, -0.6, 0.1]);
        set(idx::CHIN, [0.0, 1.35, 0.05]);
        set(idx::CHEEK_L, [-0.75, 0.4, 0.55]);
        set(idx::CHEEK_R, [0.75, 0.4, 0.55]);

        let rot = rotation(-self.yaw, self.pitch, self.roll);
        let origin = V3([f32::midpoint(self.center[0], 1.0) * fu(self.frame[0]), f32::midpoint(-self.center[1], 1.0) * fu(self.frame[1]), 0.0]);
        for p in &mut points {
            let rotated = apply(&rot, p.minus(head_centre)).plus(head_centre);
            *p = origin.plus(rotated.scale(self.eye_distance_px));
        }
        Landmarks { points, frame_width: self.frame[0], frame_height: self.frame[1], score: self.score }
    }
}

type Matrix = [[f32; 3]; 3];

/// `Rz(gamma) * Rx(alpha) * Ry(beta)`.
fn rotation(beta: f32, alpha: f32, gamma: f32) -> Matrix {
    let (sb, cb) = beta.sin_cos();
    let (sa, ca) = alpha.sin_cos();
    let (sg, cg) = gamma.sin_cos();
    [[cg * cb - sg * sa * sb, -sg * ca, cg * sb + sg * sa * cb], [sg * cb + cg * sa * sb, cg * ca, sg * sb - cg * sa * cb], [-ca * sb, sa, ca * cb]]
}

fn apply(m: &Matrix, v: V3) -> V3 {
    let row = |r: &[f32; 3]| r[0] * v.x() + r[1] * v.y() + r[2] * v.z();
    V3([row(&m[0]), row(&m[1]), row(&m[2])])
}

/// A short loop of a face talking, blinking, glancing and turning its head, `count` frames long (one full cycle).
#[must_use]
pub fn demo_faces(count: usize) -> Vec<SyntheticFace> {
    let n = f(count.max(1));
    (0..count)
        .map(|i| {
            let t = f(i) / n * std::f32::consts::TAU;
            let talk = (t * 6.0).sin().max(0.0);
            // A blink near the start of each half cycle.
            let blink = if (t * 2.0).rem_euclid(std::f32::consts::TAU) < 0.25 { 0.0 } else { 1.0 };
            SyntheticFace {
                yaw: 0.35 * t.sin(),
                pitch: 0.12 * (t * 2.0).sin(),
                roll: 0.1 * (t * 3.0).sin(),
                center: [0.15 * t.sin(), 0.05 * t.cos()],
                brow_raise: [0.08 * (t * 2.0).sin().max(0.0), 0.08 * (t * 2.0).sin().max(0.0)],
                eye_open: [blink, blink],
                gaze: [0.6 * t.sin(), 0.2 * t.cos()],
                mouth_open: 0.3 * talk,
                smile: 0.06 * (t).cos().max(0.0),
                ..SyntheticFace::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_full_mesh_of_finite_points() {
        let l = SyntheticFace::default().landmarks();
        assert_eq!(l.points.len(), MESH_POINTS_WITH_IRIS);
        assert!(l.points.iter().all(|p| p.is_finite()));
        assert!(l.has_irises());
    }

    #[test]
    fn frontal_eyes_are_level_and_one_eye_distance_apart() {
        let l = SyntheticFace::default().landmarks();
        let (a, b) = (l.get(idx::EYE_L_OUTER).unwrap(), l.get(idx::EYE_R_OUTER).unwrap());
        assert!((b.x() - a.x() - 120.0).abs() < 1e-3);
        assert!((b.y() - a.y()).abs() < 1e-3);
    }

    #[test]
    fn demo_loop_is_finite_and_varied() {
        let faces = demo_faces(60);
        assert_eq!(faces.len(), 60);
        assert!(faces.iter().all(|f| f.landmarks().points.iter().all(|p| p.is_finite())));
        assert!(faces.iter().any(|f| f.eye_open[0] == 0.0), "blinks");
        assert!(faces.iter().any(|f| f.mouth_open > 0.2), "talks");
        assert!(demo_faces(0).is_empty());
    }
}
