//! Walk (this program's own `Charcraft.Walk.js`): a basic side-on walk cycle driven by two keys.

use std::f64::consts::TAU;

use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Smoother, add_group, axis};

/// Parameters of Walk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WalkParams {
    /// Key that walks left.
    #[serde(rename = "leftKey")]
    pub left_key: String,
    /// Key that walks right.
    #[serde(rename = "rightKey")]
    pub right_key: String,
    /// Steps per second.
    pub speed: f64,
    /// Stage pixels travelled per step.
    #[serde(rename = "strideLength")]
    pub stride_length: f64,
    /// How far the body rises on each step, stage pixels.
    pub bounce: f64,
    /// Sway of the body from side to side, degrees.
    pub sway: f64,
    /// Swing of the arms (groups tagged left and right arm), degrees.
    #[serde(rename = "armSwing")]
    pub arm_swing: f64,
    /// Swing of the legs, degrees.
    #[serde(rename = "legSwing")]
    pub leg_swing: f64,
    /// The root moves across the stage while walking.
    #[serde(rename = "moveBody")]
    pub move_body: bool,
    /// Name of the left leg group.
    #[serde(rename = "leftLeg")]
    pub left_leg: String,
    /// Name of the right leg group.
    #[serde(rename = "rightLeg")]
    pub right_leg: String,
    /// Time to get up to speed and to stop, in hundredths of a second.
    #[serde(rename = "easeRate")]
    pub ease_rate: f64,
}

impl Default for WalkParams {
    fn default() -> Self {
        Self {
            left_key: "left".to_owned(),
            right_key: "right".to_owned(),
            speed: 2.5,
            stride_length: 50.0,
            bounce: 10.0,
            sway: 2.0,
            arm_swing: 25.0,
            leg_swing: 30.0,
            move_body: true,
            left_leg: "left leg".to_owned(),
            right_leg: "right leg".to_owned(),
            ease_rate: 15.0,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "leftKey", "", "Key that walks left.";
    "rightKey", "", "Key that walks right.";
    "speed", "1/s", "Steps per second.";
    "strideLength", "px", "Distance per step.";
    "bounce", "px", "Rise of the body on each step.";
    "sway", "deg", "Sideways rock of the body.";
    "armSwing", "deg", "Arm swing, opposite to the legs.";
    "legSwing", "deg", "Leg swing.";
    "moveBody", "", "Carry the puppet across the stage.";
    "leftLeg", "", "Group label of the left leg.";
    "rightLeg", "", "Group label of the right leg.";
};

/// Basic walk cycle: while a walk key is held, the legs swing, the arms counter-swing, the body bounces and sways
/// and the puppet travels. It eases in and out and stays where it stopped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Walk {
    /// The parameters.
    pub params: WalkParams,
    amount: Smoother,
    phase: f64,
    travelled: f64,
}

impl Behavior for Walk {
    behavior_common!("Charcraft.Walk.js", "Walk", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let dir = axis(cx.inputs.key_down(&p.left_key), cx.inputs.key_down(&p.right_key));
        let amount = self.amount.update(dir.abs(), cx.dt, (p.ease_rate / 100.0).max(0.0));
        let speed = p.speed.max(0.0);
        // One cycle is two steps.
        self.phase = (self.phase + cx.dt * speed * amount / 2.0).rem_euclid(1.0);
        if p.move_body {
            let half = cx.rig.stage.width / 2.0;
            self.travelled = (self.travelled + dir * amount * cx.dt * speed * p.stride_length).clamp(-half, half);
        }
        if amount < 1e-4 && self.travelled == 0.0 {
            return;
        }
        let s = (self.phase * TAU).sin();
        let lift = if amount >= 1e-4 { (self.phase * TAU).sin().abs() * p.bounce * amount } else { 0.0 };
        pose.root = pose.root.then(&Xform::moved(self.travelled, -lift));
        if amount >= 1e-4 {
            add_group(pose, ids::BODY, &Xform::rotated_degrees(s * p.sway * amount));
            add_group(pose, ids::LEFT_ARM, &Xform::rotated_degrees(-s * p.arm_swing * amount));
            add_group(pose, ids::RIGHT_ARM, &Xform::rotated_degrees(s * p.arm_swing * amount));
            add_group(pose, &p.left_leg, &Xform::rotated_degrees(s * p.leg_swing * amount));
            add_group(pose, &p.right_leg, &Xform::rotated_degrees(-s * p.leg_swing * amount));
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
