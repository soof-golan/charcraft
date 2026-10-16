//! Playback: evaluate a scene at a frame into one [`PuppetPose`] per puppet track.
//!
//! Each track owns a [`BehaviorStack`] (the puppet's own, with the track's edits and a seed derived from the
//! scene's). Per frame the takes of the track are layered over empty inputs ([`resolve`]) and the stack steps
//! once. The stacks are stateful (smoothing, blink schedules, springs), so a frame's pose depends on every frame
//! before it. To make that deterministic *and* scrubbable the evaluator always steps whole frames from the
//! start, and keeps a snapshot of each stack every [`CHECKPOINT_EVERY`] frames: a jump to any frame restarts from
//! the nearest earlier snapshot and gives exactly the pose a straight run from frame 0 gives.

use std::collections::BTreeMap;

use charcraft_behaviors::api::apply;
use charcraft_behaviors::{BehaviorStack, Inputs, Rig};
use charcraft_puppet::PuppetPose;

use crate::error::{Error, Result};
use crate::scene::{PuppetTrack, Scene, behavior_target};
use crate::take::resolve;

/// A lane value as the JSON a behavior parameter takes: whole numbers as integers (integer parameters refuse
/// `60.0`), the rest as floats.
#[must_use]
pub fn param_json(v: f64) -> serde_json::Value {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1.0e12 { serde_json::json!(anim_core::num::round_f64_to_i64(v)) } else { serde_json::json!(v) }
}

/// Sets the behavior parameters that the track keyframes to their values at scene time `t`. A lane that names a
/// missing slot or parameter is skipped (the commands refuse such lanes, so only a stale file has them).
fn apply_lanes(stack: &mut BehaviorStack, track: &PuppetTrack, t: anim_core::Tick) {
    for lane in &track.lanes {
        let (Some((index, name)), Some(value)) = (behavior_target(&lane.target), lane.value_at(t)) else { continue };
        if let Some(slot) = stack.slot_mut(index) {
            let _ = charcraft_behaviors::behavior::set_param(slot.behavior.as_mut(), name, param_json(value));
        }
    }
}

/// Frames between stack snapshots.
pub const CHECKPOINT_EVERY: i64 = 60;
/// Most snapshots kept per track.
pub const MAX_CHECKPOINTS: usize = 4096;

/// The behavior stacks puppets start from, by cast key (see [`Cast::templates`](crate::Cast::templates)).
pub type Templates = BTreeMap<String, BehaviorStack>;

/// The stack a track starts from.
///
/// That is its puppet's template, or the standard stack of every behavior when the puppet is not known (a scene
/// being edited without its cast), with the scene's seed for this track and the track's own edits applied.
///
/// # Errors
/// [`Error::Behaviors`] when one of the track's edits does not apply.
pub fn track_stack(scene: &Scene, templates: &Templates, index: usize, track: &PuppetTrack) -> Result<BehaviorStack> {
    let base = templates.get(&track.puppet).cloned().unwrap_or_else(|| BehaviorStack::standard(Rig::default()));
    let seed = scene.seed.wrapping_add(u64::try_from(index).unwrap_or(0));
    let mut stack = base.with_seed(seed);
    for op in &track.behaviors {
        apply(&mut stack, op)?;
    }
    // Edits may have reseeded the stack on purpose; otherwise make sure it starts clean.
    stack.reset();
    Ok(stack)
}

/// The inputs a track's behaviors see at frame `frame`: its enabled takes layered over nothing.
#[must_use]
pub fn track_inputs(scene: &Scene, track: &PuppetTrack, frame: i64) -> Inputs {
    resolve(&track.takes, Inputs::at(scene.frame_time(frame)))
}

#[derive(Debug, Clone)]
struct Run {
    pristine: BehaviorStack,
    stack: BehaviorStack,
    /// The next frame the stack will step.
    next: i64,
    last: Option<(i64, PuppetPose)>,
    checkpoints: BTreeMap<i64, BehaviorStack>,
}

impl Run {
    fn new(stack: BehaviorStack) -> Self {
        Self { pristine: stack.clone(), stack, next: 0, last: None, checkpoints: BTreeMap::new() }
    }

    fn pose(&mut self, scene: &Scene, track: &PuppetTrack, frame: i64) -> PuppetPose {
        let frame = frame.max(0);
        if let Some((at, pose)) = &self.last
            && *at == frame
        {
            return pose.clone();
        }
        if frame < self.next {
            // Going back: restart from the nearest snapshot strictly before the frame.
            if let Some((at, stack)) = self.checkpoints.range(..frame).next_back() {
                self.stack = stack.clone();
                self.next = at.saturating_add(1);
            } else {
                self.stack = self.pristine.clone();
                self.next = 0;
            }
        }
        let mut pose = PuppetPose::neutral();
        while self.next <= frame {
            let step = self.next;
            apply_lanes(&mut self.stack, track, scene.frame_time(step));
            pose = self.stack.step(&track_inputs(scene, track, step));
            if step > 0 && step % CHECKPOINT_EVERY == 0 && self.checkpoints.len() < MAX_CHECKPOINTS {
                self.checkpoints.entry(step).or_insert_with(|| self.stack.clone());
            }
            self.next = step.saturating_add(1);
        }
        self.last = Some((frame, pose.clone()));
        pose
    }
}

/// Plays a scene: poses for every track at any frame, deterministically.
#[derive(Debug, Clone)]
pub struct Evaluator {
    scene: Scene,
    runs: Vec<Run>,
}

