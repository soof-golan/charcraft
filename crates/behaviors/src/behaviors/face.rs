//! Face (`Adobe.FaceTracker2.js`): moves the head, eyebrows, eyes and mouth with the tracked face.

use std::collections::VecDeque;

use anim_core::Vec2;
use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Viseme, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::input::FaceInput;
use crate::util::{Smoother, Smoother2, add_group, percent};

/// How far the head travels across the stage when the face goes from the centre to the edge of the camera frame,
/// as a fraction of half the stage, at a strength of 100%.
pub const HEAD_TRAVEL: f64 = 0.3;
/// How far the head shifts for a 45 degree turn, as a fraction of the stage's width, at a parallax of 100%.
pub const PARALLAX_TRAVEL: f64 = 0.02;

/// Parameters of Face, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaceParams {
    /// Eyes follow the blink only: openness is read as shut or open, not as a partial lid.
    #[serde(rename = "blinkOnly")]
    pub blink_only: bool,
    /// Tilt of an eyebrow when it is lowered, degrees.
    #[serde(rename = "downEyebrowsTilt")]
    pub down_eyebrows_tilt: f64,
    /// Shows the pose-to-pose settings (a switch of the editor; the settings always apply when the level is above 0).
    #[serde(rename = "enablePoseToPoseAdvancedParams")]
    pub enable_pose_to_pose_advanced_params: bool,
    /// Eye strength, percent: how much of the tracked blink and gaze reaches the puppet.
    #[serde(rename = "eyeFactor")]
    pub eye_factor: f64,
    /// Eyebrow strength, percent.
    #[serde(rename = "eyebrowFactor")]
    pub eyebrow_factor: f64,
    /// Pose-to-pose: how far the eyebrows have to move from the held pose (a fraction of their range) before a new
    /// pose is taken, at a filtering level of 50.
    #[serde(rename = "eyebrowPosMeanThresh")]
    pub eyebrow_pos_mean_thresh: f64,
    /// Pose-to-pose: the least eyebrow movement (a fraction of their range) that still forces a new pose after the
    /// longest interval.
    #[serde(rename = "eyebrowPosVarThresh")]
    pub eyebrow_pos_var_thresh: f64,
    /// Pose-to-pose: a new pose is the mean of this many recent readings.
    #[serde(rename = "filterNumSamples")]
    pub filter_num_samples: f64,
    /// Head position strength, percent.
    #[serde(rename = "headPosFactor")]
    pub head_pos_factor: f64,
    /// Pose-to-pose: how far the head position and scale have to move from the held pose (a fraction of their
    /// range) before a new pose is taken, at a filtering level of 50.
    #[serde(rename = "headPosMeanThresh")]
    pub head_pos_mean_thresh: f64,
    /// Pose-to-pose: the least head movement (a fraction of the range) that still forces a new pose after the
    /// longest interval.
    #[serde(rename = "headPosVarThresh")]
    pub head_pos_var_thresh: f64,
    /// Vertical head position strength, percent.
    #[serde(rename = "headPosVerticalFactor")]
    pub head_pos_vertical_factor: f64,
    /// Head rotation strength, percent.
    #[serde(rename = "headRotFactor")]
    pub head_rot_factor: f64,
    /// Pose-to-pose: how far the head angles have to turn from the held pose (a fraction of 30 degrees) before a
    /// new pose is taken, at a filtering level of 50.
    #[serde(rename = "headRotMeanThresh")]
    pub head_rot_mean_thresh: f64,
    /// Pose-to-pose: the least head turn (a fraction of 30 degrees) that still forces a new pose after the longest
    /// interval.
    #[serde(rename = "headRotVarThresh")]
    pub head_rot_var_thresh: f64,
    /// Head scale strength, percent.
    #[serde(rename = "headScaleFactor")]
    pub head_scale_factor: f64,
    /// Pose-to-pose longest hold, seconds: after this long any movement over the var threshold takes a new pose.
    #[serde(rename = "maxInterval")]
    pub max_interval: f64,
    /// Pose-to-pose shortest gap between poses, seconds.
    #[serde(rename = "minInterval")]
    pub min_interval: f64,
    /// Pose-to-pose: how long a pose is held, seconds, at a filtering level of 100 (half that at 50), on top of
    /// the shortest gap.
    #[serde(rename = "minPoseDuration")]
    pub min_pose_duration: f64,
    /// Mouth strength, percent: how much the tracked mouth picks the mouth shape. 0 leaves it to Lip Sync.
    #[serde(rename = "mouthFactor")]
    pub mouth_factor: f64,
    /// Both eyebrows move as one (the mean of the two).
    #[serde(rename = "moveEyebrowsTogether")]
    pub move_eyebrows_together: bool,
    /// Parallax strength, percent: how much a head turn shifts the head sideways.
    #[serde(rename = "parallaxFactor")]
    pub parallax_factor: f64,
    /// Pose-to-pose filtering level, 0 to 100: 0 follows the face continuously; above it the head and eyebrows
    /// jump from one held pose to the next, and the higher the level the larger the move and the longer the hold
    /// that a new pose needs.
    #[serde(rename = "poseFilteringLevel")]
    pub pose_filtering_level: f64,
    /// Smoothing: the lag of the tracking, in hundredths of a second.
    #[serde(rename = "smoothingRate")]
    pub smoothing_rate: f64,
    /// Tilt of an eyebrow when it is raised, degrees.
    #[serde(rename = "upEyebrowsTilt")]
    pub up_eyebrows_tilt: f64,
}

