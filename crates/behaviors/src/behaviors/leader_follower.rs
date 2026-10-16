//! Leader and Follower (this program's own `Charcraft.LeaderFollower.js`): one group copies another's movement a
//! moment later, so a ponytail trails the head and a shadow puppet lags its master.

use std::collections::VecDeque;

use charcraft_puppet::tags::normalize;
use charcraft_puppet::{PuppetPose, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{add_group, percent};

/// Parameters of Leader and Follower.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LeaderFollowerParams {
    /// The group that leads (a name, a tag or `head`).
    pub leader: String,
    /// The group that follows.
    pub follower: String,
    /// Seconds the follower lags.
    pub delay: f64,
    /// How much of the leader's rotation the follower copies, percent (negative swings the other way).
    pub rotation: f64,
    /// How much of its movement, percent.
    pub movement: f64,
    /// How much of its scaling, percent.
    pub scaling: f64,
}

impl Default for LeaderFollowerParams {
    fn default() -> Self {
        Self { leader: "head".to_owned(), follower: "follower".to_owned(), delay: 0.15, rotation: 100.0, movement: 100.0, scaling: 100.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "leader", "", "The group that leads (a name, a tag or head).";
    "follower", "", "The group that follows.";
    "delay", "s", "Seconds the follower lags.";
    "rotation", "%", "Share of the leader's rotation the follower copies.";
    "movement", "%", "Share of its movement.";
    "scaling", "%", "Share of its scaling.";
};

/// What the pose has done to the group so far.
fn xform_of(pose: &PuppetPose, name: &str) -> Xform {
    let key = normalize(name);
    let mut x = Xform::IDENTITY;
    if key == "head" {
        x = x.then(&pose.head);
    }
    if key == "root" {
        x = x.then(&pose.root);
    }
    for (k, v) in &pose.groups {
        if k == name || normalize(k) == key {
            x = x.then(v);
        }
    }
    x.sanitized()
}

/// Makes a group repeat another's movement after a delay.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LeaderFollower {
    /// The parameters.
    pub params: LeaderFollowerParams,
    history: VecDeque<(f64, Xform)>,
}

impl Behavior for LeaderFollower {
    behavior_common!("Charcraft.LeaderFollower.js", "Leader and Follower", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        if p.leader.trim().is_empty() || p.follower.trim().is_empty() {
            return;
        }
        if self.history.back().is_some_and(|(t, _)| cx.time < *t) {
            self.history.clear();
        }
        self.history.push_back((cx.time, xform_of(pose, &p.leader)));
        let wanted = cx.time - p.delay.max(0.0);
        while self.history.get(1).is_some_and(|(t, _)| *t <= wanted) {
            self.history.pop_front();
        }
        // Before the leader has been watched for a whole delay, the follower waits at rest.
        let Some((_, led)) = self.history.front().filter(|(t, _)| *t <= wanted + 1e-9 || p.delay <= 0.0) else { return };
        let share = |v: f64, k: f64| 1.0 + (v - 1.0) * k;
        let (rot, mov, sca) = (percent(p.rotation), percent(p.movement), percent(p.scaling));
        let copy = Xform {
            translation: led.translation * mov,
            rotation: led.rotation * rot,
            scale: anim_core::Vec2::new(share(led.scale.x, sca), share(led.scale.y, sca)),
        };
        if !copy.is_identity() {
            add_group(pose, &p.follower, &copy);
        }
    }

    fn reset(&mut self, _seed: u64) {
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn run(b: &mut LeaderFollower, frames: u32, lead: impl Fn(f64) -> f64) -> Vec<f64> {
        let rig = Rig::default();
        (0..frames)
            .map(|f| {
                let t = f64::from(f) / 60.0;
                let mut pose = PuppetPose::neutral();
                pose.head = Xform::rotated_degrees(lead(t));
                b.step(&Cx { inputs: &Inputs::at(Tick::from_seconds_f64(t)), rig: &rig, time: t, dt: 1.0 / 60.0 }, &mut pose);
                pose.groups.get("follower").map_or(0.0, |x| x.rotation.to_degrees())
            })
            .collect()
    }

    #[test]
    fn the_follower_repeats_the_leader_after_the_delay() {
        let mut b = LeaderFollower { params: LeaderFollowerParams { delay: 0.25, ..LeaderFollowerParams::default() }, ..LeaderFollower::default() };
        let seen = run(&mut b, 120, |t| (t * 40.0).min(40.0));
        // The leader turns 40 degrees a second until it reaches 40 at t = 1; at t = 0.5 the follower shows t = 0.25.
        assert!((seen[30] - 10.0).abs() < 1.0, "{}", seen[30]);
        assert!((seen[110] - 40.0).abs() < 1e-6, "{}", seen[110]);
        assert!(seen[5].abs() < 1e-9, "it waits at rest before the first delay has passed");
    }

    #[test]
    fn shares_and_empty_names() {
        let mut b =
            LeaderFollower { params: LeaderFollowerParams { delay: 0.0, rotation: -50.0, ..LeaderFollowerParams::default() }, ..LeaderFollower::default() };
        assert!((run(&mut b, 3, |_| 20.0)[2] + 10.0).abs() < 1e-6);
        let mut none =
            LeaderFollower { params: LeaderFollowerParams { leader: String::new(), ..LeaderFollowerParams::default() }, ..LeaderFollower::default() };
        assert!(run(&mut none, 3, |_| 20.0).iter().all(|v| *v == 0.0));
    }
}
