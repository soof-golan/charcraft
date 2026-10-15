//! The two-stage face pipeline, independent of the inference runtime: crop, detect, crop again, landmarks.
//!
//! It follows `MediaPipe`'s published face landmarker graph (Apache-2.0), also used by `EffectCraft`'s pure-Rust port:
//! the detector (`BlazeFace`, short range) sees a 128x128 letterboxed copy of the frame scaled to -1..1 and answers with
//! 896 SSD anchors (strides 8, 16, 16, 16; two anchors per cell per layer), each a box and six keypoints, decoded
//! and merged by weighted non-maximum suppression. The mesh model sees a 256x256 crop (0..1) of a square 1.5 times
//! the face, rotated so the eyes are level, and answers with 478 landmarks and a face-presence logit. Once a face is
//! found, the next frame's crop comes from the previous landmarks and the detector is not run at all.
//!
//! The networks themselves sit behind [`FaceNet`], so the geometry here is tested without ONNX Runtime.

use crate::cast::{f, floor_i64, fu};
use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::landmarks::{Landmarks, MESH_POINTS_WITH_IRIS, V3};
use crate::source::LandmarkBackend;

/// Side of the detector's input in pixels.
pub const DETECTOR_SIZE: usize = 128;
/// Side of the mesh model's input in pixels.
pub const MESH_SIZE: usize = 256;
/// Number of SSD anchors the detector scores.
pub const ANCHORS: usize = 896;
/// Values per anchor in the detector's regression output: box centre and size, then six keypoints.
pub const REGRESSION_STRIDE: usize = 16;
/// Detection and presence threshold (`MediaPipe`'s default).
pub const MIN_SCORE: f32 = 0.5;
const NMS_IOU: f32 = 0.3;
/// Mesh crops are this much larger than the face.
const ROI_SCALE: f32 = 1.5;
/// Mesh points whose direction levels the crop: the outer eye corners.
const EYE_LINE: [usize; 2] = [33, 263];

/// The detector's raw output for one crop.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectorOutput {
    /// `ANCHORS * REGRESSION_STRIDE` regression values.
    pub regressors: Vec<f32>,
    /// `ANCHORS` score logits.
    pub scores: Vec<f32>,
}

/// The mesh model's raw output for one crop.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshOutput {
    /// `478 * 3` values: x and y in crop pixels (0..256), z in the same scale.
    pub points: Vec<f32>,
    /// Face-presence logit.
    pub presence: f32,
}

/// The two neural networks. Inputs are NHWC `f32` buffers.
pub trait FaceNet {
    /// Runs the detector on a `DETECTOR_SIZE`-square RGB buffer scaled to -1..1.
    ///
    /// # Errors
    /// Runtime failures or unexpected output shapes.
    fn detect(&mut self, input: &[f32]) -> Result<DetectorOutput>;

    /// Runs the mesh model on a `MESH_SIZE`-square RGB buffer scaled to 0..1.
    ///
    /// # Errors
    /// Runtime failures or unexpected output shapes.
    fn mesh(&mut self, input: &[f32]) -> Result<MeshOutput>;
}

/// A rotated square region of a frame: centre, side and angle (radians; the direction of the square's x axis, y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roi {
    /// Centre in frame pixels.
    pub center: [f32; 2],
    /// Side in frame pixels.
    pub size: f32,
    /// Rotation in radians.
    pub angle: f32,
}

impl Roi {
    /// The frame position of the square's normalised point `(u, v)` (each 0..1).
    #[must_use]
    pub fn to_frame(&self, u: f32, v: f32) -> [f32; 2] {
        let (s, c) = self.angle.sin_cos();
        let (x, y) = ((u - 0.5) * self.size, (v - 0.5) * self.size);
        [self.center[0] + c * x - s * y, self.center[1] + s * x + c * y]
    }
}

/// What a crop reads outside the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Border {
    /// Black (the detector's letterbox).
    Zero,
    /// The nearest edge pixel (the mesh model).
    Replicate,
}

