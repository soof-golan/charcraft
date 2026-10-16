//! The behaviors and the registry that creates them by Character Animator id.

use charcraft_ltab::{Key, LValue};
use serde_json::Value;

use crate::behavior::Behavior;
use crate::error::{Error, Result};

pub mod arm_ik;
pub mod blinker;
pub mod body_tracker;
pub mod breathe;
pub mod cycle_layers;
pub mod dragger;
pub mod dynamics;
pub mod eye_gaze;
pub mod face;
pub mod fader;
pub mod handle_fixer;
pub mod head_turner;
pub mod layer_picker;
pub mod leader_follower;
pub mod lipsync;
pub mod look_at_camera;
pub mod magnets;
pub mod motion_lines;
pub mod motion_trigger;
pub mod particles;
pub mod replay;
pub mod shadow;
pub mod transform;
pub mod triggers;
pub mod walk;
pub mod wiggler;

pub use arm_ik::{ArmIk, ArmIkParams};
pub use blinker::{Blinker, BlinkerParams};
pub use body_tracker::{BodyTracker, BodyTrackerParams};
pub use breathe::{Breathe, BreatheParams};
pub use cycle_layers::{CycleLayers, CycleLayersParams};
pub use dragger::{Dragger, DraggerParams};
pub use dynamics::{Dynamics, DynamicsParams};
pub use eye_gaze::{EyeGaze, EyeGazeParams};
pub use face::{Face, FaceParams};
pub use fader::{Fader, FaderParams};
pub use handle_fixer::{HandleFixer, HandleFixerParams};
pub use head_turner::{HeadTurner, HeadTurnerParams};
pub use layer_picker::{LayerPicker, LayerPickerParams};
pub use leader_follower::{LeaderFollower, LeaderFollowerParams};
pub use lipsync::{LipSync, LipSyncParams};
pub use look_at_camera::{LookAtCamera, LookAtCameraParams};
pub use magnets::{Magnets, MagnetsParams};
pub use motion_lines::{MotionLines, MotionLinesParams};
pub use motion_trigger::{MotionTrigger, MotionTriggerParams};
pub use particles::{Particles, ParticlesParams};
pub use replay::{Replay, ReplayParams, ReplayState};
pub use shadow::{Shadow, ShadowParams};
pub use transform::{Transform, TransformParams};
pub use triggers::{SwapChoice, TriggerMode, TriggerRule, Triggers, TriggersParams};
pub use walk::{Walk, WalkParams};
pub use wiggler::{Wiggler, WigglerParams};

/// Ids of the behaviors this crate implements, in the order a new puppet gets them.
///
/// Face and Lip Sync come first because they set the head and mouth others build on; Dragger and Physics come last
/// because they move handles of groups the earlier behaviors have already posed.
pub const IDS: &[&str] = &[
    "Adobe.FaceTracker2.js",
    "Adobe.HeadTurner.js",
    "Adobe.LipSync2.js",
    "Adobe.EyeGaze.js",
    "Charcraft.LookAtCamera.js",
    "Adobe.Blinker.js",
    "Adobe.Transform.js",
    "Charcraft.Walk.js",
    "Charcraft.Breathe.js",
    "Adobe.MotionTrigger.js",
    "Adobe.Triggers.js",
    "Adobe.CycleLayers.js",
    "Charcraft.LayerPicker.js",
    "Charcraft.BodyTracker.js",
    "Charcraft.Wiggler.js",
    "Charcraft.LeaderFollower.js",
    "Adobe.ArmIK.js",
    "Adobe.HandleFixer.js",
    "Adobe.Dragger3.js",
    "Charcraft.Magnets.js",
    "Adobe.RigDynamics.js",
    "Charcraft.Replay.js",
    "Charcraft.Fader.js",
    "Charcraft.Particles.js",
    "Charcraft.Shadow.js",
    "Charcraft.MotionLines.js",
];

