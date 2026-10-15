//! Triggers (`Adobe.Triggers.js`): turns key presses into the triggers the artwork reacts to.

use std::collections::{BTreeMap, BTreeSet};

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};

/// How a key drives its trigger.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    /// On while the key is held.
    #[default]
    Hold,
    /// Each press switches it on or off.
    Latch,
    /// A press switches it on for a number of seconds.
    Timed {
        /// How long it stays on.
        seconds: f64,
    },
}

/// A member of a swap set a trigger chooses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapChoice {
    /// The swap set's name.
    pub set: String,
    /// The label of the member to show.
    pub member: String,
}

/// What one key does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerRule {
    /// The key (lower case, as in [`crate::Inputs::keys`]).
    pub key: String,
    /// How the key acts.
    #[serde(default)]
    pub mode: TriggerMode,
    /// A swap set member the key chooses when pressed (the choice stays until another key changes it).
    #[serde(default)]
    pub swap: Option<SwapChoice>,
}

/// Parameters of Triggers. The first three are Character Animator's; `rules` is this program's way to describe
/// what each key does, since the artwork's own triggers live on its layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggersParams {
    /// Volume of audio played by triggers (stored; audio is not played here).
    #[serde(rename = "audioVolume")]
    pub audio_volume: f64,
    /// Most nested replays of a trigger (stored).
    #[serde(rename = "maxReplayDepth")]
    pub max_replay_depth: f64,
    /// Only play trigger audio when nothing else plays (stored).
    #[serde(rename = "onlyPlayAudioWhenStopped")]
    pub only_play_audio_when_stopped: bool,
    /// Special behaviour for particular keys; every other held key simply triggers while held.
    pub rules: Vec<TriggerRule>,
}

impl Default for TriggersParams {
    fn default() -> Self {
        Self { audio_volume: 0.0, max_replay_depth: 4.0, only_play_audio_when_stopped: false, rules: Vec::new() }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "audioVolume", "", "Stored; trigger audio is not played by this crate.";
    "maxReplayDepth", "", "Stored.";
    "onlyPlayAudioWhenStopped", "", "Stored.";
    "rules", "", "List of {key, mode: hold|latch|{timed:{seconds}}, swap: {set, member}}.";
};

/// Fills [`PuppetPose::triggers`] and [`PuppetPose::swaps`] from the keys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Triggers {
    /// The parameters.
    pub params: TriggersParams,
    previous: BTreeSet<String>,
    latched: BTreeSet<String>,
    timed: BTreeMap<String, f64>,
    swaps: BTreeMap<String, String>,
}

impl Behavior for Triggers {
    behavior_common!("Adobe.Triggers.js", "Triggers", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let keys: BTreeSet<String> = cx.inputs.keys.iter().map(|k| k.to_lowercase()).collect();
        for rule in &self.params.rules {
            let key = rule.key.to_lowercase();
            if keys.contains(&key) && !self.previous.contains(&key) {
                match rule.mode {
                    TriggerMode::Hold => {}
                    TriggerMode::Latch => {
                        if !self.latched.remove(&key) {
                            self.latched.insert(key.clone());
                        }
                    }
                    TriggerMode::Timed { seconds } => {
                        self.timed.insert(key.clone(), cx.time + seconds.max(0.0));
                    }
                }
                if let Some(choice) = &rule.swap {
                    self.swaps.insert(choice.set.clone(), choice.member.clone());
                }
            }
        }
        self.timed.retain(|_, until| *until > cx.time);

        let special: BTreeSet<String> = self.params.rules.iter().filter(|r| r.mode != TriggerMode::Hold).map(|r| r.key.to_lowercase()).collect();
        pose.triggers.extend(keys.iter().filter(|k| !special.contains(*k)).cloned());
        pose.triggers.extend(self.latched.iter().cloned());
        pose.triggers.extend(self.timed.keys().cloned());
        for (set, member) in &self.swaps {
            pose.swaps.insert(set.clone(), member.clone());
        }
        self.previous = keys;
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
