//! The document the editor and the tools share: a scene plus the recording settings and the behavior stacks.
//!
//! Everything in it is cheap to clone (no pictures, no sound), so it can be the snapshot type of an
//! [`anim_session::Session`] and every edit is one undoable step.

use anim_core::Tick;
use anim_session::SizeHint;
use serde::{Deserialize, Serialize};

use crate::audio::MediaStore;
use crate::error::{Error, Result};
use crate::playback::{Evaluator, Templates};
use crate::recorder::{Arming, Recorder, Recording};
use crate::scene::{AudioClip, Scene};

/// The scene being edited and how to record into it.
#[derive(Debug, Clone)]
pub struct RecordDoc {
    /// The scene.
    pub scene: Scene,
    /// What the next recording captures.
    pub arming: Arming,
    /// The behavior stacks of the puppets, by cast key. Set it from [`Cast::templates`](crate::Cast::templates)
    /// so tools see the poses the window shows; without it every puppet plays the standard behaviors.
    pub templates: Templates,
    /// The other scenes of the project, in order, without the open one (which is [`Self::scene`]).
    parked: Vec<Scene>,
    /// Where the open scene sits among all scenes.
    active: usize,
}

/// Most scenes a project holds.
pub const MAX_SCENES: usize = 256;

/// Where a finished recording landed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Committed {
    /// The index of the new take on its track.
    pub take: Option<usize>,
    /// The media name and clip index of recorded sound.
    pub clip: Option<(String, usize)>,
}

impl RecordDoc {
    /// A document around a scene with nothing armed.
    #[must_use]
    pub fn new(scene: Scene) -> Self {
        Self { scene, arming: Arming::default(), templates: Templates::new(), parked: Vec::new(), active: 0 }
    }

    /// A document of several scenes with scene `active` open.
    ///
    /// # Errors
    /// [`Error::Invalid`] without scenes or for an `active` past the end; [`Error::TooLarge`].
    pub fn with_scenes(mut scenes: Vec<Scene>, active: usize) -> Result<Self> {
        if scenes.is_empty() || active >= scenes.len() {
            return Err(Error::invalid("a project needs at least one scene and the open scene must be one of them"));
        }
        if scenes.len() > MAX_SCENES {
            return Err(Error::TooLarge { what: "the scene list", limit: MAX_SCENES });
        }
        let scene = scenes.remove(active);
        let mut doc = Self::new(scene);
        doc.parked = scenes;
        doc.active = active;
        Ok(doc)
    }

    /// Number of scenes.
    #[must_use]
    pub const fn scene_count(&self) -> usize {
        self.parked.len().saturating_add(1)
    }

    /// Index of the open scene.
    #[must_use]
    pub const fn active_scene(&self) -> usize {
        self.active
    }

    /// Scene `index` (the open one included).
    #[must_use]
    pub fn scene_at(&self, index: usize) -> Option<&Scene> {
        match index.cmp(&self.active) {
            std::cmp::Ordering::Less => self.parked.get(index),
            std::cmp::Ordering::Equal => Some(&self.scene),
            std::cmp::Ordering::Greater => self.parked.get(index.saturating_sub(1)),
        }
    }

    /// All scenes in order.
    pub fn scenes(&self) -> impl Iterator<Item = &Scene> {
        (0..self.scene_count()).filter_map(|i| self.scene_at(i))
    }

    /// All scenes in order, taken out of the document (the open scene included).
    fn explode(&mut self) -> Vec<Scene> {
        let mut all = std::mem::take(&mut self.parked);
        let open = std::mem::replace(&mut self.scene, Scene::new("", anim_core::Size::new(1.0, 1.0), anim_core::FrameRate::FPS_24, Tick::ZERO));
        all.insert(self.active.min(all.len()), open);
        all
    }

    /// Puts `all` back, opening scene `active`.
    fn implode(&mut self, mut all: Vec<Scene>, active: usize) {
        let active = active.min(all.len().saturating_sub(1));
        self.scene = all.remove(active);
        self.parked = all;
        self.active = active;
    }

