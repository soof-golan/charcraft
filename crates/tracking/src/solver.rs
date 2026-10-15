//! From measurements to [`FaceInput`]: neutral-face calibration, sensitivity ranges and smoothing.

use anim_core::Tick;
use serde::{Deserialize, Serialize};

use crate::input::{FaceInput, HeadPose};
use crate::landmarks::Landmarks;
use crate::measure::{Measures, Orientation, measure};
use crate::synthetic::SyntheticFace;

/// The neutral face expressions are measured against.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Measurements of the face at rest, looking at the camera.
    pub neutral: Measures,
    /// Whether this came from a captured neutral face (`false` for the built-in default).
    pub captured: bool,
}

impl Default for Calibration {
    /// A typical face: good enough to start with, better after [`Calibration::from_samples`].
    fn default() -> Self {
        let typical = measure(&SyntheticFace::default().landmarks()).unwrap_or_default();
        Self { neutral: Measures { orientation: Orientation::default(), center: [0.0, 0.0], size: 0.12, gaze: [0.0, 0.0], ..typical }, captured: false }
    }
}

impl Calibration {
    /// The average of `samples` taken while the subject holds a neutral face looking at the camera. `None` when
    /// there are no samples.
    #[must_use]
    pub fn from_samples(samples: &[Measures]) -> Option<Self> {
        let n = u16::try_from(samples.len()).ok().filter(|n| *n > 0)?;
        let k = 1.0 / f32::from(n);
        let sum = |f: fn(&Measures) -> f32| samples.iter().map(f).sum::<f32>() * k;
        let neutral = Measures {
            orientation: Orientation { yaw: sum(|m| m.orientation.yaw), pitch: sum(|m| m.orientation.pitch), roll: sum(|m| m.orientation.roll) },
            center: [sum(|m| m.center[0]), sum(|m| m.center[1])],
            size: sum(|m| m.size),
            brow_l: sum(|m| m.brow_l),
            brow_r: sum(|m| m.brow_r),
            eye_l: sum(|m| m.eye_l),
            eye_r: sum(|m| m.eye_r),
            gaze: [sum(|m| m.gaze[0]), sum(|m| m.gaze[1])],
            mouth_open: sum(|m| m.mouth_open),
            mouth_width: sum(|m| m.mouth_width),
            smile: sum(|m| m.smile),
        };
        Some(Self { neutral, captured: true })
    }
}

/// How far each measurement must move from neutral to reach the end of its output range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sensitivity {
    /// Eyebrow travel (in eye distances) for the full -1 to 1 range.
    pub brow: f32,
    /// Eye height, as a fraction of the neutral eye height, at which the eye counts as shut.
    pub eye_shut: f32,
    /// Eye height, as a fraction of the neutral eye height, at which the eye counts as open.
    pub eye_open: f32,
    /// Inner-lip gap (in eye distances) for a fully open mouth.
    pub mouth_open: f32,
    /// Change of mouth width, as a fraction of neutral, for the full -1 to 1 range.
    pub mouth_wide: f32,
    /// Corner lift (over mouth width) for the full -1 to 1 range.
    pub smile: f32,
    /// Pupil travel (in half eye widths) for the full -1 to 1 range.
    pub gaze: f32,
}

impl Default for Sensitivity {
    fn default() -> Self {
        Self { brow: 0.10, eye_shut: 0.35, eye_open: 0.85, mouth_open: 0.45, mouth_wide: 0.25, smile: 0.12, gaze: 0.8 }
    }
}

/// One-Euro filter parameters (Casiez et al.): a low cutoff steadies slow movement, the speed term keeps fast
/// movement from lagging.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// Cutoff in Hz at rest.
    pub min_cutoff: f32,
    /// How much the cutoff rises with speed.
    pub beta: f32,
}

/// Smoothing per kind of channel. Blinks and speech need little smoothing; head and gaze need more.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Smoothing {
    /// Switch smoothing off entirely.
    pub enabled: bool,
    /// Head pose and position.
    pub head: Filter,
    /// Eyebrows.
    pub brows: Filter,
    /// Eyelids.
    pub eyes: Filter,
    /// Pupils.
    pub gaze: Filter,
    /// Mouth.
    pub mouth: Filter,
}

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            enabled: true,
            head: Filter { min_cutoff: 2.0, beta: 1.5 },
            brows: Filter { min_cutoff: 2.5, beta: 0.5 },
            eyes: Filter { min_cutoff: 10.0, beta: 0.0 },
            gaze: Filter { min_cutoff: 3.0, beta: 0.5 },
            mouth: Filter { min_cutoff: 10.0, beta: 0.0 },
        }
    }
}

