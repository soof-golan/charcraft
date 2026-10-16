//! Sound: the media store, the scene's audio mix for export, and WAV bytes for recorded sound.

use std::collections::BTreeMap;

use anim_audio::{MixFormat, MixInput, PcmBuffer, mix};
use anim_core::{Tick, TimeRange};

use crate::error::{Error, Result};
use crate::scene::Scene;

/// Sound by name. Clips refer to entries by name; the store is kept beside the scene, not in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaStore {
    sounds: BTreeMap<String, PcmBuffer>,
}

/// Most sounds a store holds.
pub const MAX_SOUNDS: usize = 1024;

impl MediaStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a sound under `name` (replacing one of that name).
    ///
    /// # Errors
    /// [`Error::Invalid`] for a name [`valid_media_name`] rejects; [`Error::TooLarge`].
    pub fn insert(&mut self, name: &str, buffer: PcmBuffer) -> Result<()> {
        if !valid_media_name(name) {
            return Err(Error::invalid(format!("{name:?} is not a usable media name")));
        }
        if self.sounds.len() >= MAX_SOUNDS && !self.sounds.contains_key(name) {
            return Err(Error::TooLarge { what: "the media store", limit: MAX_SOUNDS });
        }
        self.sounds.insert(name.to_owned(), buffer);
        Ok(())
    }

    /// The sound called `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PcmBuffer> {
        self.sounds.get(name)
    }

    /// The names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sounds.keys().map(String::as_str)
    }

    /// Number of sounds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sounds.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }

    /// A name for a new sound that no entry has: `base`, `base 2`, `base 3`, ...
    #[must_use]
    pub fn unused_name(&self, base: &str) -> String {
        if !self.sounds.contains_key(base) {
            return base.to_owned();
        }
        (2_u32..=u32::MAX).map(|n| format!("{base} {n}")).find(|n| !self.sounds.contains_key(n)).unwrap_or_else(|| base.to_owned())
    }
}

/// Whether `name` is safe as a media name and as a file stem: 1 to 64 characters of letters, digits, space,
/// `.`, `_` and `-`, starting with a letter or digit.
#[must_use]
pub fn valid_media_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().count() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

/// The scene's sound mixed to `format`, from time 0 to the end of the scene.
///
/// Clips whose media is missing from the store are skipped (the names are returned so a tool can say so).
///
/// # Errors
/// Whatever the mixer reports.
pub fn mix_scene(scene: &Scene, media: &MediaStore, format: MixFormat) -> Result<(PcmBuffer, Vec<String>)> {
    let mut missing = Vec::new();
    // Slices first: the mixer borrows them.
    let mut pieces: Vec<(PcmBuffer, Tick, f32, f32)> = Vec::new();
    for track in scene.audio.iter().filter(|t| t.enabled) {
        for clip in track.clips.iter().filter(|c| c.enabled) {
            let Some(buffer) = media.get(&clip.media) else {
                if !missing.contains(&clip.media) {
                    missing.push(clip.media.clone());
                }
                continue;
            };
            let available = Tick(buffer.duration().0.saturating_sub(clip.trim_in.0));
            let length = clip.duration.map_or(available, |d| d.min(available));
            if length.0 <= 0 {
                continue;
            }
            let slice = buffer.slice_ticks(TimeRange::new(clip.trim_in, length));
            pieces.push((slice, clip.start, clip.gain * track.gain, clip.pan));
        }
    }
    let inputs: Vec<MixInput<'_>> = pieces.iter().map(|(buffer, start, gain, pan)| MixInput { buffer, start: *start, gain: *gain, pan: *pan }).collect();
    let mixed = mix(&inputs, format, Some(scene.duration))?;
    Ok((mixed, missing))
}

/// The mix as an encoder soundtrack: samples clamped to -1..1.
#[must_use]
pub fn video_track(mixed: &PcmBuffer) -> anim_video::AudioTrack {
    anim_video::AudioTrack {
        samples: mixed.samples.iter().map(|s| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }).collect(),
        sample_rate: mixed.sample_rate,
        channels: mixed.channels,
    }
}

