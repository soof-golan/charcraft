//! What the behaviors see each tick: the face, the voice, pointers, keys and the clock.

use std::collections::{BTreeMap, BTreeSet};

use anim_core::{Point, Tick, Vec2};
use anim_lipsync::Viseme as VoiceViseme;
use serde::{Deserialize, Serialize};

/// One reading of a tracked face, in a camera-independent form.
///
/// Angles are degrees, amounts are normalised, positions are fractions of the camera frame. The neutral face
/// ([`FaceInput::default`]) looks straight ahead with open eyes and a closed mouth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaceInput {
    /// Turn left (negative) or right (positive), degrees.
    pub head_yaw: f64,
    /// Look down (negative) or up (positive), degrees.
    pub head_pitch: f64,
    /// Tilt, degrees, clockwise on screen positive.
    pub head_roll: f64,
    /// Where the head is in the frame, -1 to 1 on each axis (x right, y down), 0 at the centre.
    pub head_translation: Vec2,
    /// Distance to the camera: 1 is the calibrated distance, above 1 is closer.
    pub head_scale: f64,
    /// Left eyebrow, -1 (frowning) to 1 (raised).
    pub eyebrow_left: f64,
    /// Right eyebrow, -1 (frowning) to 1 (raised).
    pub eyebrow_right: f64,
    /// Left eye openness, 0 (shut) to 1 (open).
    pub eye_open_left: f64,
    /// Right eye openness, 0 (shut) to 1 (open).
    pub eye_open_right: f64,
    /// Where the eyes look, -1 to 1 on each axis (x right, y down).
    pub gaze: Vec2,
    /// How far the jaw is open, 0 to 1.
    pub mouth_open: f64,
    /// How wide the mouth is stretched, 0 (pursed) to 1 (wide).
    pub mouth_wide: f64,
    /// How much the corners are lifted, 0 to 1.
    pub smile: f64,
}

impl Default for FaceInput {
    fn default() -> Self {
        Self {
            head_yaw: 0.0,
            head_pitch: 0.0,
            head_roll: 0.0,
            head_translation: Vec2::ZERO,
            head_scale: 1.0,
            eyebrow_left: 0.0,
            eyebrow_right: 0.0,
            eye_open_left: 1.0,
            eye_open_right: 1.0,
            gaze: Vec2::ZERO,
            mouth_open: 0.0,
            mouth_wide: 0.5,
            smile: 0.0,
        }
    }
}

impl FaceInput {
    /// The same face with every non-finite number replaced by its neutral value and every range enforced.
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let n = Self::default();
        let f = |v: f64, d: f64| if v.is_finite() { v } else { d };
        let u = |v: f64, d: f64| f(v, d).clamp(-1.0, 1.0);
        let z = |v: f64, d: f64| f(v, d).clamp(0.0, 1.0);
        Self {
            head_yaw: f(self.head_yaw, n.head_yaw).clamp(-90.0, 90.0),
            head_pitch: f(self.head_pitch, n.head_pitch).clamp(-90.0, 90.0),
            head_roll: f(self.head_roll, n.head_roll).clamp(-180.0, 180.0),
            head_translation: Vec2::new(u(self.head_translation.x, 0.0), u(self.head_translation.y, 0.0)),
            head_scale: f(self.head_scale, n.head_scale).clamp(0.25, 4.0),
            eyebrow_left: u(self.eyebrow_left, 0.0),
            eyebrow_right: u(self.eyebrow_right, 0.0),
            eye_open_left: z(self.eye_open_left, 1.0),
            eye_open_right: z(self.eye_open_right, 1.0),
            gaze: Vec2::new(u(self.gaze.x, 0.0), u(self.gaze.y, 0.0)),
            mouth_open: z(self.mouth_open, 0.0),
            mouth_wide: z(self.mouth_wide, 0.5),
            smile: z(self.smile, 0.0),
        }
    }
}

/// A joint of the tracked body, named for the performer's own sides: the performer's left arm drives the puppet's
/// left arm, so the puppet moves like a mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Joint {
    /// The nose.
    Nose,
    /// Left eye.
    LeftEye,
    /// Right eye.
    RightEye,
    /// Left ear.
    LeftEar,
    /// Right ear.
    RightEar,
    /// Left shoulder.
    LeftShoulder,
    /// Right shoulder.
    RightShoulder,
    /// Left elbow.
    LeftElbow,
    /// Right elbow.
    RightElbow,
    /// Left wrist.
    LeftWrist,
    /// Right wrist.
    RightWrist,
    /// Left hip.
    LeftHip,
    /// Right hip.
    RightHip,
    /// Left knee.
    LeftKnee,
    /// Right knee.
    RightKnee,
    /// Left ankle.
    LeftAnkle,
    /// Right ankle.
    RightAnkle,
}

