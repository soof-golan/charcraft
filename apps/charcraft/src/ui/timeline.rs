//! The scene as the timeline widget wants it: a row per puppet track, a row under it per take, a row per sound.

use anim_core::Rgba8;
use anim_ui::TimelineModel;
use anim_ui::timeline::{LayerKind, LayerRow, Span, Waveform};
use charcraft_record::{MediaStore, Scene};

use crate::view::View;

/// Row ids: puppet tracks, their takes and the sound tracks never collide.
const TAKE_BASE: u64 = 10_000;
const AUDIO_BASE: u64 = 900_000_000;

/// The row id of a puppet track.
#[must_use]
pub fn track_row(track: usize) -> u64 {
    u64::try_from(track).unwrap_or(0)
}

/// The row id of take `take` of `track`.
#[must_use]
pub fn take_row(track: usize, take: usize) -> u64 {
    TAKE_BASE.saturating_mul(u64::try_from(track).unwrap_or(0).saturating_add(1)).saturating_add(u64::try_from(take).unwrap_or(0).saturating_add(1))
}

/// The track a row belongs to, and the take when it is a take row.
#[must_use]
pub fn row_target(id: u64) -> Option<(usize, Option<usize>)> {
    if id >= AUDIO_BASE {
        return None;
    }
    if id < TAKE_BASE {
        return usize::try_from(id).ok().map(|t| (t, None));
    }
    let track = usize::try_from((id / TAKE_BASE).saturating_sub(1)).ok()?;
    let take = usize::try_from((id % TAKE_BASE).saturating_sub(1)).ok()?;
    Some((track, Some(take)))
}

const PALETTE: [Rgba8; 6] = [
    Rgba8 { r: 0x4f, g: 0x9d, b: 0xff, a: 255 },
    Rgba8 { r: 0xe5, g: 0x7a, b: 0x39, a: 255 },
    Rgba8 { r: 0x43, g: 0xa0, b: 0x47, a: 255 },
    Rgba8 { r: 0xab, g: 0x47, b: 0xbc, a: 255 },
    Rgba8 { r: 0xe5, g: 0x39, b: 0x35, a: 255 },
    Rgba8 { r: 0x00, g: 0xac, b: 0xc1, a: 255 },
];

fn frames_of(scene: &Scene, seconds: f64) -> u32 {
    u32::try_from(scene.frame_at(anim_core::Tick::from_seconds_f64(seconds.max(0.0))).max(0)).unwrap_or(0)
}

/// Peak of the sound over each scene frame of a clip (cached in the view).
fn peaks(view: &mut View, scene: &Scene, media: &MediaStore, name: &str, frames: usize) -> Vec<f32> {
    let Some(buffer) = media.get(name) else { return Vec::new() };
    let key = (scene.fps.num, scene.fps.den, buffer.samples.len());
    if let Some((k, cached)) = view.peaks.get(name)
        && *k == key
        && cached.len() >= frames
    {
        return cached.iter().take(frames).copied().collect();
    }
    let channels = usize::from(buffer.channels.max(1));
    let rate = anim_core::num::usize_to_f64(usize::try_from(buffer.sample_rate).unwrap_or(1));
    let per_frame = (rate / scene.fps_f64().max(0.001)).max(1.0);
    let step = anim_core::num::round_f64_to_u32(per_frame).max(1);
    let samples_per_frame = usize::try_from(step).unwrap_or(1).saturating_mul(channels);
    let out: Vec<f32> = buffer.samples.chunks(samples_per_frame).map(|c| c.iter().fold(0.0_f32, |m, s| m.max(s.abs())).min(1.0)).collect();
    view.peaks.insert(name.to_owned(), (key, out.clone()));
    out.into_iter().take(frames).collect()
}

