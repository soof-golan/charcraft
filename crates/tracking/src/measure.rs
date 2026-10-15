//! Raw face measurements from landmarks.
//!
//! Everything is measured in the head's own frame (its right, down and back axes, found from the landmarks), so
//! expressions read the same while the head turns, and divided by the distance between the outer eye corners, so
//! they do not depend on how far the subject sits from the camera. The numbers are raw: [`crate::solver`] maps them
//! to the 0 to 1 and -1 to 1 ranges the puppet wants, against a calibrated neutral face.

use serde::{Deserialize, Serialize};

use crate::landmarks::{Landmarks, V3, idx};

/// Head orientation in radians.
///
/// Conventions (camera image, un-mirrored): `yaw` is positive when the face turns toward the image's right, `pitch`
/// positive when the head looks up, `roll` positive when the head tilts clockwise as seen in the image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Orientation {
    /// Turn left/right.
    pub yaw: f32,
    /// Nod up/down.
    pub pitch: f32,
    /// Tilt.
    pub roll: f32,
}

/// Uncalibrated measurements of one face in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Measures {
    /// Head orientation.
    pub orientation: Orientation,
    /// Midpoint between the eyes in the frame: x to the right and y up, both -1 to 1.
    pub center: [f32; 2],
    /// Distance between the outer eye corners as a fraction of the frame width (a proxy for distance to the camera).
    pub size: f32,
    /// Image-left eyebrow height above the eye, in eye distances.
    pub brow_l: f32,
    /// Image-right eyebrow height above the eye.
    pub brow_r: f32,
    /// Image-left eye height over eye width (about 0.3 open, near 0 shut).
    pub eye_l: f32,
    /// Image-right eye height over eye width.
    pub eye_r: f32,
    /// Where the pupils look, in half eye widths from the eye centre: x toward image right, y up.
    pub gaze: [f32; 2],
    /// Inner-lip gap in eye distances.
    pub mouth_open: f32,
    /// Mouth corner to corner, in eye distances.
    pub mouth_width: f32,
    /// How far the mouth corners sit above the upper lip, over mouth width.
    pub smile: f32,
}

/// The head's axes in camera coordinates.
#[derive(Clone, Copy, Debug)]
struct Axes {
    right: V3,
    down: V3,
    back: V3,
}

fn axes(l: &Landmarks) -> Option<Axes> {
    let eyes = l.get(idx::EYE_R_OUTER)?.minus(l.get(idx::EYE_L_OUTER)?).unit()?;
    let cheeks = l.get(idx::CHEEK_R)?.minus(l.get(idx::CHEEK_L)?).unit()?;
    let right = eyes.plus(cheeks).unit()?;
    let spine = l.get(idx::CHIN)?.minus(l.get(idx::FOREHEAD)?).unit()?;
    let down = spine.minus(right.scale(spine.dot(right))).unit()?;
    Some(Axes { right, down, back: right.cross(down) })
}

/// Euler angles of the head axes (rotation order Z, X, Y).
fn orientation(a: &Axes) -> Orientation {
    let pitch = a.down.z().clamp(-1.0, 1.0).asin();
    let beta = (-a.right.z()).atan2(a.back.z());
    let roll = (-a.down.x()).atan2(a.down.y());
    Orientation { yaw: -beta, pitch, roll }
}

fn mean(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, n) = values.fold((0.0_f32, 0_u8), |(s, n), v| (s + v, n.saturating_add(1)));
    (n > 0).then(|| sum / f32::from(n))
}

/// Eye height over eye width for one eye.
fn eye_ratio(l: &Landmarks, lids: &[(usize, usize)], outer: usize, inner: usize, a: &Axes) -> Option<f32> {
    let width = l.get(inner)?.minus(l.get(outer)?).dot(a.right).abs();
    if width <= f32::EPSILON {
        return None;
    }
    let heights = lids.iter().map(|&(up, low)| Some(l.get(low)?.minus(l.get(up)?).dot(a.down).max(0.0))).collect::<Option<Vec<f32>>>()?;
    Some(mean(heights.into_iter())? / width)
}

/// Brow height above the eye's corner line, in eye distances.
fn brow_height(l: &Landmarks, brow: &[usize], outer: usize, inner: usize, a: &Axes, iod: f32) -> Option<f32> {
    let eye = l.get(outer)?.mid(l.get(inner)?);
    let heights = brow.iter().map(|&i| Some(eye.minus(l.get(i)?).dot(a.down))).collect::<Option<Vec<f32>>>()?;
    Some(mean(heights.into_iter())? / iod)
}