impl Default for FaceParams {
    fn default() -> Self {
        Self {
            blink_only: true,
            down_eyebrows_tilt: -20.0,
            enable_pose_to_pose_advanced_params: false,
            eye_factor: 100.0,
            eyebrow_factor: 75.0,
            eyebrow_pos_mean_thresh: 0.25,
            eyebrow_pos_var_thresh: 0.01,
            filter_num_samples: 30.0,
            head_pos_factor: 100.0,
            head_pos_mean_thresh: 0.25,
            head_pos_var_thresh: 0.01,
            head_pos_vertical_factor: 100.0,
            head_rot_factor: 75.0,
            head_rot_mean_thresh: 0.25,
            head_rot_var_thresh: 0.01,
            head_scale_factor: 25.0,
            max_interval: 3.0,
            min_interval: 0.1,
            min_pose_duration: 1.0,
            mouth_factor: 0.0,
            move_eyebrows_together: true,
            parallax_factor: 100.0,
            pose_filtering_level: 0.0,
            smoothing_rate: 15.0,
            up_eyebrows_tilt: 20.0,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "blinkOnly", "", "Read each eye as shut or open instead of following the lid.";
    "downEyebrowsTilt", "deg", "Eyebrow tilt at full lowering (left eyebrow; the right mirrors it).";
    "eyeFactor", "%", "Strength of tracked blink and gaze.";
    "eyebrowFactor", "%", "Strength of tracked eyebrow raise.";
    "headPosFactor", "%", "Strength of horizontal head movement.";
    "headPosVerticalFactor", "%", "Strength of vertical head movement.";
    "headRotFactor", "%", "Strength of head tilt.";
    "headScaleFactor", "%", "Strength of moving towards or away from the camera.";
    "mouthFactor", "%", "Strength of the tracked mouth picking Smile, Surprised, Ah, Oh or Ee. 0 leaves the mouth to Lip Sync.";
    "moveEyebrowsTogether", "", "Both eyebrows follow the mean of the two.";
    "parallaxFactor", "%", "Strength of the sideways shift a head turn causes.";
    "poseFilteringLevel", "", "Pose-to-pose: 0 follows the face; above it the head and eyebrows hold limited poses.";
    "headRotMeanThresh", "", "Pose-to-pose: head turn (fraction of 30 degrees) that takes a new pose.";
    "headPosMeanThresh", "", "Pose-to-pose: head position or scale change (fraction of the range) that takes a new pose.";
    "eyebrowPosMeanThresh", "", "Pose-to-pose: eyebrow change (fraction of the range) that takes a new pose.";
    "minInterval", "s", "Pose-to-pose: shortest gap between poses.";
    "maxInterval", "s", "Pose-to-pose: longest hold before any small move takes a new pose.";
    "minPoseDuration", "s", "Pose-to-pose: hold of a pose at level 100.";
    "filterNumSamples", "", "Pose-to-pose: a new pose is the mean of this many readings.";
    "smoothingRate", "1/100 s", "Time constant of the low-pass filter on every tracked signal; 0 follows the camera exactly.";
    "upEyebrowsTilt", "deg", "Eyebrow tilt at full raise (left eyebrow; the right mirrors it).";
};

/// Drives the head, eyebrows, eyes and mouth from [`crate::FaceInput`].
///
/// Head turns (yaw and pitch) shift the head by the parallax amount; the Head Turner behavior swaps the views. When the face is lost the puppet relaxes to neutral at the smoothing rate.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Face {
    /// The parameters.
    pub params: FaceParams,
    yaw: Smoother,
    pitch: Smoother,
    roll: Smoother,
    translation: Smoother2,
    scale: Smoother,
    brow_left: Smoother,
    brow_right: Smoother,
    eye_left: Smoother,
    eye_right: Smoother,
    gaze: Smoother2,
    mouth_open: Smoother,
    mouth_wide: Smoother,
    smile: Smoother,
    head_hold: PoseHold,
    brow_hold: PoseHold,
}

/// The size of the full range of each head signal (yaw, pitch, roll in degrees, then the two positions and the
/// scale), which the pose-to-pose thresholds are fractions of.
const HEAD_RANGES: [f64; 6] = [30.0, 30.0, 30.0, 0.5, 0.5, 0.5];

/// The pose-to-pose settings, read off the parameters.
struct HoldConfig {
    /// Filtering level as a fraction of the stock one (0 off, 1 at 50%, 2 at 100%).
    level: f64,
    min_hold: f64,
    max_interval: f64,
    samples: usize,
}

impl HoldConfig {
    fn of(p: &FaceParams) -> Self {
        let level = if p.pose_filtering_level.is_finite() { p.pose_filtering_level.clamp(0.0, 100.0) / 50.0 } else { 0.0 };
        Self {
            level,
            min_hold: p.min_interval.max(0.0) + p.min_pose_duration.max(0.0) * level / 2.0,
            max_interval: p.max_interval.max(0.0),
            samples: usize::try_from(anim_core::num::round_f64_to_u32(p.filter_num_samples.clamp(1.0, 120.0))).unwrap_or(1).max(1),
        }
    }
}

/// Holds a set of signals as limited poses: the pose changes only when the signals have moved far enough from it
/// and it has been held long enough (or has been held longest-interval long and something moved at all), and the new
/// pose is the mean of the recent readings.
#[derive(Debug, Clone, PartialEq, Default)]
struct PoseHold {
    held: Option<Vec<f64>>,
    recent: VecDeque<Vec<f64>>,
    since: f64,
}

impl PoseHold {
    /// `ranges` is the size of each signal's full range and `thresholds` the fraction of it that counts as a move.
    fn update(&mut self, now: f64, current: &[f64], ranges: &[f64], thresholds: &[f64], var_thresh: f64, cfg: &HoldConfig) -> Vec<f64> {
        self.recent.push_back(current.to_vec());
        while self.recent.len() > cfg.samples {
            self.recent.pop_front();
        }
        let Some(held) = self.held.as_ref() else {
            self.held = Some(current.to_vec());
            self.since = now;
            return current.to_vec();
        };
        // The largest move, as a multiple of what counts as a move, and as a fraction of the full range.
        let measure = |a: &[f64], b: &[f64]| {
            let mut moved = 0.0_f64;
            let mut fraction = 0.0_f64;
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                let range = ranges.get(i).copied().unwrap_or(1.0).max(1e-9);
                let thresh = (thresholds.get(i).copied().unwrap_or(0.25).max(1e-9)) * cfg.level;
                let d = (x - y).abs() / range;
                moved = moved.max(d / thresh);
                fraction = fraction.max(d);
            }
            (moved, fraction)
        };
        let (moved, fraction) = measure(current, held);
        let age = now - self.since;
        if (age >= cfg.min_hold && moved >= 1.0) || (age >= cfg.max_interval && fraction >= var_thresh.max(0.0)) {
            // The new pose is the mean of the recent readings that still agree with the current one, so a quick
            // turn is not dragged back by what the head did before it.
            let near: Vec<&Vec<f64>> = self.recent.iter().filter(|r| measure(r, current).0 < 1.0).collect();
            let n = anim_core::num::usize_to_f64(near.len().max(1));
            let mean: Vec<f64> = (0..current.len()).map(|i| near.iter().filter_map(|r| r.get(i)).sum::<f64>() / n).collect();
            self.held = Some(mean);
            self.since = now;
        }
        self.held.clone().unwrap_or_else(|| current.to_vec())
    }
}

