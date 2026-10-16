//! Recorded takes: Character Animator's `.arf` event graphs turned into [`Take`]s.
//!
//! A recording holds curves of what a behavior saw (see `charcraft-arf`); a take holds timestamped inputs. This
//! module samples the curves at [`SAMPLE_HZ`] (plus every jump of a stepped signal, so a mouth shape changes at the
//! moment it changed) and maps the values onto the inputs the behaviors read:
//!
//! | Recording | Take groups | Maps to |
//! |---|---|---|
//! | face tracker values (`Adobe.FaceTracker2.js`) | head, brows, eyes, mouth | [`FaceInput`], see below |
//! | `Viseme/Index` (or `Viseme/LookaheadIndex`) | voice | the Lip Sync viseme with that number |
//! | Dragger handle values | pointer | a [`HandleTarget`] while the handle is `Active` |
//!
//! ## Face values
//!
//! The recording stores the Face behavior's filtered tracker values, relative to the calibrated rest pose. The
//! mapping onto [`FaceInput`] (all constants are named below and covered by tests):
//!
//! * head turn: `Orient/Y`, `Orient/X`, `Orient/Z` are radians; yaw, pitch and roll are those angles in degrees.
//! * head position: `DX` and `DY` are the head's offset in the camera frame, taken as the -1 to 1 of
//!   [`FaceInput::head_translation`]; `Scale` is the head scale.
//! * eyebrows: height ratios, 1 at rest; [`BROW_RANGE`] either way is the full -1 to 1.
//! * eyes: eyelid ratios, 1 at rest; the Face behavior shows its blink layers at [`EYE_SHUT_AT`] and below, so the
//!   eye is shut up to [`EYE_SHUT_AT`] minus half of [`EYE_RAMP`] and open from half of it above, with a short ramp
//!   between that avoids a one-frame flicker.
//! * gaze: the mean of the two eyes' pupil positions, limited to the -1 to 1 range.
//! * mouth: `MouthShape` 2 is a smile and 1 a surprised mouth (the indexes of the Face behavior's own script);
//!   otherwise the opening follows `MouthSY` and the width `MouthSX`.

use anim_core::{Affine, Point, Tick, Vec2};
use anim_lipsync::Viseme;
use charcraft_arf::{Axis, Curve, EventGraph, FaceParam, HandleSpace, HandleValue, Role, Track};
use charcraft_behaviors::{FaceInput, HandleTarget};

use crate::group::{Frame, ParamGroup};
use crate::take::{MAX_SAMPLES, Sample, Take};

/// Samples per second of a decoded take.
pub const SAMPLE_HZ: f64 = 120.0;
/// The behavior whose recorded values drive the face.
pub const FACE_BEHAVIOR: &str = "Adobe.FaceTracker2.js";
/// An eyebrow ratio this far from 1 is a full raise (or a full frown).
pub const BROW_RANGE: f64 = 0.3;
/// The eyelid ratio at and below which the Face behavior shows the blink layers.
pub const EYE_SHUT_AT: f64 = 0.45;
/// Width of the ramp between a shut and an open eye, in eyelid ratio.
pub const EYE_RAMP: f64 = 0.1;
/// `MouthSY` at which the mouth counts as closed.
pub const MOUTH_CLOSED_AT: f64 = 0.9;
/// How far `MouthSY` has to rise above [`MOUTH_CLOSED_AT`] for a fully open mouth.
pub const MOUTH_OPEN_RANGE: f64 = 0.5;
/// `MouthShape` of a surprised mouth.
pub const SHAPE_SURPRISED: f64 = 1.0;
/// `MouthShape` of a smile.
pub const SHAPE_SMILE: f64 = 2.0;
/// A held handle has `Active` at or above this.
pub const ACTIVE_AT: f64 = 0.5;

/// What the scene's take entry says about a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    /// The take number.
    pub number: u32,
    /// The display name (`take 4 - Camera Input`).
    pub name: String,
    /// Whether it plays.
    pub enabled: bool,
    /// Seconds it fades in over.
    pub fade_in: f64,
    /// Seconds it fades out over.
    pub fade_out: f64,
}