impl Joint {
    /// Every joint, in the order pose models list them (nose, eyes, ears, shoulders, elbows, wrists, hips, knees,
    /// ankles; left before right).
    pub const ALL: [Self; 17] = [
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
}

/// One reading of a tracked body, in a camera-independent form.
///
/// Positions are in body units: x grows towards the puppet's left (the screen's right), y grows downward, the
/// origin is the middle of the hips in the rest pose and one unit is the rest pose's torso length (hips to
/// shoulders). Joints the camera cannot see are missing. [`BodyInput::standard_rest`] is the typical standing
/// body that the tracker's rest pose starts as.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BodyInput {
    /// Where each visible joint is now.
    pub joints: BTreeMap<Joint, Vec2>,
    /// Where each joint was in the rest pose the performer set: the joints now are measured against it.
    pub rest: BTreeMap<Joint, Vec2>,
}

impl BodyInput {
    /// A typical standing body, arms hanging by the sides.
    #[must_use]
    pub fn standard_rest() -> BTreeMap<Joint, Vec2> {
        let side = |left: f64| {
            [
                (Joint::LeftEye, Vec2::new(0.08 * left, -1.45)),
                (Joint::LeftEar, Vec2::new(0.18 * left, -1.4)),
                (Joint::LeftShoulder, Vec2::new(0.45 * left, -1.0)),
                (Joint::LeftElbow, Vec2::new(0.5 * left, -0.4)),
                (Joint::LeftWrist, Vec2::new(0.5 * left, 0.2)),
                (Joint::LeftHip, Vec2::new(0.25 * left, 0.0)),
                (Joint::LeftKnee, Vec2::new(0.25 * left, 0.9)),
                (Joint::LeftAnkle, Vec2::new(0.25 * left, 1.8)),
            ]
        };
        let mut rest = BTreeMap::from([(Joint::Nose, Vec2::new(0.0, -1.4))]);
        for (joint, p) in side(1.0) {
            rest.insert(joint, p);
        }
        for (joint, p) in side(-1.0) {
            let mirrored = match joint {
                Joint::LeftEye => Joint::RightEye,
                Joint::LeftEar => Joint::RightEar,
                Joint::LeftShoulder => Joint::RightShoulder,
                Joint::LeftElbow => Joint::RightElbow,
                Joint::LeftWrist => Joint::RightWrist,
                Joint::LeftHip => Joint::RightHip,
                Joint::LeftKnee => Joint::RightKnee,
                _ => Joint::RightAnkle,
            };
            rest.insert(mirrored, p);
        }
        rest
    }

    /// A body standing at rest.
    #[must_use]
    pub fn at_rest() -> Self {
        let rest = Self::standard_rest();
        Self { joints: rest.clone(), rest }
    }

    /// The joint now, if it is visible, and only when its position is a finite number.
    #[must_use]
    pub fn joint(&self, joint: Joint) -> Option<Vec2> {
        self.joints.get(&joint).copied().filter(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// The joint in the rest pose: the performer's own, else the typical body's.
    #[must_use]
    pub fn rest_joint(&self, joint: Joint) -> Vec2 {
        self.rest
            .get(&joint)
            .copied()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .or_else(|| Self::standard_rest().get(&joint).copied())
            .unwrap_or(Vec2::ZERO)
    }

    /// The same body with every position kept within a sane distance (hostile numbers cannot fling the puppet).
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let keep = |m: &BTreeMap<Joint, Vec2>| -> BTreeMap<Joint, Vec2> {
            m.iter().filter(|(_, p)| p.x.is_finite() && p.y.is_finite()).map(|(j, p)| (*j, Vec2::new(p.x.clamp(-8.0, 8.0), p.y.clamp(-8.0, 8.0)))).collect()
        };
        Self { joints: keep(&self.joints), rest: keep(&self.rest) }
    }
}

/// A mouse or a finger on the stage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pointer {
    /// Identifies the finger across ticks; the mouse is 0.
    pub id: u32,
    /// Position in stage pixels (the puppet's artwork space, y down).
    pub position: Point,
    /// True while the button is held or the finger touches.
    pub down: bool,
}

