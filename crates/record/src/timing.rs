//! Timing: reading and editing *when* things happen inside a take.
//!
//! A voice take is a stream of mouth shapes; the timing view shows it as runs (a shape and the stretch it holds)
//! that can be re-shaped, moved, inserted and removed. A keys take is a stream of held keys; the timing view
//! shows each press as a run with a start and an end (a trigger's timing). Edits rewrite the take's samples, so
//! everything downstream (playback, export, further recording) sees the edited timing.

use std::collections::BTreeSet;

use anim_core::Tick;
use anim_lipsync::{Viseme, VisemeKey};

use crate::group::{Frame, ParamGroup};
use crate::scene::AudioClip;
use crate::take::{Sample, Take};

/// A mouth shape that begins at a scene time and holds until the next change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicePoint {
    /// Scene time.
    pub t: Tick,
    /// The shape (`None`: the take says nothing, the mouth stays neutral).
    pub viseme: Option<Viseme>,
}

/// A mouth shape and the stretch it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceRun {
    /// Scene time it starts.
    pub start: Tick,
    /// Scene time it ends.
    pub end: Tick,
    /// The shape.
    pub viseme: Option<Viseme>,
}

/// A key held down for a stretch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRun {
    /// The key.
    pub key: String,
    /// Scene time it goes down.
    pub start: Tick,
    /// Scene time it goes up.
    pub end: Tick,
}

const fn scene_time(take: &Take, local: Tick) -> Tick {
    Tick(local.0.saturating_sub(take.offset.0).saturating_add(take.start.0))
}

const fn in_range(take: &Take, t: Tick) -> bool {
    t.0 >= take.start.0 && t.0 < take.end().0
}

/// The change points of a take's mouth shapes, the first at the take's start.
#[must_use]
pub fn voice_points(take: &Take) -> Vec<VoicePoint> {
    let mut points = vec![VoicePoint { t: take.start, viseme: take.frame_at(take.start).and_then(|f| f.viseme) }];
    for sample in &take.samples {
        let t = scene_time(take, sample.t);
        if !in_range(take, t) || t.0 <= take.start.0 {
            continue;
        }
        if points.last().is_some_and(|p| p.viseme != sample.frame.viseme) {
            points.push(VoicePoint { t, viseme: sample.frame.viseme });
        }
    }
    points
}

/// The runs between the change points of [`voice_points`].
#[must_use]
pub fn voice_runs(take: &Take) -> Vec<VoiceRun> {
    runs_of(&voice_points(take), take.end())
}

/// Turns change points into runs ending at `end`.
#[must_use]
pub fn runs_of(points: &[VoicePoint], end: Tick) -> Vec<VoiceRun> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| VoiceRun { start: p.t, end: points.get(i.saturating_add(1)).map_or(end, |n| n.t), viseme: p.viseme })
        .filter(|r| r.end.0 > r.start.0)
        .collect()
}

/// Makes sure the take has a sample at scene time `t`, copying what the take says there.
fn ensure_sample(take: &mut Take, t: Tick) -> usize {
    let local = take.local(t);
    let index = take.samples.partition_point(|s| s.t.0 < local.0);
    if take.samples.get(index).is_some_and(|s| s.t == local) {
        return index;
    }
    let frame = take.frame_at(t).unwrap_or_default();
    take.samples.insert(index, Sample { t: local, frame });
    index
}

/// Rewrites the take's mouth shapes to follow `points` (sorted by time; each holds until the next).
pub fn set_voice_points(take: &mut Take, points: &[VoicePoint]) {
    let mut points: Vec<VoicePoint> = points.iter().copied().filter(|p| in_range(take, p.t)).collect();
    points.sort_by_key(|p| p.t);
    points.dedup_by_key(|p| p.t);
    if points.first().is_none_or(|p| p.t.0 > take.start.0) {
        points.insert(0, VoicePoint { t: take.start, viseme: None });
    }
    for p in &points {
        ensure_sample(take, p.t);
    }
    let (start, end, offset) = (take.start, take.end(), take.offset);
    for sample in &mut take.samples {
        let t = Tick(sample.t.0.saturating_sub(offset.0).saturating_add(start.0));
        if t.0 < start.0 || t.0 >= end.0 {
            continue;
        }
        let idx = points.partition_point(|p| p.t.0 <= t.0);
        sample.frame.viseme = idx.checked_sub(1).and_then(|i| points.get(i)).and_then(|p| p.viseme);
    }
}

