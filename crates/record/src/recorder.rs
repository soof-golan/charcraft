//! Recording: capture the armed parts of a timestamped input stream into a [`Take`].
//!
//! The recorder is a plain state machine. The application (or a tool) arms groups, optionally sets a punch-in
//! range, calls [`Recorder::start`], feeds it one [`Inputs`] per tick (the same value it gives the behavior
//! stack, so the take captures exactly what the puppet was driven by) and finishes with [`Recorder::finish`].
//! Microphone chunks go to [`Recorder::push_audio`].

use std::collections::BTreeSet;

use anim_audio::PcmBuffer;
use anim_core::{Tick, TimeRange};
use charcraft_behaviors::Inputs;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::group::{Frame, ParamGroup};
use crate::take::{MAX_SAMPLES, Sample, Take};

/// Most audio samples one recording keeps (about an hour of 48 kHz stereo).
pub const MAX_AUDIO_SAMPLES: usize = 48_000 * 2 * 3600;

/// What to record and over which range.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Arming {
    /// The armed groups.
    pub groups: BTreeSet<ParamGroup>,
    /// Whether the microphone is armed.
    pub audio: bool,
    /// Record only inside this range of scene time (punch in and out); `None` records everything fed.
    pub punch: Option<TimeRange>,
}

/// Sound that was recorded, and where it starts on the scene timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedAudio {
    /// Scene time of the first frame.
    pub start: Tick,
    /// The sound.
    pub buffer: PcmBuffer,
}

/// What a finished recording produced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recording {
    /// The input take (`None` when no group was armed or nothing fell inside the punch range).
    pub take: Option<Take>,
    /// The sound.
    pub audio: Option<RecordedAudio>,
}

#[derive(Debug, Clone)]
struct Active {
    started: Tick,
    /// Samples with absolute scene times.
    samples: Vec<Sample>,
    audio: Option<RecordedAudio>,
}

/// A recording in progress, or the settings for the next one.
#[derive(Debug, Clone, Default)]
pub struct Recorder {
    arming: Arming,
    active: Option<Active>,
}

impl Recorder {
    /// A recorder with nothing armed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current arming.
    #[must_use]
    pub const fn arming(&self) -> &Arming {
        &self.arming
    }

    /// Arms or disarms one group.
    pub fn set_armed(&mut self, group: ParamGroup, armed: bool) {
        if armed {
            self.arming.groups.insert(group);
        } else {
            self.arming.groups.remove(&group);
        }
    }

    /// Arms exactly these groups.
    pub fn arm_only(&mut self, groups: impl IntoIterator<Item = ParamGroup>) {
        self.arming.groups = groups.into_iter().collect();
    }

    /// Arms or disarms the microphone.
    pub const fn set_audio_armed(&mut self, armed: bool) {
        self.arming.audio = armed;
    }

    /// Sets (or clears) the punch-in range.
    pub const fn set_punch(&mut self, punch: Option<TimeRange>) {
        self.arming.punch = punch;
    }

    /// Whether a recording is running.
    #[must_use]
    pub const fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// Starts recording at scene time `at`.
    ///
    /// # Errors
    /// [`Error::NothingArmed`] when no group and no microphone is armed; [`Error::AlreadyRecording`].
    pub fn start(&mut self, at: Tick) -> Result<()> {
        if self.active.is_some() {
            return Err(Error::AlreadyRecording);
        }
        if self.arming.groups.is_empty() && !self.arming.audio {
            return Err(Error::NothingArmed);
        }
        self.active = Some(Active { started: at, samples: Vec::new(), audio: None });
        Ok(())
    }

    /// Whether scene time `t` is inside the punch range (always, without one).
    fn punched(&self, t: Tick) -> bool {
        self.arming.punch.is_none_or(|p| p.contains(t))
    }

    /// Captures one tick. `inputs.time` is scene time; ticks outside the punch range or before the start are
    /// ignored. Does nothing when no recording runs.
    pub fn feed(&mut self, inputs: &Inputs) {
        if self.arming.groups.is_empty() || !self.punched(inputs.time) {
            return;
        }
        let groups = self.arming.groups.clone();
        let Some(active) = self.active.as_mut() else { return };
        if inputs.time.0 < active.started.0 || active.samples.len() >= MAX_SAMPLES {
            return;
        }
        let frame = Frame::capture(inputs, &groups);
        // A run of identical values keeps only its first and last sample: the value holds in between.
        let n = active.samples.len();
        if n >= 2
            && let (Some(a), Some(b)) = (active.samples.get(n.saturating_sub(2)), active.samples.get(n.saturating_sub(1)))
            && a.frame == frame
            && b.frame == frame
            && let Some(last) = active.samples.last_mut()
        {
            last.t = inputs.time;
            return;
        }
        active.samples.push(Sample { t: inputs.time, frame });
    }