/// Where a recording lies on the scene timeline (a `TimeItem`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    /// Scene time the clip starts at; before the scene when negative.
    pub offset: f64,
    /// Seconds into the recording at which the clip starts.
    pub trim_in: f64,
    /// Seconds the clip lasts on the timeline.
    pub duration: f64,
}

/// A window cut to the scene (starting at 0 or later).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Clip {
    start: f64,
    duration: f64,
    /// Seconds into the recording at the clip's start.
    source: f64,
}

impl Window {
    fn clip(self) -> Option<Clip> {
        if ![self.offset, self.trim_in, self.duration].iter().all(|v| v.is_finite()) {
            return None;
        }
        let skipped = (-self.offset).max(0.0);
        let duration = self.duration - skipped;
        (duration > 0.0).then_some(Clip { start: self.offset.max(0.0), duration, source: self.trim_in + skipped })
    }
}

impl Clip {
    /// The grid of local times, `0, 1/HZ, ..` up to and including the clip's end.
    fn grid(&self) -> Vec<f64> {
        let count = anim_core::num::round_f64_to_u32(
            (self.duration * SAMPLE_HZ).ceil().min(f64::from(u32::try_from(MAX_SAMPLES).unwrap_or(u32::MAX).saturating_sub(2))),
        );
        (0..=count).map(|k| (f64::from(k) / SAMPLE_HZ).min(self.duration)).collect()
    }
}

fn find(tracks: &[Track], mut matches: impl FnMut(&Role) -> bool) -> Option<&Track> {
    tracks.iter().find(|t| matches(&t.role))
}

fn value(curve: Option<&Curve>, at: f64, default: f64) -> f64 {
    curve.and_then(|c| c.value_at(at)).filter(|v| v.is_finite()).unwrap_or(default)
}

fn new_take(meta: &Meta, groups: impl IntoIterator<Item = ParamGroup>, clip: &Clip) -> Take {
    let mut take = Take::new(meta.number, groups.into_iter().collect(), Tick::from_seconds_f64(clip.start), Tick::from_seconds_f64(clip.duration));
    take.name.clone_from(&meta.name);
    take.enabled = meta.enabled;
    take.blend_in = Tick::from_seconds_f64(meta.fade_in.max(0.0));
    take.blend_out = Tick::from_seconds_f64(meta.fade_out.max(0.0));
    take
}

/// The face tracker values of one moment, as the inputs the Face behavior reads.
#[must_use]
pub fn face_input(get: &dyn Fn(FaceParam) -> f64) -> FaceInput {
    let neutral = FaceInput::default();
    let brow = |ratio: f64| ((ratio - 1.0) / BROW_RANGE).clamp(-1.0, 1.0);
    let eye = |lid: f64| ((lid - (EYE_SHUT_AT - EYE_RAMP / 2.0)) / EYE_RAMP).clamp(0.0, 1.0);
    let gaze = |a: FaceParam, b: FaceParam| f64::midpoint(get(a), get(b)).clamp(-1.0, 1.0);
    let shape = get(FaceParam::MouthShape).round();
    let (open, wide, smile) = if (shape - SHAPE_SMILE).abs() < 0.5 {
        (0.0, neutral.mouth_wide, 1.0)
    } else if (shape - SHAPE_SURPRISED).abs() < 0.5 {
        (1.0, 0.25, 0.0)
    } else {
        (((get(FaceParam::MouthSy) - MOUTH_CLOSED_AT) / MOUTH_OPEN_RANGE).clamp(0.0, 1.0), (get(FaceParam::MouthSx) * 0.5).clamp(0.0, 1.0), 0.0)
    };
    FaceInput {
        head_yaw: get(FaceParam::OrientY).to_degrees(),
        head_pitch: get(FaceParam::OrientX).to_degrees(),
        head_roll: get(FaceParam::OrientZ).to_degrees(),
        head_translation: Vec2::new(get(FaceParam::Dx), get(FaceParam::Dy)),
        head_scale: get(FaceParam::Scale),
        eyebrow_left: brow(get(FaceParam::LeftEyebrow)),
        eyebrow_right: brow(get(FaceParam::RightEyebrow)),
        eye_open_left: eye(get(FaceParam::LeftEyelid)),
        eye_open_right: eye(get(FaceParam::RightEyelid)),
        gaze: Vec2::new(gaze(FaceParam::LeftEyeGazeX, FaceParam::RightEyeGazeX), gaze(FaceParam::LeftEyeGazeY, FaceParam::RightEyeGazeY)),
        mouth_open: open,
        mouth_wide: wide,
        smile,
    }
    .sanitized()
}