/// The key presses of a take as runs, by start time.
#[must_use]
pub fn key_runs(take: &Take) -> Vec<KeyRun> {
    let mut open: Vec<(String, Tick)> = Vec::new();
    let mut out: Vec<KeyRun> = Vec::new();
    let mut held: BTreeSet<String> = take.frame_at(take.start).map(|f| f.keys).unwrap_or_default();
    for key in &held {
        open.push((key.clone(), take.start));
    }
    for sample in &take.samples {
        let t = scene_time(take, sample.t);
        if !in_range(take, t) || t.0 <= take.start.0 {
            continue;
        }
        for key in sample.frame.keys.difference(&held) {
            open.push((key.clone(), t));
        }
        for key in held.difference(&sample.frame.keys) {
            if let Some(i) = open.iter().position(|(k, _)| k == key) {
                let (key, start) = open.remove(i);
                out.push(KeyRun { key, start, end: t });
            }
        }
        held.clone_from(&sample.frame.keys);
    }
    for (key, start) in open {
        out.push(KeyRun { key, start, end: take.end() });
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.key.cmp(&b.key)));
    out
}

/// Rewrites the take's held keys to follow `runs`.
pub fn set_key_runs(take: &mut Take, runs: &[KeyRun]) {
    let (start, end, offset) = (take.start, take.end(), take.offset);
    let runs: Vec<KeyRun> = runs.iter().filter(|r| r.end.0 > r.start.0 && r.start.0 < end.0 && r.end.0 > start.0).cloned().collect();
    for run in &runs {
        ensure_sample(take, Tick(run.start.0.max(start.0)));
        if run.end.0 < end.0 {
            ensure_sample(take, run.end);
        }
    }
    for sample in &mut take.samples {
        let t = Tick(sample.t.0.saturating_sub(offset.0).saturating_add(start.0));
        if t.0 < start.0 || t.0 >= end.0 {
            continue;
        }
        sample.frame.keys = runs.iter().filter(|r| r.start.0 <= t.0 && t.0 < r.end.0).map(|r| r.key.clone()).collect();
    }
}

