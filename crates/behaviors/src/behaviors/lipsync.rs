//! Lip Sync (`Adobe.LipSync2.js`): picks the mouth shape from the voice.
//!
//! A puppet without mouth-shape layers shows no visemes: it only gets the jaw (a layer tagged Flappy Chin drops with
//! the opening of the voice). This is how Character Animator behaves too, not a gap.

use std::collections::BTreeMap;

use anim_lipsync::Viseme as Voice;
use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Viseme, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Smoother, add_group, percent};

/// How far the jaw drops at full opening, as a fraction of the stage's shorter side, at a jaw strength of 100%.
pub const JAW_TRAVEL: f64 = 0.02;

/// Parameters of Lip Sync, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LipSyncParams {
    /// Hold Ah this many milliseconds longer after the voice leaves it.
    #[serde(rename = "Trim_Ah")]
    pub trim_ah: f64,
    /// Hold D this many milliseconds longer.
    #[serde(rename = "Trim_D")]
    pub trim_d: f64,
    /// Hold Ee this many milliseconds longer.
    #[serde(rename = "Trim_Ee")]
    pub trim_ee: f64,
    /// Hold F this many milliseconds longer.
    #[serde(rename = "Trim_F")]
    pub trim_f: f64,
    /// Hold L this many milliseconds longer.
    #[serde(rename = "Trim_L")]
    pub trim_l: f64,
    /// Hold M this many milliseconds longer.
    #[serde(rename = "Trim_M")]
    pub trim_m: f64,
    /// Hold Oh this many milliseconds longer.
    #[serde(rename = "Trim_Oh")]
    pub trim_oh: f64,
    /// Hold R this many milliseconds longer.
    #[serde(rename = "Trim_R")]
    pub trim_r: f64,
    /// Hold S this many milliseconds longer.
    #[serde(rename = "Trim_S")]
    pub trim_s: f64,
    /// Hold Uh this many milliseconds longer.
    #[serde(rename = "Trim_Uh")]
    pub trim_uh: f64,
    /// Hold W-Oo this many milliseconds longer.
    #[serde(rename = "Trim_Woo")]
    pub trim_woo: f64,
    /// After this long in silence the voice stops overriding the mouth, so the Face behavior's mouth shows again.
    #[serde(rename = "faceTrackerSetMouthSilenceDurationThreshInMilliseconds")]
    pub face_tracker_set_mouth_silence_duration_thresh_in_milliseconds: f64,
    /// Jaw strength, percent: how far a layer tagged `Jaw` drops with an open mouth.
    #[serde(rename = "jawFactor")]
    pub jaw_factor: f64,
    /// Kept for compatibility; the in-between mouth shapes are not generated.
    #[serde(rename = "useTransitionMouthShapes")]
    pub use_transition_mouth_shapes: bool,
    /// Where the voice comes from: `microphone`, `scene` (the sound clips of the scene) or `both` (the microphone
    /// when it is on, the scene's sound otherwise).
    #[serde(rename = "audioSource")]
    pub audio_source: String,
}

impl LipSyncParams {
    /// Created from the defaults Character Animator stores (everything 0 except the two below).
    #[must_use]
    pub fn stock() -> Self {
        Self { face_tracker_set_mouth_silence_duration_thresh_in_milliseconds: 500.0, jaw_factor: 100.0, audio_source: "both".to_owned(), ..Self::default() }
    }

    /// The voice mouth shape this behavior listens to.
    fn voice(&self, inputs: &crate::input::Inputs) -> Option<Voice> {
        match self.audio_source.as_str() {
            "microphone" => inputs.viseme,
            "scene" => inputs.scene_viseme,
            _ => inputs.viseme.or(inputs.scene_viseme),
        }
    }

