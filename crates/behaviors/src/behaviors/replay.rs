//! Replay (this program's own `Charcraft.Replay.js`): hold a key to record a performance, press another to play it back.
//!
//! The performance is what the behaviors before this one made of the puppet while recording: the body and
//! head moves, the face, the mouth and the handles' pulls.

use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::percent;

/// Parameters of Replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayParams {
    /// Hold this key to record; a new recording replaces the old one.
    #[serde(rename = "recordKey")]
    pub record_key: String,
    /// Press this key to play the recording from its start.
    #[serde(rename = "playKey")]
    pub play_key: String,
    /// Start over when the recording ends.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// Playback speed, percent.
    pub speed: f64,
    /// Longest recording, seconds; recording stops there.
    #[serde(rename = "maxSeconds")]
    pub max_seconds: f64,
    /// Replay only these groups (names or tags, comma separated); empty replays everything.
    pub only: String,
}

impl Default for ReplayParams {
    fn default() -> Self {
        Self { record_key: "r".to_owned(), play_key: "p".to_owned(), looping: false, speed: 100.0, max_seconds: 30.0, only: String::new() }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "recordKey", "", "Hold to record a performance.";
    "playKey", "", "Press to play the recording.";
    "loop", "", "Start over when the recording ends.";
    "speed", "%", "Playback speed.";
    "maxSeconds", "s", "Longest recording.";
    "only", "", "Replay only these groups (comma separated); empty replays everything.";
};

/// What Replay is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayState {
    /// Nothing.
    #[default]
    Idle,
    /// Recording.
    Recording,
    /// Playing the recording back.
    Playing,
}

/// The parts of a pose a recording keeps.
fn keep(pose: &PuppetPose, only: &[String]) -> PuppetPose {
    let wanted = |key: &str| {
        only.is_empty() || only.iter().any(|o| o.eq_ignore_ascii_case(key) || charcraft_puppet::tags::normalize(o) == charcraft_puppet::tags::normalize(key))
    };
    let mut out = PuppetPose::neutral();
    if only.is_empty() {
        out.root = pose.root;
        out.head = pose.head;
        out.eyebrow_left = pose.eyebrow_left;
        out.eyebrow_right = pose.eyebrow_right;
        out.gaze = pose.gaze;
        out.pupil_left = pose.pupil_left;
        out.pupil_right = pose.pupil_right;
        out.blink_left = pose.blink_left;
        out.blink_right = pose.blink_right;
        out.viseme = pose.viseme;
        out.breathe = pose.breathe;
        out.swaps.clone_from(&pose.swaps);
    }
    out.groups = pose.groups.iter().filter(|(k, _)| wanted(k)).map(|(k, v)| (k.clone(), *v)).collect();
    out.drag = pose.drag.iter().filter(|(k, _)| only.is_empty() || wanted(k.split('/').next().unwrap_or(k))).map(|(k, v)| (k.clone(), *v)).collect();
    out
}

/// Puts a recorded frame into the live pose, replacing what it covers.
fn put(pose: &mut PuppetPose, frame: &PuppetPose, whole: bool) {
    if whole {
        pose.root = frame.root;
        pose.head = frame.head;
        pose.eyebrow_left = frame.eyebrow_left;
        pose.eyebrow_right = frame.eyebrow_right;
        pose.gaze = frame.gaze;
        pose.pupil_left = frame.pupil_left;
        pose.pupil_right = frame.pupil_right;
        pose.blink_left = frame.blink_left;
        pose.blink_right = frame.blink_right;
        pose.viseme = frame.viseme;
        pose.breathe = frame.breathe;
        pose.swaps.clone_from(&frame.swaps);
        pose.groups.clone_from(&frame.groups);
        pose.drag.clone_from(&frame.drag);
    } else {
        for (k, v) in &frame.groups {
            pose.groups.insert(k.clone(), *v);
        }
        for (k, v) in &frame.drag {
            pose.drag.insert(k.clone(), *v);
        }
    }
}

/// Records and replays performances.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Replay {
    /// The parameters.
    pub params: ReplayParams,
    state: ReplayState,
    frames: Vec<(f64, PuppetPose)>,
    clock: f64,
    record_was_down: bool,
    play_was_down: bool,
}

impl Replay {
    /// What it is doing.
    #[must_use]
    pub const fn state(&self) -> ReplayState {
        self.state
    }

    /// How long the recording is, seconds.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.frames.last().map_or(0.0, |f| f.0)
    }

    /// Number of recorded frames.
    #[must_use]
    pub const fn frames(&self) -> usize {
        self.frames.len()
    }

    fn only(&self) -> Vec<String> {
        self.params.only.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
    }
}