/// The neutral value of a face parameter (what a missing channel reads as).
#[must_use]
pub const fn neutral_value(param: FaceParam) -> f64 {
    match param {
        FaceParam::Dx
        | FaceParam::Dy
        | FaceParam::Dz
        | FaceParam::OrientX
        | FaceParam::OrientY
        | FaceParam::OrientZ
        | FaceParam::MouthDx
        | FaceParam::MouthDy
        | FaceParam::MouthShape
        | FaceParam::LeftEyeGazeX
        | FaceParam::LeftEyeGazeY
        | FaceParam::RightEyeGazeX
        | FaceParam::RightEyeGazeY => 0.0,
        FaceParam::LeftEyebrow | FaceParam::RightEyebrow | FaceParam::LeftEyelid | FaceParam::RightEyelid | FaceParam::MouthSx | FaceParam::Scale => 1.0,
        // Closed at rest.
        FaceParam::MouthSy => MOUTH_CLOSED_AT,
    }
}

/// The behavior instances (`TIB47`) that recorded face values in a graph, in order.
#[must_use]
pub fn face_instances(tracks: &[Track]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for t in tracks {
        if let Role::Face { behavior, instance, .. } = &t.role
            && behavior == FACE_BEHAVIOR
            && !found.contains(instance)
        {
            found.push(instance.clone());
        }
    }
    found
}

/// A face take from the values of behavior instance `instance` (the only one when `None`).
///
/// `None` when the recording holds no face values of the instance or the window is empty.
#[must_use]
pub fn face_take(graph: &EventGraph, instance: Option<&str>, meta: &Meta, window: Window) -> Option<Take> {
    let clip = window.clip()?;
    let tracks = graph.tracks();
    let wanted = instance.map(str::to_owned).or_else(|| face_instances(&tracks).into_iter().next())?;
    let curves: Vec<(FaceParam, Option<&Curve>)> = FaceParam::ALL
        .iter()
        .map(|&p| {
            (
                p,
                find(&tracks, |r| matches!(r, Role::Face { behavior, instance, param } if behavior == FACE_BEHAVIOR && *instance == wanted && *param == p))
                    .map(|t| &t.curve),
            )
        })
        .collect();
    if curves.iter().all(|(_, c)| c.is_none()) {
        return None;
    }
    let mut take = new_take(meta, ParamGroup::FACE, &clip);
    for local in clip.grid() {
        let at = graph.start + clip.source + local;
        let get = |p: FaceParam| value(curves.iter().find(|(q, _)| *q == p).and_then(|(_, c)| *c), at, neutral_value(p));
        take.samples.push(Sample { t: Tick::from_seconds_f64(local), frame: Frame { face: Some(face_input(&get)), ..Frame::default() } });
    }
    Some(take)
}

/// The viseme a `Viseme/Index` value names. Negative numbers (before the first word) are the neutral mouth.
#[must_use]
pub fn viseme_of(index: f64) -> Viseme {
    if !index.is_finite() || index < 0.0 {
        return Viseme::Neutral;
    }
    usize::try_from(anim_core::num::round_f64_to_u32(index)).ok().and_then(|i| Viseme::ALL.get(i).copied()).unwrap_or(Viseme::Neutral)
}