    const fn trim_ms(&self, v: Viseme) -> f64 {
        let ms = match v {
            Viseme::Ah => self.trim_ah,
            Viseme::D => self.trim_d,
            Viseme::Ee => self.trim_ee,
            Viseme::F => self.trim_f,
            Viseme::L => self.trim_l,
            Viseme::M => self.trim_m,
            Viseme::Oh => self.trim_oh,
            Viseme::R => self.trim_r,
            Viseme::S => self.trim_s,
            Viseme::Uh => self.trim_uh,
            Viseme::WOo => self.trim_woo,
            _ => 0.0,
        };
        if ms.is_finite() { ms.max(0.0) } else { 0.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "Trim_Ah", "ms", "Extra hold time of the Ah shape. The other Trim_* parameters do the same for their shapes.";
    "faceTrackerSetMouthSilenceDurationThreshInMilliseconds", "ms", "Silence this long hands the mouth back to the Face behavior.";
    "jawFactor", "%", "Jaw drop strength for a layer tagged Jaw.";
    "useTransitionMouthShapes", "", "Accepted and stored; transition shapes are not generated.";
    "audioSource", "", "Where the voice comes from.", ["microphone", "scene", "both"];
};

/// The puppet's mouth shape for a voice shape.
#[must_use]
pub const fn to_puppet(v: Voice) -> Viseme {
    match v {
        Voice::Neutral => Viseme::Neutral,
        Voice::Ah => Viseme::Ah,
        Voice::D => Viseme::D,
        Voice::Ee => Viseme::Ee,
        Voice::F => Viseme::F,
        Voice::L => Viseme::L,
        Voice::M => Viseme::M,
        Voice::Oh => Viseme::Oh,
        Voice::R => Viseme::R,
        Voice::S => Viseme::S,
        Voice::Uh => Viseme::Uh,
        Voice::WOo => Viseme::WOo,
    }
}

/// How open the jaw is for a mouth shape, 0 to 1.
#[must_use]
pub const fn openness(v: Viseme) -> f64 {
    match v {
        Viseme::Ah | Viseme::Surprised => 1.0,
        Viseme::Oh | Viseme::Uh => 0.7,
        Viseme::D | Viseme::L | Viseme::R | Viseme::WOo => 0.45,
        Viseme::Ee | Viseme::S | Viseme::Smile => 0.25,
        Viseme::F => 0.15,
        Viseme::Neutral | Viseme::M => 0.0,
    }
}

/// Maps the voice's mouth shape onto the puppet, with trims, a silence hand-off and a jaw.
#[derive(Debug, Clone, PartialEq)]
pub struct LipSync {
    /// The parameters.
    pub params: LipSyncParams,
    shown: Viseme,
    shown_since: f64,
    last_wanted_at: BTreeMap<u8, f64>,
    jaw: Smoother,
}

impl Default for LipSync {
    fn default() -> Self {
        Self { params: LipSyncParams::stock(), shown: Viseme::Neutral, shown_since: 0.0, last_wanted_at: BTreeMap::new(), jaw: Smoother::default() }
    }
}

const fn slot(v: Viseme) -> u8 {
    match v {
        Viseme::Neutral => 0,
        Viseme::Ah => 1,
        Viseme::D => 2,
        Viseme::Ee => 3,
        Viseme::F => 4,
        Viseme::L => 5,
        Viseme::M => 6,
        Viseme::Oh => 7,
        Viseme::R => 8,
        Viseme::S => 9,
        Viseme::Uh => 10,
        Viseme::WOo => 11,
        Viseme::Smile => 12,
        Viseme::Surprised => 13,
    }
}

impl Behavior for LipSync {
    behavior_common!("Adobe.LipSync2.js", "Lip Sync", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let t = cx.time;
        let Some(voice) = self.params.voice(cx.inputs) else {
            self.jaw.update(0.0, cx.dt, 0.04);
            return;
        };
        let wanted = to_puppet(voice);
        self.last_wanted_at.insert(slot(wanted), t);
        if wanted != self.shown {
            // The shape on screen may be held a little longer.
            let hold = self.params.trim_ms(self.shown) / 1000.0;
            let held_until = self.last_wanted_at.get(&slot(self.shown)).map_or(f64::NEG_INFINITY, |at| at + hold);
            if t >= held_until || self.shown == Viseme::Neutral {
                self.shown = wanted;
                self.shown_since = t;
            }
        }
        let thresh = self.params.face_tracker_set_mouth_silence_duration_thresh_in_milliseconds.max(0.0) / 1000.0;
        let silent_long_enough = self.shown == Viseme::Neutral && t - self.shown_since >= thresh && cx.inputs.face.is_some();
        if !silent_long_enough {
            pose.viseme = Some(self.shown);
        }
        let jaw = self.jaw.update(openness(pose.viseme.unwrap_or(Viseme::Neutral)), cx.dt, 0.04);
        let drop = jaw * percent(self.params.jaw_factor) * JAW_TRAVEL * cx.rig.unit();
        if cx.rig.has_tag(ids::JAW) && drop > 1e-6 {
            add_group(pose, ids::JAW, &Xform::moved(0.0, drop));
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