impl Evaluator {
    /// Prepares the stacks of every track. The evaluator keeps its own copy of the scene.
    ///
    /// # Errors
    /// [`Error::Invalid`] for a scene [`Scene::sanitize`] would reject; [`Error::Behaviors`] for a bad edit.
    pub fn new(scene: &Scene, templates: &Templates) -> Result<Self> {
        let mut scene = scene.clone();
        scene.sanitize()?;
        let runs = scene.tracks.iter().enumerate().map(|(i, t)| track_stack(&scene, templates, i, t).map(Run::new)).collect::<Result<Vec<_>>>()?;
        Ok(Self { scene, runs })
    }

    /// The scene being played.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The pose of puppet track `track` at `frame`.
    ///
    /// # Errors
    /// [`Error::NoTrack`].
    pub fn pose(&mut self, track: usize, frame: i64) -> Result<PuppetPose> {
        let def = self.scene.tracks.get(track).ok_or(Error::NoTrack(track))?;
        let run = self.runs.get_mut(track).ok_or(Error::NoTrack(track))?;
        Ok(run.pose(&self.scene, def, frame))
    }

    /// The poses of every track at `frame`, in track order. Disabled tracks give the neutral pose (and do not
    /// advance).
    pub fn poses(&mut self, frame: i64) -> Vec<PuppetPose> {
        let Self { scene, runs } = self;
        scene.tracks.iter().zip(runs.iter_mut()).map(|(def, run)| if def.enabled { run.pose(scene, def, frame) } else { PuppetPose::neutral() }).collect()
    }

    /// Poses of every track for frames `0..frames`, stepping straight through (what exporters use).
    pub fn bake(&mut self, frames: i64) -> Vec<Vec<PuppetPose>> {
        (0..frames.max(0)).map(|f| self.poses(f)).collect()
    }
}

#[cfg(test)]
mod tests {
    use anim_core::{FrameRate, Size, Tick};
    use charcraft_behaviors::FaceInput;

    use super::*;
    use crate::group::{Frame, ParamGroup};
    use crate::take::{Sample, Take};

    fn scene_with_head_take(roll: f64) -> Scene {
        let mut scene = Scene::new("t", Size::new(640.0, 360.0), FrameRate::FPS_30, Tick::from_seconds_f64(10.0));
        scene.add_track(PuppetTrack::new("Hero", "p")).unwrap();
        let mut take = Take::new(0, [ParamGroup::Head].into_iter().collect(), Tick::ZERO, Tick::from_seconds_f64(10.0));
        take.samples.push(Sample { t: Tick::ZERO, frame: Frame { face: Some(FaceInput { head_roll: roll, ..FaceInput::default() }), ..Frame::default() } });
        scene.add_take(0, take).unwrap();
        scene
    }

    #[test]
    fn a_recorded_head_roll_moves_the_head() {
        let scene = scene_with_head_take(20.0);
        let mut ev = Evaluator::new(&scene, &Templates::new()).unwrap();
        let settled = ev.pose(0, 60).unwrap();
        assert!(settled.head.rotation.abs() > 0.05, "head rotation {}", settled.head.rotation);
        let none = Scene { tracks: vec![PuppetTrack::new("Hero", "p")], ..scene };
        let mut ev = Evaluator::new(&none, &Templates::new()).unwrap();
        assert!(ev.pose(0, 60).unwrap().head.rotation.abs() < 1e-9);
    }

    #[test]
    fn scrubbing_gives_the_same_poses_as_playing_straight_through() {
        let scene = scene_with_head_take(15.0);
        let mut straight = Evaluator::new(&scene, &Templates::new()).unwrap();
        let reference = straight.bake(300);
        let mut scrub = Evaluator::new(&scene, &Templates::new()).unwrap();
        for frame in [250, 3, 130, 130, 61, 299, 0, 120, 121, 60, 59, 200] {
            let pose = scrub.pose(0, frame).unwrap();
            let frame_index = usize::try_from(frame).unwrap();
            assert_eq!(pose, reference[frame_index][0], "frame {frame}");
        }
    }

    #[test]
    fn two_evaluators_agree_and_the_seed_matters() {
        let scene = scene_with_head_take(5.0);
        let a = Evaluator::new(&scene, &Templates::new()).unwrap().bake(200);
        let b = Evaluator::new(&scene, &Templates::new()).unwrap().bake(200);
        assert_eq!(a, b);
        let mut other = scene;
        other.seed = 99;
        let c = Evaluator::new(&other, &Templates::new()).unwrap().bake(200);
        assert_ne!(a, c, "different seeds blink at different times");
    }

    #[test]
    fn track_edits_reach_the_stack() {
        let mut scene = scene_with_head_take(20.0);
        scene.tracks[0].behaviors.push(charcraft_behaviors::api::StackOp::SetEnabled { index: 0, enabled: false });
        let mut ev = Evaluator::new(&scene, &Templates::new()).unwrap();
        assert!(ev.pose(0, 60).unwrap().head.rotation.abs() < 1e-9, "with the Face behavior off the head stays");
        scene.tracks[0].behaviors.push(charcraft_behaviors::api::StackOp::Remove { index: 99 });
        assert!(Evaluator::new(&scene, &Templates::new()).is_err());
    }

    #[test]
    fn disabled_tracks_give_neutral_poses() {
        let mut scene = scene_with_head_take(20.0);
        scene.tracks[0].enabled = false;
        let mut ev = Evaluator::new(&scene, &Templates::new()).unwrap();
        assert!(ev.poses(60)[0].is_neutral());
        assert!(matches!(ev.pose(3, 0), Err(Error::NoTrack(3))));
    }
}
