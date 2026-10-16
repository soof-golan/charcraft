//! Speech from the scene's sound: mouth shapes for puppets that have no recorded voice.

use anim_audio::MixFormat;
use charcraft_behaviors::Inputs;

use crate::audio::{MediaStore, mix_scene};
use crate::group::ParamGroup;
use crate::recorder::Recorder;
use crate::scene::Scene;
use crate::take::MAX_SAMPLES;

/// Gives every enabled puppet track that has no voice take a take of the mouth shapes `anim-lipsync` finds in the
/// scene's mixed sound. Returns how many tracks got one; none when the scene has no sound.
///
/// Tracks with a recorded voice keep it: the recorded performance is what the project holds.
pub fn voice_from_audio(scene: &mut Scene, media: &MediaStore) -> usize {
    let has_sound = scene.audio.iter().filter(|t| t.enabled).flat_map(|t| &t.clips).any(|c| c.enabled && media.get(&c.media).is_some());
    if !has_sound {
        return 0;
    }
    let Ok((mixed, _missing)) = mix_scene(scene, media, MixFormat::STEREO_48K) else { return 0 };
    let mono = mixed.to_mono();
    let keys = anim_lipsync::analyze(&mono.samples, mono.sample_rate);
    let frames = scene.frame_count();
    if keys.is_empty() || usize::try_from(frames).map_or(true, |n| n > MAX_SAMPLES) {
        return 0;
    }
    let inputs: Vec<Inputs> = (0..i64::from(frames))
        .map(|f| {
            let at = scene.frame_time(f);
            Inputs::at(at).with_viseme(anim_lipsync::viseme_at(&keys, at))
        })
        .collect();
    let silent: Vec<usize> = scene
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.enabled && !t.takes.iter().any(|k| k.enabled && k.groups.contains(&ParamGroup::Voice)))
        .map(|(i, _)| i)
        .collect();
    let mut given = 0_usize;
    for index in silent {
        let mut recorder = Recorder::new();
        recorder.arm_only([ParamGroup::Voice]);
        let Ok(recording) = recorder.record_all(&inputs, Some(scene.duration)) else { continue };
        if let Some(take) = recording.take
            && scene.add_take(index, take).is_ok()
        {
            given = given.saturating_add(1);
        }
    }
    given
}
