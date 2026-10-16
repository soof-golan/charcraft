//! Particles (this program's own `Charcraft.Particles.js`): layers tagged `Particle` are thrown as copies.
//!
//! A layer is tagged by being named `Particle`, `Particles` or `Emitter`. Its copies are thrown from their own place as many small copies that fly under gravity and wind,
//! shrink or grow, fade, spin, and bounce off the floor, the stage walls and layers tagged `Collide`.

use anim_core::{Point, Vec2};
use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Sprite, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::rig::RigAnchor;
use crate::util::{Rng, compass, percent};

/// Most particles alive at once, whatever the parameters say.
pub const MAX_PARTICLES: usize = 2000;
/// The longest step simulated in one go, seconds: a stall in the frame rate does not blow the particles apart.
const MAX_STEP: f64 = 0.1;

/// Parameters of Particles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParticlesParams {
    /// A key that must be held to emit; empty emits all the time.
    #[serde(rename = "triggerKey")]
    pub trigger_key: String,
    /// Particles born per second while emitting.
    pub rate: f64,
    /// Particles born at once when the trigger key goes down.
    pub burst: f64,
    /// Seconds a particle lives.
    pub lifetime: f64,
    /// Random spread of the lifetime, percent.
    #[serde(rename = "lifetimeVariance")]
    pub lifetime_variance: f64,
    /// Initial speed, pixels per second.
    pub speed: f64,
    /// Random spread of the speed, percent.
    #[serde(rename = "speedVariance")]
    pub speed_variance: f64,
    /// Where particles are thrown, a compass direction in degrees (0 up, 90 right, 180 down).
    pub direction: f64,
    /// The width of the cone they are thrown in, degrees.
    pub spread: f64,
    /// Downward acceleration, pixels per second squared.
    pub gravity: f64,
    /// Acceleration of the wind, pixels per second squared.
    pub wind: f64,
    /// Where the wind blows to, a compass direction in degrees.
    #[serde(rename = "windDirection")]
    pub wind_direction: f64,
    /// Size at birth, percent of the artwork.
    #[serde(rename = "startScale")]
    pub start_scale: f64,
    /// Size at the end of life, percent.
    #[serde(rename = "endScale")]
    pub end_scale: f64,
    /// Random spread of the size, percent.
    #[serde(rename = "scaleVariance")]
    pub scale_variance: f64,
    /// Opacity at birth, percent.
    #[serde(rename = "startOpacity")]
    pub start_opacity: f64,
    /// Opacity at the end of life, percent.
    #[serde(rename = "endOpacity")]
    pub end_opacity: f64,
    /// Spin, degrees per second.
    pub spin: f64,
    /// Random spread of the spin, degrees per second.
    #[serde(rename = "spinVariance")]
    pub spin_variance: f64,
    /// How far from the emitter's place particles may be born, pixels.
    #[serde(rename = "emitterWidth")]
    pub emitter_width: f64,
    /// How much of the emitter's own movement particles take with them, percent.
    #[serde(rename = "inheritMotion")]
    pub inherit_motion: f64,
    /// Most particles alive at once.
    #[serde(rename = "maxParticles")]
    pub max_particles: f64,
    /// Bounce off the floor, the walls and `Collide` layers.
    pub collide: bool,
    /// The floor's height in stage pixels; 0 is the bottom of the stage.
    pub floor: f64,
    /// Bounce off the left and right edges of the stage too.
    pub walls: bool,
    /// How much speed a bounce keeps, percent.
    pub bounce: f64,
    /// How much sideways speed rubs off on the floor, percent.
    pub friction: f64,
    /// Hide the artwork the particles are copies of.
    #[serde(rename = "hideSource")]
    pub hide_source: bool,
}

