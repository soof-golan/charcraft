//! Takes: timestamped input samples of some parameter groups over a stretch of the scene.

use std::collections::BTreeSet;

use anim_core::{Tick, TimeRange};
use charcraft_behaviors::Inputs;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::group::{Frame, ParamGroup, apply_group};

/// Most samples one take may hold.
pub const MAX_SAMPLES: usize = 2_000_000;

/// The inputs at a moment of a take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Time since the take's start.
    pub t: Tick,
    /// What was captured.
    pub frame: Frame,
}

/// One recording of some parameter groups.
///
/// A take lies on a track from `start` for `duration`. Inside that range it replaces the groups it recorded
/// (`groups`) of whatever lies below it, fading in over `blend_in` and out over `blend_out`; outside it, or when
/// switched off, it has no say. Samples are sorted by time and hold their value until the next one (the face
/// interpolates between samples).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    /// The take number (`take 3`), counted per scene.
    pub number: u32,
    /// The display name (`take 3 - Face`).
    pub name: String,
    /// The groups this take overrides.
    pub groups: BTreeSet<ParamGroup>,
    /// Whether it plays.
    pub enabled: bool,
    /// Where on the scene timeline it starts.
    pub start: Tick,
    /// How long it lasts.
    pub duration: Tick,
    /// Fade-in length.
    pub blend_in: Tick,
    /// Fade-out length.
    pub blend_out: Tick,
    /// The recorded samples, in time order.
    pub samples: Vec<Sample>,
    /// How far into the recording the take starts: trimming the front of a take moves `start` and grows this, so
    /// the samples stay where they were recorded and the trim can be undone.
    #[serde(default)]
    pub offset: Tick,
}

impl Take {
    /// A take with no samples yet.
    #[must_use]
    pub fn new(number: u32, groups: BTreeSet<ParamGroup>, start: Tick, duration: Tick) -> Self {
        Self {
            number,
            name: Self::default_name(number, &groups),
            groups,
            enabled: true,
            start,
            duration,
            blend_in: Tick::ZERO,
            blend_out: Tick::ZERO,
            samples: Vec::new(),
            offset: Tick::ZERO,
        }
    }

    /// Cuts the front off: the take now starts at scene time `at` (clamped to inside the take). The samples keep
    /// their place in the recording.
    pub fn trim_start(&mut self, at: Tick) {
        let at = Tick(at.0.clamp(self.start.0, self.end().0));
        let cut = at.0.saturating_sub(self.start.0);
        self.offset = Tick(self.offset.0.saturating_add(cut));
        self.duration = Tick(self.duration.0.saturating_sub(cut));
        self.start = at;
        self.blend_in = Tick::ZERO;
        self.fit_blends();
    }

    /// Cuts the back off: the take now ends at scene time `at` (clamped to inside the take).
    pub fn trim_end(&mut self, at: Tick) {
        let at = Tick(at.0.clamp(self.start.0, self.end().0));
        self.duration = Tick(at.0.saturating_sub(self.start.0));
        self.blend_out = Tick::ZERO;
        self.fit_blends();
    }

    fn fit_blends(&mut self) {
        let room = self.duration.0.max(0);
        self.blend_in = Tick(self.blend_in.0.clamp(0, room));
        self.blend_out = Tick(self.blend_out.0.clamp(0, room.saturating_sub(self.blend_in.0)));
    }

    /// Splits the take at scene time `at` into two that play exactly as the whole did. `self` keeps the first
    /// part; the second part is returned (`None` when `at` is not strictly inside).
    pub fn split_at(&mut self, at: Tick) -> Option<Self> {
        if at.0 <= self.start.0 || at.0 >= self.end().0 {
            return None;
        }
        let mut second = self.clone();
        second.trim_start(at);
        second.blend_out = self.blend_out;
        second.fit_blends();
        self.trim_end(at);
        Some(second)
    }

    /// The position in the recording (what the samples use) of scene time `t`.
    #[must_use]
    pub const fn local(&self, t: Tick) -> Tick {
        Tick(t.0.saturating_sub(self.start.0).saturating_add(self.offset.0))
    }

    /// `take 3 - head, brows`: Character Animator's naming, with the groups for the behaviors.
    #[must_use]
    pub fn default_name(number: u32, groups: &BTreeSet<ParamGroup>) -> String {
        let names: Vec<&str> = if groups.len() == ParamGroup::ALL.len() { vec!["all"] } else { groups.iter().map(|g| g.label()).collect() };
        format!("take {number} - {}", names.join(", "))
    }