/// A voice take from `Viseme/Index`, or `Viseme/LookaheadIndex` when that is all the recording has (the transcript
/// recordings).
#[must_use]
pub fn voice_take(graph: &EventGraph, meta: &Meta, window: Window) -> Option<Take> {
    let clip = window.clip()?;
    let tracks = graph.tracks();
    let track = find(&tracks, |r| *r == Role::VisemeIndex).or_else(|| find(&tracks, |r| *r == Role::VisemeLookahead))?;
    let enabled = find(&tracks, |r| *r == Role::VisemeInputEnabled).map(|t| &t.curve);
    let origin = graph.start + clip.source;
    let mut times = clip.grid();
    // A mouth shape changes at the moment the curve jumps.
    for s in &track.curve.segments {
        let local = s.start() - origin;
        if s.start().to_bits() == s.end().to_bits() && s.v[0].to_bits() != s.v[3].to_bits() && local > 0.0 && local < clip.duration {
            times.push(local);
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| a.to_bits() == b.to_bits());
    let mut take = new_take(meta, [ParamGroup::Voice], &clip);
    let mut last = None;
    for local in times {
        let at = origin + local;
        let off = enabled.is_some_and(|c| c.value_at(at).is_some_and(|v| v < ACTIVE_AT));
        let viseme = if off { None } else { Some(viseme_of(value(Some(&track.curve), at, -1.0))) };
        if last == Some(viseme) {
            continue;
        }
        last = Some(viseme);
        take.samples.push(Sample { t: Tick::from_seconds_f64(local), frame: Frame { viseme, ..Frame::default() } });
    }
    Some(take)
}

/// A draggable handle of the recording: its address in the recording and the rig handle that plays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleBinding {
    /// The behavior instance (`TIB45`).
    pub instance: String,
    /// The address in the recording (`HandleTarget/../B2/B8/B4:/Draggable/`).
    pub group: String,
    /// The rig handle's key (`Arm Right/Draggable`).
    pub key: String,
}

/// The distinct draggable handles a recording holds: `(instance, group)` pairs in order.
#[must_use]
pub fn handle_groups(tracks: &[Track]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for t in tracks {
        if let Role::Handle { instance, group, .. } = &t.role {
            let pair = (instance.clone(), group.clone());
            if !found.contains(&pair) {
                found.push(pair);
            }
        }
    }
    found
}

/// A pointer take from one draggable handle. `to_puppet` maps the recorded scene position (pixels from the middle
/// of the frame, y down) into the puppet's artwork space.
#[must_use]
pub fn handle_take(graph: &EventGraph, binding: &HandleBinding, to_puppet: &Affine, meta: &Meta, window: Window) -> Option<Take> {
    let clip = window.clip()?;
    let tracks = graph.tracks();
    let pick = |v: HandleValue| {
        find(&tracks, |r| matches!(r, Role::Handle { instance, group, value } if *instance == binding.instance && *group == binding.group && *value == v))
            .map(|t| &t.curve)
    };
    let active = pick(HandleValue::Active)?;
    let x = pick(HandleValue::Translation(HandleSpace::Scene, Axis::X))?;
    let y = pick(HandleValue::Translation(HandleSpace::Scene, Axis::Y))?;
    let mut take = new_take(meta, [ParamGroup::Pointer], &clip);
    let mut was_held = false;
    for local in clip.grid() {
        let at = graph.start + clip.source + local;
        let held = value(Some(active), at, 0.0) >= ACTIVE_AT;
        if !held && !was_held {
            continue;
        }
        was_held = held;
        // Before the first sample a take holds that sample, so the free time before the grab needs a sample of its own.
        if take.samples.is_empty() && local > 0.0 {
            take.samples.push(Sample { t: Tick::ZERO, frame: Frame::default() });
        }
        let handles = if held {
            let position = *to_puppet * Point::new(value(Some(x), at, 0.0), value(Some(y), at, 0.0));
            vec![HandleTarget { handle: binding.key.clone(), position, held: true }]
        } else {
            Vec::new()
        };
        take.samples.push(Sample { t: Tick::from_seconds_f64(local), frame: Frame { handles, ..Frame::default() } });
    }
    // A handle held from the first sample would otherwise lose the sample that lets go of it at the end.
    if take.samples.is_empty() {
        take.samples.push(Sample { t: Tick::ZERO, frame: Frame::default() });
    }
    Some(take)
}