impl Default for ParticlesParams {
    fn default() -> Self {
        Self {
            trigger_key: String::new(),
            rate: 20.0,
            burst: 0.0,
            lifetime: 2.0,
            lifetime_variance: 25.0,
            speed: 250.0,
            speed_variance: 30.0,
            direction: 0.0,
            spread: 40.0,
            gravity: 400.0,
            wind: 0.0,
            wind_direction: 90.0,
            start_scale: 100.0,
            end_scale: 100.0,
            scale_variance: 20.0,
            start_opacity: 100.0,
            end_opacity: 0.0,
            spin: 0.0,
            spin_variance: 0.0,
            emitter_width: 0.0,
            inherit_motion: 0.0,
            max_particles: 300.0,
            collide: false,
            floor: 0.0,
            walls: false,
            bounce: 50.0,
            friction: 20.0,
            hide_source: true,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "triggerKey", "", "Key that must be held to emit; empty emits all the time.";
    "rate", "1/s", "Particles born per second.";
    "burst", "", "Particles born at once when the trigger key goes down.";
    "lifetime", "s", "How long a particle lives.";
    "lifetimeVariance", "%", "Random spread of the lifetime.";
    "speed", "px/s", "Initial speed.";
    "speedVariance", "%", "Random spread of the speed.";
    "direction", "deg", "Compass direction particles are thrown in (0 up, 90 right).";
    "spread", "deg", "Width of the cone of directions.";
    "gravity", "px/s2", "Downward acceleration.";
    "wind", "px/s2", "Acceleration of the wind.";
    "windDirection", "deg", "Compass direction the wind blows to.";
    "startScale", "%", "Size at birth.";
    "endScale", "%", "Size at the end of life.";
    "scaleVariance", "%", "Random spread of the size.";
    "startOpacity", "%", "Opacity at birth.";
    "endOpacity", "%", "Opacity at the end of life.";
    "spin", "deg/s", "Spin.";
    "spinVariance", "deg/s", "Random spread of the spin.";
    "emitterWidth", "px", "How far from the emitter's place particles may be born.";
    "inheritMotion", "%", "How much of the emitter's movement particles take along.";
    "maxParticles", "", "Most particles alive at once.";
    "collide", "", "Bounce off the floor, the walls and layers tagged Collide.";
    "floor", "px", "Height of the floor; 0 is the bottom of the stage.";
    "walls", "", "Bounce off the sides of the stage too.";
    "bounce", "%", "Speed a bounce keeps.";
    "friction", "%", "Sideways speed that rubs off on the floor.";
    "hideSource", "", "Hide the artwork the particles are copies of.";
};

/// One live particle.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Particle {
    emitter: usize,
    position: Point,
    velocity: Vec2,
    age: f64,
    life: f64,
    angle: f64,
    spin: f64,
    scale: f64,
}

/// Throws copies of the layers tagged `Particle`.
#[derive(Debug, Clone, PartialEq)]
pub struct Particles {
    /// The parameters.
    pub params: ParticlesParams,
    live: Vec<Particle>,
    owed: Vec<f64>,
    last_place: Vec<Option<Point>>,
    key_was_down: bool,
    rng: Rng,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            params: ParticlesParams::default(),
            live: Vec::new(),
            owed: Vec::new(),
            last_place: Vec::new(),
            key_was_down: false,
            rng: Rng::derived(0, 0x9A47),
        }
    }
}

impl Particles {
    /// How many particles are alive.
    #[must_use]
    pub const fn alive(&self) -> usize {
        self.live.len()
    }

    fn cap(&self) -> usize {
        let m = self.params.max_particles;
        if m.is_finite() && m > 0.0 { anim_core::num::round_f64_to_u32(m.min(2000.0)).try_into().unwrap_or(MAX_PARTICLES) } else { 0 }
    }

    fn spawn(&mut self, emitter: usize, place: Point, carried: Vec2) {
        let p = self.params.clone();
        let jitter = |rng: &mut Rng, spread: f64| 1.0 + percent(spread).clamp(0.0, 1.0) * rng.range(-1.0, 1.0);
        let heading = p.direction + self.rng.range(-0.5, 0.5) * p.spread.clamp(0.0, 360.0);
        let speed = (p.speed * jitter(&mut self.rng, p.speed_variance)).max(0.0);
        let width = if p.emitter_width.is_finite() { p.emitter_width.max(0.0) } else { 0.0 };
        let offset = Vec2::new(self.rng.range(-0.5, 0.5) * width, 0.0);
        let scale = (jitter(&mut self.rng, p.scale_variance)).max(0.05);
        let life = (p.lifetime * jitter(&mut self.rng, p.lifetime_variance)).max(0.05);
        let spin = p.spin + self.rng.range(-1.0, 1.0) * p.spin_variance;
        self.live.push(Particle {
            emitter,
            position: place + offset,
            velocity: compass(heading) * speed + carried * percent(p.inherit_motion),
            age: 0.0,
            life,
            angle: 0.0,
            spin,
            scale,
        });
    }
}

