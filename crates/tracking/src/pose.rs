//! The runtime-independent half of body pose estimation.
//!
//! It prepares a square crop for a single-person pose network (`MoveNet`) and turns the output into [`Keypoints`] in
//! the frame. The network itself sits behind [`PoseNet`] (the ONNX Runtime one is `onnx::OnnxPoseNet`), so all of
//! this runs in tests against a stand-in.

use crate::body::{BodyBackend, JOINTS, Keypoint, Keypoints, MIN_SCORE};
use crate::cast::fu;
use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::pipeline::{Border, Roi, crop};

/// Side of the square the network takes, in pixels.
pub const POSE_SIZE: usize = 192;

/// A single-person pose network.
pub trait PoseNet {
    /// Runs the network on a `POSE_SIZE`-square RGB buffer, channels in 0..255, and returns `JOINTS * 3` numbers:
    /// for each joint in model order its `y`, `x` (each 0 to 1 across the square) and confidence.
    ///
    /// # Errors
    /// Runtime failures or unexpected output shapes.
    fn infer(&mut self, input: &[f32]) -> Result<Vec<f32>>;
}

/// The least mean confidence over the joints for a body to count as found.
const MIN_MEAN_SCORE: f32 = 0.12;

/// The side of a tracking crop relative to the body's extent in the previous frame.
const CROP_MARGIN: f32 = 1.5;

/// Finds a body: runs the network on the whole frame at first.
///
/// After that it looks in a square around the body of the previous frame (so a performer who stands back is still
/// seen at a useful size), falling back to the whole frame when the body is lost.
#[derive(Debug)]
pub struct PoseTracker<N> {
    net: N,
    previous: Option<Keypoints>,
}

impl<N: PoseNet> PoseTracker<N> {
    /// A tracker over a network.
    #[must_use]
    pub const fn new(net: N) -> Self {
        Self { net, previous: None }
    }

