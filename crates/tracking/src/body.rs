//! Full-body tracking: the keypoints a pose model finds, the rest pose, and the maths from keypoints to the
//! body-unit joints a puppet's Body Tracker behavior wants.
//!
//! # Conventions
//!
//! * [`Keypoints`] are in image fractions (x right, y down, 0 to 1) with the frame's aspect ratio next to them, so
//!   angles stay true on a wide frame. Joint names are the performer's own: their left shoulder is `LeftShoulder`
//!   wherever it is in the picture.
//! * [`BodyInput`] is in *body units*: the origin is the middle of the hips in the rest pose, one unit is the rest
//!   pose's torso length (hips to shoulders), x grows to the image's right (the performer's left), y grows down.
//!   Joints are measured against the **rest pose** the performer set with [`BodySolver::begin_rest_capture`]
//!   (stand as the puppet stands, arms down), so a body at rest gives the rest pose back wherever it stands in the
//!   frame, and a webcam that only sees the upper body works: missing hips are placed from the shoulders.

use anim_core::Tick;
use serde::{Deserialize, Serialize};

use crate::cast;
use crate::error::Result;
use crate::frame::Frame;
use crate::solver::{Filter, OneEuro};
use crate::source::FrameSource;

/// Number of body joints.
pub const JOINTS: usize = 17;

/// A joint, in the order pose models (COCO) list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs, reason = "the names are the documentation")]
pub enum Joint {
    Nose,
    LeftEye,
    RightEye,
    LeftEar,
    RightEar,
    LeftShoulder,
    RightShoulder,
    LeftElbow,
    RightElbow,
    LeftWrist,
    RightWrist,
    LeftHip,
    RightHip,
    LeftKnee,
    RightKnee,
    LeftAnkle,
    RightAnkle,
}

impl Joint {
    /// Every joint in model order.
    pub const ALL: [Self; JOINTS] = [
        Self::Nose,
        Self::LeftEye,
        Self::RightEye,
        Self::LeftEar,
        Self::RightEar,
        Self::LeftShoulder,
        Self::RightShoulder,
        Self::LeftElbow,
        Self::RightElbow,
        Self::LeftWrist,
        Self::RightWrist,
        Self::LeftHip,
        Self::RightHip,
        Self::LeftKnee,
        Self::RightKnee,
        Self::LeftAnkle,
        Self::RightAnkle,
    ];

    /// The joint's place in model order.
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|j| *j == self).unwrap_or(0)
    }

    /// The same joint on the other side of the body.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Nose => Self::Nose,
            Self::LeftEye => Self::RightEye,
            Self::RightEye => Self::LeftEye,
            Self::LeftEar => Self::RightEar,
            Self::RightEar => Self::LeftEar,
            Self::LeftShoulder => Self::RightShoulder,
            Self::RightShoulder => Self::LeftShoulder,
            Self::LeftElbow => Self::RightElbow,
            Self::RightElbow => Self::LeftElbow,
            Self::LeftWrist => Self::RightWrist,
            Self::RightWrist => Self::LeftWrist,
            Self::LeftHip => Self::RightHip,
            Self::RightHip => Self::LeftHip,
            Self::LeftKnee => Self::RightKnee,
            Self::RightKnee => Self::LeftKnee,
            Self::LeftAnkle => Self::RightAnkle,
            Self::RightAnkle => Self::LeftAnkle,
        }
    }
}

/// One keypoint a pose model found.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keypoint {
    /// Horizontal place as a fraction of the frame width.
    pub x: f32,
    /// Vertical place as a fraction of the frame height, growing downward.
    pub y: f32,
    /// The model's confidence, 0 to 1.
    pub score: f32,
}

/// The keypoints of one body in one frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keypoints {
    /// Width over height of the frame the fractions refer to.
    pub aspect: f32,
    /// One entry per [`Joint`], in model order.
    pub points: Vec<Keypoint>,
}

impl Keypoints {
    /// The keypoint of a joint.
    #[must_use]
    pub fn get(&self, joint: Joint) -> Option<Keypoint> {
        self.points.get(joint.index()).copied().filter(|k| k.x.is_finite() && k.y.is_finite() && k.score.is_finite())
    }