/// Pushes a point out of a box it is inside and reflects its velocity along the side it came through.
fn bounce_off_box(position: &mut Point, velocity: &mut Vec2, rect: (f64, f64, f64, f64), keep: f64) {
    let (x0, y0, x1, y1) = rect;
    if position.x <= x0 || position.x >= x1 || position.y <= y0 || position.y >= y1 {
        return;
    }
    let to = [position.x - x0, x1 - position.x, position.y - y0, y1 - position.y];
    let (side, _) = to.iter().enumerate().fold((0, f64::MAX), |best, (i, d)| if *d < best.1 { (i, *d) } else { best });
    match side {
        0 => {
            position.x = x0;
            velocity.x = -velocity.x.abs() * keep;
        }
        1 => {
            position.x = x1;
            velocity.x = velocity.x.abs() * keep;
        }
        2 => {
            position.y = y0;
            velocity.y = -velocity.y.abs() * keep;
        }
        _ => {
            position.y = y1;
            velocity.y = velocity.y.abs() * keep;
        }
    }
}

impl Behavior for Particles {
    behavior_common!("Charcraft.Particles.js", "Particles", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let emitters: Vec<&RigAnchor> = cx.rig.anchors_with(ids::PARTICLE).collect();
        self.owed.resize(emitters.len(), 0.0);
        self.last_place.resize(emitters.len(), None);
        let dt = if cx.dt.is_finite() { cx.dt.clamp(0.0, MAX_STEP) } else { 0.0 };
        let key_down = p.trigger_key.is_empty() || cx.inputs.key_down(&p.trigger_key);
        let pressed = key_down && !self.key_was_down && !p.trigger_key.is_empty();
        self.key_was_down = key_down;

        // Birth.
        let cap = self.cap();
        for (i, anchor) in emitters.iter().enumerate() {
            let place = anchor.driven_position(pose);
            let carried = match self.last_place.get(i).copied().flatten() {
                Some(before) if dt > 1e-9 => (place - before) / dt,
                _ => Vec2::ZERO,
            };
            if let Some(slot) = self.last_place.get_mut(i) {
                *slot = Some(place);
            }
            let mut births = 0_u32;
            if key_down && p.rate.is_finite() && p.rate > 0.0 {
                let owed = self.owed.get(i).copied().unwrap_or(0.0) + p.rate * dt;
                let whole = owed.floor();
                births = anim_core::num::round_f64_to_u32(whole.min(1000.0));
                if let Some(slot) = self.owed.get_mut(i) {
                    *slot = owed - whole;
                }
            }
            if pressed && p.burst.is_finite() && p.burst > 0.0 {
                births = births.saturating_add(anim_core::num::round_f64_to_u32(p.burst.min(1000.0)));
            }
            for _ in 0..births {
                if self.live.len() >= cap {
                    break;
                }
                self.spawn(i, place, carried);
            }
        }

        // Motion.
        let wind = compass(p.wind_direction) * if p.wind.is_finite() { p.wind } else { 0.0 };
        let pull = Vec2::new(0.0, if p.gravity.is_finite() { p.gravity } else { 0.0 }) + wind;
        let keep = percent(p.bounce).clamp(0.0, 1.0);
        let rub = 1.0 - percent(p.friction).clamp(0.0, 1.0);
        let floor = if p.floor.is_finite() && p.floor > 0.0 { p.floor } else { cx.rig.stage.height };
        let walls: Vec<(f64, f64, f64, f64)> = cx
            .rig
            .anchors_with("Adobe.Physics.Collide")
            .filter_map(|a| {
                let b = a.bounds?;
                let d = a.driven_position(pose) - a.position;
                Some((b.x0 + d.x, b.y0 + d.y, b.x1 + d.x, b.y1 + d.y))
            })
            .collect();
        for particle in &mut self.live {
            particle.age += dt;
            particle.velocity += pull * dt;
            particle.position += particle.velocity * dt;
            particle.angle += particle.spin * dt;
            if p.collide {
                if particle.position.y > floor {
                    particle.position.y = floor;
                    particle.velocity.y = -particle.velocity.y.abs() * keep;
                    particle.velocity.x *= rub;
                }
                if p.walls {
                    if particle.position.x < 0.0 {
                        particle.position.x = 0.0;
                        particle.velocity.x = particle.velocity.x.abs() * keep;
                    } else if particle.position.x > cx.rig.stage.width {
                        particle.position.x = cx.rig.stage.width;
                        particle.velocity.x = -particle.velocity.x.abs() * keep;
                    }
                }
                for rect in &walls {
                    bounce_off_box(&mut particle.position, &mut particle.velocity, *rect, keep);
                }
            }
        }
        self.live.retain(|q| q.age < q.life && q.position.x.is_finite() && q.position.y.is_finite());

        // Drawing.
        if p.hide_source {
            for anchor in &emitters {
                pose.visibility.insert(anchor.layer.clone(), false);
            }
        }
        for q in &self.live {
            let Some(anchor) = emitters.get(q.emitter) else { continue };
            let u = (q.age / q.life).clamp(0.0, 1.0);
            let lerp = |a: f64, b: f64| a + (b - a) * u;
            let size = percent(lerp(p.start_scale, p.end_scale)).max(0.0) * q.scale;
            pose.sprites.push(Sprite {
                layer: anchor.layer.clone(),
                xform: Xform { translation: q.position - anchor.position, rotation: q.angle.to_radians(), scale: Vec2::new(size.max(1e-3), size.max(1e-3)) },
                opacity: percent(lerp(p.start_opacity, p.end_opacity)).clamp(0.0, 1.0),
                shade: 0.0,
            });
        }
    }

