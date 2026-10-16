//! Body Tracker (this program's own `Charcraft.BodyTracker.js`): the performer's arms, legs and torso, seen by a
//! webcam, drive the puppet's limb groups through two-bone inverse kinematics.
//!
//! The tracker hands over joint positions in body units measured against the rest pose the performer set
//! ([`BodyInput`]). For each limb this behavior takes the shoulder (hip) and the wrist (ankle), solves the two
//! bones with the performer's own bone lengths, and rotates the puppet's upper and lower limb groups by how far
//! each bone turned away from its rest direction. The lower group is a child of the upper one, so it is turned by
//! the difference. The torso leans the `body` group with the shoulders, and the hips can carry the whole puppet.

use anim_core::Vec2;
use charcraft_puppet::{PuppetPose, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::ik::{self, Limb};
use crate::input::{BodyInput, Joint};
use crate::util::{Smoother, add_group, percent};

/// Parameters of Body Tracker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "one switch per body part")]
pub struct BodyTrackerParams {
    /// How much of the performer's movement the puppet copies, percent.
    pub strength: f64,
    /// Drive the arms.
    #[serde(rename = "trackArms")]
    pub track_arms: bool,
    /// Drive the legs.
    #[serde(rename = "trackLegs")]
    pub track_legs: bool,
    /// Lean the torso with the shoulders.
    #[serde(rename = "trackTorso")]
    pub track_torso: bool,
    /// Solve the limbs with inverse kinematics from the wrists and ankles; off copies the elbows and knees
    /// directly.
    #[serde(rename = "limbIk")]
    pub limb_ik: bool,
    /// Fold the arms the other way.
    #[serde(rename = "reverseBendArms")]
    pub reverse_bend_arms: bool,
    /// Fold the legs the other way.
    #[serde(rename = "reverseBendLegs")]
    pub reverse_bend_legs: bool,
    /// The least an elbow or knee stays folded, degrees.
    #[serde(rename = "minBend")]
    pub min_bend: f64,
    /// How far the hips carry the puppet across the stage, percent of the torso length in pixels per body unit.
    #[serde(rename = "moveBody")]
    pub move_body: f64,
    /// The same, up and down.
    #[serde(rename = "moveBodyVertical")]
    pub move_body_vertical: f64,
    /// Pixels of stage per body unit (the torso's length); 0 uses a third of the stage's shorter side.
    #[serde(rename = "torsoLength")]
    pub torso_length: f64,
    /// Smoothing time constant, milliseconds.
    pub smoothing: f64,
    /// How long the puppet takes to relax to its rest pose when the performer leaves the picture, milliseconds.
    #[serde(rename = "releaseTime")]
    pub release_time: f64,
    /// Group the puppet's left upper arm is in.
    #[serde(rename = "leftArm")]
    pub left_arm: String,
    /// Group of the left forearm.
    #[serde(rename = "leftForearm")]
    pub left_forearm: String,
    /// Group of the right upper arm.
    #[serde(rename = "rightArm")]
    pub right_arm: String,
    /// Group of the right forearm.
    #[serde(rename = "rightForearm")]
    pub right_forearm: String,
    /// Group of the left thigh (the whole leg when there is no shin group).
    #[serde(rename = "leftLeg")]
    pub left_leg: String,
    /// Group of the left shin.
    #[serde(rename = "leftShin")]
    pub left_shin: String,
    /// Group of the right thigh.
    #[serde(rename = "rightLeg")]
    pub right_leg: String,
    /// Group of the right shin.
    #[serde(rename = "rightShin")]
    pub right_shin: String,
    /// Group of the torso.
    pub body: String,
}

impl Default for BodyTrackerParams {
    fn default() -> Self {
        Self {
            strength: 100.0,
            track_arms: true,
            track_legs: true,
            track_torso: true,
            limb_ik: true,
            reverse_bend_arms: false,
            reverse_bend_legs: false,
            min_bend: 4.0,
            move_body: 100.0,
            move_body_vertical: 100.0,
            torso_length: 0.0,
            smoothing: 50.0,
            release_time: 300.0,
            left_arm: "left arm".to_owned(),
            left_forearm: "left forearm".to_owned(),
            right_arm: "right arm".to_owned(),
            right_forearm: "right forearm".to_owned(),
            left_leg: "left leg".to_owned(),
            left_shin: "left shin".to_owned(),
            right_leg: "right leg".to_owned(),
            right_shin: "right shin".to_owned(),
            body: "body".to_owned(),
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "strength", "%", "How much of the performer's movement the puppet copies.";
    "trackArms", "", "Drive the arms.";
    "trackLegs", "", "Drive the legs.";
    "trackTorso", "", "Lean the torso with the shoulders.";
    "limbIk", "", "Solve limbs from the wrists and ankles with inverse kinematics; off copies elbows and knees.";
    "reverseBendArms", "", "Fold the arms the other way.";
    "reverseBendLegs", "", "Fold the legs the other way.";
    "minBend", "deg", "The least an elbow or knee stays folded.";
    "moveBody", "%", "How far the hips carry the puppet sideways.";
    "moveBodyVertical", "%", "How far the hips carry the puppet up and down.";
    "torsoLength", "px", "Stage pixels per body unit; 0 is a third of the stage's shorter side.";
    "smoothing", "ms", "Smoothing time constant.";
    "releaseTime", "ms", "Time to relax to the rest pose when the performer is lost.";
    "leftArm", "", "Group of the left upper arm.";
    "leftForearm", "", "Group of the left forearm.";
    "rightArm", "", "Group of the right upper arm.";
    "rightForearm", "", "Group of the right forearm.";
    "leftLeg", "", "Group of the left thigh.";
    "leftShin", "", "Group of the left shin.";
    "rightLeg", "", "Group of the right thigh.";
    "rightShin", "", "Group of the right shin.";
    "body", "", "Group of the torso.";
};

/// One limb: its three joints and which way it folds at rest.
#[derive(Debug, Clone, Copy)]
struct LimbJoints {
    root: Joint,
    mid: Joint,
    end: Joint,
}

const LEFT_ARM: LimbJoints = LimbJoints { root: Joint::LeftShoulder, mid: Joint::LeftElbow, end: Joint::LeftWrist };
const RIGHT_ARM: LimbJoints = LimbJoints { root: Joint::RightShoulder, mid: Joint::RightElbow, end: Joint::RightWrist };
const LEFT_LEG: LimbJoints = LimbJoints { root: Joint::LeftHip, mid: Joint::LeftKnee, end: Joint::LeftAnkle };
const RIGHT_LEG: LimbJoints = LimbJoints { root: Joint::RightHip, mid: Joint::RightKnee, end: Joint::RightAnkle };

/// Channels smoothed over time: 4 limbs x (upper, lower), the lean, the move.
const CHANNELS: usize = 11;

/// The turn of each bone of a limb away from its rest direction, degrees: `(upper, lower relative to upper)`.
fn limb_turn(body: &BodyInput, joints: LimbJoints, ik_on: bool, reverse: bool, min_bend: f64) -> Option<(f64, f64)> {
    let (root, mid, end) = (body.joint(joints.root)?, body.joint(joints.mid)?, body.joint(joints.end)?);
    let (r_root, r_mid, r_end) = (body.rest_joint(joints.root), body.rest_joint(joints.mid), body.rest_joint(joints.end));
    let (l1, l2) = ((r_mid - r_root).hypot(), (r_end - r_mid).hypot());
    if l1 < 1e-6 || l2 < 1e-6 {
        return None;
    }
    let rest = Limb { upper: ik::direction(r_mid - r_root), lower: ik::direction(r_end - r_mid) };
    let now = if ik_on {
        let rest_bend = ik::bend_side(r_root, r_mid, r_end, 1.0);
        let seen = ik::bend_side(root, mid, end, rest_bend);
        let side = if reverse { -seen } else { seen };
        // A limb that is straight at rest must come back to straight, so the least fold never exceeds the rest fold.
        let rest_fold = ik::wrap(rest.lower - rest.upper).abs();
        ik::solve(root, end, l1, l2, side, min_bend.to_radians().min(rest_fold))
    } else {
        Limb { upper: ik::direction(mid - root), lower: ik::direction(end - mid) }
    };
    let upper = ik::wrap(now.upper - rest.upper);
    let lower = ik::wrap(ik::wrap(now.lower - rest.lower) - upper);
    Some((upper.to_degrees(), lower.to_degrees()))
}

/// The middle of two joints now, when both are visible.
fn middle(body: &BodyInput, a: Joint, b: Joint) -> Option<Vec2> {
    Some((body.joint(a)? + body.joint(b)?) / 2.0)
}

/// Full-body tracking: limbs through inverse kinematics, torso lean, and the hips' travel.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BodyTracker {
    /// The parameters.
    pub params: BodyTrackerParams,
    targets: [Smoother; CHANNELS],
    weight: Smoother,
}

impl BodyTracker {
    /// The turns the performer's body asks of the limbs, degrees, before smoothing and strength: for each of the
    /// left arm, right arm, left leg and right leg `(upper, lower)`, the torso lean, and the hips' travel in body
    /// units. A part that is not visible asks for nothing.
    #[must_use]
    pub fn read(&self, body: &BodyInput) -> [f64; CHANNELS] {
        let p = &self.params;
        let mut out = [0.0; CHANNELS];
        let limbs = [
            (LEFT_ARM, p.track_arms, p.reverse_bend_arms),
            (RIGHT_ARM, p.track_arms, p.reverse_bend_arms),
            (LEFT_LEG, p.track_legs, p.reverse_bend_legs),
            (RIGHT_LEG, p.track_legs, p.reverse_bend_legs),
        ];
        for (i, (joints, on, reverse)) in limbs.into_iter().enumerate() {
            if !on {
                continue;
            }
            if let Some((upper, lower)) = limb_turn(body, joints, p.limb_ik, reverse, p.min_bend) {
                let at = i.saturating_mul(2);
                if let Some(slot) = out.get_mut(at) {
                    *slot = upper;
                }
                if let Some(slot) = out.get_mut(at.saturating_add(1)) {
                    *slot = lower;
                }
            }
        }
        if p.track_torso
            && let (Some(top), Some(bottom)) = (middle(body, Joint::LeftShoulder, Joint::RightShoulder), middle(body, Joint::LeftHip, Joint::RightHip))
        {
            let rest_top = (body.rest_joint(Joint::LeftShoulder) + body.rest_joint(Joint::RightShoulder)) / 2.0;
            let rest_bottom = (body.rest_joint(Joint::LeftHip) + body.rest_joint(Joint::RightHip)) / 2.0;
            let lean = ik::wrap(ik::direction(top - bottom) - ik::direction(rest_top - rest_bottom)).to_degrees();
            if let Some(slot) = out.get_mut(8) {
                *slot = lean;
            }
        }
        let hips =
            middle(body, Joint::LeftHip, Joint::RightHip).or_else(|| middle(body, Joint::LeftShoulder, Joint::RightShoulder).map(|s| s + Vec2::new(0.0, 1.0)));
        let rest_hips = (body.rest_joint(Joint::LeftHip) + body.rest_joint(Joint::RightHip)) / 2.0;
        if let Some(h) = hips {
            if let Some(slot) = out.get_mut(9) {
                *slot = h.x - rest_hips.x;
            }
            if let Some(slot) = out.get_mut(10) {
                *slot = h.y - rest_hips.y;
            }
        }
        out
    }
}

impl Behavior for BodyTracker {
    behavior_common!("Charcraft.BodyTracker.js", "Body Tracker", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let tau = p.smoothing.max(0.0) / 1000.0;
        let release = p.release_time.max(0.0) / 1000.0;
        let body = cx.inputs.body.as_ref().map(BodyInput::sanitized);
        let seen = body.is_some();
        let weight = self.weight.update(if seen { 1.0 } else { 0.0 }, cx.dt, if seen { tau } else { release });
        if let Some(body) = &body {
            let wanted = self.read(body);
            for (slot, value) in self.targets.iter_mut().zip(wanted) {
                slot.update(value, cx.dt, tau);
            }
        }
        if weight < 1e-4 {
            return;
        }
        let k = percent(p.strength).clamp(0.0, 2.0) * weight;
        let at = |i: usize| self.targets.get(i).map_or(0.0, Smoother::get) * k;
        let groups: [(&str, f64); 8] = [
            (&p.left_arm, at(0)),
            (&p.left_forearm, at(1)),
            (&p.right_arm, at(2)),
            (&p.right_forearm, at(3)),
            (&p.left_leg, at(4)),
            (&p.left_shin, at(5)),
            (&p.right_leg, at(6)),
            (&p.right_shin, at(7)),
        ];
        for (name, degrees) in groups {
            if degrees.abs() > 1e-6 && !name.is_empty() {
                add_group(pose, name, &Xform::rotated_degrees(degrees));
            }
        }
        if at(8).abs() > 1e-6 && !p.body.is_empty() {
            add_group(pose, &p.body, &Xform::rotated_degrees(at(8)));
        }
        let unit = if p.torso_length.is_finite() && p.torso_length > 1.0 { p.torso_length } else { cx.rig.unit() / 3.0 };
        let shift = Vec2::new(at(9) * unit * percent(p.move_body), at(10) * unit * percent(p.move_body_vertical));
        if shift.hypot() > 1e-9 {
            pose.root = pose.root.then(&Xform::moved(shift.x, shift.y));
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn run(tracker: &mut BodyTracker, body: Option<BodyInput>, seconds: f64) -> PuppetPose {
        let rig = Rig::default();
        let mut pose = PuppetPose::neutral();
        let frames = (seconds * 60.0).round() as u32;
        for f in 0..=frames {
            let t = f64::from(f) / 60.0;
            let mut inputs = Inputs::at(Tick::from_seconds_f64(t));
            inputs.body = body.clone();
            pose = PuppetPose::neutral();
            tracker.step(&Cx { inputs: &inputs, rig: &rig, time: t, dt: if f == 0 { 0.0 } else { 1.0 / 60.0 } }, &mut pose);
        }
        pose
    }

    fn degrees(pose: &PuppetPose, group: &str) -> f64 {
        pose.groups.get(group).map_or(0.0, |x| x.rotation.to_degrees())
    }

    /// The standing body with the left wrist moved to `wrist` (elbow found by the same limb lengths).
    fn raised_left_arm(wrist: Vec2, elbow: Vec2) -> BodyInput {
        let mut body = BodyInput::at_rest();
        body.joints.insert(Joint::LeftElbow, elbow);
        body.joints.insert(Joint::LeftWrist, wrist);
        body
    }

    #[test]
    fn a_body_at_rest_leaves_the_puppet_alone() {
        let mut t = BodyTracker::default();
        let pose = run(&mut t, Some(BodyInput::at_rest()), 0.5);
        assert!(pose.groups.values().all(Xform::is_identity), "{:?}", pose.groups);
        assert!(pose.root.is_identity());
    }

    #[test]
    fn no_body_means_no_movement() {
        let mut t = BodyTracker::default();
        assert!(run(&mut t, None, 0.5).is_neutral());
    }

    #[test]
    fn a_raised_left_arm_turns_the_arm_and_forearm_groups() {
        // The arm hangs down at rest (a quarter turn clockwise from "right"); lifting it sideways to horizontal
        // (towards +x) turns it a quarter anticlockwise, that is -90 degrees.
        let shoulder = BodyInput::standard_rest()[&Joint::LeftShoulder];
        let l = ((0.5_f64 - 0.45).powi(2) + 0.6_f64.powi(2)).sqrt();
        let body = raised_left_arm(shoulder + Vec2::new(2.0 * l, 0.0), shoulder + Vec2::new(l, 0.0));
        let mut t = BodyTracker::default();
        let pose = run(&mut t, Some(body), 1.0);
        let upper = degrees(&pose, "left arm");
        assert!((upper + 85.0).abs() < 8.0, "upper arm turned {upper}");
        assert!(degrees(&pose, "left forearm").abs() < 8.0, "a straight arm does not bend");
        assert_eq!(degrees(&pose, "right arm"), 0.0);
    }

    #[test]
    fn a_bent_elbow_turns_the_forearm_relative_to_the_upper_arm() {
        let shoulder = BodyInput::standard_rest()[&Joint::LeftShoulder];
        let l = ((0.5_f64 - 0.45).powi(2) + 0.6_f64.powi(2)).sqrt();
        // Upper arm out sideways, forearm straight up.
        let body = raised_left_arm(shoulder + Vec2::new(l, -l), shoulder + Vec2::new(l, 0.0));
        let mut t = BodyTracker::default();
        let pose = run(&mut t, Some(body), 1.0);
        let upper = degrees(&pose, "left arm");
        let lower = degrees(&pose, "left forearm");
        assert!((upper + 90.0).abs() < 8.0, "{upper}");
        // The forearm points up: 180 degrees from rest (down), minus the upper arm's -90 turn.
        assert!((lower.abs() - 90.0).abs() < 12.0, "{lower}");
    }

    #[test]
    fn inverse_kinematics_agrees_with_the_elbow_for_a_consistent_arm_and_corrects_a_stretched_one() {
        let shoulder = BodyInput::standard_rest()[&Joint::LeftShoulder];
        let l = ((0.5_f64 - 0.45).powi(2) + 0.6_f64.powi(2)).sqrt();
        let consistent = raised_left_arm(shoulder + Vec2::new(l, -l), shoulder + Vec2::new(l, 0.0));
        let direct = BodyTracker { params: BodyTrackerParams { limb_ik: false, ..BodyTrackerParams::default() }, ..BodyTracker::default() };
        let solved = BodyTracker::default();
        let (a, b) = (direct.read(&consistent), solved.read(&consistent));
        assert!((a[0] - b[0]).abs() < 3.0 && (a[1] - b[1]).abs() < 3.0, "{a:?} vs {b:?}");
        // A noisy elbow far off the line: IK still keeps the bones their length, so the wrist is where it was seen.
        let mut noisy = consistent.clone();
        noisy.joints.insert(Joint::LeftElbow, shoulder + Vec2::new(l * 1.4, 0.2));
        let fixed = solved.read(&noisy);
        assert!((fixed[0] - b[0]).abs() < 3.0, "ik ignores where the elbow was seen: {fixed:?} vs {b:?}");
        let copied = direct.read(&noisy);
        assert!((copied[0] - b[0]).abs() > 5.0, "direct copying follows the noisy elbow");
    }

    #[test]
    fn a_hidden_limb_asks_for_nothing_and_missing_legs_are_fine() {
        let mut body = BodyInput::at_rest();
        for j in [Joint::LeftKnee, Joint::LeftAnkle, Joint::RightKnee, Joint::RightAnkle] {
            body.joints.remove(&j);
        }
        let want = BodyTracker::default().read(&body);
        assert!(want[4..8].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn leaning_the_shoulders_leans_the_body_group_and_hips_carry_the_puppet() {
        let mut body = BodyInput::at_rest();
        for j in [Joint::LeftShoulder, Joint::RightShoulder] {
            let p = body.joints[&j];
            body.joints.insert(j, p + Vec2::new(0.5, 0.0));
        }
        for j in [Joint::LeftHip, Joint::RightHip] {
            let p = body.joints[&j];
            body.joints.insert(j, p + Vec2::new(0.2, 0.0));
        }
        let mut t = BodyTracker::default();
        let pose = run(&mut t, Some(body), 1.0);
        assert!(degrees(&pose, "body") > 10.0, "{}", degrees(&pose, "body"));
        let unit = Rig::default().unit() / 3.0;
        assert!((pose.root.translation.x - 0.2 * unit).abs() < 1.0, "{:?}", pose.root);
    }

    #[test]
    fn losing_the_performer_relaxes_the_puppet() {
        let shoulder = BodyInput::standard_rest()[&Joint::LeftShoulder];
        let l = ((0.5_f64 - 0.45).powi(2) + 0.6_f64.powi(2)).sqrt();
        let body = raised_left_arm(shoulder + Vec2::new(2.0 * l, 0.0), shoulder + Vec2::new(l, 0.0));
        let mut t = BodyTracker::default();
        assert!(degrees(&run(&mut t, Some(body), 1.0), "left arm") < -60.0);
        let after = run(&mut t, None, 3.0);
        assert!(degrees(&after, "left arm").abs() < 1.0, "relaxed: {}", degrees(&after, "left arm"));
    }

    #[test]
    fn strength_scales_and_hostile_bodies_stay_finite() {
        let shoulder = BodyInput::standard_rest()[&Joint::LeftShoulder];
        let body = raised_left_arm(shoulder + Vec2::new(1.0, 0.0), shoulder + Vec2::new(0.5, 0.0));
        let full = degrees(&run(&mut BodyTracker::default(), Some(body.clone()), 1.0), "left arm");
        let half = BodyTracker { params: BodyTrackerParams { strength: 50.0, ..BodyTrackerParams::default() }, ..BodyTracker::default() };
        let half = degrees(&run(&mut { half }, Some(body), 1.0), "left arm");
        assert!((half - full / 2.0).abs() < 1.0, "{half} vs {full}");
        let mut bad = BodyInput::at_rest();
        bad.joints.insert(Joint::LeftWrist, Vec2::new(f64::NAN, f64::INFINITY));
        bad.joints.insert(Joint::RightWrist, Vec2::new(1e300, -1e300));
        let pose = run(&mut BodyTracker::default(), Some(bad), 0.2);
        assert!(pose.groups.values().all(|x| x.rotation.is_finite()) && pose.root.translation.x.is_finite());
    }

    #[test]
    fn parameters_round_trip() {
        let t = BodyTracker::default();
        let mut again = BodyTracker::default();
        again.set_params(&t.params()).unwrap();
        assert_eq!(again.params(), t.params());
    }
}