    /// The keypoint when the model is at least `min_score` sure of it.
    #[must_use]
    pub fn seen(&self, joint: Joint, min_score: f32) -> Option<Keypoint> {
        self.get(joint).filter(|k| k.score >= min_score)
    }

    /// The mean confidence of the keypoints.
    #[must_use]
    pub fn mean_score(&self) -> f32 {
        let n = self.points.len();
        if n == 0 { 0.0 } else { self.points.iter().map(|k| k.score).sum::<f32>() / cast::f(n) }
    }
}

/// Something that finds a body's keypoints in a frame.
pub trait BodyBackend {
    /// The keypoints of the body in `frame`, or `None` when there is none.
    ///
    /// # Errors
    /// Inference failures.
    fn keypoints(&mut self, frame: &Frame) -> Result<Option<Keypoints>>;
}

/// One camera frame's result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyObservation {
    /// Capture time since the source started.
    pub time: Tick,
    /// The body, or `None` when none was found.
    pub keypoints: Option<Keypoints>,
}

/// A stream of body observations; like [`crate::source::FaceSource`] it is made and used on the actor's thread.
pub trait BodySource {
    /// A short description for status displays.
    fn name(&self) -> String;

    /// The next observation, blocking until it arrives; `Ok(None)` when the stream has ended.
    ///
    /// # Errors
    /// Whatever the device or model reports.
    fn next(&mut self) -> Result<Option<BodyObservation>>;
}

/// A scripted or recorded body source, for tests and for machines without a camera.
#[derive(Clone, Debug)]
pub struct ScriptedBodySource {
    name: String,
    frames: Vec<BodyObservation>,
    next: usize,
    looping: bool,
    loops: i64,
    interval: Option<std::time::Duration>,
    clock: Option<std::time::Instant>,
}

impl ScriptedBodySource {
    /// Replays `frames` once, as fast as they are asked for.
    #[must_use]
    pub fn new(name: impl Into<String>, frames: Vec<BodyObservation>) -> Self {
        Self { name: name.into(), frames, next: 0, looping: false, loops: 0, interval: None, clock: None }
    }

    /// Synthetic bodies stamped at `fps` frames per second.
    #[must_use]
    pub fn from_bodies(name: impl Into<String>, bodies: &[SyntheticBody], fps: u32) -> Self {
        let step = Tick::from_units(1, i64::from(fps.max(1)));
        let frames = (0_i64..).zip(bodies).map(|(i, b)| BodyObservation { time: Tick(step.0.saturating_mul(i)), keypoints: Some(b.keypoints()) }).collect();
        Self::new(name, frames)
    }

    /// A body waving its left arm, looping forever and paced like a camera.
    #[must_use]
    pub fn demo(frames: usize, fps: u32) -> Self {
        let interval = std::time::Duration::from_secs(1).checked_div(fps.max(1)).unwrap_or_default();
        Self::from_bodies("demo body", &demo_bodies(frames), fps).looping().paced(interval)
    }

    /// Start over when the frames run out.
    #[must_use]
    pub const fn looping(mut self) -> Self {
        self.looping = true;
        self
    }

    /// Wait `interval` between frames, like a camera does.
    #[must_use]
    pub const fn paced(mut self, interval: std::time::Duration) -> Self {
        self.interval = Some(interval);
        self
    }
}

impl BodySource for ScriptedBodySource {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn next(&mut self) -> Result<Option<BodyObservation>> {
        if self.frames.is_empty() {
            return Ok(None);
        }
        if let Some(interval) = self.interval {
            let started = *self.clock.get_or_insert_with(std::time::Instant::now);
            let due = started.checked_add(interval.saturating_mul(u32::try_from(self.next).unwrap_or(u32::MAX))).unwrap_or(started);
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        let Some(obs) = self.frames.get(self.next) else {
            if !self.looping {
                return Ok(None);
            }
            self.next = 0;
            self.loops = self.loops.saturating_add(1);
            self.clock = None;
            return self.next();
        };
        self.next = self.next.saturating_add(1);
        let span = self.frames.last().map_or(0, |l| l.time.0).saturating_add(self.frames.get(1).map_or(0, |f| f.time.0));
        Ok(Some(BodyObservation { time: Tick(obs.time.0.saturating_add(span.saturating_mul(self.loops))), keypoints: obs.keypoints.clone() }))
    }
}

/// A [`FrameSource`] and a [`BodyBackend`] together make a [`BodySource`].
#[derive(Debug)]
pub struct TrackedBodyCamera<F, B> {
    frames: F,
    backend: B,
}

impl<F: FrameSource, B: BodyBackend> TrackedBodyCamera<F, B> {
    /// Combines a frame source with a keypoint backend.
    #[must_use]
    pub const fn new(frames: F, backend: B) -> Self {
        Self { frames, backend }
    }
}

impl<F: FrameSource, B: BodyBackend> BodySource for TrackedBodyCamera<F, B> {
    fn name(&self) -> String {
        self.frames.name()
    }