impl Smoothing {
    /// No smoothing: values pass through as measured.
    #[must_use]
    pub fn off() -> Self {
        Self { enabled: false, ..Self::default() }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct OneEuro {
    last: Option<(f32, f32)>,
}

impl OneEuro {
    fn alpha(dt: f32, cutoff: f32) -> f32 {
        let tau = 1.0 / (std::f32::consts::TAU * cutoff.max(1e-3));
        1.0 / (1.0 + tau / dt)
    }

    fn apply(&mut self, x: f32, dt: f32, p: Filter) -> f32 {
        let Some((prev, prev_dx)) = self.last else {
            self.last = Some((x, 0.0));
            return x;
        };
        let dx = (x - prev) / dt;
        let dx_hat = prev_dx + Self::alpha(dt, 1.0) * (dx - prev_dx);
        let cutoff = p.min_cutoff + p.beta * dx_hat.abs();
        let out = prev + Self::alpha(dt, cutoff) * (x - prev);
        self.last = Some((out, dx_hat));
        out
    }
}

const CHANNELS: usize = 15;

/// What one call to [`FaceSolver::solve`] produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solved {
    /// The face for this frame.
    pub input: FaceInput,
    /// Set on the frame a calibration capture completed; the solver has already adopted it.
    pub finished_calibration: Option<Calibration>,
}

#[derive(Clone, Debug)]
struct Capture {
    wanted: usize,
    samples: Vec<Measures>,
}

/// Turns landmarks into [`FaceInput`] frames: measures, subtracts the neutral face, maps to output ranges, smooths.
#[derive(Clone, Debug)]
pub struct FaceSolver {
    /// The neutral face in use.
    pub calibration: Calibration,
    /// Output ranges.
    pub sensitivity: Sensitivity,
    /// Smoothing.
    pub smoothing: Smoothing,
    filters: [OneEuro; CHANNELS],
    last_time: Option<Tick>,
    last: FaceInput,
    capture: Option<Capture>,
}

/// Frames further apart than this restart the smoothing instead of gliding from stale values.
const RESET_GAP_SECONDS: f64 = 0.5;

impl Default for FaceSolver {
    fn default() -> Self {
        Self {
            calibration: Calibration::default(),
            sensitivity: Sensitivity::default(),
            smoothing: Smoothing::default(),
            filters: [OneEuro::default(); CHANNELS],
            last_time: None,
            last: FaceInput::default(),
            capture: None,
        }
    }
}

const fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

const fn signed(v: f32) -> f32 {
    if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }
}

impl FaceSolver {
    /// A solver with a captured calibration.
    #[must_use]
    pub fn with_calibration(calibration: Calibration) -> Self {
        Self { calibration, ..Self::default() }
    }

    /// This solver with different smoothing.
    #[must_use]
    pub const fn with_smoothing(mut self, smoothing: Smoothing) -> Self {
        self.smoothing = smoothing;
        self
    }

    /// Starts capturing the neutral face from the next `frames` tracked frames.
    pub fn begin_calibration(&mut self, frames: usize) {
        self.capture = Some(Capture { wanted: frames.max(1), samples: Vec::new() });
    }

    /// Frames captured so far and frames wanted, while a calibration is running.
    #[must_use]
    pub fn calibration_progress(&self) -> Option<(usize, usize)> {
        self.capture.as_ref().map(|c| (c.samples.len(), c.wanted))
    }

    /// Abandons a running calibration capture.
    pub fn cancel_calibration(&mut self) {
        self.capture = None;
    }

    /// Goes back to the built-in typical face.
    pub fn reset_calibration(&mut self) {
        self.calibration = Calibration::default();
    }

    /// Forgets the smoothing state (after a pause, or a new subject).
    pub fn reset_smoothing(&mut self) {
        self.filters = [OneEuro::default(); CHANNELS];
        self.last_time = None;
    }

