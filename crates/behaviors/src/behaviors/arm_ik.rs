//! Arm IK (`Adobe.ArmIK.js`) and Limb IK: parameters are read and kept, but no inverse kinematics is solved yet.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};

/// Parameters of Arm IK, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "mirrors the stored parameter table")]
pub struct ArmIkParams {
    /// Ankle flexibility.
    #[serde(rename = "ankleFlexibility")]
    pub ankle_flexibility: f64,
    /// Which limbs the first group applies to.
    #[serde(rename = "applyTo")]
    pub apply_to: f64,
    /// Which limbs the second group applies to.
    #[serde(rename = "applyTo2")]
    pub apply_to2: f64,
    /// Bend joints automatically.
    #[serde(rename = "autoBend")]
    pub auto_bend: bool,
    /// Keep the feet on the ground.
    #[serde(rename = "autoGround")]
    pub auto_ground: bool,
    /// Elbow strength.
    #[serde(rename = "elbowStrength")]
    pub elbow_strength: f64,
    /// Overall strength.
    #[serde(rename = "globalStrength")]
    pub global_strength: f64,
    /// Place the feet on first use.
    #[serde(rename = "initFeet2")]
    pub init_feet2: bool,
    /// Left heel pin mode.
    #[serde(rename = "pinLeftHeel")]
    pub pin_left_heel: f64,
    /// Left heel pin strength.
    #[serde(rename = "pinLeftHeelStrength")]
    pub pin_left_heel_strength: f64,
    /// Left wrist pin mode.
    #[serde(rename = "pinLeftWrist")]
    pub pin_left_wrist: f64,
    /// Left wrist pin strength.
    #[serde(rename = "pinLeftWristStrength")]
    pub pin_left_wrist_strength: f64,
    /// Right heel pin mode.
    #[serde(rename = "pinRightHeel")]
    pub pin_right_heel: f64,
    /// Right heel pin strength.
    #[serde(rename = "pinRightHeelStrength")]
    pub pin_right_heel_strength: f64,
    /// Right wrist pin mode.
    #[serde(rename = "pinRightWrist")]
    pub pin_right_wrist: f64,
    /// Right wrist pin strength.
    #[serde(rename = "pinRightWristStrength")]
    pub pin_right_wrist_strength: f64,
    /// Bend the left arm the other way.
    #[serde(rename = "reverseBendLeftArm")]
    pub reverse_bend_left_arm: bool,
    /// Bend the left leg the other way.
    #[serde(rename = "reverseBendLeftLeg")]
    pub reverse_bend_left_leg: bool,
    /// Bend the right arm the other way.
    #[serde(rename = "reverseBendRightArm")]
    pub reverse_bend_right_arm: bool,
    /// Bend the right leg the other way.
    #[serde(rename = "reverseBendRightLeg")]
    pub reverse_bend_right_leg: bool,
    /// Shoulder angle limit.
    #[serde(rename = "shoulderAngleLimit")]
    pub shoulder_angle_limit: f64,
    /// Stretchiness.
    pub stretchiness: f64,
    /// Toe stickiness.
    #[serde(rename = "toeStickiness")]
    pub toe_stickiness: f64,
    /// Transition.
    pub transition: f64,
}

impl Default for ArmIkParams {
    fn default() -> Self {
        Self {
            ankle_flexibility: 10.0,
            apply_to: 0.0,
            apply_to2: 0.0,
            auto_bend: true,
            auto_ground: true,
            elbow_strength: 90.0,
            global_strength: 100.0,
            init_feet2: false,
            pin_left_heel: 2.0,
            pin_left_heel_strength: 100.0,
            pin_left_wrist: 1.0,
            pin_left_wrist_strength: 100.0,
            pin_right_heel: 2.0,
            pin_right_heel_strength: 100.0,
            pin_right_wrist: 1.0,
            pin_right_wrist_strength: 100.0,
            reverse_bend_left_arm: false,
            reverse_bend_left_leg: false,
            reverse_bend_right_arm: false,
            reverse_bend_right_leg: false,
            shoulder_angle_limit: 0.0,
            stretchiness: 10.0,
            toe_stickiness: 100.0,
            transition: 100.0,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "globalStrength", "%", "Stored; the solver is not implemented, so the behavior changes nothing.";
};

/// Arm IK: a placeholder that keeps the parameters of projects that use it and changes nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArmIk {
    /// The parameters.
    pub params: ArmIkParams,
}

impl Behavior for ArmIk {
    behavior_common!("Adobe.ArmIK.js", "Arm IK", DOCS);

    fn step(&mut self, _cx: &Cx<'_>, _pose: &mut PuppetPose) {}

    fn reset(&mut self, _seed: u64) {}
}