/// Pupil offset from the eye's centre, in half eye widths.
fn pupil(l: &Landmarks, iris: usize, outer: usize, inner: usize, lids: &[(usize, usize)], a: &Axes) -> Option<[f32; 2]> {
    let (o, i) = (l.get(outer)?, l.get(inner)?);
    let half = i.minus(o).dot(a.right).abs() * 0.5;
    if half <= f32::EPSILON {
        return None;
    }
    let corners = o.mid(i);
    let &(up, low) = lids.get(1)?;
    let lids_mid = l.get(up)?.mid(l.get(low)?);
    // Horizontally from the corner midpoint, vertically from the lid midpoint (the corners sit lower than the pupil).
    let p = l.get(iris)?;
    let x = p.minus(corners).dot(a.right) / half;
    let y = -p.minus(lids_mid).dot(a.down) / half;
    Some([x, y])
}

/// Measures `l`, or `None` when a needed landmark is missing or the face is degenerate.
#[must_use]
pub fn measure(l: &Landmarks) -> Option<Measures> {
    let a = axes(l)?;
    let (eye_lo, eye_ro) = (l.get(idx::EYE_L_OUTER)?, l.get(idx::EYE_R_OUTER)?);
    let iod = eye_ro.minus(eye_lo).length();
    if !(iod.is_finite() && iod > f32::EPSILON) || l.frame_width == 0 || l.frame_height == 0 {
        return None;
    }
    let (fw, fh) = (crate::cast::fu(l.frame_width), crate::cast::fu(l.frame_height));
    let mid = eye_lo.mid(eye_ro).mid(l.get(idx::EYE_L_INNER)?.mid(l.get(idx::EYE_R_INNER)?));
    let center = [mid.x() / fw * 2.0 - 1.0, 1.0 - mid.y() / fh * 2.0];

    let gaze = if l.has_irises() {
        let left = pupil(l, idx::IRIS_L, idx::EYE_L_OUTER, idx::EYE_L_INNER, &idx::EYE_L_LIDS, &a)?;
        let right = pupil(l, idx::IRIS_R, idx::EYE_R_OUTER, idx::EYE_R_INNER, &idx::EYE_R_LIDS, &a)?;
        [f32::midpoint(left[0], right[0]), f32::midpoint(left[1], right[1])]
    } else {
        [0.0, 0.0]
    };

    let (mouth_l, mouth_r) = (l.get(idx::MOUTH_L)?, l.get(idx::MOUTH_R)?);
    let (upper, lower) = (l.get(idx::LIP_UPPER_INNER)?, l.get(idx::LIP_LOWER_INNER)?);
    let mouth_width = mouth_r.minus(mouth_l).dot(a.right).abs();
    let corners_up = f32::midpoint(upper.minus(mouth_l).dot(a.down), upper.minus(mouth_r).dot(a.down));

    let m = Measures {
        orientation: orientation(&a),
        center,
        size: iod / fw,
        brow_l: brow_height(l, &idx::BROW_L, idx::EYE_L_OUTER, idx::EYE_L_INNER, &a, iod)?,
        brow_r: brow_height(l, &idx::BROW_R, idx::EYE_R_OUTER, idx::EYE_R_INNER, &a, iod)?,
        eye_l: eye_ratio(l, &idx::EYE_L_LIDS, idx::EYE_L_OUTER, idx::EYE_L_INNER, &a)?,
        eye_r: eye_ratio(l, &idx::EYE_R_LIDS, idx::EYE_R_OUTER, idx::EYE_R_INNER, &a)?,
        gaze,
        mouth_open: lower.minus(upper).dot(a.down).max(0.0) / iod,
        mouth_width: mouth_width / iod,
        smile: if mouth_width > f32::EPSILON { corners_up / mouth_width } else { 0.0 },
    };
    let finite = [m.orientation.yaw, m.orientation.pitch, m.orientation.roll, m.size, m.brow_l, m.brow_r, m.eye_l, m.eye_r, m.mouth_open, m.mouth_width, m.smile];
    (finite.iter().chain(&m.gaze).chain(&m.center).all(|v| v.is_finite())).then_some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::SyntheticFace;

    fn close(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn a_frontal_face_has_zero_orientation() {
        let m = measure(&SyntheticFace::default().landmarks()).unwrap();
        close(m.orientation.yaw, 0.0, 1e-3);
        close(m.orientation.pitch, 0.0, 0.05);
        close(m.orientation.roll, 0.0, 1e-3);
        close(m.center[0], 0.0, 1e-3);
        close(m.center[1], 0.0, 0.2);
    }

    #[test]
    fn orientation_round_trips_synthetic_rotations() {
        for (yaw, pitch, roll) in [(0.3, 0.0, 0.0), (-0.4, 0.0, 0.0), (0.0, 0.25, 0.0), (0.0, -0.3, 0.0), (0.0, 0.0, 0.35), (0.0, 0.0, -0.5), (0.3, -0.2, 0.15)] {
            let base = measure(&SyntheticFace::default().landmarks()).unwrap().orientation;
            let m = measure(&SyntheticFace { yaw, pitch, roll, ..SyntheticFace::default() }.landmarks()).unwrap();
            close(m.orientation.yaw - base.yaw, yaw, 0.03);
            close(m.orientation.pitch - base.pitch, pitch, 0.03);
            close(m.orientation.roll - base.roll, roll, 0.03);
        }
    }

    #[test]
    fn expressions_do_not_depend_on_head_turn() {
        let open = SyntheticFace { mouth_open: 0.3, ..SyntheticFace::default() };
        let straight = measure(&open.landmarks()).unwrap();
        let turned = measure(&SyntheticFace { yaw: 0.4, pitch: 0.2, roll: 0.3, ..open }.landmarks()).unwrap();
        close(straight.mouth_open, turned.mouth_open, 0.02);
        close(straight.mouth_width, turned.mouth_width, 0.03);
        close(straight.eye_l, turned.eye_l, 0.02);
    }

    #[test]
    fn expressions_do_not_depend_on_distance() {
        let near = measure(&SyntheticFace { eye_distance_px: 200.0, mouth_open: 0.2, ..SyntheticFace::default() }.landmarks()).unwrap();
        let far = measure(&SyntheticFace { eye_distance_px: 50.0, mouth_open: 0.2, ..SyntheticFace::default() }.landmarks()).unwrap();
        close(near.mouth_open, far.mouth_open, 1e-3);
        close(near.brow_l, far.brow_l, 1e-3);
        close(near.size / far.size, 4.0, 1e-2);
    }

    #[test]
    fn each_expression_moves_its_own_measure() {
        let base = measure(&SyntheticFace::default().landmarks()).unwrap();
        let with = |f: SyntheticFace| measure(&f.landmarks()).unwrap();
        let d = SyntheticFace::default;
        assert!(with(SyntheticFace { brow_raise: [0.1, 0.0], ..d() }).brow_l > base.brow_l + 0.05);
        close(with(SyntheticFace { brow_raise: [0.1, 0.0], ..d() }).brow_r, base.brow_r, 1e-3);
        assert!(with(SyntheticFace { eye_open: [0.0, 1.0], ..d() }).eye_l < base.eye_l * 0.1);
        close(with(SyntheticFace { eye_open: [0.0, 1.0], ..d() }).eye_r, base.eye_r, 1e-3);
        assert!(with(SyntheticFace { gaze: [0.8, 0.0], ..d() }).gaze[0] > 0.5);
        assert!(with(SyntheticFace { gaze: [-0.8, 0.0], ..d() }).gaze[0] < -0.5);
        assert!(with(SyntheticFace { gaze: [0.0, 0.8], ..d() }).gaze[1] > 0.4);
        assert!(with(SyntheticFace { mouth_open: 0.3, ..d() }).mouth_open > base.mouth_open + 0.25);
        assert!(with(SyntheticFace { smile: 0.1, ..d() }).smile > base.smile + 0.1);
        assert!(with(SyntheticFace { mouth_wide: 1.3, ..d() }).mouth_width > base.mouth_width * 1.2);
    }

    #[test]
    fn missing_or_degenerate_landmarks_measure_nothing() {
        assert!(measure(&Landmarks::default()).is_none());
        let mut l = SyntheticFace::default().landmarks();
        l.points.truncate(100);
        assert!(measure(&l).is_none());
        let mut l = SyntheticFace::default().landmarks();
        l.points = vec![V3([5.0, 5.0, 5.0]); 478];
        assert!(measure(&l).is_none());
        let mut l = SyntheticFace::default().landmarks();
        l.frame_width = 0;
        assert!(measure(&l).is_none());
    }

    #[test]
    fn works_without_irises() {
        let mut l = SyntheticFace { gaze: [0.5, 0.0], ..SyntheticFace::default() }.landmarks();
        l.points.truncate(468);
        let m = measure(&l).unwrap();
        assert_eq!(m.gaze, [0.0, 0.0]);
    }
}
