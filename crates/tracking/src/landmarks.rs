//! Face landmarks: the points a face model reports, and the few indices of `MediaPipe`'s canonical mesh we read.

use serde::{Deserialize, Serialize};

/// Points of the face mesh without the irises.
pub const MESH_POINTS: usize = 468;
/// Points of the face mesh with both irises (the model we use reports this many).
pub const MESH_POINTS_WITH_IRIS: usize = 478;

/// Indices into `MediaPipe`'s canonical face mesh. "Left" and "right" are as seen in the camera image (image-left
/// is the subject's right).
pub mod idx {
    /// Outer corner of the image-left eye.
    pub const EYE_L_OUTER: usize = 33;
    /// Inner corner of the image-left eye.
    pub const EYE_L_INNER: usize = 133;
    /// Outer corner of the image-right eye.
    pub const EYE_R_OUTER: usize = 263;
    /// Inner corner of the image-right eye.
    pub const EYE_R_INNER: usize = 362;
    /// (upper lid, lower lid) pairs across the image-left eye, outer to inner.
    pub const EYE_L_LIDS: [(usize, usize); 3] = [(160, 144), (159, 145), (158, 153)];
    /// (upper lid, lower lid) pairs across the image-right eye, inner to outer.
    pub const EYE_R_LIDS: [(usize, usize); 3] = [(385, 380), (386, 374), (387, 373)];
    /// Centre of the image-left iris (only in the 478-point mesh).
    pub const IRIS_L: usize = 468;
    /// Centre of the image-right iris (only in the 478-point mesh).
    pub const IRIS_R: usize = 473;
    /// Points along the image-left eyebrow, outer to inner.
    pub const BROW_L: [usize; 5] = [70, 63, 105, 66, 107];
    /// Points along the image-right eyebrow, inner to outer.
    pub const BROW_R: [usize; 5] = [336, 296, 334, 293, 300];
    /// Image-left mouth corner.
    pub const MOUTH_L: usize = 61;
    /// Image-right mouth corner.
    pub const MOUTH_R: usize = 291;
    /// Inner upper lip, centre.
    pub const LIP_UPPER_INNER: usize = 13;
    /// Inner lower lip, centre.
    pub const LIP_LOWER_INNER: usize = 14;
    /// Nose tip.
    pub const NOSE_TIP: usize = 1;
    /// Top of the forehead.
    pub const FOREHEAD: usize = 10;
    /// Chin.
    pub const CHIN: usize = 152;
    /// Image-left cheek edge (the face outline at ear level).
    pub const CHEEK_L: usize = 234;
    /// Image-right cheek edge.
    pub const CHEEK_R: usize = 454;
}

/// A 3-vector with named operations (the workspace lints do not allow operator arithmetic on custom types).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V3(pub [f32; 3]);

impl V3 {
    /// The x component (image right).
    #[must_use]
    pub const fn x(self) -> f32 {
        self.0[0]
    }

    /// The y component (image down).
    #[must_use]
    pub const fn y(self) -> f32 {
        self.0[1]
    }

    /// The z component (away from the camera).
    #[must_use]
    pub const fn z(self) -> f32 {
        self.0[2]
    }

    /// `self + o`.
    #[must_use]
    pub fn plus(self, o: Self) -> Self {
        Self([self.x() + o.x(), self.y() + o.y(), self.z() + o.z()])
    }

    /// `self - o`.
    #[must_use]
    pub fn minus(self, o: Self) -> Self {
        Self([self.x() - o.x(), self.y() - o.y(), self.z() - o.z()])
    }

    /// `self * k`.
    #[must_use]
    pub fn scale(self, k: f32) -> Self {
        Self([self.x() * k, self.y() * k, self.z() * k])
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, o: Self) -> f32 {
        self.x() * o.x() + self.y() * o.y() + self.z() * o.z()
    }

    /// Cross product.
    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self([self.y() * o.z() - self.z() * o.y(), self.z() * o.x() - self.x() * o.z(), self.x() * o.y() - self.y() * o.x()])
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The unit vector, or `None` for a zero or non-finite vector.
    #[must_use]
    pub fn unit(self) -> Option<Self> {
        let len = self.length();
        (len.is_finite() && len > f32::EPSILON).then(|| self.scale(1.0 / len))
    }

    /// The midpoint of `self` and `o`.
    #[must_use]
    pub fn mid(self, o: Self) -> Self {
        self.plus(o).scale(0.5)
    }

    /// Whether all components are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }
}

/// The landmarks of one face in one frame.
///
/// Coordinates are in frame pixels: `x` to the right, `y` down, `z` away from the camera in the same scale as `x`
/// and `y` (smaller is closer).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Landmarks {
    /// The points, in the order of `MediaPipe`'s canonical mesh.
    pub points: Vec<V3>,
    /// Width of the frame they were found in.
    pub frame_width: u32,
    /// Height of the frame they were found in.
    pub frame_height: u32,
    /// How sure the model is that this is a face (0 to 1).
    pub score: f32,
}

impl Landmarks {
    /// The point `i`, if present and finite.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<V3> {
        self.points.get(i).copied().filter(|p| p.is_finite())
    }

    /// Whether the irises are included.
    #[must_use]
    pub const fn has_irises(&self) -> bool {
        self.points.len() >= MESH_POINTS_WITH_IRIS
    }

    /// The bounding box `[x0, y0, x1, y1]` of the points, or `None` when there are none.
    #[must_use]
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut it = self.points.iter().filter(|p| p.is_finite());
        let first = it.next()?;
        Some(it.fold([first.x(), first.y(), first.x(), first.y()], |b, p| [b[0].min(p.x()), b[1].min(p.y()), b[2].max(p.x()), b[3].max(p.y())]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_maths() {
        let a = V3([1.0, 0.0, 0.0]);
        let b = V3([0.0, 1.0, 0.0]);
        assert_eq!(a.cross(b), V3([0.0, 0.0, 1.0]));
        assert!((a.plus(b).length() - 2.0_f32.sqrt()).abs() < 1e-6);
        assert_eq!(a.mid(b), V3([0.5, 0.5, 0.0]));
        assert_eq!(V3::default().unit(), None);
        assert_eq!(V3([f32::NAN, 0.0, 0.0]).unit(), None);
        assert_eq!(V3([0.0, 3.0, 0.0]).unit(), Some(b));
    }

    #[test]
    fn get_skips_missing_and_non_finite_points() {
        let l = Landmarks { points: vec![V3([1.0, 2.0, 3.0]), V3([f32::NAN, 0.0, 0.0])], ..Landmarks::default() };
        assert!(l.get(0).is_some());
        assert!(l.get(1).is_none());
        assert!(l.get(2).is_none());
        assert_eq!(l.bounds(), Some([1.0, 2.0, 1.0, 2.0]));
        assert!(!l.has_irises());
    }
}