/// Samples `roi` of `frame` into an `n` by `n` RGB buffer (bilinear), each channel mapped from 0..255 to `lo..hi`.
///
/// Sampling follows `MediaPipe`'s `ImageToTensor`: the square's corners map to the output corners, so output pixel `i`
/// is at `u = i / n`, and the result is rounded to a whole 0..255 value before the range map.
#[must_use]
pub fn crop(frame: &Frame, roi: &Roi, n: usize, [lo, hi]: [f32; 2], border: Border) -> Vec<f32> {
    let mut out = Vec::with_capacity(n.saturating_mul(n).saturating_mul(3));
    let (w, h) = (i64::from(frame.width()), i64::from(frame.height()));
    let pixel = |x: i64, y: i64| -> [f32; 3] {
        let (x, y) = match border {
            Border::Zero if x < 0 || y < 0 || x >= w || y >= h => return [0.0; 3],
            Border::Zero => (x, y),
            Border::Replicate => (x.clamp(0, w.saturating_sub(1).max(0)), y.clamp(0, h.saturating_sub(1).max(0))),
        };
        let at = |v: i64| u32::try_from(v).ok();
        at(x).zip(at(y)).and_then(|(x, y)| frame.pixel(x, y)).map_or([0.0; 3], |p| p.map(f32::from))
    };
    let step = 1.0 / f(n.max(1));
    for j in 0..n {
        for i in 0..n {
            let [x, y] = roi.to_frame(f(i) * step, f(j) * step);
            let (xf, yf) = (x.floor(), y.floor());
            let (fx, fy) = (x - xf, y - yf);
            let (x0, y0) = (floor_i64(xf), floor_i64(yf));
            let (x1, y1) = (x0.saturating_add(1), y0.saturating_add(1));
            let (a, b, c, d) = (pixel(x0, y0), pixel(x1, y0), pixel(x0, y1), pixel(x1, y1));
            for k in 0..3 {
                let at = |p: &[f32; 3]| p.get(k).copied().unwrap_or(0.0);
                let v = (at(&a) * (1.0 - fx) + at(&b) * fx) * (1.0 - fy) + (at(&c) * (1.0 - fx) + at(&d) * fx) * fy;
                let v = if v.is_finite() { v.round().clamp(0.0, 255.0) / 255.0 } else { 0.0 };
                out.push(lo + v * (hi - lo));
            }
        }
    }
    out
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x.clamp(-100.0, 100.0)).exp())
}

/// A detected face: box `[x0, y0, x1, y1]`, six keypoints (eyes, nose tip, mouth, ears; image-left first), score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    /// Box in frame pixels.
    pub bbox: [f32; 4],
    /// Keypoints in frame pixels.
    pub keypoints: [[f32; 2]; 6],
    /// Confidence 0..1.
    pub score: f32,
}

impl Detection {
    fn area(&self) -> f32 {
        (self.bbox[2] - self.bbox[0]).max(0.0) * (self.bbox[3] - self.bbox[1]).max(0.0)
    }
}

/// SSD anchor centres (normalised): per stride group, `fm` by `fm` cells with two anchors per layer each.
#[must_use]
pub fn anchors() -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(ANCHORS);
    for (stride, layers) in [(8_usize, 1_usize), (16, 3)] {
        let fm = DETECTOR_SIZE.div_ceil(stride);
        for y in 0..fm {
            for x in 0..fm {
                for _ in 0..layers.saturating_mul(2) {
                    out.push([(f(x) + 0.5) / f(fm), (f(y) + 0.5) / f(fm)]);
                }
            }
        }
    }
    out
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// Weighted non-maximum suppression: each cluster of overlapping detections becomes their score-weighted average,
/// keeping the best score. Best first.
#[must_use]
pub fn weighted_nms(mut dets: Vec<Detection>) -> Vec<Detection> {
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut out = vec![];
    while let Some(top) = dets.first().copied() {
        // `top` always joins its own cluster: a degenerate box (NaN or zero area) overlaps nothing, itself included.
        let (mut cluster, rest): (Vec<Detection>, Vec<Detection>) = dets.into_iter().skip(1).partition(|d| iou(&d.bbox, &top.bbox) > NMS_IOU);
        dets = rest;
        cluster.insert(0, top);
        let total: f32 = cluster.iter().map(|d| d.score).sum();
        if !(total.is_finite() && total > 0.0) {
            out.push(top);
            continue;
        }
        let mut avg = Detection { bbox: [0.0; 4], keypoints: [[0.0; 2]; 6], score: top.score };
        for d in &cluster {
            let k = d.score / total;
            for (a, v) in avg.bbox.iter_mut().zip(d.bbox) {
                *a += v * k;
            }
            for (a, p) in avg.keypoints.iter_mut().zip(&d.keypoints) {
                a[0] += p[0] * k;
                a[1] += p[1] * k;
            }
        }
        out.push(avg);
    }
    out
}

