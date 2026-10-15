//! Auto Blink (`Adobe.Blinker.js`): blinks at random intervals.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::Rng;

/// Parameters of Auto Blink, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlinkerParams {
    /// How long one blink takes, milliseconds.
    pub length: f64,
    /// How irregular the gaps are, percent: 0 blinks like a metronome, 100 anywhere from none to twice the mean gap.
    pub randomness: f64,
    /// Blinks per minute.
    pub rate: f64,
}

impl Default for BlinkerParams {
    fn default() -> Self {
        Self { length: 80.0, randomness: 100.0, rate: 30.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "length", "ms", "Duration of one blink. The blink layer shows during the middle half of it.";
    "randomness", "%", "0 is a metronome, 100 draws each gap uniformly from 0 to twice the mean gap.";
    "rate", "1/min", "Mean blinks per minute; 0 never blinks.";
};

/// Closes both eyes now and then. Blinks are drawn from a seeded stream, so a given seed blinks at the same times
/// whatever the frame rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Blinker {
    /// The parameters.
    pub params: BlinkerParams,
    rng: Rng,
    next_at: Option<f64>,
    blink_start: Option<f64>,
}

impl Default for Blinker {
    fn default() -> Self {
        Self { params: BlinkerParams::default(), rng: Rng::derived(0, 0), next_at: None, blink_start: None }
    }
}

impl Blinker {
    fn length_s(&self) -> f64 {
        (self.params.length / 1000.0).clamp(0.01, 2.0)
    }

    fn gap(&mut self) -> f64 {
        if !(self.params.rate.is_finite() && self.params.rate > 1e-6) {
            return f64::INFINITY;
        }
        let mean = 60.0 / self.params.rate;
        let r = (self.params.randomness / 100.0).clamp(0.0, 1.0);
        let u = self.rng.unit();
        (mean * (1.0 - r + 2.0 * r * u)).max(self.length_s() * 1.5)
    }

    /// The scheduled start of the next blink, once known (seconds on the stack's clock).
    #[must_use]
    pub const fn next_blink_at(&self) -> Option<f64> {
        self.next_at
    }
}

impl Behavior for Blinker {
    behavior_common!("Adobe.Blinker.js", "Auto Blink", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let len = self.length_s();
        let t = cx.time;
        if self.next_at.is_none() {
            let gap = self.gap();
            self.next_at = Some(t + gap);
        }
        // Several blinks can pass in one long tick; the loop is bounded by the count of scheduled blinks.
        for _ in 0..8 {
            if let Some(s) = self.blink_start {
                if t >= s + len {
                    self.blink_start = None;
                } else {
                    break;
                }
            }
            match self.next_at {
                Some(at) if self.blink_start.is_none() && t >= at => {
                    self.blink_start = Some(at);
                    let gap = self.gap();
                    self.next_at = Some(at + len + gap);
                }
                _ => break,
            }
        }
        if let Some(s) = self.blink_start {
            let p = ((t - s) / len).clamp(0.0, 1.0);
            let closed = 1.0 - (2.0 * p - 1.0).abs();
            pose.blink_left = pose.blink_left.max(closed);
            pose.blink_right = pose.blink_right.max(closed);
        }
    }

    fn reset(&mut self, seed: u64) {
        self.rng = Rng::derived(seed, 1);
        self.next_at = None;
        self.blink_start = None;
    }
}
