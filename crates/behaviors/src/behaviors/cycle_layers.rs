//! Cycle Layers (`Adobe.CycleLayers.js`): plays the children of groups tagged `Cycle` as an animation.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};

/// Parameters of Cycle Layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CycleLayersParams {
    /// Frames per second.
    #[serde(rename = "framesPerSecond")]
    pub frames_per_second: f64,
    /// What happens at the last layer: `loop`, `ping_pong` or `once`.
    #[serde(rename = "playMode")]
    pub play_mode: String,
    /// When it plays: `always`, `while_key_held` or `after_key_press` (play through once per press).
    pub start: String,
    /// The key for the two key modes.
    pub key: String,
    /// The layer shown when it is not playing: `first` or `last`.
    pub rest: String,
}

impl Default for CycleLayersParams {
    fn default() -> Self {
        Self { frames_per_second: 8.0, play_mode: "loop".to_owned(), start: "always".to_owned(), key: "space".to_owned(), rest: "first".to_owned() }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "framesPerSecond", "1/s", "How many layers are shown each second.";
    "playMode", "", "What happens at the last layer.", ["loop", "ping_pong", "once"];
    "start", "", "When the cycle plays.", ["always", "while_key_held", "after_key_press"];
    "key", "", "The key of the two key modes.";
    "rest", "", "The layer shown while it is not playing.", ["first", "last"];
};

/// The index of the layer shown `step` frames into a cycle of `count`.
fn frame_at(step: u32, count: usize, mode: &str) -> usize {
    let n = u32::try_from(count).unwrap_or(1).max(1);
    let last = n.saturating_sub(1);
    let at = match mode {
        "once" => step.min(last),
        "ping_pong" if n > 1 => {
            let span = last.saturating_mul(2);
            let k = step.checked_rem(span).unwrap_or(0);
            if k <= last { k } else { span.saturating_sub(k) }
        }
        _ => step.checked_rem(n).unwrap_or(0),
    };
    usize::try_from(at).unwrap_or(0)
}

/// Shows the children of each group tagged `Cycle` one after another.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CycleLayers {
    /// The parameters.
    pub params: CycleLayersParams,
    /// Seconds into the cycle.
    played: f64,
    /// A key press is playing the cycle through.
    running: bool,
    was_down: bool,
}

impl Behavior for CycleLayers {
    behavior_common!("Adobe.CycleLayers.js", "Cycle Layers", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        if cx.rig.cycles.is_empty() {
            return;
        }
        let p = &self.params;
        let down = cx.inputs.key_down(&p.key);
        let rate = if p.frames_per_second.is_finite() { p.frames_per_second.max(0.0) } else { 0.0 };
        let longest = cx.rig.cycles.iter().map(|c| c.frames.len()).max().unwrap_or(1);
        let playing = match p.start.as_str() {
            "while_key_held" => down,
            "after_key_press" => {
                if down && !self.was_down {
                    self.running = true;
                    self.played = 0.0;
                }
                let finished = self.played * rate >= f64::from(u32::try_from(longest).unwrap_or(1));
                if finished {
                    self.running = false;
                }
                self.running
            }
            _ => true,
        };
        // A cycle that a key starts shows its first layer on the frame of the press.
        let pressed = down && !self.was_down && p.start != "always";
        self.was_down = down;
        if pressed {
            self.played = 0.0;
        } else if playing {
            self.played += cx.dt;
        } else if p.start == "while_key_held" {
            self.played = 0.0;
        }

        let step = if playing { u32::try_from(anim_core::num::floor_f64_to_i32((self.played * rate).max(0.0))).unwrap_or(0) } else { 0 };
        for group in &cx.rig.cycles {
            let last = group.frames.len().saturating_sub(1);
            let at = if playing || p.start == "always" {
                frame_at(step, group.frames.len(), &p.play_mode)
            } else if p.rest == "last" {
                last
            } else {
                0
            };
            for (i, frame) in group.frames.iter().enumerate() {
                pose.visibility.insert(frame.clone(), i == at);
            }
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_play_mode() {
        let seq = |mode: &str| (0..8).map(|s| frame_at(s, 3, mode)).collect::<Vec<_>>();
        assert_eq!(seq("loop"), [0, 1, 2, 0, 1, 2, 0, 1]);
        assert_eq!(seq("ping_pong"), [0, 1, 2, 1, 0, 1, 2, 1]);
        assert_eq!(seq("once"), [0, 1, 2, 2, 2, 2, 2, 2]);
    }
}
