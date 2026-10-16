//! Motion Trigger (`Adobe.MotionTrigger.js`): shows the layer that matches how the puppet is moving.

use anim_core::Vec2;
use charcraft_puppet::PuppetPose;
use charcraft_puppet::tags::ids;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{Smoother2, percent, show_only};

/// What is watched for movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionSource {
    /// The head group's movement.
    Head,
    /// The whole puppet's movement (walking, the Transform behavior).
    Body,
    /// Both together.
    #[default]
    Both,
}

impl MotionSource {
    fn from_word(word: &str) -> Self {
        match word.trim().to_lowercase().as_str() {
            "head" => Self::Head,
            "body" => Self::Body,
            _ => Self::Both,
        }
    }
}

/// Parameters of Motion Trigger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionTriggerParams {
    /// What to watch: `head`, `body` or `both`.
    pub source: String,
    /// Speed, stage pixels per second, below which the puppet counts as at rest.
    pub threshold: f64,
    /// How long, in milliseconds, the puppet has to stay slow before the at-rest layer shows.
    #[serde(rename = "restDelay")]
    pub rest_delay: f64,
    /// How long, in milliseconds, the in-transition layer shows when the direction changes.
    #[serde(rename = "transitionTime")]
    pub transition_time: f64,
    /// Smoothing of the measured speed, in hundredths of a second.
    #[serde(rename = "smoothingRate")]
    pub smoothing_rate: f64,
}

impl Default for MotionTriggerParams {
    fn default() -> Self {
        Self { source: "both".to_owned(), threshold: 40.0, rest_delay: 150.0, transition_time: 120.0, smoothing_rate: 8.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "source", "", "What is watched for movement.", ["head", "body", "both"];
    "threshold", "px/s", "Speed under which the puppet is at rest.";
    "restDelay", "ms", "How long it has to stay slow before the at-rest layer shows.";
    "transitionTime", "ms", "How long the in-transition layer shows when the direction changes.";
    "smoothingRate", "1/100 s", "Smoothing of the measured speed.";
};

/// The five states a puppet can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Rest,
    Left,
    Right,
    Up,
    Down,
}

impl State {
    const fn tag(self) -> &'static str {
        match self {
            Self::Rest => ids::AT_REST,
            Self::Left => ids::MOVING_LEFT,
            Self::Right => ids::MOVING_RIGHT,
            Self::Up => ids::MOVING_UP,
            Self::Down => ids::MOVING_DOWN,
        }
    }
}

const STATE_TAGS: [&str; 6] = [ids::AT_REST, ids::MOVING_LEFT, ids::MOVING_RIGHT, ids::MOVING_UP, ids::MOVING_DOWN, ids::IN_TRANSITION];

/// Swaps layers tagged `At Rest`, `Moving Left`, `Moving Right`, `Moving Up`, `Moving Down` and `In Transition` by
/// how the pose built so far moves, so it should run after the behaviors that move the puppet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MotionTrigger {
    /// The parameters.
    pub params: MotionTriggerParams,
    previous: Option<Vec2>,
    speed: Smoother2,
    state: State,
    slow_for: f64,
    transition_until: f64,
}

impl Behavior for MotionTrigger {
    behavior_common!("Adobe.MotionTrigger.js", "Motion Trigger", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let all: Vec<&String> = STATE_TAGS.iter().flat_map(|t| cx.rig.paths_of(t)).collect();
        let p = &self.params;
        let position = match MotionSource::from_word(&p.source) {
            MotionSource::Head => pose.head.translation,
            MotionSource::Body => pose.root.translation,
            MotionSource::Both => pose.head.translation + pose.root.translation,
        };
        let velocity = match self.previous {
            Some(before) if cx.dt > 1e-9 => (position - before) / cx.dt,
            _ => Vec2::ZERO,
        };
        self.previous = Some(position);
        let v = self.speed.update(velocity, cx.dt, percent(p.smoothing_rate).max(0.0));
        if all.is_empty() {
            return;
        }

        let threshold = p.threshold.max(0.0);
        let next = if v.hypot() < threshold {
            self.slow_for += cx.dt;
            if self.slow_for * 1000.0 >= p.rest_delay.max(0.0) { State::Rest } else { self.state }
        } else {
            self.slow_for = 0.0;
            if v.x.abs() >= v.y.abs() {
                if v.x < 0.0 { State::Left } else { State::Right }
            } else if v.y < 0.0 {
                State::Up
            } else {
                State::Down
            }
        };
        if next != self.state {
            // Resting to moving (and back) is a start or a stop; moving one way to another is the transition.
            if self.state != State::Rest && next != State::Rest {
                self.transition_until = cx.time + p.transition_time.max(0.0) / 1000.0;
            }
            self.state = next;
        }

        let transition = cx.rig.paths_of(ids::IN_TRANSITION);
        let tag = if cx.time < self.transition_until && !transition.is_empty() {
            ids::IN_TRANSITION
        } else if cx.rig.paths_of(self.state.tag()).is_empty() {
            ids::AT_REST
        } else {
            self.state.tag()
        };
        let chosen = cx.rig.paths_of(tag).to_vec();
        if !chosen.is_empty() {
            show_only(pose, &all, &chosen);
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}
