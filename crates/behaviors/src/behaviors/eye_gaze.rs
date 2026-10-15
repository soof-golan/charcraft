//! Eye Gaze (`Adobe.EyeGaze.js`): points the eyes at the mouse, the arrow keys or the tracked gaze.

use std::f64::consts::TAU;

use anim_core::Vec2;
use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Smoother2, axis, percent};

/// How far from the eyes the cursor has to be for a full look, as a fraction of the stage's shorter side.
pub const FULL_LOOK_DISTANCE: f64 = 0.3;

/// Parameters of Eye Gaze, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EyeGazeParams {
    /// Strength of the tracked (webcam) gaze, percent.
    #[serde(rename = "eyeGazeFactor")]
    pub eye_gaze_factor: f64,
    /// Strength of the arrow keys, percent.
    #[serde(rename = "keyboardEyeGazeFactor")]
    pub keyboard_eye_gaze_factor: f64,
    /// Multiplier of the horizontal range (pupils travel less sideways than up and down).
    #[serde(rename = "leftRightScaleFactor")]
    pub left_right_scale_factor: f64,
    /// With snapping, the shortest time between two changes of look, seconds.
    #[serde(rename = "minGazeDuration")]
    pub min_gaze_duration: f64,
    /// With snapping, looks shorter than this (0 to 1) fall back to the centre.
    #[serde(rename = "minSnapOffsetDist")]
    pub min_snap_offset_dist: f64,
    /// Strength of the mouse, percent.
    #[serde(rename = "mouseEyeGazeFactor")]
    pub mouse_eye_gaze_factor: f64,
    /// With snapping, how many directions the eyes can dart to.
    #[serde(rename = "numEyeDartDirections")]
    pub num_eye_dart_directions: f64,
    /// Multiplier for looks that are diagonal rather than along an axis.
    #[serde(rename = "offAxisScaleFactor")]
    pub off_axis_scale_factor: f64,
    /// The range of the look is a rectangle (each axis limited alone) instead of a circle.
    #[serde(rename = "rectangularEyes")]
    pub rectangular_eyes: bool,
    /// Accepted and stored (the range multipliers always apply).
    #[serde(rename = "scaleTransformsV2")]
    pub scale_transforms_v2: bool,
    /// Smoothing: time constant of the low-pass filter in hundredths of a second.
    #[serde(rename = "smoothingRate")]
    pub smoothing_rate: f64,
    /// Eyes dart to the nearest of a few directions instead of gliding.
    #[serde(rename = "snapEyeGaze")]
    pub snap_eye_gaze: bool,
    /// Multiplier of the vertical range.
    #[serde(rename = "upDownScaleFactor")]
    pub up_down_scale_factor: f64,
}

impl Default for EyeGazeParams {
    fn default() -> Self {
        Self {
            eye_gaze_factor: 100.0,
            keyboard_eye_gaze_factor: 100.0,
            left_right_scale_factor: 0.7,
            min_gaze_duration: 1.0,
            min_snap_offset_dist: 0.25,
            mouse_eye_gaze_factor: 100.0,
            num_eye_dart_directions: 8.0,
            off_axis_scale_factor: 1.5,
            rectangular_eyes: false,
            scale_transforms_v2: true,
            smoothing_rate: 15.0,
            snap_eye_gaze: true,
            up_down_scale_factor: 1.5,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "eyeGazeFactor", "%", "Strength of the gaze the Face behavior tracked.";
    "keyboardEyeGazeFactor", "%", "Strength of the arrow keys.";
    "mouseEyeGazeFactor", "%", "Strength of the mouse.";
    "leftRightScaleFactor", "", "Horizontal range multiplier.";
    "upDownScaleFactor", "", "Vertical range multiplier.";
    "offAxisScaleFactor", "", "Range multiplier for diagonal looks.";
    "rectangularEyes", "", "Limit each axis alone instead of the length of the look.";
    "snapEyeGaze", "", "Dart between a few directions.";
    "numEyeDartDirections", "", "Directions to dart to.";
    "minGazeDuration", "s", "Shortest time between darts.";
    "minSnapOffsetDist", "", "Looks smaller than this stay centred.";
    "smoothingRate", "1/100 s", "Low-pass time constant.";
};

/// Eye gaze. Keyboard beats mouse, which beats the gaze already in the pose from the Face behavior.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EyeGaze {
    /// The parameters.
    pub params: EyeGazeParams,
    held: Option<(f64, Vec2)>,
    smooth: Smoother2,
}

impl EyeGaze {
    /// The look asked for and whether it comes from the keyboard or mouse (which dart) rather than the camera.
    fn raw_look(&self, cx: &Cx<'_>, pose: &PuppetPose) -> (Vec2, bool) {
        let p = &self.params;
        let i = cx.inputs;
        let key = Vec2::new(axis(i.key_down("left"), i.key_down("right")), axis(i.key_down("up"), i.key_down("down")));
        if key != Vec2::ZERO {
            return (key * percent(p.keyboard_eye_gaze_factor), true);
        }
        if let Some(ptr) = i.primary_pointer() {
            let reach = FULL_LOOK_DISTANCE * cx.rig.unit();
            return ((ptr.position - cx.rig.eyes_center) / reach * percent(p.mouse_eye_gaze_factor), true);
        }
        (pose.gaze * percent(p.eye_gaze_factor), false)
    }

    fn snapped(&mut self, look: Vec2, t: f64) -> Vec2 {
        let p = &self.params;
        let mag = look.hypot().min(1.0);
        let target = if mag < p.min_snap_offset_dist.clamp(0.0, 1.0) || !look.x.is_finite() || !look.y.is_finite() {
            Vec2::ZERO
        } else {
            let n = p.num_eye_dart_directions.clamp(1.0, 64.0).round();
            let step = TAU / n;
            let angle = look.y.atan2(look.x);
            let snapped = (angle / step).round() * step;
            Vec2::new(snapped.cos(), snapped.sin()) * mag
        };
        match self.held {
            Some((since, kept)) if t - since < p.min_gaze_duration.max(0.0) => kept,
            _ => {
                self.held = Some((t, target));
                target
            }
        }
    }
}

impl Behavior for EyeGaze {
    behavior_common!("Adobe.EyeGaze.js", "Eye Gaze", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let (mut look, darts) = self.raw_look(cx, pose);
        if !look.x.is_finite() || !look.y.is_finite() {
            look = Vec2::ZERO;
        }
        let look = if p.snap_eye_gaze && darts {
            self.snapped(look, cx.time)
        } else {
            self.held = None;
            look
        };
        let mut v = Vec2::new(look.x * p.left_right_scale_factor, look.y * p.up_down_scale_factor);
        let (ax, ay) = (v.x.abs(), v.y.abs());
        if ax > 1e-9 && ay > 1e-9 {
            v *= p.off_axis_scale_factor.max(0.0).powf(ax.min(ay) / ax.max(ay));
        }
        let v = if p.rectangular_eyes {
            Vec2::new(v.x.clamp(-1.0, 1.0), v.y.clamp(-1.0, 1.0))
        } else if v.hypot() > 1.0 {
            v / v.hypot()
        } else {
            v
        };
        pose.gaze = self.smooth.update(v, cx.dt, percent(p.smoothing_rate).max(0.0));
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
