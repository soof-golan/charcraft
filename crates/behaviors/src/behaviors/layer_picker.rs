//! Layer Picker (this program's own `Charcraft.LayerPicker.js`): exactly one of a list of layers shows.
//!
//! The layers are, say, the hands in different gestures or the hats in a wardrobe; a key per layer picks one, or one
//! key steps through them.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};

/// Parameters of Layer Picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerPickerParams {
    /// The layers to pick from (names or paths, comma separated).
    pub layers: String,
    /// One key per layer, in the same order (comma separated); a key picks its layer.
    pub keys: String,
    /// A key that steps to the next layer; empty has none.
    #[serde(rename = "nextKey")]
    pub next_key: String,
    /// A key that steps to the previous layer; empty has none.
    #[serde(rename = "previousKey")]
    pub previous_key: String,
    /// The layer shown at the start, counting from 1; 0 shows none until a key is pressed.
    pub initial: f64,
    /// After the last layer, step on to the first (and back from the first to the last).
    pub wrap: bool,
}

impl Default for LayerPickerParams {
    fn default() -> Self {
        Self { layers: String::new(), keys: String::new(), next_key: String::new(), previous_key: String::new(), initial: 1.0, wrap: true }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "layers", "", "The layers to pick from (comma separated).";
    "keys", "", "One key per layer, in order.";
    "nextKey", "", "Key that steps to the next layer.";
    "previousKey", "", "Key that steps to the previous layer.";
    "initial", "", "The layer shown at the start, counting from 1; 0 shows none.";
    "wrap", "", "Step on from the last layer to the first.";
};

fn list(text: &str) -> Vec<&str> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).collect()
}

/// Shows one layer of a list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerPicker {
    /// The parameters.
    pub params: LayerPickerParams,
    /// The chosen layer.
    chosen: Option<usize>,
    /// Whether the first tick has chosen the starting layer.
    started: bool,
    held: Vec<String>,
}

impl LayerPicker {
    /// The index of the layer shown.
    #[must_use]
    pub const fn chosen(&self) -> Option<usize> {
        self.chosen
    }
}

impl Behavior for LayerPicker {
    behavior_common!("Charcraft.LayerPicker.js", "Layer Picker", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let layers = list(&p.layers);
        if layers.is_empty() {
            return;
        }
        let initial = anim_core::num::round_f64_to_u32(p.initial.clamp(0.0, 1000.0)).checked_sub(1).and_then(|i| usize::try_from(i).ok());
        let mut chosen = if self.started { self.chosen } else { initial.filter(|i| *i < layers.len()) };
        let pressed = |held: &[String], key: &str| !key.is_empty() && cx.inputs.key_down(key) && !held.iter().any(|h| h.eq_ignore_ascii_case(key));
        for (i, key) in list(&p.keys).into_iter().enumerate().take(layers.len()) {
            if pressed(&self.held, key) {
                chosen = Some(i);
            }
        }
        let count = layers.len();
        if pressed(&self.held, &p.next_key) {
            chosen = Some(match chosen {
                None => 0,
                Some(i) if i.saturating_add(1) < count => i.saturating_add(1),
                Some(i) => {
                    if p.wrap {
                        0
                    } else {
                        i
                    }
                }
            });
        }
        if pressed(&self.held, &p.previous_key) {
            chosen = Some(match chosen {
                None => count.saturating_sub(1),
                Some(0) => {
                    if p.wrap {
                        count.saturating_sub(1)
                    } else {
                        0
                    }
                }
                Some(i) => i.saturating_sub(1),
            });
        }
        self.held = cx.inputs.keys.iter().cloned().collect();
        self.chosen = chosen;
        self.started = true;
        for (i, name) in layers.iter().enumerate() {
            pose.visibility.insert((*name).to_owned(), Some(i) == chosen);
        }
    }

    fn reset(&mut self, _seed: u64) {
        self.chosen = None;
        self.started = false;
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    fn shown(p: &mut LayerPicker, keys: &[&str]) -> Vec<String> {
        let mut inputs = Inputs::at(Tick::ZERO);
        for k in keys {
            inputs = inputs.with_key(k);
        }
        let mut pose = PuppetPose::neutral();
        p.step(&Cx { inputs: &inputs, rig: &Rig::default(), time: 0.0, dt: 0.0 }, &mut pose);
        pose.visibility.into_iter().filter(|(_, on)| *on).map(|(k, _)| k).collect()
    }

    fn picker(p: LayerPickerParams) -> LayerPicker {
        LayerPicker { params: LayerPickerParams { layers: "Fist, Open, Point".into(), ..p }, ..LayerPicker::default() }
    }

    #[test]
    fn keys_pick_layers_and_only_one_shows() {
        let mut p = picker(LayerPickerParams { keys: "1,2,3".into(), ..LayerPickerParams::default() });
        assert_eq!(shown(&mut p, &[]), ["Fist"]);
        assert_eq!(shown(&mut p, &["3"]), ["Point"]);
        assert_eq!(shown(&mut p, &[]), ["Point"], "the choice stays after the key is let go");
        assert_eq!(shown(&mut p, &["2"]), ["Open"]);
    }

    #[test]
    fn stepping_keys_move_once_per_press_and_wrap() {
        let mut p = picker(LayerPickerParams { next_key: "right".into(), previous_key: "left".into(), ..LayerPickerParams::default() });
        assert_eq!(shown(&mut p, &["right"]), ["Open"]);
        assert_eq!(shown(&mut p, &["right"]), ["Open"], "holding does not repeat");
        shown(&mut p, &[]);
        shown(&mut p, &["right"]);
        shown(&mut p, &[]);
        assert_eq!(shown(&mut p, &["right"]), ["Fist"], "wraps round");
        shown(&mut p, &[]);
        assert_eq!(shown(&mut p, &["left"]), ["Point"]);
        let mut stop = picker(LayerPickerParams { previous_key: "left".into(), wrap: false, ..LayerPickerParams::default() });
        assert_eq!(shown(&mut stop, &["left"]), ["Fist"]);
    }

    #[test]
    fn a_zero_initial_shows_nothing_and_an_empty_list_does_nothing() {
        let mut p = picker(LayerPickerParams { initial: 0.0, ..LayerPickerParams::default() });
        assert!(shown(&mut p, &[]).is_empty());
        let mut none = LayerPicker::default();
        assert!(shown(&mut none, &["1"]).is_empty());
    }
}