#[cfg(test)]
mod tests {
    use charcraft_arf::{face_channel, handle_channel};

    use super::*;

    fn meta() -> Meta {
        Meta { number: 3, name: "take 3 - Camera Input".into(), enabled: true, fade_in: 0.0, fade_out: 0.5 }
    }

    fn graph(start: f64, end: f64, channels: &[(String, Curve)]) -> EventGraph {
        EventGraph::from_curves(start, end, channels)
    }

    fn hold(name: &str, value: f64, start: f64, end: f64) -> (String, Curve) {
        (name.to_owned(), Curve::from_samples(&[(start, value), (end, value)], false))
    }

    fn face_graph(values: &[(FaceParam, f64)]) -> EventGraph {
        graph(1000.0, 1004.0, &values.iter().map(|&(p, v)| hold(&face_channel(FACE_BEHAVIOR, "TIB7", p), v, 1000.0, 1004.0)).collect::<Vec<_>>())
    }

    const WINDOW: Window = Window { offset: 0.0, trim_in: 0.0, duration: 4.0 };

    #[test]
    fn a_face_at_rest_reads_as_a_neutral_face() {
        let rest: Vec<(FaceParam, f64)> = FaceParam::ALL.iter().map(|&p| (p, neutral_value(p))).collect();
        let take = face_take(&face_graph(&rest), Some("TIB7"), &meta(), WINDOW).unwrap();
        let face = take.samples[10].frame.face.unwrap();
        assert_eq!(face, FaceInput { eye_open_left: 1.0, eye_open_right: 1.0, mouth_wide: 0.5, ..FaceInput::default() });
        assert_eq!(take.groups.len(), 4);
        assert_eq!(take.name, "take 3 - Camera Input");
        assert_eq!(take.blend_out, Tick::from_seconds_f64(0.5));
    }

    #[test]
    fn values_map_onto_the_face() {
        let g = face_graph(&[
            (FaceParam::OrientY, 0.5),
            (FaceParam::OrientX, -0.25),
            (FaceParam::OrientZ, 0.1),
            (FaceParam::Dx, 0.3),
            (FaceParam::Dy, -0.2),
            (FaceParam::Scale, 1.2),
            (FaceParam::LeftEyebrow, 1.15),
            (FaceParam::RightEyebrow, 0.85),
            (FaceParam::LeftEyelid, 0.3),
            (FaceParam::RightEyelid, 0.9),
            (FaceParam::LeftEyeGazeX, 0.2),
            (FaceParam::RightEyeGazeX, 0.4),
            (FaceParam::LeftEyeGazeY, 12.0),
            (FaceParam::RightEyeGazeY, 12.0),
            (FaceParam::MouthSy, 1.15),
        ]);
        let face = face_take(&g, None, &meta(), WINDOW).unwrap().samples[3].frame.face.unwrap();
        assert!((face.head_yaw - 0.5_f64.to_degrees()).abs() < 1e-9);
        assert!((face.head_pitch + 0.25_f64.to_degrees()).abs() < 1e-9);
        assert!((face.head_roll - 0.1_f64.to_degrees()).abs() < 1e-9);
        assert_eq!(face.head_translation, Vec2::new(0.3, -0.2));
        assert!((face.head_scale - 1.2).abs() < 1e-12);
        assert!((face.eyebrow_left - 0.5).abs() < 1e-9 && (face.eyebrow_right + 0.5).abs() < 1e-9);
        assert_eq!((face.eye_open_left, face.eye_open_right), (0.0, 1.0), "0.3 is shut, 0.9 is open");
        assert!((face.gaze.x - 0.3).abs() < 1e-9 && face.gaze.y == 1.0, "an outlier is limited: {:?}", face.gaze);
        assert!((face.mouth_open - 0.5).abs() < 1e-9);
    }