    /// Opens scene `index`. Recording settings stay as they are.
    ///
    /// # Errors
    /// [`Error::Invalid`] for a scene that does not exist.
    pub fn select_scene(&mut self, index: usize) -> Result<()> {
        if index >= self.scene_count() {
            return Err(Error::invalid(format!("no scene {index}")));
        }
        let all = self.explode();
        self.implode(all, index);
        Ok(())
    }

    /// Adds a scene at `index` (the end when `None`) without opening it. Returns its index.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn add_scene(&mut self, scene: Scene, index: Option<usize>) -> Result<usize> {
        if self.scene_count() >= MAX_SCENES {
            return Err(Error::TooLarge { what: "the scene list", limit: MAX_SCENES });
        }
        let open = self.active;
        let mut all = self.explode();
        let at = index.unwrap_or(all.len()).min(all.len());
        all.insert(at, scene);
        let open = if at <= open { open.saturating_add(1) } else { open };
        self.implode(all, open);
        Ok(at)
    }

    /// Removes scene `index`. The open scene stays open unless it is the one removed (then its neighbour opens).
    ///
    /// # Errors
    /// [`Error::Invalid`] for the only scene or a scene that does not exist.
    pub fn remove_scene(&mut self, index: usize) -> Result<Scene> {
        if self.scene_count() <= 1 {
            return Err(Error::invalid("a project needs at least one scene"));
        }
        if index >= self.scene_count() {
            return Err(Error::invalid(format!("no scene {index}")));
        }
        let open = self.active;
        let mut all = self.explode();
        let removed = all.remove(index);
        let open = if index < open { open.saturating_sub(1) } else { open };
        self.implode(all, open);
        Ok(removed)
    }

    /// Moves scene `from` to position `to`; the open scene stays open.
    ///
    /// # Errors
    /// [`Error::Invalid`] for a scene that does not exist.
    pub fn move_scene(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.scene_count() || to >= self.scene_count() {
            return Err(Error::invalid("no such scene"));
        }
        let open = self.active;
        let mut all = self.explode();
        let moved = all.remove(from);
        all.insert(to, moved);
        let open = if open == from {
            to
        } else if from < to && open > from && open <= to {
            open.saturating_sub(1)
        } else if to < from && open >= to && open < from {
            open.saturating_add(1)
        } else {
            open
        };
        self.implode(all, open);
        Ok(())
    }

    /// A recorder with the document's arming.
    #[must_use]
    pub fn recorder(&self) -> Recorder {
        let mut recorder = Recorder::new();
        recorder.arm_only(self.arming.groups.iter().copied());
        recorder.set_audio_armed(self.arming.audio);
        recorder.set_punch(self.arming.punch);
        recorder
    }

    /// Puts a finished recording on puppet track `track`: the input take on top of the track's takes, recorded
    /// sound as a clip on audio track `audio_track` (media name from `media`).
    ///
    /// # Errors
    /// [`Error::NoTrack`] and the limits of [`Scene::add_take`], [`Scene::add_clip`] and [`MediaStore::insert`].
    pub fn commit(&mut self, track: usize, recording: Recording, audio_track: usize, media: &mut MediaStore) -> Result<Committed> {
        // Check before changing anything.
        self.scene.track(track)?;
        let mut done = Committed { take: None, clip: None };
        if let Some(take) = recording.take {
            done.take = Some(self.scene.add_take(track, take)?);
        }
        if let Some(audio) = recording.audio {
            let name = media.unused_name(&format!("take {}", self.scene.take_counter.saturating_sub(1)));
            let length = audio.buffer.duration();
            media.insert(&name, audio.buffer)?;
            let index = self.scene.add_clip(audio_track, AudioClip::new(&name, &name, audio.start))?;
            let end = Tick(audio.start.0.saturating_add(length.0));
            if end.0 > self.scene.duration.0 {
                self.scene.duration = end;
            }
            done.clip = Some((name, index));
        }
        Ok(done)
    }

    /// An evaluator for the document's scene.
    ///
    /// # Errors
    /// Whatever [`Evaluator::new`] reports.
    pub fn evaluator(&self) -> Result<Evaluator> {
        Evaluator::new(&self.scene, &self.templates)
    }

    /// A track index checked against the scene.
    ///
    /// # Errors
    /// [`Error::NoTrack`].
    pub fn check_track(&self, track: usize) -> Result<usize> {
        self.scene.track(track).map(|_| track).map_err(|_| Error::NoTrack(track))
    }
}

