//! Fader (this program's own `Charcraft.Fader.js`): layers fade in while a key is held and fade out when it is let
//! go, or the other way round.

use std::collections::BTreeMap;

use charcraft_puppet::PuppetPose;
use charcraft_puppet::tags::ids;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};

/// Parameters of Fader.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FaderParams {
    /// More layers to fade besides the ones named `Fader` (names or paths, comma separated).
    pub layers: String,
    /// The key that fades them in; empty keeps them faded in.
    #[serde(rename = "triggerKey")]
    pub trigger_key: String,
    /// Seconds to fade in.
    #[serde(rename = "fadeIn")]
    pub fade_in: f64,
    /// Seconds to fade out.
    #[serde(rename = "fadeOut")]
    pub fade_out: f64,
    /// Start faded out (the layers appear on the key) rather than faded in (they vanish on the key).
    #[serde(rename = "startHidden")]
    pub start_hidden: bool,
    /// The opacity when faded out, percent.
    #[serde(rename = "minOpacity")]
    pub min_opacity: f64,
}

impl Default for FaderParams {
    fn default() -> Self {
        Self { layers: String::new(), trigger_key: "f".to_owned(), fade_in: 0.5, fade_out: 0.5, start_hidden: true, min_opacity: 0.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "layers", "", "Layers to fade (comma separated names or paths).";
    "triggerKey", "", "Key that fades them in (or out, with startHidden off).";
    "fadeIn", "s", "Seconds to fade in.";
    "fadeOut", "s", "Seconds to fade out.";
    "startHidden", "", "The layers appear on the key; off makes them vanish on the key.";
    "minOpacity", "%", "Opacity when faded out.";
};

/// Fades layers in and out with a key.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fader {
    /// The parameters.
    pub params: FaderParams,
    level: Option<f64>,
    last: BTreeMap<String, f64>,
}

impl Fader {
    /// The opacity each faded layer had on the last tick.
    #[must_use]
    pub const fn opacities(&self) -> &BTreeMap<String, f64> {
        &self.last
    }
}

impl Behavior for Fader {
    behavior_common!("Charcraft.Fader.js", "Fader", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let held = !p.trigger_key.is_empty() && cx.inputs.key_down(&p.trigger_key);
        let shown = if p.start_hidden { held } else { !held };
        let seconds = if shown { p.fade_in } else { p.fade_out }.max(0.0);
        // A linear ramp over the fade time; the first tick starts where the key says, without fading.
        let step = if seconds > 1e-9 { cx.dt.max(0.0) / seconds } else { 1.0 };
        let now = self.level.unwrap_or(if shown { 1.0 } else { 0.0 });
        let level = if shown { (now + step).min(1.0) } else { (now - step).max(0.0) };
        self.level = Some(level);
        let floor = (p.min_opacity / 100.0).clamp(0.0, 1.0);
        let opacity = floor + (1.0 - floor) * level;
        let tagged = cx.rig.anchors_with(ids::FADER).map(|a| a.layer.as_str());
        for name in p.layers.split(',').map(str::trim).filter(|s| !s.is_empty()).chain(tagged) {
            if opacity < 1.0 - 1e-9 {
                let slot = pose.opacity.entry(name.to_owned()).or_insert(1.0);
                *slot *= opacity;
            }
            self.last.insert(name.to_owned(), opacity);
        }
    }

    fn reset(&mut self, _seed: u64) {
        self.level = None;
        self.last.clear();
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn opacity_after(f: &mut Fader, frames: u32, key: bool) -> f64 {
        let rig = Rig::default();
        let mut last = 1.0;
        for i in 0..frames {
            let t = f64::from(i) / 60.0;
            let mut inputs = Inputs::at(Tick::from_seconds_f64(t));
            if key {
                inputs = inputs.with_key("f");
            }
            let mut pose = PuppetPose::neutral();
            f.step(&Cx { inputs: &inputs, rig: &rig, time: t, dt: 1.0 / 60.0 }, &mut pose);
            last = pose.opacity.get("fader").copied().unwrap_or(1.0);
        }
        last
    }

    #[test]
    fn layers_fade_in_while_the_key_is_held_and_out_when_it_is_let_go() {
        let mut f = Fader { params: FaderParams { layers: "fader".into(), ..FaderParams::default() }, ..Fader::default() };
        assert_eq!(opacity_after(&mut f, 3, false), 0.0, "starts hidden");
        let half = opacity_after(&mut f, 15, true);
        assert!((half - 0.5).abs() < 0.1, "half way after half the fade time: {half}");
        assert_eq!(opacity_after(&mut f, 60, true), 1.0);
        let fading = opacity_after(&mut f, 15, false);
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert_eq!(opacity_after(&mut f, 60, false), 0.0);
    }

    #[test]
    fn the_inverse_mode_hides_on_the_key_and_a_floor_keeps_a_ghost() {
        let mut f =
            Fader { params: FaderParams { layers: "fader".into(), start_hidden: false, min_opacity: 20.0, ..FaderParams::default() }, ..Fader::default() };
        assert_eq!(opacity_after(&mut f, 3, false), 1.0);
        let gone = opacity_after(&mut f, 120, true);
        assert!((gone - 0.2).abs() < 1e-9, "{gone}");
    }
}