    fn next(&mut self) -> Result<Option<BodyObservation>> {
        let Some((time, frame)) = self.frames.next_frame()? else { return Ok(None) };
        let keypoints = self.backend.keypoints(&frame)?;
        Ok(Some(BodyObservation { time, keypoints }))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Body units.

/// The typical standing body in body units (hips' middle at the origin, one unit of torso), arms by the sides, in
/// model order. It is what the rest pose is until the performer sets their own.
#[must_use]
pub const fn standard_rest() -> [[f32; 2]; JOINTS] {
    [
        [0.0, -1.4],
        [0.08, -1.45],
        [-0.08, -1.45],
        [0.18, -1.4],
        [-0.18, -1.4],
        [0.45, -1.0],
        [-0.45, -1.0],
        [0.5, -0.4],
        [-0.5, -0.4],
        [0.5, 0.2],
        [-0.5, 0.2],
        [0.25, 0.0],
        [-0.25, 0.0],
        [0.25, 0.9],
        [-0.25, 0.9],
        [0.25, 1.8],
        [-0.25, 1.8],
    ]
}

/// The rest pose and the framing it was taken in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyCalibration {
    /// Where the hips' middle was, in frame-height units (x already multiplied by the aspect).
    pub hip_center: [f32; 2],
    /// The torso's length (hips to shoulders) in frame-height units.
    pub torso: f32,
    /// The joints at rest, in body units.
    pub rest: [[f32; 2]; JOINTS],
    /// Whether this was captured from the performer (`false` for the built-in default).
    pub captured: bool,
}

impl Default for BodyCalibration {
    /// A seated performer framed from the waist up: hips below the picture, a torso a third of its height.
    fn default() -> Self {
        Self { hip_center: [0.66, 1.05], torso: 0.34, rest: standard_rest(), captured: false }
    }
}

/// Smoothing of the joints.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodySmoothing {
    /// Switch smoothing off entirely.
    pub enabled: bool,
    /// The One-Euro filter every joint coordinate runs through.
    pub filter: Filter,
}

impl Default for BodySmoothing {
    fn default() -> Self {
        Self { enabled: true, filter: Filter { min_cutoff: 1.5, beta: 0.8 } }
    }
}

/// The tracked body for one frame, joints in body units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyInput {
    /// Whether a body was found in this frame. When `false` the joints are all missing.
    pub tracked: bool,
    /// The model's mean confidence, 0 to 1.
    pub confidence: f32,
    /// Where each joint is now, in model order; `None` for joints the model is unsure of.
    pub joints: [Option<[f32; 2]>; JOINTS],
    /// The rest pose the joints are measured against.
    pub rest: [[f32; 2]; JOINTS],
}

impl Default for BodyInput {
    fn default() -> Self {
        Self { tracked: false, confidence: 0.0, joints: [None; JOINTS], rest: standard_rest() }
    }
}

impl BodyInput {
    /// The joint, if it was seen.
    #[must_use]
    pub fn joint(&self, joint: Joint) -> Option<[f32; 2]> {
        self.joints.get(joint.index()).copied().flatten()
    }

    /// Whether every number is finite.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.confidence.is_finite()
            && self.joints.iter().flatten().all(|p| p.iter().all(|v| v.is_finite()))
            && self.rest.iter().all(|p| p.iter().all(|v| v.is_finite()))
    }

    /// Swaps left and right: for performers who want the puppet to move like their reflection.
    #[must_use]
    pub fn mirrored(&self) -> Self {
        let mut out = *self;
        for joint in Joint::ALL {
            let to = joint.opposite().index();
            if let (Some(slot), Some(p)) = (out.joints.get_mut(to), self.joints.get(joint.index())) {
                *slot = p.map(|[x, y]| [-x, y]);
            }
            if let (Some(slot), Some([x, y])) = (out.rest.get_mut(to), self.rest.get(joint.index())) {
                *slot = [-x, *y];
            }
        }
        out
    }
}

