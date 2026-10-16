//! Walk (this program's own `Charcraft.Walk.js`): a side-on walk cycle driven by two keys, in several styles.

use std::f64::consts::TAU;

use anim_core::Vec2;
use charcraft_puppet::tags::ids;
use charcraft_puppet::{HandleKind, PuppetPose, Xform};
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
    /// The way of walking: `walk`, `strut`, `prance`, `sneak`, `march`, `run` or `shuffle`. The other numbers are
    /// scaled by what the style does differently.
    pub style: String,
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
    /// While standing, the feet (handles of the walk cycle named ankle, heel, toe or foot) stay where they are
    /// whatever the body does.
    #[serde(rename = "pinFeet")]
    pub pin_feet: bool,
}

impl Default for WalkParams {
    fn default() -> Self {
        Self {
            left_key: "left".to_owned(),
            right_key: "right".to_owned(),
            style: "walk".to_owned(),
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
            pin_feet: true,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "leftKey", "", "Key that walks left.";
    "rightKey", "", "Key that walks right.";
    "style", "", "The way of walking.", ["walk", "strut", "prance", "sneak", "march", "run", "shuffle"];
    "speed", "1/s", "Steps per second.";
    "strideLength", "px", "Distance per step.";
    "bounce", "px", "Rise of the body on each step.";
    "sway", "deg", "Sideways rock of the body.";
    "armSwing", "deg", "Arm swing, opposite to the legs.";
    "legSwing", "deg", "Leg swing.";
    "moveBody", "", "Carry the puppet across the stage.";
    "leftLeg", "", "Group label of the left leg.";
    "rightLeg", "", "Group label of the right leg.";
    "easeRate", "1/100 s", "Time to get up to speed and to stop.";
    "pinFeet", "", "Keep the feet planted while standing.";
};

/// How a style changes the numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Gait {
    speed: f64,
    stride: f64,
    bounce: f64,
    sway: f64,
    arms: f64,
    legs: f64,
    /// Degrees the body leans into the direction of travel (negative leans back).
    lean: f64,
    /// How far the body sinks, as a fraction of the stage's shorter side.
    crouch: f64,
    /// 1 is a plain sine swing; above it the swing holds at its ends (a march).
    stiffness: f64,
}

impl Gait {
    fn of(style: &str) -> Self {
        let g = |speed, stride, bounce, sway, arms, legs, lean, crouch, stiffness| Self { speed, stride, bounce, sway, arms, legs, lean, crouch, stiffness };
        match style.trim().to_lowercase().as_str() {
            "strut" => g(0.8, 1.15, 0.8, 2.5, 0.6, 1.1, -3.0, 0.0, 1.0),
            "prance" => g(1.3, 0.9, 2.0, 0.6, 1.2, 1.3, 0.0, 0.0, 1.0),
            "sneak" => g(0.6, 0.6, 0.25, 0.5, 0.4, 0.7, 8.0, 0.04, 1.0),
            "march" => g(1.1, 1.0, 1.2, 0.2, 1.3, 1.0, 0.0, 0.0, 0.5),
            "run" => g(2.0, 1.8, 1.6, 1.0, 1.6, 1.7, 10.0, 0.0, 1.0),
            "shuffle" => g(1.2, 0.4, 0.15, 0.3, 0.2, 0.4, 3.0, 0.01, 1.0),
            _ => g(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0),
        }
    }
}

/// The names of the styles, for lists.
pub const STYLES: [&str; 7] = ["walk", "strut", "prance", "sneak", "march", "run", "shuffle"];

/// Sine with a flattened or sharpened crest: `shape` 1 is a sine, below 1 is squarer.
fn shaped(s: f64, shape: f64) -> f64 {
    s.signum() * s.abs().powf(shape.clamp(0.1, 4.0))
}

/// Whether a handle is part of a foot.
fn is_foot(key: &str) -> bool {
    let k = key.to_lowercase();
    ["ankle", "heel", "toe", "foot"].iter().any(|w| k.contains(w))
}

/// Walk cycle in the manner of the chosen style.
///
/// While a walk key is held, the legs swing, the arms counter-swing, the body bounces and sways and the puppet
/// travels. It eases in and out and stays where it stopped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Walk {
    /// The parameters.
    pub params: WalkParams,
    amount: Smoother,
    phase: f64,
    travelled: f64,
    facing: f64,
}

impl Behavior for Walk {
    behavior_common!("Charcraft.Walk.js", "Walk", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let gait = Gait::of(&p.style);
        let dir = axis(cx.inputs.key_down(&p.left_key), cx.inputs.key_down(&p.right_key));
        if dir != 0.0 {
            self.facing = dir;
        }
        let amount = self.amount.update(dir.abs(), cx.dt, (p.ease_rate / 100.0).max(0.0));
        let speed = p.speed.max(0.0) * gait.speed;
        // One cycle is two steps.
        self.phase = (self.phase + cx.dt * speed * amount / 2.0).rem_euclid(1.0);
        if p.move_body {
            let half = cx.rig.stage.width / 2.0;
            self.travelled = (self.travelled + dir * amount * cx.dt * speed * p.stride_length * gait.stride).clamp(-half, half);
        }
        let standing = amount < 1e-4;
        if standing && self.travelled == 0.0 && !p.pin_feet {
            return;
        }
        let base = pose.root.translation;
        if !standing {
            let s = shaped((self.phase * TAU).sin(), gait.stiffness);
            let lift = (self.phase * TAU).sin().abs() * p.bounce * gait.bounce * amount;
            let sink = gait.crouch * cx.rig.unit() * amount;
            pose.root = pose.root.then(&Xform::moved(self.travelled, sink - lift));
            let lean = gait.lean * self.facing * amount;
            add_group(pose, ids::BODY, &Xform::rotated_degrees(s * p.sway * gait.sway * amount + lean));
            add_group(pose, ids::LEFT_ARM, &Xform::rotated_degrees(-s * p.arm_swing * gait.arms * amount));
            add_group(pose, ids::RIGHT_ARM, &Xform::rotated_degrees(s * p.arm_swing * gait.arms * amount));
            add_group(pose, &p.left_leg, &Xform::rotated_degrees(s * p.leg_swing * gait.legs * amount));
            add_group(pose, &p.right_leg, &Xform::rotated_degrees(-s * p.leg_swing * gait.legs * amount));
        } else if self.travelled != 0.0 {
            pose.root = pose.root.then(&Xform::moved(self.travelled, 0.0));
        }
        if standing && p.pin_feet {
            // The ground stays put: whatever moved the body since the walk began, the feet go back to where the
            // puppet's travel left them.
            let ground = base + Vec2::new(self.travelled, 0.0);
            for handle in cx.rig.handles.iter().filter(|h| h.kind == HandleKind::Walk && is_foot(&h.key)) {
                let offset = handle.position + ground - handle.driven_position(pose);
                if offset.hypot() < 1e-6 {
                    continue;
                }
                let local = handle.to_group_space(pose, offset);
                if local != Vec2::ZERO {
                    pose.drag.insert(handle.key.clone(), pose.drag_of(&handle.key) + local);
                }
            }
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