    #[test]
    fn the_eye_shuts_at_the_blink_threshold() {
        let open = |lid: f64| face_input(&|p| if p == FaceParam::LeftEyelid { lid } else { neutral_value(p) }).eye_open_left;
        assert_eq!(open(0.40), 0.0);
        assert_eq!(open(0.44), 0.0 + (0.44 - 0.40) / 0.1);
        assert!(open(0.45) < 0.6 && open(0.45) > 0.4);
        assert!((open(0.5) - 1.0).abs() < 1e-12);
        assert!((open(2.5) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn mouth_shapes_are_smile_and_surprise() {
        let shaped = |shape: f64| face_input(&|p| if p == FaceParam::MouthShape { shape } else { neutral_value(p) });
        assert_eq!(shaped(2.0).smile, 1.0);
        assert_eq!(shaped(2.0).mouth_open, 0.0);
        assert_eq!(shaped(1.0).mouth_open, 1.0);
        assert_eq!(shaped(0.0).smile, 0.0);
    }

    #[test]
    fn a_clip_starting_before_the_scene_is_cut() {
        let g = graph(500.0, 510.0, &[hold("Viseme/Index", 3.0, 500.0, 510.0)]);
        // Placed 2 s before the scene start, 6 s long: 4 s remain, from 2 s into the recording.
        let take = voice_take(&g, &meta(), Window { offset: -2.0, trim_in: 0.0, duration: 6.0 }).unwrap();
        assert_eq!((take.start, take.duration), (Tick::ZERO, Tick::from_seconds_f64(4.0)));
        assert!(voice_take(&g, &meta(), Window { offset: -6.0, trim_in: 0.0, duration: 6.0 }).is_none(), "nothing left");
        assert!(voice_take(&g, &meta(), Window { offset: f64::NAN, trim_in: 0.0, duration: 1.0 }).is_none());
    }

    #[test]
    fn the_voice_changes_mouth_at_the_jump() {
        let steps = Curve::from_samples(&[(0.0, -2.0), (1.0003, 1.0), (2.0, 7.0), (3.0, 7.0)], true);
        let g = graph(0.0, 3.0, &[("Viseme/Index".to_owned(), steps)]);
        let take = voice_take(&g, &meta(), Window { offset: 0.0, trim_in: 0.0, duration: 3.0 }).unwrap();
        let seen: Vec<(f64, Option<Viseme>)> = take.samples.iter().map(|s| (s.t.seconds(), s.frame.viseme)).collect();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(seen[0].1, Some(Viseme::Neutral), "-2 is the neutral mouth");
        assert!((seen[1].0 - 1.0003).abs() < 1e-6 && seen[1].1 == Some(Viseme::Ah), "{seen:?}");
        assert!((seen[2].0 - 2.0).abs() < 1e-6 && seen[2].1 == Some(Viseme::Oh));
        assert_eq!(take.frame_at(Tick::from_seconds_f64(1.5)).unwrap().viseme, Some(Viseme::Ah));
    }

    #[test]
    fn the_lookahead_stands_in_for_the_index_and_trim_shifts_the_content() {
        let steps = Curve::from_samples(&[(0.0, 2.0), (4.0, 9.0), (8.0, 9.0)], true);
        let g = graph(0.0, 8.0, &[("Viseme/LookaheadIndex".to_owned(), steps)]);
        // The clip shows the recording from 3 s on, so the change at 4 s lands 1 s into the clip.
        let take = voice_take(&g, &meta(), Window { offset: 0.0, trim_in: 3.0, duration: 5.0 }).unwrap();
        assert_eq!(take.frame_at(Tick::from_seconds_f64(0.5)).unwrap().viseme, Some(Viseme::D));
        assert_eq!(take.frame_at(Tick::from_seconds_f64(1.5)).unwrap().viseme, Some(Viseme::S));
        // A negative trim shows a lead-in of the first value.
        let late = voice_take(&g, &meta(), Window { offset: 0.0, trim_in: -2.0, duration: 8.0 }).unwrap();
        assert_eq!(late.frame_at(Tick::from_seconds_f64(1.0)).unwrap().viseme, Some(Viseme::D));
        assert_eq!(late.frame_at(Tick::from_seconds_f64(5.0)).unwrap().viseme, Some(Viseme::D), "4 s of the recording is 6 s of the clip");
        assert_eq!(late.frame_at(Tick::from_seconds_f64(6.5)).unwrap().viseme, Some(Viseme::S));
    }

    #[test]
    fn a_dragged_handle_becomes_a_target_in_puppet_space() {
        let group = "HandleTarget/../B2/B8/B4:/Draggable/";
        let name = |v| handle_channel("TIB45", group, v);
        let g = graph(
            100.0,
            104.0,
            &[
                (name(HandleValue::Active), Curve::from_samples(&[(100.0, 0.0), (101.0, 1.0), (103.0, 0.0), (104.0, 0.0)], true)),
                (name(HandleValue::Translation(HandleSpace::Scene, Axis::X)), Curve::from_samples(&[(100.0, -100.0), (104.0, -100.0)], false)),
                (name(HandleValue::Translation(HandleSpace::Scene, Axis::Y)), Curve::from_samples(&[(100.0, 50.0), (104.0, 50.0)], false)),
            ],
        );
        let binding = HandleBinding { instance: "TIB45".into(), group: group.into(), key: "Arm Right/Draggable".into() };
        // Puppet pixels = 2 x (scene pixels from the middle) + (10, 20).
        let to_puppet = Affine::translate(Vec2::new(10.0, 20.0)) * Affine::scale(2.0);
        let take = handle_take(&g, &binding, &to_puppet, &meta(), Window { offset: 0.0, trim_in: 0.0, duration: 4.0 }).unwrap();
        assert!(take.frame_at(Tick::from_seconds_f64(0.5)).unwrap().handles.is_empty(), "not held yet");
        let held = take.frame_at(Tick::from_seconds_f64(1.5)).unwrap();
        assert_eq!(held.handles.len(), 1);
        assert_eq!(held.handles[0].handle, "Arm Right/Draggable");
        assert!((held.handles[0].position.x + 190.0).abs() < 1e-6 && (held.handles[0].position.y - 120.0).abs() < 1e-6, "{:?}", held.handles[0].position);
        assert!(take.frame_at(Tick::from_seconds_f64(3.5)).unwrap().handles.is_empty(), "let go");
        assert!(take.groups.contains(&ParamGroup::Pointer));
    }

    #[test]
    fn viseme_numbers_follow_lip_syncs_order() {
        for (i, v) in Viseme::ALL.iter().enumerate() {
            assert_eq!(viseme_of(f64::from(u32::try_from(i).unwrap())), *v);
        }
        assert_eq!(viseme_of(12.0), Viseme::Neutral);
        assert_eq!(viseme_of(f64::NAN), Viseme::Neutral);
        assert_eq!(viseme_of(-1.0), Viseme::Neutral);
    }

    #[test]
    fn a_recording_without_the_values_gives_no_take() {
        let g = graph(0.0, 1.0, &[hold("Audio/Amplitude", 0.1, 0.0, 1.0)]);
        let window = Window { offset: 0.0, trim_in: 0.0, duration: 1.0 };
        assert!(face_take(&g, None, &meta(), window).is_none());
        assert!(voice_take(&g, &meta(), window).is_none());
        assert!(face_take(&face_graph(&[(FaceParam::Dx, 0.1)]), Some("TIB99"), &meta(), WINDOW).is_none(), "another instance's values");
    }
}