/// The scene's soundtrack for the video encoder, or `None` when no clip sounds.
///
/// # Errors
/// Whatever the mixer reports.
pub fn scene_soundtrack(scene: &Scene, media: &MediaStore, format: MixFormat) -> Result<Option<anim_video::AudioTrack>> {
    let any = scene.audio.iter().filter(|t| t.enabled).flat_map(|t| &t.clips).any(|c| c.enabled && media.get(&c.media).is_some());
    if !any {
        return Ok(None);
    }
    Ok(Some(video_track(&mix_scene(scene, media, format)?.0)))
}

/// A 32-bit float WAV file of the sound (what [`anim_audio::decode_bytes`] reads back exactly).
#[must_use]
pub fn wav_bytes(buffer: &PcmBuffer) -> Vec<u8> {
    let data_len = buffer.samples.len().saturating_mul(4);
    let channels = buffer.channels;
    let byte_rate = buffer.sample_rate.saturating_mul(u32::from(channels)).saturating_mul(4);
    let block_align = channels.saturating_mul(4);
    let riff_len = u32::try_from(data_len.saturating_add(36)).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(data_len.saturating_add(44));
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&buffer.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&32_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data_len).unwrap_or(u32::MAX).to_le_bytes());
    for sample in &buffer.samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use anim_core::{FrameRate, Size};

    use super::*;
    use crate::scene::AudioClip;

    fn tone(rate: u32, seconds: f32, value: f32) -> PcmBuffer {
        let frames = anim_core::num::round_f64_to_u32(f64::from(seconds) * f64::from(rate));
        PcmBuffer::new(rate, 1, vec![value; usize::try_from(frames).unwrap()])
    }

    #[test]
    fn wav_bytes_round_trip_through_the_decoder() {
        let buffer = PcmBuffer::new(22_050, 2, (0..2000_u16).map(|i| f32::from(i) / 2000.0 - 0.5).collect());
        let back = anim_audio::decode_bytes(&wav_bytes(&buffer), Some("x.wav")).unwrap();
        assert_eq!((back.sample_rate, back.channels), (22_050, 2));
        assert_eq!(back.samples, buffer.samples);
    }

    #[test]
    fn media_names_are_file_safe() {
        for good in ["voice", "take 3.wav", "a-b_c"] {
            assert!(valid_media_name(good), "{good}");
        }
        for bad in ["", ".hidden", "../x", "a/b", "a\\b", "ü", &"x".repeat(65)] {
            assert!(!valid_media_name(bad), "{bad:?}");
        }
        let mut store = MediaStore::new();
        assert!(store.insert("../evil", tone(8000, 0.1, 0.1)).is_err());
        store.insert("a", tone(8000, 0.1, 0.1)).unwrap();
        assert_eq!(store.unused_name("a"), "a 2");
        assert_eq!(store.unused_name("b"), "b");
    }

    #[test]
    fn the_mix_places_trims_and_scales_clips() {
        let mut scene = Scene::new("s", Size::new(100.0, 100.0), FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
        let mut media = MediaStore::new();
        media.insert("tone", tone(8000, 1.0, 0.5)).unwrap();
        let mut clip = AudioClip::new("c", "tone", Tick::from_seconds_f64(0.5));
        clip.duration = Some(Tick::from_seconds_f64(0.25));
        scene.add_clip(0, clip).unwrap();
        scene.add_clip(0, AudioClip::new("gone", "nowhere", Tick::ZERO)).unwrap();
        let (mixed, missing) = mix_scene(&scene, &media, MixFormat { sample_rate: 8000, channels: 1 }).unwrap();
        assert_eq!(missing, ["nowhere"]);
        assert_eq!(mixed.frames(), 16_000, "the mix is as long as the scene");
        assert!(mixed.samples[2000].abs() < 1e-6, "silence before the clip");
        assert!(mixed.samples[4100].abs() > 0.1, "the clip sounds from 0.5 s");
        assert!(mixed.samples[6100].abs() < 1e-6, "and stops after its 0.25 s");
        let track = scene_soundtrack(&scene, &media, MixFormat::STEREO_48K).unwrap().unwrap();
        assert_eq!((track.sample_rate, track.channels), (48_000, 2));
        assert!(track.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn no_sound_no_soundtrack() {
        let scene = Scene::new("s", Size::new(100.0, 100.0), FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
        assert!(scene_soundtrack(&scene, &MediaStore::new(), MixFormat::STEREO_48K).unwrap().is_none());
    }
}
