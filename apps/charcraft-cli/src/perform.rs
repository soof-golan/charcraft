//! A scripted performance: mouth shapes from audio, random blinks and breathing, as a pose for any moment.
//!
//! This is the headless stand-in for Character Animator's Lip Sync, Blinker and Breathe behaviors. It is a pure
//! function of (keys, seed, time), so exports are reproducible.

use anim_core::{Tick, TimeRange};
use anim_lipsync::{VisemeKey, viseme_at};
use charcraft_puppet::{PuppetPose, Viseme};

/// How long one blink lasts.
const BLINK_LENGTH_SECONDS: f64 = 0.2;
/// Shortest and longest pause between blinks.
const BLINK_GAP_MIN_SECONDS: f64 = 2.0;
const BLINK_GAP_SPAN_SECONDS: f64 = 3.5;
/// Seconds per breath.
const BREATH_SECONDS: f64 = 4.0;

/// What to perform.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub seed: u64,
    pub blink: bool,
    pub breathe: bool,
}

/// A performance: everything needed to pose a puppet at a time.
#[derive(Debug, Clone)]
pub struct Performance {
    keys: Vec<VisemeKey>,
    blinks: Vec<TimeRange>,
    breathe: bool,
}

/// A small deterministic generator (xorshift64*), enough for blink timing.
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        // A zero state would stay zero forever.
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// The next value in `0..1`.
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let bits = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        f64::from(u32::try_from(bits).unwrap_or(0)) / 16_777_216.0
    }
}

impl Performance {
    /// Plans blinks over `length` and keeps the mouth keys.
    #[must_use]
    pub fn new(keys: Vec<VisemeKey>, length: Tick, options: Options) -> Self {
        let mut blinks = Vec::new();
        if options.blink {
            let mut rng = Rng::new(options.seed);
            let mut at = Tick::from_seconds_f64(BLINK_GAP_MIN_SECONDS.mul_add(0.5, rng.unit()));
            while at < length && blinks.len() < 100_000 {
                let duration = Tick::from_seconds_f64(BLINK_LENGTH_SECONDS);
                blinks.push(TimeRange::new(at, duration));
                // Now and then a double blink.
                let gap = if rng.unit() < 0.15 { 0.12 } else { BLINK_GAP_SPAN_SECONDS.mul_add(rng.unit(), BLINK_GAP_MIN_SECONDS) };
                at = Tick(at.0.saturating_add(duration.0).saturating_add(Tick::from_seconds_f64(gap).0));
            }
        }
        Self { keys, blinks, breathe: options.breathe }
    }

    /// The pose at time `t` (a puppet's neutral pose with the performance applied).
    #[must_use]
    pub fn pose_at(&self, t: Tick) -> PuppetPose {
        let mut pose = PuppetPose::neutral();
        if !self.keys.is_empty() {
            pose.viseme = Viseme::from_label(viseme_at(&self.keys, t).ca_layer_name());
        }
        let blink = self.blink_at(t);
        pose.blink_left = blink;
        pose.blink_right = blink;
        if self.breathe {
            pose.breathe = 0.5f64.mul_add((std::f64::consts::TAU * t.seconds() / BREATH_SECONDS).sin(), 0.5);
        }
        pose
    }

    /// How closed the eyes are at `t`, 0 to 1: a triangle over each blink, peaking in the middle.
    #[must_use]
    pub fn blink_at(&self, t: Tick) -> f64 {
        // Blinks are in order; the one that can contain `t` is the last one starting at or before it.
        let index = self.blinks.partition_point(|b| b.start <= t);
        let Some(blink) = index.checked_sub(1).and_then(|i| self.blinks.get(i)) else { return 0.0 };
        if !blink.contains(t) || blink.duration.0 <= 0 {
            return 0.0;
        }
        let x = Tick(t.0.saturating_sub(blink.start.0)).seconds() / blink.duration.seconds();
        (1.0 - 2.0f64.mul_add(x, -1.0).abs()).clamp(0.0, 1.0)
    }

    /// The planned blinks.
    #[cfg(test)]
    #[must_use]
    pub fn blinks(&self) -> &[TimeRange] {
        &self.blinks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(seed: u64) -> Options {
        Options { seed, blink: true, breathe: true }
    }

    #[test]
    fn same_seed_same_blinks() {
        let length = Tick::from_seconds_f64(60.0);
        let a = Performance::new(Vec::new(), length, opts(7));
        let b = Performance::new(Vec::new(), length, opts(7));
        let c = Performance::new(Vec::new(), length, opts(8));
        assert_eq!(a.blinks(), b.blinks());
        assert_ne!(a.blinks(), c.blinks());
        assert!(a.blinks().len() > 5);
    }

    #[test]
    fn blinks_peak_in_the_middle_and_open_between() {
        let p = Performance::new(Vec::new(), Tick::from_seconds_f64(30.0), opts(1));
        let first = p.blinks()[0];
        let mid = Tick(first.start.0 + first.duration.0 / 2);
        assert!(p.blink_at(mid) > 0.99);
        assert!(p.blink_at(first.start) < 0.01);
        assert!(p.blink_at(Tick::ZERO) < f64::EPSILON);
        assert!(p.pose_at(mid).blink_left > 0.99);
    }

    #[test]
    fn blinking_can_be_switched_off() {
        let p = Performance::new(Vec::new(), Tick::from_seconds_f64(30.0), Options { seed: 1, blink: false, breathe: false });
        assert_eq!(p.blinks(), []);
        assert!(p.pose_at(Tick::from_seconds_f64(3.0)).is_neutral());
    }

    #[test]
    fn the_mouth_follows_the_keys() {
        let keys = vec![
            VisemeKey { start: Tick::ZERO, viseme: anim_lipsync::Viseme::Neutral },
            VisemeKey { start: Tick::from_seconds_f64(1.0), viseme: anim_lipsync::Viseme::Oh },
        ];
        let p = Performance::new(keys, Tick::from_seconds_f64(3.0), Options { seed: 1, blink: false, breathe: false });
        assert_eq!(p.pose_at(Tick::from_seconds_f64(0.5)).viseme, Some(Viseme::Neutral));
        assert_eq!(p.pose_at(Tick::from_seconds_f64(1.5)).viseme, Some(Viseme::Oh));
    }

    #[test]
    fn every_lip_sync_shape_has_a_puppet_shape() {
        for v in anim_lipsync::Viseme::ALL {
            assert!(Viseme::from_label(v.ca_layer_name()).is_some(), "{v:?}");
        }
    }
}
