//! Wiggler (this program's own `Charcraft.Wiggler.js`): groups shake, sway or pulse on their own, like a jelly, a
//! hanging lamp or an excited tail.

use std::f64::consts::TAU;

use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Rng, add_group, percent};

/// Parameters of Wiggler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WigglerParams {
    /// More groups to wiggle besides the ones tagged `Wiggle` (names or tags, comma separated).
    pub groups: String,
    /// A key that must be held; empty wiggles all the time.
    #[serde(rename = "triggerKey")]
    pub trigger_key: String,
    /// Rotation swing, degrees either side.
    pub angle: f64,
    /// Sideways travel, pixels either side.
    #[serde(rename = "moveX")]
    pub move_x: f64,
    /// Vertical travel, pixels either side.
    #[serde(rename = "moveY")]
    pub move_y: f64,
    /// Pulsing, percent either side of the normal size.
    pub scale: f64,
    /// Wiggles per second.
    pub speed: f64,
    /// How irregular the motion is, percent: 0 is a clean sine, 100 is mostly noise.
    pub randomness: f64,
    /// Seconds to ease in and out when the key goes down and up.
    pub ease: f64,
}

impl Default for WigglerParams {
    fn default() -> Self {
        Self { groups: String::new(), trigger_key: String::new(), angle: 8.0, move_x: 0.0, move_y: 0.0, scale: 0.0, speed: 1.5, randomness: 40.0, ease: 0.2 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "groups", "", "Groups to wiggle (comma separated names or tags).";
    "triggerKey", "", "Key that must be held; empty wiggles all the time.";
    "angle", "deg", "Rotation swing either side.";
    "moveX", "px", "Sideways travel either side.";
    "moveY", "px", "Vertical travel either side.";
    "scale", "%", "Pulsing either side of the normal size.";
    "speed", "1/s", "Wiggles per second.";
    "randomness", "%", "How irregular the motion is.";
    "ease", "s", "Time to ease in and out.";
};

/// Shakes groups with a smooth, repeatable noise.
#[derive(Debug, Clone, PartialEq)]
pub struct Wiggler {
    /// The parameters.
    pub params: WigglerParams,
    phases: [f64; 6],
    amount: crate::util::Smoother,
    clock: f64,
}

impl Default for Wiggler {
    fn default() -> Self {
        let mut w = Self { params: WigglerParams::default(), phases: [0.0; 6], amount: crate::util::Smoother::default(), clock: 0.0 };
        w.reset(0);
        w
    }
}

impl Wiggler {
    /// Five channels of motion at one moment, each -1 to 1: rotation, x, y, scale. The clean part is a sine, the
    /// irregular part a sum of unrelated sines, so two channels never move together.
    #[must_use]
    pub fn sample(&self, clock: f64) -> [f64; 4] {
        let r = percent(self.params.randomness).clamp(0.0, 1.0);
        let at = |i: usize, ratio: f64| {
            let phase = self.phases.get(i).copied().unwrap_or(0.0);
            let clean = (clock * TAU + phase).sin();
            let rough = f64::midpoint((clock * TAU * ratio + phase * 1.7).sin(), (clock * TAU * ratio * 2.3 + phase * 0.6).sin());
            clean * (1.0 - r) + rough * r
        };
        [at(0, 1.0), at(1, 1.31), at(2, 0.77), at(3, 1.57)]
    }
}

impl Behavior for Wiggler {
    behavior_common!("Charcraft.Wiggler.js", "Wiggler", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let on = p.trigger_key.is_empty() || cx.inputs.key_down(&p.trigger_key);
        let amount = self.amount.update(if on { 1.0 } else { 0.0 }, cx.dt, p.ease.max(0.0));
        self.clock += cx.dt.max(0.0) * p.speed.max(0.0);
        if amount < 1e-4 {
            return;
        }
        let [rot, dx, dy, pulse] = self.sample(self.clock);
        let grow = 1.0 + percent(p.scale) * pulse * amount;
        let x = Xform {
            translation: anim_core::Vec2::new(dx * p.move_x * amount, dy * p.move_y * amount),
            rotation: (rot * p.angle * amount).to_radians(),
            scale: anim_core::Vec2::new(grow, grow),
        };
        for name in p.groups.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            add_group(pose, name, &x);
        }
        if cx.rig.has_tag(ids::WIGGLE) {
            add_group(pose, ids::WIGGLE, &x);
        }
    }

    fn reset(&mut self, seed: u64) {
        let mut rng = Rng::derived(seed, 0x0071_661E);
        for slot in &mut self.phases {
            *slot = rng.range(0.0, TAU);
        }
        self.amount.reset();
        self.clock = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn run(w: &mut Wiggler, seconds: f64, key: Option<&str>) -> Vec<f64> {
        let mut rig = Rig::default();
        rig.tags.insert(ids::WIGGLE.to_owned());
        (0..(seconds * 60.0) as u32)
            .map(|f| {
                let t = f64::from(f) / 60.0;
                let mut inputs = Inputs::at(Tick::from_seconds_f64(t));
                if let Some(k) = key {
                    inputs = inputs.with_key(k);
                }
                let mut pose = PuppetPose::neutral();
                w.step(&Cx { inputs: &inputs, rig: &rig, time: t, dt: 1.0 / 60.0 }, &mut pose);
                pose.groups.get(ids::WIGGLE).map_or(0.0, |x| x.rotation.to_degrees())
            })
            .collect()
    }

    #[test]
    fn it_swings_within_the_angle_and_is_not_still() {
        let mut w = Wiggler::default();
        let angles = run(&mut w, 4.0, None);
        let (lo, hi) = angles.iter().fold((f64::MAX, f64::MIN), |(l, h), a| (l.min(*a), h.max(*a)));
        assert!(hi > 3.0 && lo < -3.0, "{lo} {hi}");
        assert!(hi <= 8.0 + 1e-9 && lo >= -8.0 - 1e-9);
    }

    #[test]
    fn the_key_gates_it_and_the_seed_repeats_it() {
        let mut gated = Wiggler { params: WigglerParams { trigger_key: "w".into(), ..WigglerParams::default() }, ..Wiggler::default() };
        assert!(run(&mut gated, 1.0, None).iter().all(|a| *a == 0.0));
        assert!(run(&mut gated, 1.0, Some("w")).iter().any(|a| a.abs() > 1.0));
        let (mut a, mut b) = (Wiggler::default(), Wiggler::default());
        a.reset(5);
        b.reset(5);
        assert_eq!(run(&mut a, 1.0, None), run(&mut b, 1.0, None));
        b.reset(6);
        assert_ne!(run(&mut a, 1.0, None), run(&mut b, 1.0, None));
    }
}
