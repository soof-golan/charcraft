//! Parameter groups: the slices of the input streams that are armed, recorded and overridden separately.
//!
//! Character Animator records per behavior (and per parameter group inside it): arming Face records the face,
//! arming Lip Sync records the voice, and a later take replaces only what it recorded. Here the groups are the
//! parts of [`Inputs`] a behavior reads, finer than a behavior for the face so a take of just the eyebrows can
//! lie over a take of the head.

use std::collections::BTreeSet;

use anim_core::Vec2;
use charcraft_behaviors::{FaceInput, HandleTarget, Inputs, Pointer};
use serde::{Deserialize, Serialize};

/// A separately armed and overridden part of the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamGroup {
    /// Head turn, nod, tilt, position and distance (Face behavior).
    Head,
    /// Both eyebrows (Face behavior).
    Brows,
    /// Eyelids and gaze (Face and Eye Gaze behaviors).
    Eyes,
    /// Mouth opening, width and smile read from the face (Face behavior).
    Mouth,
    /// The mouth shape the voice makes (Lip Sync behavior).
    Voice,
    /// Mice and fingers (Dragger behavior).
    Pointer,
    /// Held keys (Triggers, Walk and Eye Gaze behaviors).
    Keys,
}

impl ParamGroup {
    /// Every group.
    pub const ALL: [Self; 7] = [Self::Head, Self::Brows, Self::Eyes, Self::Mouth, Self::Voice, Self::Pointer, Self::Keys];

    /// The groups that come from the tracked face.
    pub const FACE: [Self; 4] = [Self::Head, Self::Brows, Self::Eyes, Self::Mouth];

    /// The name in tools and take names.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Brows => "brows",
            Self::Eyes => "eyes",
            Self::Mouth => "mouth",
            Self::Voice => "voice",
            Self::Pointer => "pointer",
            Self::Keys => "keys",
        }
    }

    /// The Character Animator behavior that reads this group.
    #[must_use]
    pub const fn behavior(self) -> &'static str {
        match self {
            Self::Head | Self::Brows | Self::Eyes | Self::Mouth => "Adobe.FaceTracker2.js",
            Self::Voice => "Adobe.LipSync2.js",
            Self::Pointer => "Adobe.Dragger3.js",
            Self::Keys => "Adobe.Triggers.js",
        }
    }

    /// Whether the group is a part of the face.
    #[must_use]
    pub const fn is_face(self) -> bool {
        matches!(self, Self::Head | Self::Brows | Self::Eyes | Self::Mouth)
    }

    /// Moves this group's fields of `dst` towards `src`: `weight` 1 copies them, 0 leaves them.
    pub fn blend_face(self, dst: &mut FaceInput, src: &FaceInput, weight: f64) {
        let w = weight.clamp(0.0, 1.0);
        match self {
            Self::Head => {
                dst.head_yaw = mix(dst.head_yaw, src.head_yaw, w);
                dst.head_pitch = mix(dst.head_pitch, src.head_pitch, w);
                dst.head_roll = mix(dst.head_roll, src.head_roll, w);
                dst.head_translation = mix2(dst.head_translation, src.head_translation, w);
                dst.head_scale = mix(dst.head_scale, src.head_scale, w);
            }
            Self::Brows => {
                dst.eyebrow_left = mix(dst.eyebrow_left, src.eyebrow_left, w);
                dst.eyebrow_right = mix(dst.eyebrow_right, src.eyebrow_right, w);
            }
            Self::Eyes => {
                dst.eye_open_left = mix(dst.eye_open_left, src.eye_open_left, w);
                dst.eye_open_right = mix(dst.eye_open_right, src.eye_open_right, w);
                dst.gaze = mix2(dst.gaze, src.gaze, w);
            }
            Self::Mouth => {
                dst.mouth_open = mix(dst.mouth_open, src.mouth_open, w);
                dst.mouth_wide = mix(dst.mouth_wide, src.mouth_wide, w);
                dst.smile = mix(dst.smile, src.smile, w);
            }
            Self::Voice | Self::Pointer | Self::Keys => {}
        }
    }
}

/// `a` to `b` by `w`; exactly `b` at 1 so a full-weight take reproduces what was recorded bit for bit.
fn mix(a: f64, b: f64, w: f64) -> f64 {
    if w >= 1.0 { b } else { a + (b - a) * w }
}

