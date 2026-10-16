//! Look at Camera (this program's own `Charcraft.LookAtCamera.js`): the eyes turn to the viewer and the head comes
//! back to face front, whatever the face or the pointer were making them do.

use charcraft_puppet::{PuppetPose, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Smoother, percent};

/// Parameters of Look at Camera.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookAtCameraParams {
    /// A key that must be held; empty looks at the camera all the time.
    #[serde(rename = "triggerKey")]
    pub trigger_key: String,
    /// How completely the eyes turn to the camera, percent.
    #[serde(rename = "eyeStrength")]
    pub eye_strength: f64,
    /// How completely the head faces front, percent.
    #[serde(rename = "headStrength")]
    pub head_strength: f64,
    /// Seconds to turn.
    pub ease: f64,
}

impl Default for LookAtCameraParams {
    fn default() -> Self {
        Self { trigger_key: "c".to_owned(), eye_strength: 100.0, head_strength: 50.0, ease: 0.25 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "triggerKey", "", "Key that must be held; empty looks at the camera all the time.";
    "eyeStrength", "%", "How completely the eyes turn to the camera.";
    "headStrength", "%", "How completely the head faces front.";
    "ease", "s", "Seconds to turn.";
};

/// Turns the eyes to the viewer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LookAtCamera {
    /// The parameters.
    pub params: LookAtCameraParams,
    amount: Smoother,
}

impl Behavior for LookAtCamera {
    behavior_common!("Charcraft.LookAtCamera.js", "Look at Camera", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let on = p.trigger_key.is_empty() || cx.inputs.key_down(&p.trigger_key);
        let amount = self.amount.update(if on { 1.0 } else { 0.0 }, cx.dt, p.ease.max(0.0));
        if amount < 1e-4 {
            return;
        }
        let eyes = (percent(p.eye_strength) * amount).clamp(0.0, 1.0);
        let head = (percent(p.head_strength) * amount).clamp(0.0, 1.0);
        pose.gaze *= 1.0 - eyes;
        pose.pupil_left = pose.pupil_left.map(|v| v * (1.0 - eyes));
        pose.pupil_right = pose.pupil_right.map(|v| v * (1.0 - eyes));
        pose.head = Xform { rotation: pose.head.rotation * (1.0 - head), translation: pose.head.translation * (1.0 - head), ..pose.head };
    }

    fn reset(&mut self, _seed: u64) {
        self.amount.reset();
    }
}

#[cfg(test)]
mod tests {
    use anim_core::{Tick, Vec2};

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn look(l: &mut LookAtCamera, key: bool, frames: u32) -> PuppetPose {
        let rig = Rig::default();
        let mut pose = PuppetPose::neutral();
        for i in 0..frames {
            let t = f64::from(i) / 60.0;
            let mut inputs = Inputs::at(Tick::from_seconds_f64(t));
            if key {
                inputs = inputs.with_key("c");
            }
            pose = PuppetPose::neutral();
            pose.gaze = Vec2::new(0.8, -0.4);
            pose.pupil_left = Some(Vec2::new(0.5, 0.0));
            pose.head = Xform::rotated_degrees(20.0);
            l.step(&Cx { inputs: &inputs, rig: &rig, time: t, dt: 1.0 / 60.0 }, &mut pose);
        }
        pose
    }

    #[test]
    fn while_the_key_is_held_the_eyes_centre_and_the_head_comes_back() {
        let mut l = LookAtCamera::default();
        let off = look(&mut l, false, 30);
        assert_eq!(off.gaze, Vec2::new(0.8, -0.4));
        let on = look(&mut l, true, 120);
        assert!(on.gaze.hypot() < 0.01, "{:?}", on.gaze);
        assert!(on.pupil_left.is_some_and(|v| v.hypot() < 0.01));
        assert!((on.head.rotation.to_degrees() - 10.0).abs() < 0.2, "head half way front: {}", on.head.rotation.to_degrees());
    }

    #[test]
    fn it_eases() {
        let mut l = LookAtCamera::default();
        look(&mut l, false, 3);
        let early = look(&mut l, true, 5);
        assert!(early.gaze.hypot() > 0.1 && early.gaze.hypot() < 0.89);
    }
}