/// A voice take from the mouth shapes of an analysed clip: starts where the clip does and lasts as long as the
/// clip plays. `media_length` is the length of the sound when known.
#[must_use]
pub fn clip_voice_take(clip: &AudioClip, media_length: Option<Tick>) -> Option<Take> {
    if clip.visemes.is_empty() {
        return None;
    }
    let last = clip.visemes.last().map_or(Tick::ZERO, |k| k.start);
    let media_end = media_length.unwrap_or_else(|| Tick(last.0.saturating_add(Tick::from_seconds_f64(0.5).0)));
    let window = clip.duration.unwrap_or_else(|| Tick(media_end.0.saturating_sub(clip.trim_in.0))).0.max(0);
    if window == 0 {
        return None;
    }
    let mut take = Take::new(0, std::iter::once(ParamGroup::Voice).collect(), clip.start, Tick(window));
    let keys: &[VisemeKey] = &clip.visemes;
    let first_in = keys.partition_point(|k| k.start.0 <= clip.trim_in.0);
    let current = first_in.checked_sub(1).and_then(|i| keys.get(i)).map_or(Viseme::Neutral, |k| k.viseme);
    take.samples.push(Sample { t: Tick::ZERO, frame: Frame { viseme: Some(current), ..Frame::default() } });
    for key in keys.iter().skip(first_in) {
        let local = key.start.0.saturating_sub(clip.trim_in.0);
        if local >= window {
            break;
        }
        take.samples.push(Sample { t: Tick(local), frame: Frame { viseme: Some(key.viseme), ..Frame::default() } });
    }
    Some(take)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: f64) -> Tick {
        Tick::from_seconds_f64(v)
    }

    fn talk() -> Take {
        let mut take = Take::new(1, [ParamGroup::Voice].into_iter().collect(), s(1.0), s(4.0));
        for (t, v) in [(0.0, Viseme::Neutral), (1.0, Viseme::Ah), (2.0, Viseme::Oh), (3.0, Viseme::Neutral)] {
            take.samples.push(Sample { t: s(t), frame: Frame { viseme: Some(v), ..Frame::default() } });
        }
        take
    }

    fn at(take: &Take, t: f64) -> Option<Viseme> {
        take.frame_at(s(t)).and_then(|f| f.viseme)
    }

    #[test]
    fn voice_runs_follow_the_samples() {
        let runs = voice_runs(&talk());
        assert_eq!(runs.len(), 4);
        assert_eq!((runs[1].start, runs[1].end, runs[1].viseme), (s(2.0), s(3.0), Some(Viseme::Ah)));
        assert_eq!(runs[3].end, s(5.0), "the last run lasts to the end of the take");
    }

    #[test]
    fn editing_the_points_retimes_and_reshapes() {
        let mut take = talk();
        let mut points = voice_points(&take);
        points[1].t = s(2.5);
        points[2].viseme = Some(Viseme::Ee);
        set_voice_points(&mut take, &points);
        assert_eq!(at(&take, 2.2), Some(Viseme::Neutral));
        assert_eq!(at(&take, 2.6), Some(Viseme::Ah));
        assert_eq!(at(&take, 3.2), Some(Viseme::Ee));
        assert_eq!(voice_runs(&take).len(), 4);
        // An inserted point splits a run.
        points.insert(2, VoicePoint { t: s(2.8), viseme: Some(Viseme::M) });
        set_voice_points(&mut take, &points);
        assert_eq!(at(&take, 2.9), Some(Viseme::M));
        assert_eq!(at(&take, 2.7), Some(Viseme::Ah));
    }

    #[test]
    fn edits_survive_a_trimmed_take() {
        let mut take = talk();
        take.trim_start(s(2.0));
        let before = voice_runs(&take);
        assert_eq!(before[0].start, s(2.0));
        assert_eq!(before[0].viseme, Some(Viseme::Ah));
        let mut points = voice_points(&take);
        points[1].viseme = Some(Viseme::Uh);
        set_voice_points(&mut take, &points);
        assert_eq!(at(&take, 3.5), Some(Viseme::Uh));
        assert_eq!(at(&take, 2.5), Some(Viseme::Ah));
    }

    fn pressed() -> Take {
        let mut take = Take::new(1, [ParamGroup::Keys].into_iter().collect(), s(0.0), s(4.0));
        let held = |keys: &[&str]| Frame { keys: keys.iter().map(|k| (*k).to_owned()).collect(), ..Frame::default() };
        for (t, keys) in [(0.0, vec![]), (1.0, vec!["a"]), (2.0, vec!["a", "b"]), (3.0, vec!["b"]), (3.5, vec![])] {
            take.samples.push(Sample { t: s(t), frame: held(&keys) });
        }
        take
    }

    #[test]
    fn key_runs_pair_presses_and_releases() {
        let runs = key_runs(&pressed());
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].key.as_str(), runs[0].start, runs[0].end), ("a", s(1.0), s(3.0)));
        assert_eq!((runs[1].key.as_str(), runs[1].start, runs[1].end), ("b", s(2.0), s(3.5)));
    }

    #[test]
    fn editing_key_runs_moves_the_press() {
        let mut take = pressed();
        let mut runs = key_runs(&take);
        runs[0].start = s(0.5);
        runs[0].end = s(1.5);
        runs.push(KeyRun { key: "c".to_owned(), start: s(2.5), end: s(2.75) });
        set_key_runs(&mut take, &runs);
        let down = |t: f64, k: &str| take.frame_at(s(t)).unwrap().keys.contains(k);
        assert!(!down(0.4, "a") && down(0.6, "a") && down(1.4, "a") && !down(1.6, "a"));
        assert!(down(2.6, "c") && !down(2.8, "c") && down(2.8, "b"));
        assert_eq!(key_runs(&take).len(), 3);
    }

    #[test]
    fn a_clip_becomes_a_voice_take() {
        let mut clip = AudioClip::new("v", "v", s(2.0));
        clip.visemes = vec![
            VisemeKey { start: s(0.0), viseme: Viseme::Neutral },
            VisemeKey { start: s(1.0), viseme: Viseme::Ah },
            VisemeKey { start: s(2.0), viseme: Viseme::Oh },
        ];
        let take = clip_voice_take(&clip, Some(s(3.0))).unwrap();
        assert_eq!((take.start, take.duration), (s(2.0), s(3.0)));
        assert_eq!(at(&take, 3.5), Some(Viseme::Ah));
        clip.trim_in = s(1.5);
        clip.duration = Some(s(1.0));
        let trimmed = clip_voice_take(&clip, Some(s(3.0))).unwrap();
        assert_eq!(trimmed.duration, s(1.0));
        assert_eq!(at(&trimmed, 2.1), Some(Viseme::Ah), "the shape that was showing when the trim begins");
        assert_eq!(at(&trimmed, 2.7), Some(Viseme::Oh));
        assert!(clip_voice_take(&AudioClip::new("x", "x", Tick::ZERO), None).is_none());
    }
}