/// What one call to [`BodySolver::solve`] produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodySolved {
    /// The body for this frame.
    pub input: BodyInput,
    /// Set on the frame a rest-pose capture completed; the solver has already adopted it.
    pub finished_rest: Option<BodyCalibration>,
}

#[derive(Clone, Debug)]
struct RestCapture {
    wanted: usize,
    frames: Vec<Keypoints>,
}

/// Keypoints below this confidence are treated as not seen.
pub const MIN_SCORE: f32 = 0.3;

/// Frames further apart than this restart the smoothing.
const RESET_GAP_SECONDS: f64 = 0.5;

/// From keypoints to [`BodyInput`]: the rest pose, the body-unit maths and smoothing.
#[derive(Clone, Debug)]
pub struct BodySolver {
    /// The rest pose.
    pub calibration: BodyCalibration,
    /// Smoothing.
    pub smoothing: BodySmoothing,
    /// Swap left and right.
    pub mirror: bool,
    filters: [[OneEuro; 2]; JOINTS],
    last_time: Option<Tick>,
    capture: Option<RestCapture>,
}

impl Default for BodySolver {
    fn default() -> Self {
        Self {
            calibration: BodyCalibration::default(),
            smoothing: BodySmoothing::default(),
            mirror: false,
            filters: [[OneEuro::default(); 2]; JOINTS],
            last_time: None,
            capture: None,
        }
    }
}