fn mix2(a: Vec2, b: Vec2, w: f64) -> Vec2 {
    Vec2::new(mix(a.x, b.x, w), mix(a.y, b.y, w))
}

/// What a take stores at one sample: the inputs minus the clock.
///
/// Only the fields of the groups the take recorded mean anything; the others hold their neutral value.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    /// The face (`None`: no face was tracked).
    pub face: Option<FaceInput>,
    /// The voice's mouth shape.
    pub viseme: Option<anim_lipsync::Viseme>,
    /// Pointers.
    pub pointers: Vec<Pointer>,
    /// Draggable handles held at a position (what Character Animator records for a drag).
    pub handles: Vec<HandleTarget>,
    /// Held keys.
    pub keys: BTreeSet<String>,
}

/// Most pointers and handles one sample keeps (hostile files could list millions).
pub const MAX_POINTERS: usize = 16;
/// Most keys one sample keeps.
pub const MAX_KEYS: usize = 64;
/// Longest key name kept.
pub const MAX_KEY_LEN: usize = 32;

impl Frame {
    /// Takes the armed groups out of one tick of inputs.
    #[must_use]
    pub fn capture(inputs: &Inputs, groups: &BTreeSet<ParamGroup>) -> Self {
        let mut frame = Self::default();
        if groups.iter().any(|g| g.is_face())
            && let Some(face) = inputs.face
        {
            let face = face.sanitized();
            let mut kept = FaceInput::default();
            for group in ParamGroup::FACE.iter().filter(|g| groups.contains(g)) {
                group.blend_face(&mut kept, &face, 1.0);
            }
            frame.face = Some(kept);
        }
        if groups.contains(&ParamGroup::Voice) {
            frame.viseme = inputs.viseme;
        }
        if groups.contains(&ParamGroup::Pointer) {
            frame.pointers = inputs.pointers.iter().filter(|p| p.position.x.is_finite() && p.position.y.is_finite()).take(MAX_POINTERS).copied().collect();
            frame.handles = inputs.handle_targets.iter().filter(|h| h.position.x.is_finite() && h.position.y.is_finite()).take(MAX_POINTERS).cloned().collect();
        }
        if groups.contains(&ParamGroup::Keys) {
            frame.keys = inputs.keys.iter().take(MAX_KEYS).map(|k| k.chars().take(MAX_KEY_LEN).collect()).collect();
        }
        frame
    }

    /// The frame `weight` of the way from `self` to `next`. The face and pointer positions interpolate; the voice
    /// and the key sets are discrete and hold `self`.
    #[must_use]
    pub fn lerp(&self, next: &Self, weight: f64) -> Self {
        let mut out = self.clone();
        let w = weight.clamp(0.0, 1.0);
        if let (Some(a), Some(b)) = (out.face.as_mut(), next.face.as_ref()) {
            for group in ParamGroup::FACE {
                group.blend_face(a, b, w);
            }
        }
        for pointer in &mut out.pointers {
            if let Some(other) = next.pointers.iter().find(|p| p.id == pointer.id) {
                pointer.position.x = mix(pointer.position.x, other.position.x, w);
                pointer.position.y = mix(pointer.position.y, other.position.y, w);
            }
        }
        for handle in &mut out.handles {
            if let Some(other) = next.handles.iter().find(|h| h.handle == handle.handle && h.held) {
                handle.position.x = mix(handle.position.x, other.position.x, w);
                handle.position.y = mix(handle.position.y, other.position.y, w);
            }
        }
        out
    }

    /// Repairs hostile values from a file.
    pub fn sanitize(&mut self) {
        self.face = self.face.map(|f| f.sanitized());
        self.pointers.retain(|p| p.position.x.is_finite() && p.position.y.is_finite());
        self.pointers.truncate(MAX_POINTERS);
        self.handles.retain(|h| h.position.x.is_finite() && h.position.y.is_finite());
        self.handles.truncate(MAX_POINTERS);
        self.keys = std::mem::take(&mut self.keys).into_iter().take(MAX_KEYS).map(|k| k.chars().take(MAX_KEY_LEN).collect()).collect();
    }
}