/// A behavior with its default parameters.
///
/// # Errors
///
/// [`Error::UnknownBehavior`] when the id is not in [`IDS`].
pub fn create(id: &str) -> Result<Box<dyn Behavior>> {
    Ok(match id {
        "Adobe.FaceTracker2.js" => Box::<Face>::default(),
        "Adobe.HeadTurner.js" => Box::<HeadTurner>::default(),
        "Adobe.MotionTrigger.js" => Box::<MotionTrigger>::default(),
        "Adobe.CycleLayers.js" => Box::<CycleLayers>::default(),
        "Adobe.LipSync2.js" => Box::<LipSync>::default(),
        "Adobe.EyeGaze.js" => Box::<EyeGaze>::default(),
        "Adobe.Blinker.js" => Box::<Blinker>::default(),
        "Adobe.Transform.js" => Box::<Transform>::default(),
        "Charcraft.Walk.js" => Box::<Walk>::default(),
        "Charcraft.Breathe.js" => Box::<Breathe>::default(),
        "Adobe.Triggers.js" => Box::<Triggers>::default(),
        "Adobe.ArmIK.js" => Box::<ArmIk>::default(),
        "Adobe.HandleFixer.js" => Box::<HandleFixer>::default(),
        "Adobe.Dragger3.js" => Box::<Dragger>::default(),
        "Adobe.RigDynamics.js" => Box::<Dynamics>::default(),
        "Charcraft.BodyTracker.js" => Box::<BodyTracker>::default(),
        "Charcraft.Particles.js" => Box::<Particles>::default(),
        "Charcraft.Replay.js" => Box::<Replay>::default(),
        "Charcraft.Magnets.js" => Box::<Magnets>::default(),
        "Charcraft.Wiggler.js" => Box::<Wiggler>::default(),
        "Charcraft.Fader.js" => Box::<Fader>::default(),
        "Charcraft.LookAtCamera.js" => Box::<LookAtCamera>::default(),
        "Charcraft.MotionLines.js" => Box::<MotionLines>::default(),
        "Charcraft.Shadow.js" => Box::<Shadow>::default(),
        "Charcraft.LeaderFollower.js" => Box::<LeaderFollower>::default(),
        "Charcraft.LayerPicker.js" => Box::<LayerPicker>::default(),
        other => return Err(Error::UnknownBehavior(other.to_owned())),
    })
}

/// A behavior with defaults overlaid by `params` (a JSON object).
///
/// # Errors
///
/// [`Error::UnknownBehavior`] or [`Error::BadParams`].
pub fn create_with(id: &str, params: &Value) -> Result<Box<dyn Behavior>> {
    let mut b = create(id)?;
    b.set_params(params)?;
    Ok(b)
}

/// A behavior from the parameter pairs Character Animator files hold.
///
/// Give the stored defaults first and a puppet's overrides after: later pairs win, unknown names are ignored, and
/// every missing parameter takes this crate's default.
///
/// # Errors
///
/// [`Error::UnknownBehavior`] or [`Error::BadParams`].
pub fn create_from_pairs(id: &str, pairs: &[(String, LValue)]) -> Result<Box<dyn Behavior>> {
    let table = LValue::Table(pairs.iter().map(|(k, v)| (Key::Str(k.clone()), v.clone())).collect());
    let value: Value = charcraft_ltab::from_lvalue(&table).map_err(|e| Error::BadParams { behavior: id.to_owned(), message: e.to_string() })?;
    create_with(id, &value)
}

/// Writes a behavior's parameters back as an ltab table, for saving into a project.
#[must_use]
pub fn params_to_lvalue(behavior: &dyn Behavior) -> LValue {
    charcraft_ltab::to_lvalue(&behavior.params()).unwrap_or_else(|_| LValue::empty_table())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_creates_a_behavior_with_that_id() {
        for id in IDS {
            let b = create(id).unwrap();
            assert_eq!(b.id(), *id);
            assert!(b.params().is_object(), "{id}");
        }
        assert_eq!(create("Adobe.Nothing.js").unwrap_err(), Error::UnknownBehavior("Adobe.Nothing.js".into()));
    }

    #[test]
    fn defaults_round_trip_through_json() {
        for id in IDS {
            let b = create(id).unwrap();
            let again = create_with(id, &b.params()).unwrap();
            assert_eq!(again.params(), b.params(), "{id}");
        }
    }

    #[test]
    fn docs_name_real_parameters() {
        for id in IDS {
            let b = create(id).unwrap();
            let params = b.params();
            for d in b.docs() {
                assert!(params.get(d.name).is_some(), "{id}: documented parameter {} does not exist", d.name);
            }
        }
    }
}