    /// Maps raw measurements to unsmoothed output values, in the order of the filter channels.
    fn map(&self, m: &Measures) -> [f32; CHANNELS] {
        let (n, s) = (&self.calibration.neutral, &self.sensitivity);
        let open = |ratio: f32, neutral: f32| {
            let r = if neutral > f32::EPSILON { ratio / neutral } else { 1.0 };
            unit((r - s.eye_shut) / (s.eye_open - s.eye_shut).max(f32::EPSILON))
        };
        let width = if n.mouth_width > f32::EPSILON { m.mouth_width / n.mouth_width - 1.0 } else { 0.0 };
        [
            m.orientation.yaw - n.orientation.yaw,
            m.orientation.pitch - n.orientation.pitch,
            m.orientation.roll - n.orientation.roll,
            m.center[0] - n.center[0],
            m.center[1] - n.center[1],
            if n.size > f32::EPSILON { m.size / n.size } else { 1.0 },
            signed((m.brow_l - n.brow_l) / s.brow),
            signed((m.brow_r - n.brow_r) / s.brow),
            open(m.eye_l, n.eye_l),
            open(m.eye_r, n.eye_r),
            signed((m.gaze[0] - n.gaze[0]) / s.gaze),
            signed((m.gaze[1] - n.gaze[1]) / s.gaze),
            unit((m.mouth_open - n.mouth_open) / s.mouth_open),
            signed(width / s.mouth_wide),
            signed((m.smile - n.smile) / s.smile),
        ]
    }

    fn smooth(&mut self, mut v: [f32; CHANNELS], time: Tick) -> [f32; CHANNELS] {
        if !self.smoothing.enabled {
            return v;
        }
        let gap = self.last_time.map(|t| (time.seconds() - t.seconds()).max(0.0));
        if gap.is_some_and(|g| g > RESET_GAP_SECONDS) {
            self.filters = [OneEuro::default(); CHANNELS];
        }
        // Frames stamped at (almost) the same instant carry no speed information: use a nominal 60 Hz step.
        let dt = gap.filter(|g| *g > 1e-4).map_or(1.0 / 60.0, anim_core::num::f64_to_f32);
        let p = &self.smoothing;
        let params = [p.head, p.head, p.head, p.head, p.head, p.head, p.brows, p.brows, p.eyes, p.eyes, p.gaze, p.gaze, p.mouth, p.mouth, p.mouth];
        for ((value, filter), params) in v.iter_mut().zip(self.filters.iter_mut()).zip(params) {
            *value = filter.apply(*value, dt, params);
        }
        v
    }