    fn reset(&mut self, seed: u64) {
        *self = Self { params: self.params.clone(), rng: Rng::derived(seed, 0x9A47), ..Self::default() };
    }
}

#[cfg(test)]
mod tests {
    use anim_core::{Rect, Tick};

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn rig_with_emitter() -> Rig {
        Rig::default().with_anchor(RigAnchor::new(ids::PARTICLE, "Spark", Point::new(500.0, 400.0)))
    }

    fn run(b: &mut Particles, rig: &Rig, seconds: f64, key: Option<&str>) -> PuppetPose {
        let mut pose = PuppetPose::neutral();
        let frames = (seconds * 60.0).round() as u32;
        for f in 0..frames {
            let t = f64::from(f) / 60.0;
            let mut inputs = Inputs::at(Tick::from_seconds_f64(t));
            if let Some(k) = key {
                inputs = inputs.with_key(k);
            }
            pose = PuppetPose::neutral();
            b.step(&Cx { inputs: &inputs, rig, time: t, dt: 1.0 / 60.0 }, &mut pose);
        }
        pose
    }

    #[test]
    fn particles_are_born_at_the_rate_and_die_at_the_end_of_their_life() {
        let rig = rig_with_emitter();
        let mut b =
            Particles { params: ParticlesParams { rate: 30.0, lifetime: 1.0, lifetime_variance: 0.0, ..ParticlesParams::default() }, ..Particles::default() };
        run(&mut b, &rig, 0.5, None);
        assert!((14..=16).contains(&b.alive()), "alive {}", b.alive());
        run(&mut b, &rig, 3.0, None);
        assert!((29..=31).contains(&b.alive()), "steady state is rate x lifetime: {}", b.alive());
        let mut quiet = Particles { params: ParticlesParams { rate: 0.0, ..ParticlesParams::default() }, ..Particles::default() };
        run(&mut quiet, &rig, 1.0, None);
        assert_eq!(quiet.alive(), 0);
    }

    #[test]
    fn they_fly_in_the_chosen_direction_and_fall_under_gravity() {
        let rig = rig_with_emitter();
        let params = ParticlesParams {
            rate: 10.0,
            spread: 0.0,
            speed_variance: 0.0,
            direction: 90.0,
            speed: 300.0,
            gravity: 0.0,
            lifetime: 5.0,
            ..ParticlesParams::default()
        };
        let mut b = Particles { params, ..Particles::default() };
        let pose = run(&mut b, &rig, 0.5, None);
        let first = pose.sprites.first().unwrap();
        assert!(first.xform.translation.x > 100.0 && first.xform.translation.y.abs() < 1e-6, "{:?}", first.xform);
        let mut falling = Particles { params: ParticlesParams { speed: 0.0, spread: 0.0, gravity: 1000.0, ..b.params.clone() }, ..Particles::default() };
        let pose = run(&mut falling, &rig, 0.5, None);
        let oldest = pose.sprites.first().unwrap();
        assert!(oldest.xform.translation.y > 50.0, "{:?}", oldest.xform);
        assert_eq!(pose.visibility.get("Spark"), Some(&false), "the source is hidden");
    }

