//! Head Turner (`Adobe.HeadTurner.js`): swaps between the frontal, quarter, profile, upward and downward views of
//! the head as it turns.

use charcraft_puppet::PuppetPose;
use charcraft_puppet::tags::ids;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Smoother, percent, show_only};

/// Parameters of Head Turner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeadTurnerParams {
    /// Turn, in degrees, from which the quarter views show.
    #[serde(rename = "quarterAngle")]
    pub quarter_angle: f64,
    /// Turn, in degrees, from which the profile views show.
    #[serde(rename = "profileAngle")]
    pub profile_angle: f64,
    /// Nod, in degrees, from which the upward and downward views show (only while looking nearly straight on).
    #[serde(rename = "pitchAngle")]
    pub pitch_angle: f64,
    /// Dead band around every threshold, degrees: a view is kept until the head is this far past the limit.
    pub hysteresis: f64,
    /// Smoothing of the tracked angles, in hundredths of a second.
    #[serde(rename = "smoothingRate")]
    pub smoothing_rate: f64,
    /// Strength of the tracked turn, percent: 50 needs twice the turn to reach a view.
    pub sensitivity: f64,
    /// Swaps the left and right views (for a camera that does not mirror).
    pub mirror: bool,
    /// Show the upward and downward views.
    #[serde(rename = "useVerticalViews")]
    pub use_vertical_views: bool,
}

impl Default for HeadTurnerParams {
    fn default() -> Self {
        Self {
            quarter_angle: 12.0,
            profile_angle: 32.0,
            pitch_angle: 15.0,
            hysteresis: 4.0,
            smoothing_rate: 5.0,
            sensitivity: 100.0,
            mirror: false,
            use_vertical_views: true,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "quarterAngle", "deg", "Turn from which the three-quarter views show.";
    "profileAngle", "deg", "Turn from which the profile views show.";
    "pitchAngle", "deg", "Nod from which the upward and downward views show.";
    "hysteresis", "deg", "Dead band around each threshold, so a head held near one does not flicker.";
    "smoothingRate", "1/100 s", "Smoothing of the tracked angles.";
    "sensitivity", "%", "How much of the tracked turn counts.";
    "mirror", "", "Swaps the left and right views.";
    "useVerticalViews", "", "Show the upward and downward views.";
};

/// The views, in the order they are tried, with the tags that carry them.
const VIEWS: [&str; 7] = [ids::LEFT_PROFILE, ids::LEFT_QUARTER, ids::FRONT, ids::RIGHT_QUARTER, ids::RIGHT_PROFILE, ids::UPWARD, ids::DOWNWARD];

/// Shows the view of the head that matches where it points.
///
/// The layers carry the tags `Frontal`, `Left Quarter`, `Right Quarter`, `Left Profile`, `Right Profile`, `Upward`
/// and `Downward`. Missing views fall back towards the front: a puppet without quarter views jumps from frontal to
/// profile. A puppet with none of these layers is left alone.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeadTurner {
    /// The parameters.
    pub params: HeadTurnerParams,
    yaw: Smoother,
    pitch: Smoother,
    turn: u8,
    nod: i8,
}

impl HeadTurner {
    /// The turn level (0 frontal, 1 quarter, 2 profile) for a turn of `angle` degrees, given the current level.
    fn level(&self, angle: f64) -> u8 {
        let p = &self.params;
        let half = p.hysteresis.max(0.0) / 2.0;
        let limits = [p.quarter_angle.max(0.0), p.profile_angle.max(p.quarter_angle).max(0.0)];
        let mut level = self.turn;
        while level < 2 && limits.get(usize::from(level)).is_some_and(|l| angle > l + half) {
            level = level.saturating_add(1);
        }
        while level > 0 && limits.get(usize::from(level.saturating_sub(1))).is_some_and(|l| angle < l - half) {
            level = level.saturating_sub(1);
        }
        level
    }

    /// The view tag for the current state, falling back to what the rig has.
    fn view(&self, cx: &Cx<'_>, right: bool) -> &'static str {
        let have = |tag: &str| !cx.rig.paths_of(tag).is_empty();
        if self.nod != 0 && self.turn == 0 {
            let tag = if self.nod > 0 { ids::UPWARD } else { ids::DOWNWARD };
            if have(tag) {
                return tag;
            }
        }
        let (quarter, profile) = if right { (ids::RIGHT_QUARTER, ids::RIGHT_PROFILE) } else { (ids::LEFT_QUARTER, ids::LEFT_PROFILE) };
        match self.turn {
            2 if have(profile) => profile,
            2 | 1 if have(quarter) => quarter,
            // No profile layers: the quarter view is the nearest.
            _ => ids::FRONT,
        }
    }
}

impl Behavior for HeadTurner {
    behavior_common!("Adobe.HeadTurner.js", "Head Turner", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let all: Vec<&String> = VIEWS.iter().flat_map(|t| cx.rig.paths_of(t)).collect();
        if all.is_empty() {
            return;
        }
        let p = &self.params;
        let tau = percent(p.smoothing_rate).max(0.0);
        let face = cx.inputs.face.map(|f| f.sanitized());
        let k = percent(p.sensitivity).max(0.05);
        let yaw = self.yaw.update(face.map_or(0.0, |f| f.head_yaw) * k * if p.mirror { -1.0 } else { 1.0 }, cx.dt, tau);
        let pitch = self.pitch.update(face.map_or(0.0, |f| f.head_pitch) * k, cx.dt, tau);

        self.turn = self.level(yaw.abs());
        let half = p.hysteresis.max(0.0) / 2.0;
        let limit = p.pitch_angle.max(0.0);
        self.nod = if !p.use_vertical_views {
            0
        } else if pitch.abs() > limit + half {
            if pitch > 0.0 { 1 } else { -1 }
        } else if pitch.abs() < limit - half {
            0
        } else {
            self.nod
        };

        let tag = self.view(cx, yaw > 0.0);
        let chosen: Vec<String> = cx.rig.paths_of(tag).to_vec();
        if chosen.is_empty() {
            return;
        }
        show_only(pose, &all, &chosen);
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