    /// Appends a chunk of microphone sound that starts at scene time `at`. Chunks must share one format and
    /// arrive in order; the part of a chunk before the start of the recording or outside the punch range is cut
    /// off (so the sound covers exactly the punch), a hole between chunks is filled with silence and an overlap
    /// with the previous chunk is not repeated, so every sample stays at its true scene time. Chunks that break
    /// the limits are dropped.
    pub fn push_audio(&mut self, at: Tick, chunk: &PcmBuffer) {
        if !self.arming.audio || !chunk.is_valid_format() {
            return;
        }
        let punch = self.arming.punch;
        let Some(active) = self.active.as_mut() else { return };
        let chunk_end = Tick(at.0.saturating_add(chunk.duration().0));
        let from = punch.map_or(active.started, |p| p.start.max(active.started)).max(at);
        let to = punch.map_or(chunk_end, |p| p.end().min(chunk_end));
        if to.0 <= from.0 {
            return;
        }
        let piece = chunk.slice_ticks(TimeRange::from_bounds(Tick(from.0.saturating_sub(at.0)), Tick(to.0.saturating_sub(at.0))));
        if piece.samples.is_empty() {
            return;
        }
        let Some(rec) = active.audio.as_mut() else {
            active.audio = Some(RecordedAudio { start: from, buffer: piece });
            return;
        };
        let have_end = Tick(rec.start.0.saturating_add(rec.buffer.duration().0));
        let rate = i64::from(rec.buffer.sample_rate);
        let channels = usize::from(rec.buffer.channels);
        if from.0 >= have_end.0 {
            let gap = usize::try_from(Tick(from.0.saturating_sub(have_end.0)).to_units_floor(rate)).unwrap_or(usize::MAX);
            let pad = gap.saturating_mul(channels);
            if rec.buffer.samples.len().saturating_add(pad).saturating_add(piece.samples.len()) > MAX_AUDIO_SAMPLES {
                return;
            }
            rec.buffer.samples.resize(rec.buffer.samples.len().saturating_add(pad), 0.0);
            rec.buffer.append(&piece);
        } else {
            let fresh = piece.slice_ticks(TimeRange::new(Tick(have_end.0.saturating_sub(from.0)), Tick::MAX));
            if rec.buffer.samples.len().saturating_add(fresh.samples.len()) <= MAX_AUDIO_SAMPLES {
                rec.buffer.append(&fresh);
            }
        }
    }

    /// Stops and returns what was recorded. `end` is the scene time the recording stopped at.
    ///
    /// # Errors
    /// [`Error::NotRecording`].
    pub fn finish(&mut self, end: Tick) -> Result<Recording> {
        let active = self.active.take().ok_or(Error::NotRecording)?;
        let whole = TimeRange::from_bounds(active.started, Tick(end.0.max(active.started.0)));
        let range = self.arming.punch.map_or(Some(whole), |punch| punch.intersect(&whole));
        let take = range.filter(|r| !r.is_empty() && !active.samples.is_empty() && !self.arming.groups.is_empty()).map(|r| {
            let mut take = Take::new(0, self.arming.groups.clone(), r.start, r.duration);
            take.samples =
                active.samples.into_iter().filter(|s| r.contains(s.t)).map(|s| Sample { t: Tick(s.t.0.saturating_sub(r.start.0)), frame: s.frame }).collect();
            take
        });
        let take = take.filter(|t| !t.samples.is_empty());
        Ok(Recording { take, audio: active.audio })
    }

    /// Drops a running recording.
    pub fn abort(&mut self) {
        self.active = None;
    }

    /// Records a whole prepared run of inputs at once (a script, or a replay of a performance) and returns the
    /// result. The recording ends one tick after the last input unless `end` says otherwise.
    ///
    /// # Errors
    /// [`Error::NothingArmed`], [`Error::AlreadyRecording`].
    pub fn record_all(&mut self, inputs: &[Inputs], end: Option<Tick>) -> Result<Recording> {
        let start = inputs.first().map_or(Tick::ZERO, |i| i.time);
        self.start(start)?;
        for input in inputs {
            self.feed(input);
        }
        let last = inputs.last().map_or(start, |i| Tick(i.time.0.saturating_add(1)));
        self.finish(end.unwrap_or(last))
    }
}

#[cfg(test)]
mod tests {
    use charcraft_behaviors::FaceInput;

    use super::*;