/// Lays one group of a take's frame over the inputs below it, at `weight` (0 leaves them, 1 replaces them).
///
/// The face blends field by field. A take that saw no face leaves the face below alone. Voice, pointers and keys
/// are discrete: the take wins from half weight, so a blend-in switches over in the middle of the ramp.
pub fn apply_group(inputs: &mut Inputs, group: ParamGroup, frame: &Frame, weight: f64) {
    if weight <= 0.0 {
        return;
    }
    match group {
        ParamGroup::Head | ParamGroup::Brows | ParamGroup::Eyes | ParamGroup::Mouth => {
            if let Some(src) = frame.face.as_ref() {
                let dst = inputs.face.get_or_insert_with(FaceInput::default);
                group.blend_face(dst, src, weight);
            }
        }
        ParamGroup::Voice if weight >= 0.5 => inputs.viseme = frame.viseme,
        ParamGroup::Pointer if weight >= 0.5 => {
            inputs.pointers.clone_from(&frame.pointers);
            inputs.handle_targets.clone_from(&frame.handles);
        }
        ParamGroup::Keys if weight >= 0.5 => inputs.keys.clone_from(&frame.keys),
        ParamGroup::Voice | ParamGroup::Pointer | ParamGroup::Keys => {}
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;

    fn groups(list: &[ParamGroup]) -> BTreeSet<ParamGroup> {
        list.iter().copied().collect()
    }

    #[test]
    fn capture_keeps_only_armed_groups() {
        let face = FaceInput { head_yaw: 12.0, eyebrow_left: 0.7, mouth_open: 0.4, ..FaceInput::default() };
        let inputs = Inputs::at(Tick::ZERO).with_face(face).with_viseme(anim_lipsync::Viseme::Oh).with_key("a");
        let frame = Frame::capture(&inputs, &groups(&[ParamGroup::Head]));
        let kept = frame.face.unwrap();
        assert!((kept.head_yaw - 12.0).abs() < 1e-12);
        assert!(kept.eyebrow_left.abs() < 1e-12 && kept.mouth_open.abs() < 1e-12, "other groups are neutral");
        assert_eq!(frame.viseme, None);
        assert!(frame.keys.is_empty());
        let all = Frame::capture(&inputs, &groups(&ParamGroup::ALL));
        assert_eq!(all.viseme, Some(anim_lipsync::Viseme::Oh));
        assert!(all.keys.contains("a"));
    }

    #[test]
    fn full_weight_reproduces_the_recorded_value_exactly() {
        let mut base = Inputs::at(Tick::ZERO).with_face(FaceInput { head_roll: 3.3, ..FaceInput::default() });
        let frame = Frame { face: Some(FaceInput { head_roll: 0.1 + 0.2, ..FaceInput::default() }), ..Frame::default() };
        apply_group(&mut base, ParamGroup::Head, &frame, 1.0);
        assert_eq!(base.face.unwrap().head_roll.to_bits(), (0.1_f64 + 0.2).to_bits());
    }

    #[test]
    fn half_weight_blends_the_face_and_switches_discrete_groups() {
        let mut base = Inputs::at(Tick::ZERO).with_face(FaceInput::default()).with_key("x");
        let frame =
            Frame { face: Some(FaceInput { head_roll: 10.0, ..FaceInput::default() }), keys: ["y".to_owned()].into_iter().collect(), ..Frame::default() };
        apply_group(&mut base, ParamGroup::Head, &frame, 0.5);
        apply_group(&mut base, ParamGroup::Keys, &frame, 0.25);
        assert!((base.face.unwrap().head_roll - 5.0).abs() < 1e-12);
        assert!(base.key_down("x"), "below half weight the keys below stay");
        apply_group(&mut base, ParamGroup::Keys, &frame, 0.5);
        assert!(base.key_down("y") && !base.key_down("x"));
    }

    #[test]
    fn hostile_frames_are_repaired() {
        let mut frame = Frame {
            face: Some(FaceInput { head_yaw: f64::NAN, eyebrow_left: 9.0, ..FaceInput::default() }),
            pointers: (0..100).map(|id| Pointer { id, position: anim_core::Point::new(f64::INFINITY, 0.0), down: true }).collect(),
            keys: (0..500).map(|i| format!("{i}-{}", "k".repeat(200))).collect(),
            ..Frame::default()
        };
        frame.sanitize();
        let face = frame.face.unwrap();
        assert!(face.head_yaw.is_finite() && face.eyebrow_left <= 1.0);
        assert!(frame.pointers.is_empty());
        assert!(frame.keys.len() <= MAX_KEYS && frame.keys.iter().all(|k| k.chars().count() <= MAX_KEY_LEN));
    }
}
