//! Physics (`Adobe.RigDynamics.js`): handles tagged `Dangle` swing like they hang from their group.

use std::collections::BTreeMap;

use anim_core::{Point, Vec2};
use charcraft_puppet::{HandleKind, PuppetPose};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Rng, compass, percent};

/// Natural frequency of a dangling handle at stiffness 100% and gravity 10, in hertz.
pub const BASE_FREQUENCY: f64 = 2.0;
/// Wind acceleration in stage pixels per second squared per unit of wind strength.
pub const WIND_ACCELERATION: f64 = 40.0;
/// Longest step the integrator takes, seconds.
pub const MAX_SUBSTEP: f64 = 1.0 / 240.0;

/// Parameters of Physics, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DynamicsParams {
    /// Shape used for collisions (stored, collisions with other bodies are not simulated).
    #[serde(rename = "GeometryType")]
    pub geometry_type: f64,
    /// Extra damping, percent.
    pub damping: f64,
    /// Mass per area, percent: heavier handles are pushed less by wind.
    pub density: f64,
    /// Friction, percent; adds to damping.
    pub friction: f64,
    /// Compass direction gravity pulls (180 is down), degrees.
    #[serde(rename = "gravityDirection")]
    pub gravity_direction: f64,
    /// Gravity strength; with stiffness it sets how quickly a handle swings back.
    #[serde(rename = "gravityStrength")]
    pub gravity_strength: f64,
    /// Motor strength (stored, no effect).
    #[serde(rename = "motorStrength")]
    pub motor_strength: f64,
    /// Bounciness on reaching the end of the swing, percent.
    pub restitution: f64,
    /// Squash and stretch (stored, no effect).
    pub squashiness: f64,
    /// When physics starts (stored, no effect: it always runs).
    #[serde(rename = "startWhen")]
    pub start_when: f64,
    /// Stiffness, percent: how hard the handle is pulled back to rest.
    pub stiffness: f64,
    /// Handles bounce at the end of their reach.
    #[serde(rename = "wallCollide")]
    pub wall_collide: bool,
    /// Compass direction the wind blows towards (90 is right), degrees.
    #[serde(rename = "windDirection")]
    pub wind_direction: f64,
    /// Wind strength.
    #[serde(rename = "windStrength")]
    pub wind_strength: f64,
    /// How much the wind gusts, percent.
    #[serde(rename = "windVariation")]
    pub wind_variation: f64,
}

impl Default for DynamicsParams {
    fn default() -> Self {
        Self {
            geometry_type: 0.0,
            damping: 0.0,
            density: 100.0,
            friction: 30.0,
            gravity_direction: 180.0,
            gravity_strength: 10.0,
            motor_strength: 0.0,
            restitution: 20.0,
            squashiness: 0.0,
            start_when: 0.0,
            stiffness: 100.0,
            wall_collide: true,
            wind_direction: 90.0,
            wind_strength: 0.0,
            wind_variation: 0.0,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "stiffness", "%", "How hard a dangling handle is pulled back to its rest position.";
    "gravityStrength", "", "With stiffness, sets the swing frequency (10 at stiffness 100 swings twice a second).";
    "gravityDirection", "deg", "Compass direction of gravity; the artwork is assumed to hang the way gravity pulls.";
    "damping", "%", "Damping, added to friction.";
    "friction", "%", "Friction, added to damping.";
    "density", "%", "Heavier handles are pushed less by wind.";
    "restitution", "%", "Bounce at the end of the reach.";
    "wallCollide", "", "Handles stop at their full reach and bounce.";
    "windStrength", "", "Steady push on every dangling handle.";
    "windDirection", "deg", "Compass direction of the wind.";
    "windVariation", "%", "Gust depth.";
};

#[derive(Debug, Clone, PartialEq)]
struct Mass {
    position: Point,
    velocity: Vec2,
    driven: Point,
}

/// A damped spring-mass per `Dangle` handle.
///
/// The handle's rest position, carried by the pose of its group and the groups around it, is the anchor; moving
/// the anchor swings the mass, so a nodding head makes hair sway.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dynamics {
    /// The parameters.
    pub params: DynamicsParams,
    masses: BTreeMap<String, Mass>,
    phases: (f64, f64),
}

impl Dynamics {
    fn wind(&self, t: f64) -> Vec2 {
        let p = &self.params;
        let gust = 1.0 + percent(p.wind_variation) * (t * 1.3 + self.phases.0).sin() * (t * 0.7 + self.phases.1).sin();
        compass(p.wind_direction) * (p.wind_strength * WIND_ACCELERATION * gust * (100.0 / p.density.max(10.0)))
    }