    /// Forgets the last body.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    fn run(&mut self, frame: &Frame, roi: &Roi) -> Result<Option<Keypoints>> {
        let input = crop(frame, roi, POSE_SIZE, [0.0, 255.0], Border::Zero);
        let out = self.net.infer(&input)?;
        if out.len() < JOINTS.saturating_mul(3) {
            return Err(Error::Inference(format!("pose network gave {} numbers, expected {}", out.len(), JOINTS.saturating_mul(3))));
        }
        let (w, h) = (fu(frame.width()).max(1.0), fu(frame.height()).max(1.0));
        let points: Vec<Keypoint> = (0..JOINTS)
            .map(|i| {
                let at = |k: usize| out.get(i.saturating_mul(3).saturating_add(k)).copied().unwrap_or(0.0);
                let (y, x, score) = (at(0), at(1), at(2));
                let [px, py] = roi.to_frame(x, y);
                Keypoint { x: px / w, y: py / h, score: if score.is_finite() { score.clamp(0.0, 1.0) } else { 0.0 } }
            })
            .collect();
        let keypoints = Keypoints { aspect: w / h, points };
        Ok((keypoints.mean_score() >= MIN_MEAN_SCORE).then_some(keypoints))
    }

    /// The region to look in next: the previous body's box, widened, or `None` for the whole frame.
    fn region(&self, frame: &Frame) -> Option<Roi> {
        let k = self.previous.as_ref()?;
        let (w, h) = (fu(frame.width()), fu(frame.height()));
        let seen: Vec<[f32; 2]> = crate::body::Joint::ALL.iter().filter_map(|j| k.seen(*j, MIN_SCORE)).map(|p| [p.x * w, p.y * h]).collect();
        if seen.len() < 6 {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [x, y] in &seen {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        let side = ((x1 - x0).max(y1 - y0) * CROP_MARGIN).max(w.max(h) * 0.4);
        // A crop as big as the frame is just the whole frame.
        (side < w.max(h) * 0.95).then_some(Roi { center: [f32::midpoint(x0, x1), f32::midpoint(y0, y1)], size: side, angle: 0.0 })
    }
}

impl<N: PoseNet> BodyBackend for PoseTracker<N> {
    fn keypoints(&mut self, frame: &Frame) -> Result<Option<Keypoints>> {
        let whole = Roi { center: [fu(frame.width()) / 2.0, fu(frame.height()) / 2.0], size: fu(frame.width()).max(fu(frame.height())).max(1.0), angle: 0.0 };
        let found = match self.region(frame) {
            Some(roi) => match self.run(frame, &roi)? {
                Some(k) => Some(k),
                // Lost inside the crop: look at the whole frame before giving up.
                None => self.run(frame, &whole)?,
            },
            None => self.run(frame, &whole)?,
        };
        self.previous.clone_from(&found);
        Ok(found)
    }
}

/// Counts for the tests: how many numbers a joint list takes.
#[must_use]
pub const fn output_len() -> usize {
    JOINTS.saturating_mul(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::Joint;
    use crate::cast::f;

    /// A network that reports every joint at the given square-relative place, `[x, y]`, and counts its runs.
    struct Fixed {
        at: [f32; 2],
        score: f32,
        seen: usize,
    }

    impl PoseNet for Fixed {
        fn infer(&mut self, input: &[f32]) -> Result<Vec<f32>> {
            self.seen += usize::from(input.len() == POSE_SIZE * POSE_SIZE * 3);
            Ok((0..JOINTS).flat_map(|_| [self.at[1], self.at[0], self.score]).collect())
        }
    }

    fn frame() -> Frame {
        Frame::solid(640, 480, [10, 20, 30]).unwrap()
    }

    #[test]
    fn the_square_maps_back_onto_the_frame() {
        let mut t = PoseTracker::new(Fixed { at: [0.5, 0.5], score: 0.9, seen: 0 });
        let k = t.keypoints(&frame()).unwrap().unwrap();
        assert!((k.aspect - 4.0 / 3.0).abs() < 1e-6);
        let nose = k.get(Joint::Nose).unwrap();
        assert!((nose.x - 0.5).abs() < 1e-5 && (nose.y - 0.5).abs() < 1e-5, "{nose:?}");
        // The right edge of the square is the right edge of the frame; the square's top is above the frame (the
        // letterbox).
        let mut edge = PoseTracker::new(Fixed { at: [1.0, 0.0], score: 0.9, seen: 0 });
        let p = edge.keypoints(&frame()).unwrap().unwrap().get(Joint::Nose).unwrap();
        assert!((p.x - 1.0).abs() < 1e-5 && (p.y - (240.0 - 320.0) / 480.0).abs() < 1e-5, "{p:?}");
    }

    #[test]
    fn a_body_the_network_is_unsure_of_is_not_found() {
        let mut t = PoseTracker::new(Fixed { at: [0.5, 0.5], score: 0.05, seen: 0 });
        assert!(t.keypoints(&frame()).unwrap().is_none());
        assert_eq!(t.net.seen, 1, "a miss on the whole frame is not retried");
    }

    #[test]
    fn once_found_the_next_frame_looks_in_a_crop_and_a_miss_falls_back_to_the_whole_frame() {
        // A body whose joints spread over part of the frame: the net reports them at three places.
        struct Spread(usize);
        impl PoseNet for Spread {
            fn infer(&mut self, _: &[f32]) -> Result<Vec<f32>> {
                self.0 += 1;
                Ok((0..JOINTS).flat_map(|i| [0.4 + 0.01 * f(i), 0.4 + 0.01 * f(i), 0.9]).collect())
            }
        }
        let mut t = PoseTracker::new(Spread(0));
        let fr = frame();
        let first = t.keypoints(&fr).unwrap().unwrap();
        assert!(t.region(&fr).is_some(), "the body is small in the frame, so the next look is a crop");
        let second = t.keypoints(&fr).unwrap().unwrap();
        // Same network output, but relative to a smaller square: the points are close to the body's centre.
        let spread = |k: &Keypoints| {
            let xs: Vec<f32> = k.points.iter().map(|p| p.x).collect();
            xs.iter().copied().fold(f32::MIN, f32::max) - xs.iter().copied().fold(f32::MAX, f32::min)
        };
        assert!(spread(&second) < spread(&first), "{} vs {}", spread(&second), spread(&first));
        t.reset();
        assert!(t.region(&fr).is_none());
    }

    #[test]
    fn a_network_with_the_wrong_output_is_an_error_not_a_panic() {
        struct Short;
        impl PoseNet for Short {
            fn infer(&mut self, _: &[f32]) -> Result<Vec<f32>> {
                Ok(vec![0.0; 10])
            }
        }
        assert!(matches!(PoseTracker::new(Short).keypoints(&frame()), Err(Error::Inference(_))));
        assert_eq!(output_len(), 51);
    }

    #[test]
    fn non_finite_scores_count_as_zero() {
        let mut t = PoseTracker::new(Fixed { at: [0.5, 0.5], score: f32::NAN, seen: 0 });
        assert!(t.keypoints(&frame()).unwrap().is_none());
    }
}