/// A draggable handle held at a position.
///
/// This is what Character Animator records for a drag: the handle's target, not the mouse that pulled it, so a
/// recorded drag moves the same handle however the puppet is placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandleTarget {
    /// The handle's key, `<group label>/<handle name>` (`Arm Right/Draggable`).
    pub handle: String,
    /// Where the handle is held, in stage pixels (the puppet's artwork space, y down).
    pub position: Point,
    /// True while it is held; false lets go of it.
    pub held: bool,
}

/// Everything the world tells the behaviors at one tick.
///
/// Inputs are plain data: a behavior stack fed the same inputs, seed and starting state produces the same poses.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Inputs {
    /// When this reading was taken.
    pub time: Tick,
    /// The tracked face, if a camera is on and sees one.
    pub face: Option<FaceInput>,
    /// The tracked body, if a camera is on and sees one.
    pub body: Option<BodyInput>,
    /// The mouth shape the voice currently makes (from `anim-lipsync`), if a microphone or track is on.
    pub viseme: Option<VoiceViseme>,
    /// The mouth shape of the scene's audio (a sound clip on an audio track) at this time, if there is any.
    pub scene_viseme: Option<VoiceViseme>,
    /// Mice and fingers.
    pub pointers: Vec<Pointer>,
    /// Draggable handles held directly (recorded drags).
    pub handle_targets: Vec<HandleTarget>,
    /// Keys held down, lower case (`a`, `space`, `left`, `up`, `1`).
    pub keys: BTreeSet<String>,
}

impl Inputs {
    /// Inputs at a time with nothing else going on.
    #[must_use]
    pub fn at(time: Tick) -> Self {
        Self { time, ..Self::default() }
    }

    /// Adds a face.
    #[must_use]
    pub const fn with_face(mut self, face: FaceInput) -> Self {
        self.face = Some(face);
        self
    }

    /// Adds a body.
    #[must_use]
    pub fn with_body(mut self, body: BodyInput) -> Self {
        self.body = Some(body);
        self
    }

    /// Adds a voice mouth shape.
    #[must_use]
    pub const fn with_viseme(mut self, viseme: VoiceViseme) -> Self {
        self.viseme = Some(viseme);
        self
    }

    /// Adds the mouth shape of the scene's audio.
    #[must_use]
    pub const fn with_scene_viseme(mut self, viseme: VoiceViseme) -> Self {
        self.scene_viseme = Some(viseme);
        self
    }

    /// Adds a pointer.
    #[must_use]
    pub fn with_pointer(mut self, id: u32, x: f64, y: f64, down: bool) -> Self {
        self.pointers.push(Pointer { id, position: Point::new(x, y), down });
        self
    }

    /// Holds a key (names are lower-cased).
    #[must_use]
    pub fn with_key(mut self, key: &str) -> Self {
        self.keys.insert(key.to_lowercase());
        self
    }

    /// Whether a key is held (case-insensitive).
    #[must_use]
    pub fn key_down(&self, key: &str) -> bool {
        self.keys.contains(&key.to_lowercase())
    }

    /// The first finite pointer, the one eyes follow.
    #[must_use]
    pub fn primary_pointer(&self) -> Option<&Pointer> {
        self.pointers.iter().find(|p| p.position.x.is_finite() && p.position.y.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_repairs_hostile_faces() {
        let bad = FaceInput { head_yaw: f64::NAN, eye_open_left: 7.0, head_scale: f64::INFINITY, gaze: Vec2::new(9.0, f64::NAN), ..FaceInput::default() };
        let ok = bad.sanitized();
        assert_eq!(ok.head_yaw, 0.0);
        assert_eq!(ok.eye_open_left, 1.0);
        assert_eq!(ok.head_scale, 1.0);
        assert_eq!(ok.gaze, Vec2::new(1.0, 0.0));
    }

    #[test]
    fn keys_are_case_insensitive() {
        let i = Inputs::at(Tick::ZERO).with_key("Left");
        assert!(i.key_down("LEFT"));
        assert!(!i.key_down("right"));
    }

    #[test]
    fn inputs_survive_json() {
        let i = Inputs::at(Tick::from_seconds_f64(0.5)).with_face(FaceInput::default()).with_viseme(VoiceViseme::Ah).with_pointer(0, 1.0, 2.0, true);
        let json = serde_json::to_string(&i).unwrap();
        assert_eq!(serde_json::from_str::<Inputs>(&json).unwrap(), i);
    }
}