/// Builds the model the timeline widget shows.
#[must_use]
pub fn model(view: &mut View, scene: &Scene, media: &MediaStore, playing: bool) -> TimelineModel {
    let frame_count = scene.frame_count().max(1);
    let mut layers = Vec::new();
    for (i, track) in scene.tracks.iter().enumerate() {
        let color = PALETTE.iter().cycle().nth(i).copied().unwrap_or_default();
        let mut row = LayerRow::new(track_row(i), track.name.clone());
        row.color = color;
        row.visible = track.enabled;
        row.kind = LayerKind::Folder;
        row.expanded = true;
        row.spans = vec![Span::key(0, frame_count)];
        layers.push(row);
        for (k, take) in track.takes.iter().enumerate() {
            let start = frames_of(scene, take.start.seconds());
            let len = frames_of(scene, take.duration.seconds()).max(1);
            let mut take_row = LayerRow::new(take_row(i, k), format!("{} {}", take.name, groups_label(&take.groups)));
            take_row.depth = 1;
            take_row.color = color;
            take_row.visible = take.enabled;
            let mut span = Span::key(start, len);
            span.label = Some(take.name.clone());
            take_row.spans = if start > 0 { vec![Span::blank(0, start), span] } else { vec![span] };
            layers.push(take_row);
        }
    }
    for (a, audio) in scene.audio.iter().enumerate() {
        let mut row = LayerRow::new(AUDIO_BASE.saturating_add(u64::try_from(a).unwrap_or(0)), audio.name.clone());
        row.color = Rgba8 { r: 0x8d, g: 0x6e, b: 0x63, a: 255 };
        row.visible = audio.enabled;
        let mut spans: Vec<Span> = Vec::new();
        let mut cursor = 0_u32;
        let mut clips: Vec<_> = audio.clips.iter().collect();
        clips.sort_by_key(|c| c.start);
        for clip in clips {
            let start = frames_of(scene, clip.start.seconds()).max(cursor);
            let seconds = clip.duration.map(anim_core::Tick::seconds).or_else(|| media.get(&clip.media).map(anim_audio::PcmBuffer::seconds)).unwrap_or(0.5);
            let len = frames_of(scene, seconds).max(1);
            if start > cursor {
                spans.push(Span::blank(cursor, start.saturating_sub(cursor)));
            }
            let mut span = Span::key(start, len);
            span.label = Some(clip.name.clone());
            let wave = peaks(view, scene, media, &clip.media, usize::try_from(len).unwrap_or(0));
            if !wave.is_empty() {
                span.sound = Some(Waveform { peaks: wave });
            }
            cursor = start.saturating_add(len);
            spans.push(span);
        }
        row.spans = spans;
        layers.push(row);
    }
    TimelineModel { layers, frame_count, current_frame: frames_of(scene, view.playhead).min(frame_count.saturating_sub(1)), fps: anim_core::num::f64_to_f32(scene.fps_f64()), playing, ..TimelineModel::default() }
}

fn groups_label(groups: &std::collections::BTreeSet<charcraft_record::ParamGroup>) -> String {
    let names: Vec<String> = groups.iter().map(|g| format!("{g:?}")).collect();
    format!("[{}]", names.join(" "))
}

#[cfg(test)]
mod tests {
    use anim_core::{FrameRate, Size, Tick};
    use charcraft_record::{PuppetTrack, Scene};

    use super::*;

    #[test]
    fn row_ids_round_trip() {
        assert_eq!(row_target(track_row(3)), Some((3, None)));
        assert_eq!(row_target(take_row(2, 5)), Some((2, Some(5))));
        assert_eq!(row_target(AUDIO_BASE), None);
    }

    #[test]
    fn a_scene_becomes_rows() {
        let mut scene = Scene::new("s", Size::new(640.0, 360.0), FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
        scene.add_track(PuppetTrack::new("Hero", "p")).unwrap();
        scene.audio.push(charcraft_record::AudioTrack::new("Voice"));
        let mut view = View::default();
        let m = model(&mut view, &scene, &MediaStore::new(), false);
        assert_eq!(m.frame_count, 60);
        assert_eq!(m.layers.len(), 2);
        assert_eq!(m.layers[0].name, "Hero");
    }
}
