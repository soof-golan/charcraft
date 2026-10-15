//! Breathe (this program's own `Charcraft.Breathe.js`): a slow swell of the layers tagged `Breathe`.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{percent, smoothstep};

/// Parameters of Breathe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BreatheParams {
    /// Breaths per minute.
    pub rate: f64,
    /// How much of the layers' full swell is used, percent.
    pub strength: f64,
    /// The share of each breath spent breathing in, percent.
    #[serde(rename = "inhaleRatio")]
    pub inhale_ratio: f64,
}

impl Default for BreatheParams {
    fn default() -> Self {
        Self { rate: 14.0, strength: 100.0, inhale_ratio: 40.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "rate", "1/min", "Breaths per minute; 0 holds the breath.";
    "strength", "%", "Fraction of the layers' full swell.";
    "inhaleRatio", "%", "Share of each breath spent breathing in.";
};

/// Sets [`PuppetPose::breathe`] from the clock, so it is a pure function of time.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Breathe {
    /// The parameters.
    pub params: BreatheParams,
}

impl Breathe {
    /// The breath at a time, 0 (empty) to 1 (full), before strength.
    #[must_use]
    pub fn level(&self, seconds: f64) -> f64 {
        let rate = self.params.rate;
        if !rate.is_finite() || rate <= 1e-6 || !seconds.is_finite() {
            return 0.0;
        }
        let period = 60.0 / rate;
        let phase = (seconds / period).rem_euclid(1.0);
        let inhale = percent(self.params.inhale_ratio).clamp(0.05, 0.95);
        if phase < inhale { smoothstep(phase / inhale) } else { 1.0 - smoothstep((phase - inhale) / (1.0 - inhale)) }
    }
}

impl Behavior for Breathe {
    behavior_common!("Charcraft.Breathe.js", "Breathe", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let v = self.level(cx.time) * percent(self.params.strength).clamp(0.0, 1.0);
        pose.breathe = (pose.breathe + v).clamp(0.0, 1.0);
    }

    fn reset(&mut self, _seed: u64) {}
}