/// A keypoint in frame-height units.
fn iso(k: Keypoint, aspect: f32) -> [f32; 2] {
    [k.x * aspect, k.y]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl BodyCalibration {
    /// The rest pose averaged from `frames` of the performer standing as the puppet stands. `None` when no frame
    /// shows both shoulders.
    #[must_use]
    pub fn from_frames(frames: &[Keypoints]) -> Option<Self> {
        let usable: Vec<&Keypoints> =
            frames.iter().filter(|k| k.seen(Joint::LeftShoulder, MIN_SCORE).is_some() && k.seen(Joint::RightShoulder, MIN_SCORE).is_some()).collect();
        if usable.is_empty() {
            return None;
        }
        // Mean of each joint over the frames that saw it, in frame-height units.
        let mean = |joint: Joint| -> Option<[f32; 2]> {
            let seen: Vec<[f32; 2]> = usable.iter().filter_map(|f| f.seen(joint, MIN_SCORE).map(|p| iso(p, f.aspect))).collect();
            let m = u16::try_from(seen.len()).ok().filter(|m| *m > 0)?;
            let inv = 1.0 / f32::from(m);
            Some([seen.iter().map(|p| p[0]).sum::<f32>() * inv, seen.iter().map(|p| p[1]).sum::<f32>() * inv])
        };
        let (ls, rs) = (mean(Joint::LeftShoulder)?, mean(Joint::RightShoulder)?);
        let shoulders = [f32::midpoint(ls[0], rs[0]), f32::midpoint(ls[1], rs[1])];
        let hips = mean(Joint::LeftHip).zip(mean(Joint::RightHip)).map(|(l, r)| [f32::midpoint(l[0], r[0]), f32::midpoint(l[1], r[1])]);
        // Without hips in view the torso is taken as the shoulder width over the typical body's ratio.
        let typical = standard_rest();
        let width_ratio = typical.get(Joint::LeftShoulder.index()).zip(typical.get(Joint::RightShoulder.index())).map_or(0.9, |(l, r)| (l[0] - r[0]).abs());
        let (hip_center, torso) = hips.map_or_else(
            || {
                let torso = (dist(ls, rs) / width_ratio.max(0.1)).max(1e-3);
                ([shoulders[0], shoulders[1] + torso], torso)
            },
            |h| (h, dist(shoulders, h).max(1e-3)),
        );
        let mut rest = standard_rest();
        for joint in Joint::ALL {
            if let (Some(p), Some(slot)) = (mean(joint), rest.get_mut(joint.index())) {
                *slot = [(p[0] - hip_center[0]) / torso, (p[1] - hip_center[1]) / torso];
            }
        }
        Some(Self { hip_center, torso, rest, captured: true })
    }
}

impl BodySolver {
    /// A solver with a captured rest pose.
    #[must_use]
    pub fn with_calibration(calibration: BodyCalibration) -> Self {
        Self { calibration, ..Self::default() }
    }

    /// Starts capturing the rest pose from the next `frames` frames that show the shoulders.
    pub fn begin_rest_capture(&mut self, frames: usize) {
        self.capture = Some(RestCapture { wanted: frames.max(1), frames: Vec::new() });
    }

    /// Abandons a running capture.
    pub fn cancel_rest_capture(&mut self) {
        self.capture = None;
    }

    /// Goes back to the built-in rest pose.
    pub fn reset_calibration(&mut self) {
        self.calibration = BodyCalibration::default();
        self.reset_smoothing();
    }

    /// Frames captured so far and wanted, while a capture runs.
    #[must_use]
    pub fn capture_progress(&self) -> Option<(usize, usize)> {
        self.capture.as_ref().map(|c| (c.frames.len(), c.wanted))
    }

    /// Forgets the smoothing state.
    pub fn reset_smoothing(&mut self) {
        self.filters = [[OneEuro::default(); 2]; JOINTS];
        self.last_time = None;
    }

    fn filter(&mut self, mut points: [Option<[f32; 2]>; JOINTS], time: Tick) -> [Option<[f32; 2]>; JOINTS] {
        if !self.smoothing.enabled {
            return points;
        }
        let gap = self.last_time.map(|t| (time.seconds() - t.seconds()).max(0.0));
        if gap.is_some_and(|g| g > RESET_GAP_SECONDS) {
            self.filters = [[OneEuro::default(); 2]; JOINTS];
        }
        let dt = gap.filter(|g| *g > 1e-4).map_or(1.0 / 60.0, anim_core::num::f64_to_f32);
        let p = self.smoothing.filter;
        for (point, filters) in points.iter_mut().zip(self.filters.iter_mut()) {
            match point {
                Some([x, y]) => {
                    *x = filters.first_mut().map_or(*x, |f| f.apply(*x, dt, p));
                    *y = filters.get_mut(1).map_or(*y, |f| f.apply(*y, dt, p));
                }
                // A joint that drops out starts fresh when it comes back, rather than gliding from where it was.
                None => *filters = [OneEuro::default(); 2],
            }
        }
        points
    }

    /// Solves one frame. `keypoints` is `None` when no body was found; the result then has `tracked` false.
    pub fn solve(&mut self, time: Tick, keypoints: Option<&Keypoints>) -> BodySolved {
        let Some(k) = keypoints.filter(|k| k.points.len() >= JOINTS) else {
            let input = BodyInput { rest: self.rest_for_output(), ..BodyInput::default() };
            return BodySolved { input, finished_rest: None };
        };
        let mut finished_rest = None;
        if let Some(capture) = &mut self.capture
            && k.seen(Joint::LeftShoulder, MIN_SCORE).is_some()
            && k.seen(Joint::RightShoulder, MIN_SCORE).is_some()
        {
            capture.frames.push(k.clone());
            if capture.frames.len() >= capture.wanted {
                finished_rest = BodyCalibration::from_frames(&capture.frames);
                self.capture = None;
                if let Some(c) = finished_rest {
                    self.calibration = c;
                    self.reset_smoothing();
                }
            }
        }
        let c = self.calibration;
        let mut joints = [None; JOINTS];
        for joint in Joint::ALL {
            if let (Some(p), Some(slot)) = (k.seen(joint, MIN_SCORE), joints.get_mut(joint.index())) {
                let [x, y] = iso(p, k.aspect);
                *slot = Some([(x - c.hip_center[0]) / c.torso.max(1e-3), (y - c.hip_center[1]) / c.torso.max(1e-3)]);
            }
        }
        let joints = self.filter(joints, time);
        self.last_time = Some(time);
        let mut input = BodyInput { tracked: true, confidence: k.mean_score().clamp(0.0, 1.0), joints, rest: c.rest };
        if self.mirror {
            input = input.mirrored();
        }
        BodySolved { input, finished_rest }
    }

    fn rest_for_output(&self) -> [[f32; 2]; JOINTS] {
        if self.mirror { BodyInput { rest: self.calibration.rest, ..BodyInput::default() }.mirrored().rest } else { self.calibration.rest }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A synthetic performer.

/// A parametric standing body in the keypoint layout, for tests, demos and development without a camera.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyntheticBody {
    /// Left arm raised out to the side, degrees: 0 hangs down, 90 is horizontal, 180 straight up.
    pub left_arm: f32,
    /// How far the left elbow is bent so the forearm turns further out, degrees.
    pub left_elbow: f32,
    /// The same for the right arm.
    pub right_arm: f32,
    /// The right elbow's bend.
    pub right_elbow: f32,
    /// The whole body's sideways shift, in torso lengths.
    pub shift: f32,
    /// Whether the legs are in the picture (a webcam seldom shows them).
    pub legs: bool,
    /// Whether the hips are in the picture.
    pub hips: bool,
    /// Where the hips' middle is in the frame, frame-height units.
    pub hip_center: [f32; 2],
    /// The torso's length, frame-height units.
    pub torso: f32,
    /// The frame's width over its height.
    pub aspect: f32,
}

impl Default for SyntheticBody {
    fn default() -> Self {
        Self {
            left_arm: 0.0,
            left_elbow: 0.0,
            right_arm: 0.0,
            right_elbow: 0.0,
            shift: 0.0,
            legs: true,
            hips: true,
            hip_center: [0.66, 0.62],
            torso: 0.2,
            aspect: 4.0 / 3.0,
        }
    }
}

impl SyntheticBody {
    /// The keypoints of this body.
    #[must_use]
    pub fn keypoints(&self) -> Keypoints {
        let rest = standard_rest();
        let mut pos = rest;
        // An arm: upper bone and forearm lengths come from the standard body.
        let arm = |pos: &mut [[f32; 2]; JOINTS], shoulder: Joint, elbow: Joint, wrist: Joint, side: f32, raise: f32, bend: f32| {
            let s = rest.get(shoulder.index()).copied().unwrap_or([0.0; 2]);
            let e = rest.get(elbow.index()).copied().unwrap_or([0.0; 2]);
            let w = rest.get(wrist.index()).copied().unwrap_or([0.0; 2]);
            let (l1, l2) = (dist(s, e), dist(e, w));
            let up = raise.to_radians();
            let elbow_at = [s[0] + side * l1 * up.sin(), s[1] + l1 * up.cos()];
            let fore = (raise + bend).to_radians();
            let wrist_at = [elbow_at[0] + side * l2 * fore.sin(), elbow_at[1] + l2 * fore.cos()];
            if let Some(slot) = pos.get_mut(elbow.index()) {
                *slot = elbow_at;
            }
            if let Some(slot) = pos.get_mut(wrist.index()) {
                *slot = wrist_at;
            }
        };
        arm(&mut pos, Joint::LeftShoulder, Joint::LeftElbow, Joint::LeftWrist, 1.0, self.left_arm, self.left_elbow);
        arm(&mut pos, Joint::RightShoulder, Joint::RightElbow, Joint::RightWrist, -1.0, self.right_arm, self.right_elbow);
        let points = Joint::ALL
            .iter()
            .zip(pos)
            .map(|(joint, [x, y])| {
                let hidden = (!self.legs && matches!(joint, Joint::LeftKnee | Joint::RightKnee | Joint::LeftAnkle | Joint::RightAnkle))
                    || (!self.hips && matches!(joint, Joint::LeftHip | Joint::RightHip));
                let px = (self.hip_center[0] + (x + self.shift) * self.torso) / self.aspect;
                let py = self.hip_center[1] + y * self.torso;
                Keypoint { x: px, y: py, score: if hidden { 0.02 } else { 0.9 } }
            })
            .collect();
        Keypoints { aspect: self.aspect, points }
    }
}

/// A body that stands still, raises its left arm to wave, lowers it, and leans from side to side: `frames` frames.
#[must_use]
pub fn demo_bodies(frames: usize) -> Vec<SyntheticBody> {
    let n = frames.max(1);
    (0..n)
        .map(|i| {
            let t = cast::f(i) / cast::f(n);
            let phase = t * std::f32::consts::TAU;
            SyntheticBody {
                left_arm: 100.0 + 25.0 * (phase * 3.0).sin(),
                left_elbow: 40.0 * (phase * 3.0).sin(),
                right_arm: 20.0 + 10.0 * phase.sin(),
                shift: 0.15 * phase.sin(),
                legs: false,
                hips: false,
                // Framed the way the built-in rest pose expects a seated performer.
                hip_center: BodyCalibration::default().hip_center,
                torso: BodyCalibration::default().torso,
                ..SyntheticBody::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(left_arm: f32) -> SyntheticBody {
        SyntheticBody { left_arm, ..SyntheticBody::default() }
    }

    fn unsmoothed() -> BodySolver {
        BodySolver { smoothing: BodySmoothing { enabled: false, ..BodySmoothing::default() }, ..BodySolver::default() }
    }

    /// A solver that has captured the standing body in the default synthetic framing.
    fn calibrated() -> BodySolver {
        let mut s = unsmoothed();
        s.begin_rest_capture(3);
        for i in 0..3 {
            s.solve(Tick::from_units(i, 30), Some(&SyntheticBody::default().keypoints()));
        }
        s
    }

    #[test]
    fn keypoints_follow_the_layout_and_hide_what_is_out_of_frame() {
        let k = SyntheticBody { legs: false, ..SyntheticBody::default() }.keypoints();
        assert_eq!(k.points.len(), JOINTS);
        assert!(k.seen(Joint::LeftShoulder, MIN_SCORE).is_some());
        assert!(k.seen(Joint::LeftKnee, MIN_SCORE).is_none());
        assert!(k.get(Joint::LeftKnee).is_some(), "the point exists, the model just is not sure of it");
        let l = k.get(Joint::LeftShoulder).unwrap();
        let r = k.get(Joint::RightShoulder).unwrap();
        assert!(l.x > r.x, "the performer's left is at the image's right");
    }

    #[test]
    fn capturing_the_rest_pose_finds_the_framing() {
        let s = calibrated();
        assert!(s.calibration.captured);
        assert!((s.calibration.torso - 0.2).abs() < 1e-3, "{}", s.calibration.torso);
        assert!((s.calibration.hip_center[0] - 0.66).abs() < 1e-3 && (s.calibration.hip_center[1] - 0.62).abs() < 1e-3, "{:?}", s.calibration.hip_center);
    }

    #[test]
    fn a_body_at_rest_comes_out_at_its_rest_pose_wherever_it_was_captured() {
        let mut s = calibrated();
        let out = s.solve(Tick::from_units(5, 30), Some(&SyntheticBody::default().keypoints())).input;
        assert!(out.tracked && out.is_valid());
        for joint in Joint::ALL {
            let (now, rest) = (out.joint(joint).unwrap(), out.rest[joint.index()]);
            assert!(dist(now, rest) < 1e-3, "{joint:?}: {now:?} vs {rest:?}");
        }
    }

    #[test]
    fn a_raised_arm_and_a_sideways_step_show_up_in_body_units() {
        let mut s = calibrated();
        let out = s.solve(Tick::from_units(5, 30), Some(&SyntheticBody { left_arm: 90.0, shift: 0.5, ..SyntheticBody::default() }.keypoints())).input;
        let wrist = out.joint(Joint::LeftWrist).unwrap();
        let shoulder = out.joint(Joint::LeftShoulder).unwrap();
        assert!((wrist[1] - shoulder[1]).abs() < 0.05, "the wrist is level with the shoulder: {wrist:?} {shoulder:?}");
        assert!(wrist[0] - shoulder[0] > 1.0, "and out to the side: {wrist:?}");
        let hip = out.joint(Joint::LeftHip).unwrap();
        assert!((hip[0] - (0.25 + 0.5)).abs() < 1e-3, "the step moved the hips half a torso: {hip:?}");
    }

    #[test]
    fn a_webcam_that_only_sees_the_shoulders_still_gets_a_rest_pose() {
        let upper = SyntheticBody { legs: false, hips: false, hip_center: [0.66, 0.9], torso: 0.3, ..SyntheticBody::default() };
        let mut s = unsmoothed();
        s.begin_rest_capture(2);
        for i in 0..2 {
            s.solve(Tick::from_units(i, 30), Some(&upper.keypoints()));
        }
        assert!(s.calibration.captured);
        assert!((s.calibration.torso - 0.3).abs() < 0.02, "torso from the shoulder width: {}", s.calibration.torso);
        let out = s.solve(Tick::from_units(9, 30), Some(&upper.keypoints())).input;
        assert!(out.joint(Joint::LeftHip).is_none() && out.joint(Joint::LeftKnee).is_none());
        assert!(dist(out.joint(Joint::LeftElbow).unwrap(), out.rest[Joint::LeftElbow.index()]) < 0.05);
    }

    #[test]
    fn capture_waits_for_both_shoulders_and_reports_progress() {
        let mut s = unsmoothed();
        s.begin_rest_capture(2);
        let hidden = SyntheticBody::default().keypoints();
        let mut blind = hidden.clone();
        blind.points[Joint::LeftShoulder.index()].score = 0.0;
        assert!(s.solve(Tick::ZERO, Some(&blind)).finished_rest.is_none());
        assert_eq!(s.capture_progress(), Some((0, 2)));
        s.solve(Tick::from_units(1, 30), Some(&hidden));
        assert_eq!(s.capture_progress(), Some((1, 2)));
        let done = s.solve(Tick::from_units(2, 30), Some(&hidden));
        assert!(done.finished_rest.is_some() && s.capture_progress().is_none());
        s.reset_calibration();
        assert!(!s.calibration.captured);
    }

    #[test]
    fn losing_the_body_reports_untracked() {
        let mut s = calibrated();
        let out = s.solve(Tick::from_units(5, 30), None).input;
        assert!(!out.tracked && out.joints.iter().all(Option::is_none));
    }

    #[test]
    fn mirroring_swaps_the_sides() {
        let mut s = calibrated();
        let plain = s.solve(Tick::from_units(5, 30), Some(&body(90.0).keypoints())).input;
        let mirrored = plain.mirrored();
        let (a, b) = (plain.joint(Joint::LeftWrist).unwrap(), mirrored.joint(Joint::RightWrist).unwrap());
        assert!((a[0] + b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
        assert_eq!(mirrored.mirrored(), plain);
    }

    #[test]
    fn smoothing_steadies_a_jittery_joint_but_follows_a_real_move() {
        let mut s = BodySolver { calibration: calibrated().calibration, ..BodySolver::default() };
        let mut last = 0.0;
        for i in 0..30 {
            let jitter = if i % 2 == 0 { 0.01 } else { -0.01 };
            let mut k = SyntheticBody::default().keypoints();
            k.points[Joint::LeftWrist.index()].y += jitter;
            let out = s.solve(Tick::from_units(i, 30), Some(&k)).input;
            last = out.joint(Joint::LeftWrist).unwrap()[1];
        }
        assert!((last - 0.2).abs() < 0.03, "steadied: {last}");
        let mut moved = 0.0;
        for i in 30..60 {
            let out = s.solve(Tick::from_units(i, 30), Some(&body(90.0).keypoints())).input;
            moved = out.joint(Joint::LeftWrist).unwrap()[1];
        }
        assert!(moved < -0.5, "followed the lift: {moved}");
    }

    #[test]
    fn scripted_sources_replay_and_loop() {
        let mut once = ScriptedBodySource::from_bodies("t", &demo_bodies(4), 30);
        let mut n = 0;
        while once.next().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 4);
        let mut looped = ScriptedBodySource::from_bodies("t", &demo_bodies(4), 30).looping();
        let times: Vec<Tick> = (0..9).filter_map(|_| looped.next().unwrap().map(|o| o.time)).collect();
        assert_eq!(times.len(), 9);
        assert!(times.windows(2).all(|w| w[0] < w[1]), "time never jumps back");
        assert!(ScriptedBodySource::new("e", vec![]).looping().next().unwrap().is_none());
    }

    #[test]
    fn hostile_keypoints_never_produce_non_finite_joints() {
        let mut s = BodySolver::default();
        let mut k = SyntheticBody::default().keypoints();
        k.points.fill(Keypoint { x: f32::NAN, y: f32::INFINITY, score: 0.9 });
        k.points[0] = Keypoint { x: 1e30, y: -1e30, score: 0.9 };
        let out = s.solve(Tick::ZERO, Some(&k)).input;
        assert!(out.is_valid());
        let short = Keypoints { aspect: 1.0, points: vec![] };
        assert!(!s.solve(Tick::from_units(1, 30), Some(&short)).input.tracked);
    }
}