    /// Solves one frame. `landmarks` is `None` when no face was found; the result is then the last face with
    /// `tracked` false.
    pub fn solve(&mut self, time: Tick, landmarks: Option<&Landmarks>) -> Solved {
        let Some((l, m)) = landmarks.and_then(|l| measure(l).map(|m| (l, m))) else {
            self.last = FaceInput { tracked: false, confidence: 0.0, ..self.last };
            return Solved { input: self.last, finished_calibration: None };
        };

        let mut finished_calibration = None;
        if let Some(capture) = &mut self.capture {
            capture.samples.push(m);
            if capture.samples.len() >= capture.wanted {
                finished_calibration = Calibration::from_samples(&capture.samples);
                self.capture = None;
                if let Some(c) = finished_calibration {
                    self.calibration = c;
                    // Filter state holds values measured against the old neutral face.
                    self.filters = [OneEuro::default(); CHANNELS];
                }
            }
        }

        let raw = self.map(&m);
        let [yaw, pitch, roll, x, y, scale, brow_l, brow_r, eye_l, eye_r, gaze_x, gaze_y, mouth_open, mouth_wide, smile] = self.smooth(raw, time);
        self.last_time = Some(time);
        self.last = FaceInput {
            tracked: true,
            confidence: unit(l.score),
            head: HeadPose { yaw, pitch, roll, x, y, scale },
            brow_l: signed(brow_l),
            brow_r: signed(brow_r),
            eye_open_l: unit(eye_l),
            eye_open_r: unit(eye_r),
            gaze: [signed(gaze_x), signed(gaze_y)],
            mouth_open: unit(mouth_open),
            mouth_wide: signed(mouth_wide),
            smile: signed(smile),
        };
        Solved { input: self.last, finished_calibration }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solve_once(solver: &mut FaceSolver, face: &SyntheticFace) -> FaceInput {
        solver.solve(Tick::ZERO, Some(&face.landmarks())).input
    }

    fn unsmoothed() -> FaceSolver {
        FaceSolver { smoothing: Smoothing::off(), ..FaceSolver::default() }
    }

    fn captured_neutral(face: &SyntheticFace) -> FaceSolver {
        let m = measure(&face.landmarks()).unwrap();
        FaceSolver { calibration: Calibration::from_samples(&[m, m, m]).unwrap(), smoothing: Smoothing::off(), ..FaceSolver::default() }
    }

    #[test]
    fn the_neutral_face_maps_to_neutral_values() {
        let face = SyntheticFace { yaw: 0.2, pitch: -0.1, roll: 0.05, eye_distance_px: 90.0, center: [0.3, -0.2], ..SyntheticFace::default() };
        let mut s = captured_neutral(&face);
        let out = solve_once(&mut s, &face);
        assert!(out.tracked && out.is_valid());
        assert!(out.head.yaw.abs() < 1e-4 && out.head.pitch.abs() < 1e-4 && out.head.roll.abs() < 1e-4);
        assert!(out.head.x.abs() < 1e-4 && out.head.y.abs() < 1e-4 && (out.head.scale - 1.0).abs() < 1e-4);
        assert!(out.brow_l.abs() < 1e-3 && out.brow_r.abs() < 1e-3 && out.mouth_open.abs() < 1e-3 && out.smile.abs() < 1e-3 && out.mouth_wide.abs() < 1e-3);
        assert!((out.eye_open_l - 1.0).abs() < 1e-3 && (out.eye_open_r - 1.0).abs() < 1e-3);
        assert!(out.gaze[0].abs() < 1e-3 && out.gaze[1].abs() < 1e-3);
    }

    #[test]
    fn the_built_in_default_reads_a_typical_neutral_face_as_neutral() {
        let out = solve_once(&mut unsmoothed(), &SyntheticFace::default());
        assert!(out.brow_l.abs() < 0.05 && out.mouth_open.abs() < 0.05 && out.smile.abs() < 0.05);
        assert!(out.eye_open_l > 0.99);
    }

    #[test]
    fn expressions_map_to_their_channels() {
        let neutral = SyntheticFace::default();
        let mut s = captured_neutral(&neutral);
        let mut with = |f: SyntheticFace| solve_once(&mut s, &f);
        assert!(with(SyntheticFace { brow_raise: [0.1, 0.0], ..neutral }).brow_l > 0.9);
        assert!(with(SyntheticFace { brow_raise: [-0.05, -0.05], ..neutral }).brow_r < -0.4);
        assert!(with(SyntheticFace { eye_open: [0.0, 1.0], ..neutral }).eye_open_l < 0.01);
        assert!(with(SyntheticFace { eye_open: [0.5, 0.5], ..neutral }).eye_open_l > 0.1);
        assert!(with(SyntheticFace { mouth_open: 0.45, ..neutral }).mouth_open > 0.95);
        assert!(with(SyntheticFace { mouth_open: 0.2, ..neutral }).mouth_open > 0.3);
        assert!(with(SyntheticFace { smile: 0.1, ..neutral }).smile > 0.5);
        assert!(with(SyntheticFace { smile: -0.1, ..neutral }).smile < -0.5);
        assert!(with(SyntheticFace { mouth_wide: 1.3, ..neutral }).mouth_wide > 0.9);
        assert!(with(SyntheticFace { mouth_wide: 0.8, ..neutral }).mouth_wide < -0.5);
        assert!(with(SyntheticFace { gaze: [1.0, 0.0], ..neutral }).gaze[0] > 0.5);
        assert!(with(SyntheticFace { yaw: 0.4, ..neutral }).head.yaw > 0.3);
        assert!(with(SyntheticFace { center: [0.5, 0.0], ..neutral }).head.x > 0.4);
        assert!(with(SyntheticFace { center: [0.0, 0.5], ..neutral }).head.y > 0.4);
        assert!(with(SyntheticFace { eye_distance_px: 240.0, ..neutral }).head.scale > 1.9);
    }

    #[test]
    fn outputs_are_always_in_range() {
        let neutral = SyntheticFace::default();
        let mut s = captured_neutral(&neutral);
        for f in [
            SyntheticFace { brow_raise: [5.0, -5.0], mouth_open: 9.0, smile: 9.0, mouth_wide: 9.0, gaze: [9.0, -9.0], ..neutral },
            SyntheticFace { eye_open: [-3.0, 30.0], mouth_wide: 0.01, smile: -9.0, ..neutral },
            SyntheticFace { yaw: 1.4, pitch: 1.0, roll: 3.0, ..neutral },
        ] {
            assert!(solve_once(&mut s, &f).is_valid(), "{f:?}");
        }
    }

    #[test]
    fn no_face_keeps_the_last_values_and_says_so() {
        let mut s = unsmoothed();
        let seen = solve_once(&mut s, &SyntheticFace { mouth_open: 0.3, ..SyntheticFace::default() });
        assert!(seen.tracked);
        let lost = s.solve(Tick::from_seconds_f64(0.033), None).input;
        assert!(!lost.tracked && lost.confidence.abs() < f32::EPSILON);
        assert!((lost.mouth_open - seen.mouth_open).abs() < 1e-6);
        let degenerate = Landmarks { points: vec![], ..Landmarks::default() };
        assert!(!s.solve(Tick::ZERO, Some(&degenerate)).input.tracked);
        assert!(!FaceSolver::default().solve(Tick::ZERO, None).input.tracked);
    }

    #[test]
    fn calibration_captures_the_average_of_the_neutral_frames() {
        let mut s = FaceSolver { smoothing: Smoothing::off(), ..FaceSolver::default() };
        let rest = SyntheticFace { yaw: 0.1, mouth_open: 0.05, eye_distance_px: 100.0, ..SyntheticFace::default() };
        s.begin_calibration(3);
        assert_eq!(s.calibration_progress(), Some((0, 3)));
        let mut done = None;
        for i in 0..3 {
            let out = s.solve(Tick::from_seconds_f64(f64::from(i) / 30.0), Some(&rest.landmarks()));
            done = out.finished_calibration.or(done);
        }
        let c = done.expect("calibration finishes after three frames");
        assert!(c.captured && s.calibration == c && s.calibration_progress().is_none());
        let out = solve_once(&mut s, &rest);
        assert!(out.head.yaw.abs() < 1e-3 && out.mouth_open.abs() < 1e-3, "the rest pose reads as neutral: {out:?}");
        s.reset_calibration();
        assert!(!s.calibration.captured);
    }

    #[test]
    fn calibration_ignores_frames_without_a_face_and_can_be_cancelled() {
        let mut s = unsmoothed();
        s.begin_calibration(2);
        s.solve(Tick::ZERO, None);
        assert_eq!(s.calibration_progress(), Some((0, 2)));
        s.cancel_calibration();
        assert!(s.calibration_progress().is_none());
        assert!(Calibration::from_samples(&[]).is_none());
    }

    #[test]
    fn smoothing_steadies_jitter_but_follows_real_movement() {
        let mut s = FaceSolver::default();
        let at = |i: u32| Tick::from_seconds_f64(f64::from(i) / 60.0);
        // Jitter around a rest value.
        let mut spread = 0.0_f32;
        for i in 0..60 {
            let jitter = if i % 2 == 0 { 0.02 } else { -0.02 };
            let out = s.solve(at(i), Some(&SyntheticFace { yaw: jitter, ..SyntheticFace::default() }.landmarks())).input;
            if i > 20 {
                spread = spread.max(out.head.yaw.abs());
            }
        }
        assert!(spread < 0.01, "jitter of 0.02 rad is damped: {spread}");
        // A step to a large movement is followed within a few frames.
        let mut yaw = 0.0;
        for i in 60..75 {
            yaw = s.solve(at(i), Some(&SyntheticFace { yaw: 0.5, ..SyntheticFace::default() }.landmarks())).input.head.yaw;
        }
        assert!(yaw > 0.4, "{yaw}");
    }

    #[test]
    fn a_long_gap_restarts_smoothing() {
        let mut s = FaceSolver::default();
        s.solve(Tick::ZERO, Some(&SyntheticFace::default().landmarks()));
        let jumped = s.solve(Tick::from_seconds_f64(5.0), Some(&SyntheticFace { yaw: 0.5, ..SyntheticFace::default() }.landmarks())).input;
        assert!(jumped.head.yaw > 0.45, "no glide from stale state: {}", jumped.head.yaw);
    }

    #[test]
    fn identical_timestamps_do_not_blow_up() {
        let mut s = FaceSolver::default();
        for _ in 0..5 {
            assert!(solve_once(&mut s, &SyntheticFace { yaw: 0.2, ..SyntheticFace::default() }).is_valid());
        }
    }
}