impl SizeHint for RecordDoc {
    fn size_hint(&self) -> usize {
        self.parked.iter().map(Scene::size_hint).fold(self.scene.size_hint(), usize::saturating_add)
    }
}

#[cfg(test)]
mod tests {
    use anim_audio::PcmBuffer;
    use anim_core::{FrameRate, Size};
    use charcraft_behaviors::{FaceInput, Inputs};

    use super::*;
    use crate::group::ParamGroup;
    use crate::recorder::RecordedAudio;
    use crate::scene::PuppetTrack;

    fn doc() -> RecordDoc {
        let mut scene = Scene::new("s", Size::new(320.0, 180.0), FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
        scene.add_track(PuppetTrack::new("Hero", "p")).unwrap();
        RecordDoc::new(scene)
    }

    fn named(name: &str) -> Scene {
        Scene::new(name, Size::new(320.0, 180.0), FrameRate::FPS_30, Tick::from_seconds_f64(1.0))
    }

    fn order(d: &RecordDoc) -> Vec<String> {
        d.scenes().map(|s| s.name.clone()).collect()
    }

    #[test]
    fn scenes_open_add_move_and_remove_keeping_their_order() {
        let mut d = RecordDoc::with_scenes(vec![named("a"), named("b"), named("c")], 1).unwrap();
        assert_eq!((d.scene.name.as_str(), d.active_scene(), d.scene_count()), ("b", 1, 3));
        assert_eq!(order(&d), ["a", "b", "c"]);
        d.select_scene(2).unwrap();
        assert_eq!((d.scene.name.as_str(), d.active_scene()), ("c", 2));
        assert_eq!(order(&d), ["a", "b", "c"]);
        d.scene.name = "c2".into();
        d.select_scene(0).unwrap();
        assert_eq!(order(&d), ["a", "b", "c2"], "edits to the open scene stay with it");
        assert!(d.select_scene(3).is_err());
        assert_eq!(d.add_scene(named("x"), Some(0)).unwrap(), 0);
        assert_eq!(order(&d), ["x", "a", "b", "c2"]);
        assert_eq!(d.scene.name, "a", "adding does not change the open scene");
        d.move_scene(0, 3).unwrap();
        assert_eq!(order(&d), ["a", "b", "c2", "x"]);
        assert_eq!(d.scene.name, "a");
        d.remove_scene(1).unwrap();
        assert_eq!((order(&d), d.scene.name.as_str(), d.active_scene()), (vec!["a".to_owned(), "c2".into(), "x".into()], "a", 0));
        d.remove_scene(0).unwrap();
        assert_eq!(d.scene.name, "c2", "the neighbour opens");
        d.remove_scene(0).unwrap();
        assert!(d.remove_scene(0).is_err(), "the last scene stays");
        assert!(RecordDoc::with_scenes(Vec::new(), 0).is_err() && RecordDoc::with_scenes(vec![named("a")], 1).is_err());
    }

    #[test]
    fn a_recording_lands_on_the_track_and_in_the_media_store() {
        let mut d = doc();
        d.arming.groups.insert(ParamGroup::Head);
        d.arming.audio = true;
        let mut recorder = d.recorder();
        let inputs: Vec<Inputs> = (0..60_u32)
            .map(|f| Inputs::at(d.scene.frame_time(i64::from(f))).with_face(FaceInput { head_roll: f64::from(f), ..FaceInput::default() }))
            .collect();
        let mut recording = recorder.record_all(&inputs, None).unwrap();
        recording.audio = Some(RecordedAudio { start: Tick::from_seconds_f64(0.5), buffer: PcmBuffer::new(8000, 1, vec![0.1; 16_000]) });
        let mut media = MediaStore::new();
        let done = d.commit(0, recording, 0, &mut media).unwrap();
        assert_eq!(done.take, Some(0));
        assert_eq!(done.clip, Some(("take 1".to_owned(), 0)));
        assert!(media.get("take 1").is_some());
        assert_eq!(d.scene.duration, Tick::from_seconds_f64(2.5), "the scene grows to hold the sound");
        assert!(matches!(d.commit(9, Recording::default(), 0, &mut media), Err(Error::NoTrack(9))));
    }
}
