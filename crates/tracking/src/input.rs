//! [`FaceInput`]: what the tracker hands to the puppet, one value set per camera frame.

use serde::{Deserialize, Serialize};

/// Head pose relative to the calibrated neutral pose.
///
/// Angles are in radians with the camera-image conventions of [`crate::measure::Orientation`]: `yaw` positive turns
/// toward the image's right, `pitch` positive looks up, `roll` positive tilts clockwise in the image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadPose {
    /// Turn left/right.
    pub yaw: f32,
    /// Nod up/down.
    pub pitch: f32,
    /// Tilt.
    pub roll: f32,
    /// Horizontal position of the head in the frame, relative to neutral: -1 to 1, positive to the image's right.
    pub x: f32,
    /// Vertical position, relative to neutral: -1 to 1, positive up.
    pub y: f32,
    /// Distance proxy: 1 at the calibrated distance, above 1 when closer to the camera.
    pub scale: f32,
}

impl Default for HeadPose {
    fn default() -> Self {
        Self { yaw: 0.0, pitch: 0.0, roll: 0.0, x: 0.0, y: 0.0, scale: 1.0 }
    }
}

/// The tracked face for one frame, every value already mapped to the range a puppet behaviour wants.
///
/// "Left" and "right" are as seen in the camera image. A webcam image is usually shown mirrored; call
/// [`FaceInput::mirrored`] to swap sides so the puppet moves like a mirror.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceInput {
    /// Whether a face was found in this frame. When `false` the other fields hold the last values seen.
    pub tracked: bool,
    /// The landmark model's confidence, 0 to 1.
    pub confidence: f32,
    /// Head pose and position.
    pub head: HeadPose,
    /// Image-left eyebrow: -1 (lowered) to 1 (raised), 0 at the neutral face.
    pub brow_l: f32,
    /// Image-right eyebrow, as `brow_l`.
    pub brow_r: f32,
    /// Image-left eye: 0 (shut) to 1 (open).
    pub eye_open_l: f32,
    /// Image-right eye: 0 (shut) to 1 (open).
    pub eye_open_r: f32,
    /// Pupil position: x toward the image's right and y up, each -1 to 1.
    pub gaze: [f32; 2],
    /// Mouth opening: 0 (closed) to 1 (wide open).
    pub mouth_open: f32,
    /// Mouth width: -1 (pursed) to 1 (stretched), 0 at neutral.
    pub mouth_wide: f32,
    /// Mouth corners: -1 (down) to 1 (smile), 0 at neutral.
    pub smile: f32,
}

impl Default for FaceInput {
    fn default() -> Self {
        Self {
            tracked: false,
            confidence: 0.0,
            head: HeadPose::default(),
            brow_l: 0.0,
            brow_r: 0.0,
            eye_open_l: 1.0,
            eye_open_r: 1.0,
            gaze: [0.0, 0.0],
            mouth_open: 0.0,
            mouth_wide: 0.0,
            smile: 0.0,
        }
    }
}

impl FaceInput {
    /// How shut both eyes are: 0 (open) to 1 (blinking).
    #[must_use]
    pub fn blink(&self) -> f32 {
        1.0 - f32::midpoint(self.eye_open_l, self.eye_open_r)
    }

    /// The face as seen in a mirror: sides swap and the horizontal movements flip.
    #[must_use]
    pub fn mirrored(&self) -> Self {
        Self {
            head: HeadPose { yaw: -self.head.yaw, roll: -self.head.roll, x: -self.head.x, ..self.head },
            brow_l: self.brow_r,
            brow_r: self.brow_l,
            eye_open_l: self.eye_open_r,
            eye_open_r: self.eye_open_l,
            gaze: [-self.gaze[0], self.gaze[1]],
            ..*self
        }
    }

    /// Whether every value is finite and in its documented range (what the solver guarantees).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let unit = |v: f32, lo: f32| v.is_finite() && (lo..=1.0).contains(&v);
        let pose = [self.head.yaw, self.head.pitch, self.head.roll, self.head.x, self.head.y, self.head.scale];
        pose.iter().all(|v| v.is_finite())
            && unit(self.confidence, 0.0)
            && unit(self.brow_l, -1.0)
            && unit(self.brow_r, -1.0)
            && unit(self.eye_open_l, 0.0)
            && unit(self.eye_open_r, 0.0)
            && unit(self.gaze[0], -1.0)
            && unit(self.gaze[1], -1.0)
            && unit(self.mouth_open, 0.0)
            && unit(self.mouth_wide, -1.0)
            && unit(self.smile, -1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_a_neutral_untracked_face() {
        let d = FaceInput::default();
        assert!(!d.tracked && d.is_valid());
        assert!(d.blink().abs() < f32::EPSILON);
    }

    #[test]
    fn mirroring_swaps_sides_and_is_an_involution() {
        let f = FaceInput {
            tracked: true,
            head: HeadPose { yaw: 0.3, roll: -0.1, x: 0.2, ..HeadPose::default() },
            brow_l: 0.5,
            eye_open_r: 0.0,
            gaze: [0.4, 0.1],
            ..FaceInput::default()
        };
        let m = f.mirrored();
        assert!((m.head.yaw + 0.3).abs() < 1e-6 && (m.head.roll - 0.1).abs() < 1e-6 && (m.head.x + 0.2).abs() < 1e-6);
        assert!((m.brow_r - 0.5).abs() < 1e-6 && m.brow_l.abs() < 1e-6);
        assert!(m.eye_open_l.abs() < 1e-6 && (m.eye_open_r - 1.0).abs() < 1e-6);
        assert_eq!(m.gaze, [-0.4, 0.1]);
        assert_eq!(m.mirrored(), f);
    }

    #[test]
    fn out_of_range_values_are_invalid() {
        assert!(!FaceInput { smile: 1.5, ..FaceInput::default() }.is_valid());
        assert!(!FaceInput { mouth_open: f32::NAN, ..FaceInput::default() }.is_valid());
    }

    #[test]
    fn serialises_to_json_for_tools() {
        let json = serde_json::to_string(&FaceInput::default()).unwrap();
        let back: FaceInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FaceInput::default());
    }
}