fn bounds(points: &[[f32; 2]]) -> [f32; 4] {
    points
        .iter()
        .fold([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// The crop a face's mesh is computed in: `bbox` grown to a square 1.5 times its long side, turned so `from` to
/// `to` is level.
#[must_use]
pub fn roi_of(bbox: [f32; 4], from: [f32; 2], to: [f32; 2]) -> Roi {
    Roi {
        center: [f32::midpoint(bbox[0], bbox[2]), f32::midpoint(bbox[1], bbox[3])],
        size: (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) * ROI_SCALE,
        angle: (to[1] - from[1]).atan2(to[0] - from[0]),
    }
}

/// The crop that frames a face found earlier.
fn roi_of_face(face: &Landmarks) -> Option<Roi> {
    let (a, b) = (face.get(*EYE_LINE.first()?)?, face.get(*EYE_LINE.get(1)?)?);
    let pts: Vec<[f32; 2]> = face.points.iter().filter(|p| p.is_finite()).map(|p| [p.x(), p.y()]).collect();
    let roi = roi_of(bounds(&pts), [a.x(), a.y()], [b.x(), b.y()]);
    (roi.size.is_finite() && roi.size > 4.0 && roi.center.iter().all(|c| c.is_finite())).then_some(roi)
}

/// Decodes the detector output into detections in frame pixels.
fn decode(out: &DetectorOutput, anchors: &[[f32; 2]], roi: &Roi) -> Vec<Detection> {
    let s = f(DETECTOR_SIZE);
    let to = |u: f32, v: f32| roi.to_frame(u, v);
    let mut dets = vec![];
    for ((r, &logit), a) in out.regressors.as_chunks::<REGRESSION_STRIDE>().0.iter().zip(&out.scores).zip(anchors) {
        let score = sigmoid(logit);
        if score < MIN_SCORE {
            continue;
        }
        let v = |i: usize| r.get(i).copied().unwrap_or_default();
        let (cx, cy, bw, bh) = (v(0) / s + a[0], v(1) / s + a[1], v(2) / s, v(3) / s);
        let corners = [to(cx - bw / 2.0, cy - bh / 2.0), to(cx + bw / 2.0, cy - bh / 2.0), to(cx - bw / 2.0, cy + bh / 2.0), to(cx + bw / 2.0, cy + bh / 2.0)];
        let mut keypoints = [[0.0_f32; 2]; 6];
        for (k, p) in keypoints.iter_mut().enumerate() {
            let at = k.saturating_mul(2).saturating_add(4);
            *p = to(v(at) / s + a[0], v(at.saturating_add(1)) / s + a[1]);
        }
        dets.push(Detection { bbox: bounds(&corners), keypoints, score });
    }
    weighted_nms(dets)
}

/// Finds a face and follows it from frame to frame, on any [`FaceNet`].
#[derive(Debug)]
pub struct FaceTracker<N> {
    net: N,
    anchors: Vec<[f32; 2]>,
    previous: Option<Landmarks>,
}

impl<N: FaceNet> FaceTracker<N> {
    /// A tracker on `net`.
    #[must_use]
    pub fn new(net: N) -> Self {
        Self { net, anchors: anchors(), previous: None }
    }

    /// Forgets the face being followed; the next frame runs the detector.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    /// The network, for inspection.
    #[must_use]
    pub const fn net(&self) -> &N {
        &self.net
    }

    fn detect(&mut self, frame: &Frame, roi: &Roi) -> Result<Vec<Detection>> {
        let input = crop(frame, roi, DETECTOR_SIZE, [-1.0, 1.0], Border::Zero);
        let out = self.net.detect(&input)?;
        if out.regressors.len() != ANCHORS * REGRESSION_STRIDE || out.scores.len() != ANCHORS {
            return Err(Error::Inference(format!("face detector answered {} regressors and {} scores", out.regressors.len(), out.scores.len())));
        }
        Ok(decode(&out, &self.anchors, roi))
    }

    fn mesh(&mut self, frame: &Frame, roi: &Roi) -> Result<Landmarks> {
        let input = crop(frame, roi, MESH_SIZE, [0.0, 1.0], Border::Replicate);
        let out = self.net.mesh(&input)?;
        if out.points.len() != MESH_POINTS_WITH_IRIS * 3 {
            return Err(Error::Inference(format!("face mesh answered {} values, expected {}", out.points.len(), MESH_POINTS_WITH_IRIS * 3)));
        }
        let side = f(MESH_SIZE);
        let points = out
            .points
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[px, py, pz]| {
                let [x, y] = roi.to_frame(px / side, py / side);
                V3([x, y, pz / side * roi.size])
            })
            .collect();
        Ok(Landmarks { points, frame_width: frame.width(), frame_height: frame.height(), score: sigmoid(out.presence) })
    }

    /// The mesh from `roi`, refined once from its own landmarks (a detector box is coarse).
    fn mesh_refined(&mut self, frame: &Frame, roi: &Roi) -> Result<Option<Landmarks>> {
        let first = self.mesh(frame, roi)?;
        if first.score < MIN_SCORE {
            return Ok(None);
        }
        let Some(r) = roi_of_face(&first) else { return Ok(None) };
        let refined = self.mesh(frame, &r)?;
        Ok(Some(if refined.score >= MIN_SCORE { refined } else { first }))
    }

    fn find(&mut self, frame: &Frame) -> Result<Option<Landmarks>> {
        let side = fu(frame.width().max(frame.height()));
        let whole = Roi { center: [fu(frame.width()) / 2.0, fu(frame.height()) / 2.0], size: side, angle: 0.0 };
        if whole.size < 8.0 {
            return Ok(None);
        }
        let dets = self.detect(frame, &whole)?;
        // The subject sits closest to the camera: take the largest face.
        let Some(best) = dets.into_iter().max_by(|a, b| a.area().total_cmp(&b.area())) else { return Ok(None) };
        let roi = roi_of(best.bbox, best.keypoints[0], best.keypoints[1]);
        if !(roi.size.is_finite() && roi.size > 4.0) {
            return Ok(None);
        }
        self.mesh_refined(frame, &roi)
    }
}

impl<N: FaceNet> LandmarkBackend for FaceTracker<N> {
    fn landmarks(&mut self, frame: &Frame) -> Result<Option<Landmarks>> {
        if let Some(roi) = self.previous.as_ref().and_then(roi_of_face) {
            let face = self.mesh(frame, &roi)?;
            if face.score >= MIN_SCORE {
                self.previous = Some(face.clone());
                return Ok(Some(face));
            }
        }
        // No face yet, or the followed one was lost (a fast move): look again in the whole frame.
        self.previous = None;
        let found = self.find(frame)?;
        self.previous.clone_from(&found);
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::measure;
    use crate::synthetic::SyntheticFace;

    fn close(a: f32, b: f32, tol: f32) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn anchors_match_the_detector_layout() {
        let a = anchors();
        assert_eq!(a.len(), ANCHORS);
        // Stride 8: 16x16 cells, two anchors each; then stride 16: 8x8 cells, six each.
        assert_eq!(a[0], [0.5 / 16.0, 0.5 / 16.0]);
        assert_eq!(a[1], a[0]);
        assert_eq!(a[2], [1.5 / 16.0, 0.5 / 16.0]);
        assert_eq!(a[512], [0.5 / 8.0, 0.5 / 8.0]);
        assert_eq!(a[517], a[512]);
        assert_eq!(a[518], [1.5 / 8.0, 0.5 / 8.0]);
    }

    #[test]
    fn roi_maps_corners_and_rotates() {
        let r = Roi { center: [10.0, 20.0], size: 4.0, angle: 0.0 };
        assert_eq!(r.to_frame(0.0, 0.0), [8.0, 18.0]);
        assert_eq!(r.to_frame(1.0, 1.0), [12.0, 22.0]);
        let q = Roi { angle: std::f32::consts::FRAC_PI_2, ..r };
        let p = q.to_frame(1.0, 0.5);
        close(p[0], 10.0, 1e-5);
        close(p[1], 22.0, 1e-5);
    }

    fn ramp() -> Frame {
        Frame::new(4, 4, (0..16_u8).flat_map(|i| [i * 16, 0, 255]).collect()).unwrap()
    }

    #[test]
    fn crop_one_to_one_reproduces_the_frame() {
        let c = crop(&ramp(), &Roi { center: [2.0, 2.0], size: 4.0, angle: 0.0 }, 4, [0.0, 1.0], Border::Zero);
        for i in 0..16 {
            close(c[i * 3], f32::from(u8::try_from(i * 16).unwrap()) / 255.0, 1e-6);
            close(c[i * 3 + 2], 1.0, 1e-6);
        }
    }

    #[test]
    fn crop_borders_are_zero_or_replicated() {
        let far = Roi { center: [-10.0, -10.0], size: 4.0, angle: 0.0 };
        assert!(crop(&ramp(), &far, 2, [-1.0, 1.0], Border::Zero).iter().all(|v| (*v + 1.0).abs() < f32::EPSILON));
        let r = crop(&ramp(), &far, 2, [0.0, 255.0], Border::Replicate);
        for p in r.chunks(3) {
            assert!(p[0].abs() < 1e-3 && (p[2] - 255.0).abs() < 1e-3, "{p:?}");
        }
    }

    #[test]
    fn crop_survives_absurd_rois() {
        let roi = Roi { center: [1e30, -1e30], size: 1e30, angle: 1.0 };
        for border in [Border::Zero, Border::Replicate] {
            let c = crop(&ramp(), &roi, 4, [-1.0, 1.0], border);
            assert_eq!(c.len(), 48);
            assert!(c.iter().all(|v| v.is_finite()));
        }
        let nan = Roi { center: [f32::NAN; 2], size: f32::NAN, angle: f32::NAN };
        assert!(crop(&ramp(), &nan, 4, [0.0, 1.0], Border::Zero).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn nms_averages_overlapping_detections() {
        let d = |x: f32, s: f32| Detection { bbox: [x, 0.0, x + 10.0, 10.0], keypoints: [[x, 0.0]; 6], score: s };
        let out = weighted_nms(vec![d(0.0, 0.9), d(1.0, 0.9), d(50.0, 0.6)]);
        assert_eq!(out.len(), 2);
        close(out[0].bbox[0], 0.5, 1e-5);
        assert_eq!(out[1].bbox[0], 50.0);
    }

    #[test]
    fn nms_terminates_on_degenerate_boxes() {
        let d = Detection { bbox: [f32::NAN; 4], keypoints: [[f32::NAN; 2]; 6], score: 0.7 };
        assert_eq!(weighted_nms(vec![d, d, d]).len(), 3);
        let z = Detection { bbox: [0.0; 4], keypoints: [[0.0; 2]; 6], score: 0.0 };
        assert_eq!(weighted_nms(vec![z, z]).len(), 2);
        let n = Detection { score: f32::NAN, ..z };
        assert_eq!(weighted_nms(vec![n, z, d]).len(), 3);
    }

    /// A pretend network: the detector finds one face at the middle of the crop, the mesh answers with a synthetic
    /// face laid out in crop coordinates.
    #[derive(Default)]
    struct Fake {
        detects: usize,
        meshes: usize,
        presence: f32,
        see_face: bool,
        mesh_face: Option<SyntheticFace>,
    }

    impl FaceNet for Fake {
        fn detect(&mut self, input: &[f32]) -> Result<DetectorOutput> {
            if input.len() != DETECTOR_SIZE * DETECTOR_SIZE * 3 {
                return Err(Error::Inference("wrong detector input size".into()));
            }
            self.detects += 1;
            let mut regressors = vec![0.0; ANCHORS * REGRESSION_STRIDE];
            let mut scores = vec![-10.0; ANCHORS];
            if self.see_face {
                // Anchor 0 is at the top-left of the crop; shift its box to the middle with the regression.
                let a = anchors()[0];
                let s = f(DETECTOR_SIZE);
                let (dx, dy) = ((0.5 - a[0]) * s, (0.5 - a[1]) * s);
                scores[0] = 5.0;
                regressors[..REGRESSION_STRIDE].copy_from_slice(&[
                    dx,
                    dy,
                    60.0,
                    60.0,
                    dx - 10.0,
                    dy - 5.0,
                    dx + 10.0,
                    dy - 5.0,
                    dx,
                    dy,
                    dx,
                    dy + 10.0,
                    dx - 25.0,
                    dy,
                    dx + 25.0,
                    dy,
                ]);
            }
            Ok(DetectorOutput { regressors, scores })
        }

        fn mesh(&mut self, input: &[f32]) -> Result<MeshOutput> {
            if input.len() != MESH_SIZE * MESH_SIZE * 3 {
                return Err(Error::Inference("wrong mesh input size".into()));
            }
            self.meshes += 1;
            let face =
                self.mesh_face.unwrap_or_else(|| SyntheticFace { eye_distance_px: 70.0, center: [0.0, 0.0], frame: [256, 256], ..SyntheticFace::default() });
            let points = face.landmarks().points.iter().flat_map(|p| p.0).collect();
            Ok(MeshOutput { points, presence: self.presence })
        }
    }

    fn tracker(see_face: bool) -> FaceTracker<Fake> {
        FaceTracker::new(Fake { presence: 5.0, see_face, ..Fake::default() })
    }

    fn frame() -> Frame {
        Frame::solid(640, 480, [90, 80, 70]).unwrap()
    }

    #[test]
    fn finds_a_face_and_maps_its_landmarks_into_the_frame() {
        let mut t = tracker(true);
        let l = t.landmarks(&frame()).unwrap().expect("a face");
        assert_eq!((l.frame_width, l.frame_height), (640, 480));
        assert!(l.score > 0.9 && l.points.len() == MESH_POINTS_WITH_IRIS);
        assert_eq!((t.net().detects, t.net().meshes), (1, 2), "detector once, mesh twice (found, then refined)");
        // The face sits where the detector put it: the middle of the (letterboxed, square) frame crop.
        let m = measure(&l).expect("measurable");
        close(m.center[0], 0.0, 0.05);
        // Ratios survive the crop-to-frame mapping.
        let reference = measure(&SyntheticFace::default().landmarks()).unwrap();
        close(m.mouth_width, reference.mouth_width, 1e-3);
        close(m.eye_l, reference.eye_l, 1e-3);
    }

    #[test]
    fn once_found_the_next_frame_skips_the_detector() {
        let mut t = tracker(true);
        t.landmarks(&frame()).unwrap().unwrap();
        let (d, m) = (t.net().detects, t.net().meshes);
        t.landmarks(&frame()).unwrap().unwrap();
        assert_eq!((t.net().detects, t.net().meshes), (d, m + 1));
        t.reset();
        t.landmarks(&frame()).unwrap().unwrap();
        assert_eq!(t.net().detects, d + 1);
    }

    #[test]
    fn a_lost_face_triggers_a_new_detection() {
        let mut t = tracker(true);
        t.landmarks(&frame()).unwrap().unwrap();
        t.net.presence = -5.0;
        t.net.see_face = false;
        let before = t.net().detects;
        assert!(t.landmarks(&frame()).unwrap().is_none());
        assert_eq!(t.net().detects, before + 1, "it looked again");
        assert!(t.landmarks(&frame()).unwrap().is_none());
    }

    #[test]
    fn no_detection_means_no_face() {
        assert!(tracker(false).landmarks(&frame()).unwrap().is_none());
    }

    #[test]
    fn a_detection_the_mesh_rejects_means_no_face() {
        let mut t = tracker(true);
        t.net.presence = -5.0;
        assert!(t.landmarks(&frame()).unwrap().is_none());
    }

    #[test]
    fn wrong_network_shapes_are_errors_not_panics() {
        struct Bad;
        impl FaceNet for Bad {
            fn detect(&mut self, _: &[f32]) -> Result<DetectorOutput> {
                Ok(DetectorOutput { regressors: vec![0.0; 3], scores: vec![] })
            }
            fn mesh(&mut self, _: &[f32]) -> Result<MeshOutput> {
                Ok(MeshOutput { points: vec![0.0; 5], presence: 9.0 })
            }
        }
        let mut t = FaceTracker::new(Bad);
        assert!(matches!(t.landmarks(&frame()), Err(Error::Inference(_))));
        t.previous = Some(SyntheticFace::default().landmarks());
        assert!(matches!(t.landmarks(&frame()), Err(Error::Inference(_))));
    }

    #[test]
    fn tiny_frames_are_handled() {
        let mut t = tracker(true);
        assert!(t.landmarks(&Frame::solid(2, 2, [0; 3]).unwrap()).unwrap().is_none());
    }
}