impl Behavior for Replay {
    behavior_common!("Charcraft.Replay.js", "Replay", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let record_down = !p.record_key.is_empty() && cx.inputs.key_down(&p.record_key);
        let play_down = !p.play_key.is_empty() && cx.inputs.key_down(&p.play_key);
        let record_pressed = record_down && !self.record_was_down;
        let play_pressed = play_down && !self.play_was_down;
        self.record_was_down = record_down;
        self.play_was_down = play_down;
        let only = self.only();

        if record_pressed {
            self.frames.clear();
            self.clock = 0.0;
            self.state = ReplayState::Recording;
        } else if self.state == ReplayState::Recording && !record_down {
            self.state = ReplayState::Idle;
        }
        if play_pressed && !self.frames.is_empty() && self.state != ReplayState::Recording {
            self.state = ReplayState::Playing;
            self.clock = 0.0;
        }

        match self.state {
            ReplayState::Idle => {}
            ReplayState::Recording => {
                if record_pressed {
                    self.clock = 0.0;
                } else {
                    self.clock += cx.dt.max(0.0);
                }
                if self.clock > p.max_seconds.max(0.0) {
                    self.state = ReplayState::Idle;
                } else {
                    self.frames.push((self.clock, keep(pose, &only)));
                }
            }
            ReplayState::Playing => {
                self.clock += cx.dt.max(0.0) * percent(p.speed).max(0.0);
                let end = self.duration();
                if self.clock > end {
                    if p.looping && end > 1e-9 {
                        self.clock = self.clock.rem_euclid(end);
                    } else {
                        self.state = ReplayState::Idle;
                        return;
                    }
                }
                let at = self.frames.partition_point(|f| f.0 <= self.clock).saturating_sub(1);
                if let Some((_, frame)) = self.frames.get(at) {
                    put(pose, frame, only.is_empty());
                }
            }
        }
    }

    fn reset(&mut self, _seed: u64) {
        *self = Self { params: self.params.clone(), ..Self::default() };
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;
    use charcraft_puppet::Xform;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::Rig;

    struct Rig2 {
        replay: Replay,
        rig: Rig,
        frame: u32,
    }

    impl Rig2 {
        fn new(params: ReplayParams) -> Self {
            Self { replay: Replay { params, ..Replay::default() }, rig: Rig::default(), frame: 0 }
        }

        /// One frame with the keys held, after an earlier behavior set the left arm to `arm` degrees.
        fn tick(&mut self, keys: &[&str], arm: f64) -> PuppetPose {
            let t = f64::from(self.frame) / 60.0;
            let mut inputs = Inputs::at(Tick::from_seconds_f64(t));
            for k in keys {
                inputs = inputs.with_key(k);
            }
            let mut pose = PuppetPose::neutral();
            pose.groups.insert("left arm".into(), Xform::rotated_degrees(arm));
            pose.groups.insert("right arm".into(), Xform::rotated_degrees(-arm));
            self.replay.step(&Cx { inputs: &inputs, rig: &self.rig, time: t, dt: if self.frame == 0 { 0.0 } else { 1.0 / 60.0 } }, &mut pose);
            self.frame += 1;
            pose
        }
    }

    fn arm(pose: &PuppetPose) -> f64 {
        pose.groups.get("left arm").map_or(f64::NAN, |x| x.rotation.to_degrees())
    }

    #[test]
    fn a_recorded_performance_plays_back_when_asked() {
        let mut r = Rig2::new(ReplayParams::default());
        for i in 0..60 {
            r.tick(&["r"], f64::from(i));
        }
        assert_eq!(r.replay.state(), ReplayState::Recording);
        r.tick(&[], 0.0);
        assert_eq!(r.replay.state(), ReplayState::Idle);
        assert!((r.replay.duration() - 59.0 / 60.0).abs() < 1e-9, "{}", r.replay.duration());
        // Idle: the live value shows.
        assert_eq!(arm(&r.tick(&[], 5.0)), 5.0);
        let first = r.tick(&["p"], 99.0);
        assert_eq!(r.replay.state(), ReplayState::Playing);
        assert!(arm(&first).abs() < 1.5, "playback starts at the first frame, not the live value: {}", arm(&first));
        let mut last = 0.0;
        for _ in 0..30 {
            last = arm(&r.tick(&[], 99.0));
        }
        assert!((last - 30.0).abs() < 3.0, "half way through: {last}");
        for _ in 0..40 {
            r.tick(&[], 99.0);
        }
        assert_eq!(r.replay.state(), ReplayState::Idle, "it stops at the end");
        assert_eq!(arm(&r.tick(&[], 7.0)), 7.0);
    }

    #[test]
    fn looping_and_speed_and_restriction_work() {
        let mut r = Rig2::new(ReplayParams { looping: true, speed: 200.0, only: "left arm".into(), ..ReplayParams::default() });
        for i in 0..60 {
            r.tick(&["r"], f64::from(i));
        }
        r.tick(&[], 0.0);
        r.tick(&["p"], 0.0);
        for _ in 0..100 {
            let pose = r.tick(&[], 0.0);
            assert_eq!(r.replay.state(), ReplayState::Playing, "it keeps looping");
            // Only the left arm is replayed; the right one stays live (0 here).
            assert_eq!(pose.groups.get("right arm").map(|x| x.rotation), Some(Xform::rotated_degrees(-0.0).rotation));
        }
        assert!(r.replay.frames() > 50);
    }

    #[test]
    fn nothing_recorded_means_nothing_to_play() {
        let mut r = Rig2::new(ReplayParams::default());
        r.tick(&["p"], 4.0);
        assert_eq!(r.replay.state(), ReplayState::Idle);
    }

    #[test]
    fn recording_stops_at_the_limit_and_a_new_recording_replaces_the_old() {
        let mut r = Rig2::new(ReplayParams { max_seconds: 0.5, ..ReplayParams::default() });
        for _ in 0..120 {
            r.tick(&["r"], 1.0);
        }
        assert_eq!(r.replay.state(), ReplayState::Idle);
        assert!(r.replay.duration() <= 0.5 + 1e-9);
        r.tick(&[], 0.0);
        r.tick(&["r"], 1.0);
        assert_eq!(r.replay.frames(), 1, "a new recording starts empty");
    }
}