    /// Frequency (rad/s) and damping ratio from the parameters.
    fn spring(&self) -> (f64, f64) {
        let p = &self.params;
        let w0 = BASE_FREQUENCY * std::f64::consts::TAU;
        let k = (percent(p.stiffness) * (p.gravity_strength / 10.0).max(0.1)).max(0.01);
        let omega = w0 * k.sqrt();
        let zeta = (0.05 + 0.9 * percent(p.damping + p.friction)).clamp(0.02, 2.0);
        (omega, zeta)
    }
}

impl Behavior for Dynamics {
    behavior_common!("Adobe.RigDynamics.js", "Physics", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let (omega, zeta) = self.spring();
        let restitution = percent(self.params.restitution).clamp(0.0, 1.0);
        let dt = cx.dt.clamp(0.0, 0.1);
        let steps_f = (dt / MAX_SUBSTEP).ceil().clamp(1.0, 64.0);
        let steps = anim_core::num::round_f64_to_u32(steps_f).max(1);
        let h = if dt > 0.0 { dt / steps_f } else { 0.0 };

        let wind = self.wind_at(cx.time);
        for handle in cx.rig.handles_of(HandleKind::Dangle) {
            let driven = handle.driven_position(pose);
            let mass = self.masses.entry(handle.key.clone()).or_insert_with(|| Mass { position: driven, velocity: Vec2::ZERO, driven });
            if h > 0.0 {
                let anchor_velocity = (driven - mass.driven) / dt;
                for i in 1..=steps {
                    let frac = f64::from(i) / steps_f;
                    let anchor = mass.driven + (driven - mass.driven) * frac;
                    let rel = mass.position - anchor;
                    let accel = -rel * (omega * omega) - (mass.velocity - anchor_velocity) * (2.0 * zeta * omega) + wind;
                    mass.velocity += accel * h;
                    mass.position += mass.velocity * h;
                    let rel = mass.position - anchor;
                    let reach = handle.reach.max(1.0);
                    if rel.hypot() > reach {
                        let n = rel / rel.hypot();
                        mass.position = anchor + n * reach;
                        if self.params.wall_collide {
                            let outward = mass.velocity.dot(n);
                            if outward > 0.0 {
                                mass.velocity -= n * (outward * (1.0 + restitution));
                            }
                        } else {
                            mass.velocity = Vec2::ZERO;
                        }
                    }
                }
            }
            mass.driven = driven;
            let offset = mass.position - driven;
            if offset.hypot() < 1e-4 && mass.velocity.hypot() < 1e-4 {
                mass.position = driven;
                mass.velocity = Vec2::ZERO;
                continue;
            }
            let local = handle.to_group_space(pose, offset);
            if local != Vec2::ZERO {
                pose.drag.insert(handle.key.clone(), pose.drag_of(&handle.key) + local);
            }
        }
        let keys: Vec<String> = cx.rig.handles_of(HandleKind::Dangle).map(|h| h.key.clone()).collect();
        self.masses.retain(|k, _| keys.contains(k));
    }

    fn reset(&mut self, seed: u64) {
        let mut rng = Rng::derived(seed, 5);
        self.phases = (rng.range(0.0, std::f64::consts::TAU), rng.range(0.0, std::f64::consts::TAU));
        self.masses.clear();
    }
}

impl Dynamics {
    fn wind_at(&self, t: f64) -> Vec2 {
        if self.params.wind_strength == 0.0 { Vec2::ZERO } else { self.wind(t) }
    }
}