    #[test]
    fn wind_pushes_sideways() {
        let rig = rig_with_emitter();
        let params = ParticlesParams { speed: 0.0, gravity: 0.0, wind: 500.0, wind_direction: 90.0, rate: 5.0, lifetime: 5.0, ..ParticlesParams::default() };
        let pose = run(&mut Particles { params, ..Particles::default() }, &rig, 1.0, None);
        assert!(pose.sprites[0].xform.translation.x > 100.0);
    }

    #[test]
    fn the_floor_bounces_particles_with_the_restitution() {
        let rig = rig_with_emitter();
        let params = ParticlesParams {
            collide: true,
            floor: 600.0,
            bounce: 50.0,
            gravity: 2000.0,
            speed: 0.0,
            spread: 0.0,
            rate: 1.0,
            lifetime: 10.0,
            lifetime_variance: 0.0,
            ..ParticlesParams::default()
        };
        let mut b = Particles { params, ..Particles::default() };
        let mut lowest = 0.0_f64;
        for s in 0..180 {
            let pose = run(&mut b, &rig, 1.0 / 60.0 + f64::from(s) * 0.0, None);
            for sp in &pose.sprites {
                lowest = lowest.max(sp.xform.translation.y + 400.0);
            }
        }
        assert!(lowest <= 600.0 + 1e-6, "nothing falls through the floor: {lowest}");
        assert!(lowest > 590.0, "something reached it: {lowest}");
    }

    #[test]
    fn collide_layers_stop_particles() {
        let mut anchor = RigAnchor::new("Adobe.Physics.Collide", "Shelf", Point::new(500.0, 500.0));
        anchor.bounds = Some(Rect::new(400.0, 450.0, 600.0, 520.0));
        let rig = rig_with_emitter().with_anchor(anchor);
        let params = ParticlesParams {
            collide: true,
            floor: 5000.0,
            gravity: 1500.0,
            speed: 0.0,
            spread: 0.0,
            rate: 4.0,
            lifetime: 3.0,
            lifetime_variance: 0.0,
            bounce: 0.0,
            ..ParticlesParams::default()
        };
        let mut b = Particles { params, ..Particles::default() };
        let pose = run(&mut b, &rig, 2.0, None);
        assert!(!pose.sprites.is_empty());
        for sp in &pose.sprites {
            let y = 400.0 + sp.xform.translation.y;
            assert!(y <= 450.0 + 1e-6, "rests on the shelf's top: {y}");
        }
    }

    #[test]
    fn the_trigger_key_gates_emission_and_a_burst_fires_on_press() {
        let rig = rig_with_emitter();
        let params = ParticlesParams { trigger_key: "b".into(), rate: 0.0, burst: 25.0, ..ParticlesParams::default() };
        let mut b = Particles { params, ..Particles::default() };
        run(&mut b, &rig, 0.2, None);
        assert_eq!(b.alive(), 0);
        run(&mut b, &rig, 0.1, Some("b"));
        assert_eq!(b.alive(), 25, "one burst, not one per frame");
    }

    #[test]
    fn the_cap_holds_and_the_same_seed_gives_the_same_particles() {
        let rig = rig_with_emitter();
        let params = ParticlesParams { rate: 5000.0, max_particles: 50.0, lifetime: 10.0, ..ParticlesParams::default() };
        let mut a = Particles { params: params.clone(), ..Particles::default() };
        a.reset(3);
        let mut b = Particles { params, ..Particles::default() };
        b.reset(3);
        let (pa, pb) = (run(&mut a, &rig, 0.5, None), run(&mut b, &rig, 0.5, None));
        assert_eq!(a.alive(), 50);
        assert_eq!(pa, pb);
    }

    #[test]
    fn no_emitter_layer_means_nothing_happens() {
        let mut b = Particles::default();
        assert!(run(&mut b, &Rig::default(), 1.0, None).is_neutral());
    }
}
