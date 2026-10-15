//! Small deterministic helpers: a seeded random stream, smoothing filters and pose composition.

use anim_core::Vec2;
use charcraft_puppet::{PuppetPose, Xform};

/// A splitmix64 random stream. The same seed always gives the same numbers, on every platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A stream from a seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// A stream whose seed mixes a base seed with a stream index, so each behavior gets its own numbers.
    #[must_use]
    pub const fn derived(seed: u64, index: u64) -> Self {
        let mut r = Self::new(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17));
        // Warm up so nearby seeds diverge.
        r.next_u64();
        r.next_u64();
        r
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        let top = u32::try_from(self.next_u64() >> 32).unwrap_or(0);
        f64::from(top) / 4_294_967_296.0
    }

    /// A number in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// A one-pole low-pass filter with a time constant in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Smoother {
    value: Option<f64>,
}

impl Smoother {
    /// Moves towards `target` for `dt` seconds; the first call jumps to the target. `tau <= 0` follows exactly.
    pub fn update(&mut self, target: f64, dt: f64, tau: f64) -> f64 {
        let target = if target.is_finite() { target } else { 0.0 };
        let next = match self.value {
            Some(v) if tau > 1e-9 && dt > 0.0 => {
                let alpha = 1.0 - (-dt / tau).exp();
                v + (target - v) * alpha
            }
            Some(v) if tau > 1e-9 => v,
            _ => target,
        };
        self.value = Some(next);
        next
    }

    /// The current value, 0 before the first update.
    #[must_use]
    pub fn get(&self) -> f64 {
        self.value.unwrap_or(0.0)
    }

    /// Forgets the value.
    pub const fn reset(&mut self) {
        self.value = None;
    }
}

/// A two-axis [`Smoother`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Smoother2 {
    x: Smoother,
    y: Smoother,
}

impl Smoother2 {
    /// Moves towards a target.
    pub fn update(&mut self, target: Vec2, dt: f64, tau: f64) -> Vec2 {
        Vec2::new(self.x.update(target.x, dt, tau), self.y.update(target.y, dt, tau))
    }

    /// The current value.
    #[must_use]
    pub fn get(&self) -> Vec2 {
        Vec2::new(self.x.get(), self.y.get())
    }

    /// Forgets the value.
    pub const fn reset(&mut self) {
        self.x.reset();
        self.y.reset();
    }
}

/// The unit vector of a compass angle in degrees on a y-down screen: 0 points up, 90 right, 180 down, 270 left.
#[must_use]
pub fn compass(degrees: f64) -> Vec2 {
    let r = if degrees.is_finite() { degrees.to_radians() } else { 0.0 };
    Vec2::new(r.sin(), -r.cos())
}

/// -1 for the negative key alone, 1 for the positive key alone, 0 for neither or both.
#[must_use]
pub const fn axis(negative: bool, positive: bool) -> f64 {
    match (negative, positive) {
        (true, false) => -1.0,
        (false, true) => 1.0,
        _ => 0.0,
    }
}

/// A finite number, or `fallback`.
#[must_use]
pub const fn finite_or(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v } else { fallback }
}

/// A percentage parameter as a fraction (`75` is `0.75`), 0 when it is not finite.
#[must_use]
pub const fn percent(v: f64) -> f64 {
    finite_or(v, 0.0) / 100.0
}

/// Smooth 0-to-1 ramp.
#[must_use]
pub fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Composes an extra transform onto the group the key names (a tag id, a label or a layer path).
pub fn add_group(pose: &mut PuppetPose, key: &str, extra: &Xform) {
    let slot = pose.groups.entry(key.to_owned()).or_default();
    *slot = slot.then(extra);
}

/// Removes group entries that ended up as the identity, so a neutral input leaves a neutral pose.
pub fn prune_identity(pose: &mut PuppetPose) {
    pose.groups.retain(|_, x| !x.is_identity());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_reproducible_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            let (x, y) = (a.unit(), b.unit());
            assert!((x - y).abs() < f64::EPSILON);
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::derived(1, 0).next_u64(), Rng::derived(1, 1).next_u64());
    }

    #[test]
    fn smoother_converges_without_overshoot() {
        let mut s = Smoother::default();
        assert!((s.update(0.0, 0.01, 0.1)).abs() < 1e-12);
        let mut last = 0.0;
        for _ in 0..200 {
            let v = s.update(1.0, 0.01, 0.1);
            assert!(v >= last && v <= 1.0);
            last = v;
        }
        assert!(last > 0.99);
    }

    #[test]
    fn compass_points_where_people_expect() {
        let down = compass(180.0);
        assert!(down.x.abs() < 1e-12 && (down.y - 1.0).abs() < 1e-12);
        let right = compass(90.0);
        assert!((right.x - 1.0).abs() < 1e-12 && right.y.abs() < 1e-12);
    }
}