    /// The scene time range it covers.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        TimeRange { start: self.start, duration: self.duration }
    }

    /// When it ends.
    #[must_use]
    pub const fn end(&self) -> Tick {
        Tick(self.start.0.saturating_add(self.duration.0))
    }

    /// How strongly the take applies at scene time `t`: 0 outside (or switched off), 1 in the middle, ramping over
    /// the blend lengths at the edges.
    #[must_use]
    pub fn weight_at(&self, t: Tick) -> f64 {
        if !self.enabled || self.duration.0 <= 0 || t.0 < self.start.0 || t.0 >= self.end().0 {
            return 0.0;
        }
        let since = t.0.saturating_sub(self.start.0);
        let until = self.end().0.saturating_sub(t.0);
        ramp(since, self.blend_in.0).min(ramp(until, self.blend_out.0))
    }

    /// The recorded inputs at scene time `t`, interpolated between samples; `None` outside the take or without
    /// samples.
    #[must_use]
    pub fn frame_at(&self, t: Tick) -> Option<Frame> {
        if t.0 < self.start.0 || t.0 >= self.end().0 {
            return None;
        }
        let local = self.local(t).0;
        let next_index = self.samples.partition_point(|s| s.t.0 <= local);
        let prev = next_index.checked_sub(1).and_then(|i| self.samples.get(i));
        let next = self.samples.get(next_index);
        match (prev, next) {
            (Some(a), Some(b)) => {
                let span = b.t.0.saturating_sub(a.t.0);
                let weight = if span <= 0 { 0.0 } else { ratio(local.saturating_sub(a.t.0), span) };
                Some(a.frame.lerp(&b.frame, weight))
            }
            (Some(a), None) => Some(a.frame.clone()),
            (None, Some(b)) => Some(b.frame.clone()),
            (None, None) => None,
        }
    }

    /// Sorts the samples, repairs them and checks the limits; used on everything that comes from a file.
    ///
    /// # Errors
    /// [`Error::TooLarge`] for too many samples; [`Error::Invalid`] for a negative duration.
    pub fn sanitize(&mut self) -> Result<()> {
        if self.samples.len() > MAX_SAMPLES {
            return Err(Error::TooLarge { what: "a take", limit: MAX_SAMPLES });
        }
        if self.duration.0 < 0 || self.blend_in.0 < 0 || self.blend_out.0 < 0 {
            return Err(Error::invalid(format!("take {} has a negative length", self.number)));
        }
        self.offset = Tick(self.offset.0.max(0));
        self.samples.sort_by_key(|s| s.t);
        for sample in &mut self.samples {
            sample.frame.sanitize();
        }
        Ok(())
    }
}

/// `part / whole` as a float, both positive.
#[must_use]
pub fn ratio(part: i64, whole: i64) -> f64 {
    if whole <= 0 {
        return 1.0;
    }
    (anim_core::num::i64_to_f64(part) / anim_core::num::i64_to_f64(whole)).clamp(0.0, 1.0)
}

/// 0 at the edge rising to 1 at `length` ticks in (1 everywhere when `length` is 0).
fn ramp(distance: i64, length: i64) -> f64 {
    if length <= 0 { 1.0 } else { ratio(distance, length) }
}

