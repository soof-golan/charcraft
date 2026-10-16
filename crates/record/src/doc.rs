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
}

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
        Self { scene, arming: Arming::default(), templates: Templates::new() }
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
        self.scene.size_hint()
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