    fn seconds(s: f64) -> Tick {
        Tick::from_seconds_f64(s)
    }

    fn run(seconds_long: f64, fps: f64) -> Vec<Inputs> {
        let frames = anim_core::num::round_f64_to_u32(seconds_long * fps);
        (0..frames)
            .map(|f| {
                let t = f64::from(f) / fps;
                Inputs::at(seconds(t))
                    .with_face(FaceInput { head_roll: 10.0 * t, mouth_open: 0.5, ..FaceInput::default() })
                    .with_viseme(anim_lipsync::Viseme::Ah)
            })
            .collect()
    }

    #[test]
    fn nothing_armed_cannot_start() {
        let mut r = Recorder::new();
        assert!(matches!(r.start(Tick::ZERO), Err(Error::NothingArmed)));
        r.set_armed(ParamGroup::Head, true);
        r.start(Tick::ZERO).unwrap();
        assert!(matches!(r.start(Tick::ZERO), Err(Error::AlreadyRecording)));
        assert!(r.is_recording());
        r.abort();
        assert!(matches!(r.finish(Tick::ZERO), Err(Error::NotRecording)));
    }

    #[test]
    fn only_armed_groups_are_captured() {
        let mut r = Recorder::new();
        r.arm_only([ParamGroup::Head, ParamGroup::Voice]);
        let rec = r.record_all(&run(1.0, 30.0), None).unwrap();
        let take = rec.take.unwrap();
        assert_eq!(take.groups, [ParamGroup::Head, ParamGroup::Voice].into_iter().collect());
        assert_eq!(take.samples.len(), 30);
        let first = take.samples.last().unwrap().frame.face.unwrap();
        assert!(first.head_roll > 9.0);
        assert!(first.mouth_open.abs() < 1e-12, "the mouth group was not armed");
        assert_eq!(take.samples[0].frame.viseme, Some(anim_lipsync::Viseme::Ah));
    }

    #[test]
    fn punch_in_keeps_only_the_range() {
        let mut r = Recorder::new();
        r.arm_only([ParamGroup::Head]);
        r.set_punch(Some(TimeRange::from_bounds(seconds(0.5), seconds(0.8))));
        let take = r.record_all(&run(2.0, 30.0), None).unwrap().take.unwrap();
        assert_eq!(take.start, seconds(0.5));
        assert_eq!(take.duration, Tick(seconds(0.8).0 - seconds(0.5).0));
        assert!(take.samples.first().unwrap().t.0 >= 0);
        assert!(take.samples.iter().all(|s| s.t.0 < take.duration.0));
        assert!(take.samples.len() >= 8 && take.samples.len() <= 10, "{}", take.samples.len());
        // A punch outside everything fed gives no take.
        r.set_punch(Some(TimeRange::from_bounds(seconds(5.0), seconds(6.0))));
        assert!(r.record_all(&run(1.0, 30.0), None).unwrap().take.is_none());
    }

    #[test]
    fn identical_runs_keep_only_their_ends() {
        let mut r = Recorder::new();
        r.arm_only([ParamGroup::Voice]);
        let still: Vec<Inputs> = (0..100).map(|f| Inputs::at(seconds(f64::from(f) / 30.0)).with_viseme(anim_lipsync::Viseme::Oh)).collect();
        let take = r.record_all(&still, None).unwrap().take.unwrap();
        assert_eq!(take.samples.len(), 2);
        assert_eq!(take.samples[1].t, still[99].time);
    }

    #[test]
    fn audio_chunks_accumulate_when_armed() {
        let mut r = Recorder::new();
        r.set_audio_armed(true);
        r.start(seconds(1.0)).unwrap();
        let chunk = PcmBuffer::new(8000, 1, vec![0.25; 800]);
        r.push_audio(seconds(1.0), &chunk);
        r.push_audio(seconds(1.1), &chunk);
        r.push_audio(seconds(0.5), &chunk);
        let rec = r.finish(seconds(2.0)).unwrap();
        assert!(rec.take.is_none(), "no group armed, so no input take");
        let audio = rec.audio.unwrap();
        assert_eq!(audio.start, seconds(1.0));
        assert_eq!(audio.buffer.frames(), 1600, "the chunk before the start is dropped");
    }

    #[test]
    fn unarmed_audio_is_ignored() {
        let mut r = Recorder::new();
        r.set_armed(ParamGroup::Head, true);
        r.start(Tick::ZERO).unwrap();
        r.push_audio(Tick::ZERO, &PcmBuffer::new(8000, 1, vec![0.1; 10]));
        assert!(r.finish(seconds(1.0)).unwrap().audio.is_none());
    }
}