/// Layers the takes over `base`: for each take, bottom to top, the groups it recorded replace those below it at
/// its weight. `base.time` is the scene time.
#[must_use]
pub fn resolve(takes: &[Take], mut base: Inputs) -> Inputs {
    for take in takes {
        let weight = take.weight_at(base.time);
        if weight <= 0.0 {
            continue;
        }
        let Some(frame) = take.frame_at(base.time) else { continue };
        for group in &take.groups {
            apply_group(&mut base, *group, &frame, weight);
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use charcraft_behaviors::FaceInput;

    use super::*;

    fn seconds(s: f64) -> Tick {
        Tick::from_seconds_f64(s)
    }

    fn roll_take(number: u32, from: f64, to: f64, roll: f64) -> Take {
        let mut take = Take::new(number, [ParamGroup::Head].into_iter().collect(), seconds(from), seconds(to - from));
        take.samples.push(Sample { t: Tick::ZERO, frame: Frame { face: Some(FaceInput { head_roll: roll, ..FaceInput::default() }), ..Frame::default() } });
        take
    }

    fn roll_at(takes: &[Take], t: f64) -> Option<f64> {
        resolve(takes, Inputs::at(seconds(t))).face.map(|f| f.head_roll)
    }

    #[test]
    fn a_take_only_speaks_inside_its_range() {
        let takes = [roll_take(1, 1.0, 2.0, 20.0)];
        assert_eq!(roll_at(&takes, 0.5), None);
        assert_eq!(roll_at(&takes, 1.0), Some(20.0));
        assert_eq!(roll_at(&takes, 1.99), Some(20.0));
        assert_eq!(roll_at(&takes, 2.0), None, "the range is half open");
    }

    #[test]
    fn later_takes_override_earlier_ones_where_they_overlap() {
        let takes = [roll_take(1, 0.0, 4.0, 10.0), roll_take(2, 1.0, 2.0, -30.0)];
        assert_eq!(roll_at(&takes, 0.5), Some(10.0));
        assert_eq!(roll_at(&takes, 1.5), Some(-30.0));
        assert_eq!(roll_at(&takes, 3.0), Some(10.0), "the earlier take comes back after the later one ends");
    }

    #[test]
    fn a_disabled_take_does_nothing() {
        let mut takes = [roll_take(1, 0.0, 4.0, 10.0), roll_take(2, 1.0, 2.0, -30.0)];
        takes[1].enabled = false;
        assert_eq!(roll_at(&takes, 1.5), Some(10.0));
    }

    #[test]
    fn blends_ramp_over_the_lower_take() {
        let mut upper = roll_take(2, 1.0, 3.0, 20.0);
        upper.blend_in = seconds(1.0);
        upper.blend_out = seconds(1.0);
        let takes = [roll_take(1, 0.0, 4.0, 0.0), upper];
        let at = |t: f64| roll_at(&takes, t).unwrap();
        assert!(at(1.0).abs() < 1e-9, "weight 0 at the very start");
        assert!((at(1.5) - 10.0).abs() < 1e-6);
        assert!((at(2.0) - 20.0).abs() < 1e-9);
        assert!((at(2.5) - 10.0).abs() < 1e-6);
        assert!(at(3.5).abs() < 1e-9, "back to the lower take");
    }

    #[test]
    fn groups_override_independently() {
        let mut brows = Take::new(2, [ParamGroup::Brows].into_iter().collect(), seconds(0.0), seconds(4.0));
        brows.samples.push(Sample {
            t: Tick::ZERO,
            frame: Frame { face: Some(FaceInput { eyebrow_left: 0.9, head_roll: 99.0, ..FaceInput::default() }), ..Frame::default() },
        });
        let takes = [roll_take(1, 0.0, 4.0, 15.0), brows];
        let face = resolve(&takes, Inputs::at(seconds(1.0))).face.unwrap();
        assert!((face.head_roll - 15.0).abs() < 1e-12, "the brows take must not touch the head");
        assert!((face.eyebrow_left - 0.9).abs() < 1e-12);
    }

    #[test]
    fn the_face_interpolates_between_samples() {
        let mut take = Take::new(1, [ParamGroup::Head].into_iter().collect(), seconds(0.0), seconds(2.0));
        for (t, roll) in [(0.0, 0.0), (1.0, 10.0)] {
            take.samples.push(Sample { t: seconds(t), frame: Frame { face: Some(FaceInput { head_roll: roll, ..FaceInput::default() }), ..Frame::default() } });
        }
        let roll = |t: f64| take.frame_at(seconds(t)).unwrap().face.unwrap().head_roll;
        assert!((roll(0.25) - 2.5).abs() < 1e-9);
        assert!((roll(1.5) - 10.0).abs() < 1e-12, "after the last sample the value holds");
    }

    fn ramp_take() -> Take {
        let mut take = Take::new(1, [ParamGroup::Head].into_iter().collect(), seconds(1.0), seconds(4.0));
        for (t, roll) in [(0.0, 0.0), (4.0, 40.0)] {
            take.samples.push(Sample { t: seconds(t), frame: Frame { face: Some(FaceInput { head_roll: roll, ..FaceInput::default() }), ..Frame::default() } });
        }
        take
    }

    #[test]
    fn trimming_the_front_keeps_the_samples_where_they_were_recorded() {
        let mut take = ramp_take();
        let before = take.frame_at(seconds(3.0)).unwrap().face.unwrap().head_roll;
        take.trim_start(seconds(2.0));
        assert_eq!(take.start, seconds(2.0));
        assert_eq!(take.duration, seconds(3.0));
        let after = take.frame_at(seconds(3.0)).unwrap().face.unwrap().head_roll;
        assert!((before - after).abs() < 1e-9, "trimming must not shift the performance");
        assert!(take.frame_at(seconds(1.5)).is_none());
        take.trim_end(seconds(100.0));
        assert_eq!(take.end(), seconds(5.0), "a trim cannot grow the take");
    }

    #[test]
    fn splitting_plays_the_same_as_the_whole() {
        let whole = ramp_take();
        let mut first = whole.clone();
        let second = first.split_at(seconds(2.5)).unwrap();
        assert_eq!(first.end(), seconds(2.5));
        assert_eq!(second.start, seconds(2.5));
        for t in [1.0, 1.9, 2.4, 2.5, 3.7, 4.9] {
            let from = if t < 2.5 { &first } else { &second };
            let a = whole.frame_at(seconds(t)).unwrap().face.unwrap().head_roll;
            let b = from.frame_at(seconds(t)).unwrap().face.unwrap().head_roll;
            assert!((a - b).abs() < 1e-9, "t={t}");
        }
        assert!(first.clone().split_at(seconds(0.5)).is_none() && first.clone().split_at(seconds(2.5)).is_none());
    }

    #[test]
    fn sanitizing_sorts_and_rejects_negative_lengths() {
        let mut take = roll_take(1, 0.0, 1.0, 5.0);
        take.samples.push(Sample { t: Tick(-5), frame: Frame::default() });
        take.sanitize().unwrap();
        assert!(take.samples.windows(2).all(|w| w[0].t <= w[1].t));
        take.duration = Tick(-1);
        assert!(take.sanitize().is_err());
    }
}