impl Face {
    /// The mouth shape a mouth reading asks for, if any.
    #[must_use]
    pub fn mouth_shape(&self, open: f64, wide: f64, smile: f64) -> Option<Viseme> {
        let strength = percent(self.params.mouth_factor);
        if strength <= 0.0 {
            return None;
        }
        let (open, smile) = ((open * strength).clamp(0.0, 1.0), (smile * strength).clamp(0.0, 1.0));
        if smile > 0.5 && open < 0.3 {
            Some(Viseme::Smile)
        } else if open > 0.8 && wide < 0.5 {
            Some(Viseme::Surprised)
        } else if open > 0.15 {
            Some(if wide < 0.3 {
                Viseme::Oh
            } else if wide > 0.7 {
                Viseme::Ee
            } else {
                Viseme::Ah
            })
        } else {
            None
        }
    }
}

impl Behavior for Face {
    behavior_common!("Adobe.FaceTracker2.js", "Face", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = &self.params;
        let tau = percent(p.smoothing_rate).max(0.0);
        let face = cx.inputs.face.unwrap_or_default().sanitized();
        let neutral = FaceInput::default();
        let face = if cx.inputs.face.is_some() { face } else { neutral };
        let dt = cx.dt;

        let yaw = self.yaw.update(face.head_yaw, dt, tau);
        let pitch = self.pitch.update(face.head_pitch, dt, tau);
        let roll = self.roll.update(face.head_roll, dt, tau);
        let tr = self.translation.update(face.head_translation, dt, tau);
        let scale = self.scale.update(face.head_scale, dt, tau);
        let cfg = HoldConfig::of(p);
        let (yaw, pitch, roll, tr, scale) = if cfg.level > 0.0 {
            let units = [
                p.head_rot_mean_thresh,
                p.head_rot_mean_thresh,
                p.head_rot_mean_thresh,
                p.head_pos_mean_thresh,
                p.head_pos_mean_thresh,
                p.head_pos_mean_thresh,
            ];
            let held = self.head_hold.update(
                cx.time,
                &[yaw, pitch, roll, tr.x, tr.y, scale],
                &HEAD_RANGES,
                &units,
                p.head_pos_var_thresh.min(p.head_rot_var_thresh),
                &cfg,
            );
            match held.as_slice() {
                &[yaw, pitch, roll, x, y, scale] => (yaw, pitch, roll, Vec2::new(x, y), scale),
                _ => (yaw, pitch, roll, tr, scale),
            }
        } else {
            (yaw, pitch, roll, tr, scale)
        };

        let stage = cx.rig.stage;
        let parallax = percent(p.parallax_factor) * PARALLAX_TRAVEL;
        let head = Xform {
            translation: Vec2::new(
                tr.x * stage.width * 0.5 * HEAD_TRAVEL * percent(p.head_pos_factor) + yaw / 45.0 * stage.width * parallax,
                tr.y * stage.height * 0.5 * HEAD_TRAVEL * percent(p.head_pos_vertical_factor) - pitch / 45.0 * stage.width * parallax,
            ),
            rotation: (roll * percent(p.head_rot_factor)).to_radians(),
            scale: Vec2::new(1.0, 1.0) * (1.0 + (scale - 1.0) * percent(p.head_scale_factor)),
        };
        pose.head = pose.head.then(&head);

        let (bl, br) = (self.brow_left.update(face.eyebrow_left, dt, tau), self.brow_right.update(face.eyebrow_right, dt, tau));
        let (bl, br) = if cfg.level > 0.0 {
            let posed =
                self.brow_hold.update(cx.time, &[bl, br], &[1.0, 1.0], &[p.eyebrow_pos_mean_thresh, p.eyebrow_pos_mean_thresh], p.eyebrow_pos_var_thresh, &cfg);
            match posed.as_slice() {
                &[l, r] => (l, r),
                _ => (bl, br),
            }
        } else {
            (bl, br)
        };
        let k = percent(p.eyebrow_factor);
        let (mut bl, mut br) = (bl * k, br * k);
        if p.move_eyebrows_together {
            let mean = f64::midpoint(bl, br);
            (bl, br) = (mean, mean);
        }
        pose.eyebrow_left = (pose.eyebrow_left + bl).clamp(-1.0, 1.0);
        pose.eyebrow_right = (pose.eyebrow_right + br).clamp(-1.0, 1.0);
        let tilt = |raise: f64| if raise >= 0.0 { p.up_eyebrows_tilt * raise } else { p.down_eyebrows_tilt * -raise };
        if bl.abs() > 1e-6 {
            add_group(pose, ids::LEFT_EYEBROW, &Xform::rotated_degrees(tilt(bl)));
        }
        if br.abs() > 1e-6 {
            add_group(pose, ids::RIGHT_EYEBROW, &Xform::rotated_degrees(-tilt(br)));
        }

        let closed = |open: f64| {
            let c = 1.0 - open;
            if p.blink_only { if c >= 0.5 { 1.0 } else { 0.0 } } else { c }
        };
        let ek = percent(p.eye_factor);
        let (el, er) = (self.eye_left.update(face.eye_open_left, dt, tau), self.eye_right.update(face.eye_open_right, dt, tau));
        pose.blink_left = pose.blink_left.max(closed(el) * ek);
        pose.blink_right = pose.blink_right.max(closed(er) * ek);

        let gaze = self.gaze.update(face.gaze, dt, tau);
        if cx.inputs.face.is_some() || pose.gaze != Vec2::ZERO {
            pose.gaze = Vec2::new((pose.gaze.x + gaze.x * ek).clamp(-1.0, 1.0), (pose.gaze.y + gaze.y * ek).clamp(-1.0, 1.0));
        }

        let open = self.mouth_open.update(face.mouth_open, dt, tau);
        let wide = self.mouth_wide.update(face.mouth_wide, dt, tau);
        let smile = self.smile.update(face.smile, dt, tau);
        if cx.inputs.face.is_some()
            && let Some(v) = self.mouth_shape(open, wide, smile)
        {
            pose.viseme = Some(v);
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
