//! The scene model: size, rate and length, puppet tracks with stacked takes, and audio tracks.
//!
//! A scene is plain serde data. It never holds puppets or sound (tracks name their puppet and clips name their
//! media), so cloning it for undo is cheap and a scene file stays small.

use anim_core::{Affine, FrameRate, Rgba8, Size, Tick, TimeRange, Vec2};
use charcraft_behaviors::api::StackOp;
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::error::{Error, Result};
use crate::keyframes::{Lane, sanitize_lanes};
use crate::take::Take;

/// The placement fields keyframes can animate.
pub const PLACEMENT_TARGETS: [&str; 8] = ["position_x", "position_y", "anchor_x", "anchor_y", "scale_x", "scale_y", "rotation", "opacity"];

/// The slot and parameter of a `behavior.<slot>.<parameter>` lane target.
#[must_use]
pub fn behavior_target(target: &str) -> Option<(usize, &str)> {
    let rest = target.strip_prefix("behavior.")?;
    let (slot, name) = rest.split_once('.')?;
    let slot = slot.parse().ok()?;
    (!name.is_empty()).then_some((slot, name))
}

/// Most puppet tracks.
pub const MAX_TRACKS: usize = 256;
/// Most takes on one track.
pub const MAX_TAKES: usize = 4096;
/// Most audio tracks.
pub const MAX_AUDIO_TRACKS: usize = 64;
/// Most clips on one audio track.
pub const MAX_CLIPS: usize = 4096;
/// Most behavior edits one track keeps.
pub const MAX_OPS: usize = 1024;
/// Most frames a scene may have.
pub const MAX_FRAMES: i64 = 1_000_000;
/// Largest stage side in pixels.
pub const MAX_SIDE: f64 = 16_384.0;
/// Longest name kept.
pub const MAX_NAME: usize = 256;

/// Where a track puts its puppet, as in Character Animator's Transform behavior.
///
/// The puppet's `anchor` lands on `position`, then the puppet is scaled and turned about it. Scene space is
/// centred on the middle of the frame, y down.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    /// Scene position of the anchor point.
    pub position: Vec2,
    /// The point of the puppet (relative to its origin) that sits on `position`.
    pub anchor: Vec2,
    /// Scale factors (1 = 100 %).
    pub scale: Vec2,
    /// Rotation in degrees, clockwise on screen.
    pub rotation: f64,
    /// Opacity, 0 to 1.
    pub opacity: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Self { position: Vec2::ZERO, anchor: Vec2::ZERO, scale: Vec2::new(1.0, 1.0), rotation: 0.0, opacity: 1.0 }
    }
}

impl Placement {
    /// Maps puppet pixels to scene pixels. `centre` is the middle of the frame, `skin` the puppet-space position
    /// of pixel (0, 0) and `handle` the puppet's origin handle.
    #[must_use]
    pub fn matrix(&self, centre: Vec2, skin: Vec2, handle: Vec2) -> Affine {
        Affine::translate(centre + handle + self.position)
            * Affine::rotate(self.rotation.to_radians())
            * Affine::scale_non_uniform(self.scale.x, self.scale.y)
            * Affine::translate(skin - handle - self.anchor)
    }

    /// Sets the field a lane target names (see [`PLACEMENT_TARGETS`]); other targets are ignored.
    pub fn set_field(&mut self, target: &str, v: f64) {
        match target {
            "position_x" => self.position.x = v,
            "position_y" => self.position.y = v,
            "anchor_x" => self.anchor.x = v,
            "anchor_y" => self.anchor.y = v,
            "scale_x" => self.scale.x = v,
            "scale_y" => self.scale.y = v,
            "rotation" => self.rotation = v,
            "opacity" => self.opacity = v,
            _ => {}
        }
    }

    /// The value of the field a lane target names.
    #[must_use]
    pub fn field(&self, target: &str) -> Option<f64> {
        Some(match target {
            "position_x" => self.position.x,
            "position_y" => self.position.y,
            "anchor_x" => self.anchor.x,
            "anchor_y" => self.anchor.y,
            "scale_x" => self.scale.x,
            "scale_y" => self.scale.y,
            "rotation" => self.rotation,
            "opacity" => self.opacity,
            _ => return None,
        })
    }

    /// Repairs non-finite numbers.
    pub fn sanitize(&mut self) {
        let fix = |v: f64, d: f64| if v.is_finite() { v.clamp(-1e7, 1e7) } else { d };
        self.position = Vec2::new(fix(self.position.x, 0.0), fix(self.position.y, 0.0));
        self.anchor = Vec2::new(fix(self.anchor.x, 0.0), fix(self.anchor.y, 0.0));
        self.scale = Vec2::new(fix(self.scale.x, 1.0), fix(self.scale.y, 1.0));
        self.rotation = fix(self.rotation, 0.0);
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.0, 1.0) } else { 1.0 };
    }
}

/// A puppet on the stage with the takes recorded for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PuppetTrack {
    /// The track's name.
    pub name: String,
    /// Which puppet plays here: the key in the cast (a project path such as `ProjectItems/Hero`).
    pub puppet: String,
    /// Whether the track plays and draws.
    pub enabled: bool,
    /// Where the puppet stands.
    pub placement: Placement,
    /// Edits laid over the puppet's own behavior stack (set a parameter, switch a behavior off, ...).
    pub behaviors: Vec<StackOp>,
    /// The takes, lowest first: a later take overrides an earlier one wherever they overlap.
    pub takes: Vec<Take>,
    /// A locked track cannot be edited from the timeline (recording, takes, placement).
    #[serde(default)]
    pub locked: bool,
    /// Keyframed parameters: placement fields (`position_x`, ...) and behavior parameters (`behavior.<slot>.<name>`).
    #[serde(default)]
    pub lanes: Vec<Lane>,
}

impl PuppetTrack {
    /// A track with no takes.
    #[must_use]
    pub fn new(name: &str, puppet: &str) -> Self {
        Self {
            name: name.to_owned(),
            puppet: puppet.to_owned(),
            enabled: true,
            placement: Placement::default(),
            behaviors: Vec::new(),
            takes: Vec::new(),
            locked: false,
            lanes: Vec::new(),
        }
    }

    /// The placement at scene time `t`: the track's own placement with its keyframed fields applied.
    #[must_use]
    pub fn placement_at(&self, t: Tick) -> Placement {
        let mut placement = self.placement;
        for lane in &self.lanes {
            if let Some(v) = lane.value_at(t) {
                placement.set_field(&lane.target, v);
            }
        }
        placement.sanitize();
        placement
    }

    /// The scene time the last take ends.
    #[must_use]
    pub fn end(&self) -> Tick {
        self.takes.iter().map(Take::end).max().unwrap_or(Tick::ZERO)
    }
}

/// A sound placed on an audio track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    /// The clip's name.
    pub name: String,
    /// The media it plays: a key in the [`MediaStore`](crate::MediaStore).
    pub media: String,
    /// Scene time its first used frame sounds.
    pub start: Tick,
    /// How far into the media the clip starts.
    pub trim_in: Tick,
    /// How much of the media plays (`None`: to the end).
    pub duration: Option<Tick>,
    /// Linear gain.
    pub gain: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
    /// Whether it sounds.
    pub enabled: bool,
    /// The mouth shapes found in the whole sound (times from the start of the media), so a lip sync take can be
    /// made from the clip without the sound itself. Empty until the sound has been analysed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visemes: Vec<anim_lipsync::VisemeKey>,
}

/// Most mouth shapes one clip keeps.
pub const MAX_VISEMES: usize = 2_000_000;

impl AudioClip {
    /// A clip playing all of `media` from `start`.
    #[must_use]
    pub fn new(name: &str, media: &str, start: Tick) -> Self {
        Self {
            name: name.to_owned(),
            media: media.to_owned(),
            start,
            trim_in: Tick::ZERO,
            duration: None,
            gain: 1.0,
            pan: 0.0,
            enabled: true,
            visemes: Vec::new(),
        }
    }
}

/// A track of sounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioTrack {
    /// The track's name.
    pub name: String,
    /// Whether it sounds.
    pub enabled: bool,
    /// Track gain.
    pub gain: f32,
    /// The clips.
    pub clips: Vec<AudioClip>,
}

impl AudioTrack {
    /// An empty track.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self { name: name.to_owned(), enabled: true, gain: 1.0, clips: Vec::new() }
    }
}

/// A scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// The scene's name.
    pub name: String,
    /// Frames per second, exact.
    pub fps: FrameRate,
    /// Stage size in pixels.
    pub size: Size,
    /// Stage background (`None`: transparent).
    pub background: Option<Rgba8>,
    /// Length of the scene.
    pub duration: Tick,
    /// Seed of the random streams of the behaviors; track `n` gets `seed + n`, so the same scene always plays the
    /// same blinks.
    pub seed: u64,
    /// The next take number.
    pub take_counter: u32,
    /// The puppets.
    pub tracks: Vec<PuppetTrack>,
    /// The sounds.
    pub audio: Vec<AudioTrack>,
    /// Where the camera looks.
    #[serde(default)]
    pub camera: Camera,
}

impl Scene {
    /// An empty scene.
    #[must_use]
    pub fn new(name: &str, size: Size, fps: FrameRate, duration: Tick) -> Self {
        Self {
            name: name.to_owned(),
            fps,
            size,
            background: Some(Rgba8::WHITE),
            duration,
            seed: 1,
            take_counter: 1,
            tracks: Vec::new(),
            audio: Vec::new(),
            camera: Camera::default(),
        }
    }

    /// Time of frame `f`.
    #[must_use]
    pub fn frame_time(&self, frame: i64) -> Tick {
        let num = i128::from(self.fps.num.max(1));
        let den = i128::from(self.fps.den.max(1));
        let ticks = i128::from(frame).saturating_mul(i128::from(anim_core::TICKS_PER_SECOND)).saturating_mul(den).div_euclid(num);
        Tick(i64::try_from(ticks).unwrap_or(if ticks < 0 { i64::MIN } else { i64::MAX }))
    }

    /// The frame containing time `t` (floor).
    #[must_use]
    pub fn frame_at(&self, t: Tick) -> i64 {
        let num = i128::from(self.fps.num.max(1));
        let den = i128::from(self.fps.den.max(1));
        let frames = i128::from(t.0).saturating_mul(num).div_euclid(i128::from(anim_core::TICKS_PER_SECOND).saturating_mul(den));
        i64::try_from(frames).unwrap_or(0)
    }

    /// Number of frames the scene lasts (the last may be partial).
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        if self.duration.0 <= 0 {
            return 0;
        }
        let whole = self.frame_at(self.duration);
        let frames = if self.frame_time(whole).0 < self.duration.0 { whole.saturating_add(1) } else { whole };
        u32::try_from(frames.clamp(0, MAX_FRAMES)).unwrap_or(0)
    }

    /// The scene's time range.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        TimeRange { start: Tick::ZERO, duration: self.duration }
    }

    /// Frames per second as a float, for exporters.
    #[must_use]
    pub fn fps_f64(&self) -> f64 {
        anim_core::num::i64_to_f64(self.fps.num.max(1)) / anim_core::num::i64_to_f64(self.fps.den.max(1))
    }

    /// The puppet track `index`.
    ///
    /// # Errors
    /// [`Error::NoTrack`].
    pub fn track(&self, index: usize) -> Result<&PuppetTrack> {
        self.tracks.get(index).ok_or(Error::NoTrack(index))
    }

    /// The puppet track `index`, to edit.
    ///
    /// # Errors
    /// [`Error::NoTrack`].
    pub fn track_mut(&mut self, index: usize) -> Result<&mut PuppetTrack> {
        self.tracks.get_mut(index).ok_or(Error::NoTrack(index))
    }

    /// Adds a puppet track and returns its index.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn add_track(&mut self, track: PuppetTrack) -> Result<usize> {
        if self.tracks.len() >= MAX_TRACKS {
            return Err(Error::TooLarge { what: "the track list", limit: MAX_TRACKS });
        }
        self.tracks.push(track);
        Ok(self.tracks.len().saturating_sub(1))
    }

    /// Puts a finished take on top of a track, numbers and names it, and grows the scene to hold it. Returns the
    /// take's index.
    ///
    /// # Errors
    /// [`Error::NoTrack`], [`Error::TooLarge`].
    pub fn add_take(&mut self, track: usize, mut take: Take) -> Result<usize> {
        let number = self.take_counter;
        let target = self.tracks.get_mut(track).ok_or(Error::NoTrack(track))?;
        if target.takes.len() >= MAX_TAKES {
            return Err(Error::TooLarge { what: "the take list", limit: MAX_TAKES });
        }
        if take.number == 0 {
            take.name = Take::default_name(number, &take.groups);
        }
        take.number = number;
        let end = take.end();
        target.takes.push(take);
        let index = target.takes.len().saturating_sub(1);
        self.take_counter = number.saturating_add(1);
        if end.0 > self.duration.0 {
            self.duration = end;
        }
        Ok(index)
    }

    /// Adds a clip to audio track `track`, creating audio tracks up to it. Returns the clip's index.
    ///
    /// # Errors
    /// [`Error::TooLarge`].
    pub fn add_clip(&mut self, track: usize, clip: AudioClip) -> Result<usize> {
        if track >= MAX_AUDIO_TRACKS {
            return Err(Error::TooLarge { what: "the audio track list", limit: MAX_AUDIO_TRACKS });
        }
        while self.audio.len() <= track {
            let name = format!("Audio {}", self.audio.len().saturating_add(1));
            self.audio.push(AudioTrack::new(&name));
        }
        let target = self.audio.get_mut(track).ok_or(Error::NoAudio { what: "track", index: track })?;
        if target.clips.len() >= MAX_CLIPS {
            return Err(Error::TooLarge { what: "the clip list", limit: MAX_CLIPS });
        }
        target.clips.push(clip);
        Ok(target.clips.len().saturating_sub(1))
    }

    /// Checks everything a file could get wrong and repairs what can be repaired: rates, sizes, lengths, takes,
    /// placements and names. Run on every scene that comes from outside.
    ///
    /// # Errors
    /// [`Error::Invalid`] or [`Error::TooLarge`] for what cannot be repaired.
    pub fn sanitize(&mut self) -> Result<()> {
        if self.fps.num <= 0 || self.fps.den <= 0 || self.fps.num > 1_000_000 || self.fps.den > 1_000_000 {
            return Err(Error::invalid(format!("frame rate {}/{} is out of range", self.fps.num, self.fps.den)));
        }
        self.fps = FrameRate::new(self.fps.num, self.fps.den);
        let side_ok = |v: f64| v.is_finite() && (1.0..=MAX_SIDE).contains(&v);
        if !side_ok(self.size.width) || !side_ok(self.size.height) {
            return Err(Error::invalid(format!("stage size {}x{} is out of range", self.size.width, self.size.height)));
        }
        if self.duration.0 < 0 {
            return Err(Error::invalid("negative duration"));
        }
        if i64::from(self.frame_count()) >= MAX_FRAMES && self.frame_time(MAX_FRAMES).0 < self.duration.0 {
            return Err(Error::TooLarge { what: "the scene length in frames", limit: usize::try_from(MAX_FRAMES).unwrap_or(usize::MAX) });
        }
        if self.tracks.len() > MAX_TRACKS || self.audio.len() > MAX_AUDIO_TRACKS {
            return Err(Error::TooLarge { what: "the track list", limit: MAX_TRACKS });
        }
        truncate_name(&mut self.name);
        for track in &mut self.tracks {
            truncate_name(&mut track.name);
            truncate_name(&mut track.puppet);
            track.placement.sanitize();
            sanitize_lanes(&mut track.lanes)?;
            track.lanes.retain(|l| PLACEMENT_TARGETS.contains(&l.target.as_str()) || behavior_target(&l.target).is_some());
            if track.takes.len() > MAX_TAKES || track.behaviors.len() > MAX_OPS {
                return Err(Error::TooLarge { what: "a track", limit: MAX_TAKES });
            }
            for take in &mut track.takes {
                truncate_name(&mut take.name);
                take.sanitize()?;
            }
        }
        for track in &mut self.audio {
            truncate_name(&mut track.name);
            if track.clips.len() > MAX_CLIPS {
                return Err(Error::TooLarge { what: "the clip list", limit: MAX_CLIPS });
            }
            track.gain = finite_or(track.gain, 1.0);
            for clip in &mut track.clips {
                truncate_name(&mut clip.name);
                truncate_name(&mut clip.media);
                clip.gain = finite_or(clip.gain, 1.0);
                clip.pan = finite_or(clip.pan, 0.0).clamp(-1.0, 1.0);
                clip.trim_in = Tick(clip.trim_in.0.max(0));
                clip.duration = clip.duration.map(|d| Tick(d.0.max(0)));
                if clip.visemes.len() > MAX_VISEMES {
                    return Err(Error::TooLarge { what: "a clip's mouth shapes", limit: MAX_VISEMES });
                }
                clip.visemes.sort_by_key(|k| k.start);
            }
        }
        self.camera.sanitize()?;
        Ok(())
    }

    /// Moves puppet track `from` to position `to` (the order is the stacking order: later tracks draw on top).
    /// The camera's followed track and every track index kept elsewhere follow the move.
    ///
    /// # Errors
    /// [`Error::NoTrack`].
    pub fn move_track(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.tracks.len() {
            return Err(Error::NoTrack(from));
        }
        if to >= self.tracks.len() {
            return Err(Error::NoTrack(to));
        }
        let track = self.tracks.remove(from);
        self.tracks.insert(to, track);
        if let Some(follow) = self.camera.zoom_on_face.as_mut() {
            follow.track = moved_index(follow.track, from, to);
        }
        Ok(())
    }

    /// Splits take `take` of `track` at scene time `at` into two. Returns the index of the second part.
    ///
    /// # Errors
    /// [`Error::NoTrack`], [`Error::NoTake`], [`Error::Invalid`] when `at` is not inside the take,
    /// [`Error::TooLarge`].
    pub fn split_take(&mut self, track: usize, take: usize, at: Tick) -> Result<usize> {
        let number = self.take_counter;
        let target = self.tracks.get_mut(track).ok_or(Error::NoTrack(track))?;
        if target.takes.len() >= MAX_TAKES {
            return Err(Error::TooLarge { what: "the take list", limit: MAX_TAKES });
        }
        let first = target.takes.get_mut(take).ok_or(Error::NoTake { track, take })?;
        let mut second = first.split_at(at).ok_or_else(|| Error::invalid("the split time must lie inside the take"))?;
        second.number = number;
        second.name = format!("{} (2)", first.name);
        let index = take.saturating_add(1);
        target.takes.insert(index, second);
        self.take_counter = number.saturating_add(1);
        Ok(index)
    }

    /// A rough size in bytes, for undo budgets.
    #[must_use]
    pub fn size_hint(&self) -> usize {
        let samples: usize = self.tracks.iter().flat_map(|t| &t.takes).map(|t| t.samples.len()).sum();
        samples.saturating_mul(160).saturating_add(4096)
    }
}

/// Where index `i` ends up when the item at `from` moves to `to`.
const fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i.saturating_sub(1)
    } else if to < from && i >= to && i < from {
        i.saturating_add(1)
    } else {
        i
    }
}

const fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

fn truncate_name(name: &mut String) {
    if name.chars().count() > MAX_NAME {
        *name = name.chars().take(MAX_NAME).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::ParamGroup;

    fn scene() -> Scene {
        Scene::new("test", Size::new(1920.0, 1080.0), FrameRate::FPS_30, Tick::from_seconds_f64(10.0))
    }

    #[test]
    fn frame_times_are_exact() {
        let s = scene();
        assert_eq!(s.frame_time(0), Tick::ZERO);
        assert_eq!(s.frame_time(30), Tick::from_seconds_f64(1.0));
        assert_eq!(s.frame_at(Tick::from_seconds_f64(1.0)), 30);
        assert_eq!(s.frame_count(), 300);
        let mut ntsc = scene();
        ntsc.fps = FrameRate::FPS_29_97;
        for f in [0, 1, 29, 30, 1000, 107_892] {
            assert_eq!(ntsc.frame_at(ntsc.frame_time(f)), f, "frame {f}");
        }
        assert!((ntsc.fps_f64() - 29.97).abs() < 0.001);
    }

    #[test]
    fn a_partial_last_frame_counts() {
        let mut s = scene();
        s.duration = Tick::from_seconds_f64(10.01);
        assert_eq!(s.frame_count(), 301);
        s.duration = Tick::ZERO;
        assert_eq!(s.frame_count(), 0);
    }

    #[test]
    fn adding_takes_numbers_them_and_grows_the_scene() {
        let mut s = scene();
        let t = s.add_track(PuppetTrack::new("Hero", "ProjectItems/Hero")).unwrap();
        let take = Take::new(0, [ParamGroup::Head].into_iter().collect(), Tick::from_seconds_f64(8.0), Tick::from_seconds_f64(5.0));
        let i = s.add_take(t, take).unwrap();
        assert_eq!(i, 0);
        assert_eq!(s.tracks[0].takes[0].number, 1);
        assert_eq!(s.tracks[0].takes[0].name, "take 1 - head");
        assert_eq!(s.take_counter, 2);
        assert_eq!(s.duration, Tick::from_seconds_f64(13.0));
        assert!(matches!(s.add_take(5, Take::new(0, Default::default(), Tick::ZERO, Tick::ZERO)), Err(Error::NoTrack(5))));
    }

    #[test]
    fn hostile_scenes_are_rejected_or_repaired() {
        let mut s = scene();
        s.fps = FrameRate { num: 0, den: 1 };
        assert!(s.sanitize().is_err());
        let mut s = scene();
        s.size = Size::new(f64::NAN, 10.0);
        assert!(s.sanitize().is_err());
        let mut s = scene();
        s.duration = Tick(i64::MAX);
        assert!(s.sanitize().is_err());
        let mut s = scene();
        s.add_track(PuppetTrack::new(&"x".repeat(1000), "p")).unwrap();
        s.tracks[0].placement.scale = Vec2::new(f64::NAN, f64::INFINITY);
        s.tracks[0].placement.opacity = 9.0;
        s.add_clip(0, AudioClip::new("c", "m", Tick::ZERO)).unwrap();
        s.audio[0].clips[0].gain = f32::NAN;
        s.audio[0].clips[0].pan = 7.0;
        s.sanitize().unwrap();
        assert_eq!(s.tracks[0].name.chars().count(), MAX_NAME);
        assert_eq!(s.tracks[0].placement.scale, Vec2::new(1.0, 1.0));
        assert!((s.tracks[0].placement.opacity - 1.0).abs() < 1e-12);
        assert!((s.audio[0].clips[0].gain - 1.0).abs() < 1e-6 && (s.audio[0].clips[0].pan - 1.0).abs() < 1e-6);
    }

    #[test]
    fn moving_a_track_reorders_and_the_camera_keeps_its_face() {
        let mut s = scene();
        for name in ["a", "b", "c"] {
            s.add_track(PuppetTrack::new(name, name)).unwrap();
        }
        s.camera.zoom_on_face = Some(crate::camera::ZoomOnFace { track: 0, ..Default::default() });
        s.move_track(0, 2).unwrap();
        assert_eq!(s.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        assert_eq!(s.camera.zoom_on_face.unwrap().track, 2, "the camera follows track a to its new place");
        s.move_track(1, 0).unwrap();
        assert_eq!(s.camera.zoom_on_face.unwrap().track, 2);
        s.move_track(2, 0).unwrap();
        assert_eq!(s.camera.zoom_on_face.unwrap().track, 0);
        assert!(s.move_track(0, 3).is_err() && s.move_track(3, 0).is_err());
    }

    #[test]
    fn splitting_a_take_numbers_the_second_half() {
        let mut s = scene();
        s.add_track(PuppetTrack::new("a", "a")).unwrap();
        s.add_take(0, Take::new(0, [ParamGroup::Head].into_iter().collect(), Tick::ZERO, Tick::from_seconds_f64(4.0))).unwrap();
        let second = s.split_take(0, 0, Tick::from_seconds_f64(1.0)).unwrap();
        assert_eq!(second, 1);
        assert_eq!(s.tracks[0].takes[0].duration, Tick::from_seconds_f64(1.0));
        assert_eq!(s.tracks[0].takes[1].start, Tick::from_seconds_f64(1.0));
        assert_eq!(s.tracks[0].takes[1].number, 2);
        assert!(s.split_take(0, 0, Tick::from_seconds_f64(9.0)).is_err());
        assert!(s.split_take(0, 5, Tick::ZERO).is_err());
    }

    #[test]
    fn keyframed_placement_follows_the_lanes() {
        let mut t = PuppetTrack::new("a", "a");
        let mut lane = Lane::new("position_x");
        lane.set_key(Tick::ZERO, 0.0, None).unwrap();
        lane.set_key(Tick::from_seconds_f64(2.0), 200.0, None).unwrap();
        t.lanes.push(lane);
        t.placement.position.y = 7.0;
        let p = t.placement_at(Tick::from_seconds_f64(1.0));
        assert!((p.position.x - 100.0).abs() < 1e-9 && (p.position.y - 7.0).abs() < 1e-9);
        let mut s = scene();
        s.tracks.push(t);
        s.tracks[0].lanes.push(Lane::new("not a field"));
        s.sanitize().unwrap();
        assert_eq!(s.tracks[0].lanes.len(), 1);
    }

    #[test]
    fn identity_placement_centres_the_skin() {
        let m = Placement::default().matrix(Vec2::new(960.0, 540.0), Vec2::new(-100.0, -50.0), Vec2::ZERO);
        let p = m * anim_core::Point::new(0.0, 0.0);
        assert!((p.x - 860.0).abs() < 1e-9 && (p.y - 490.0).abs() < 1e-9);
    }

    #[test]
    fn the_scene_round_trips_through_json() {
        let mut s = scene();
        let t = s.add_track(PuppetTrack::new("Hero", "ProjectItems/Hero")).unwrap();
        let mut take = Take::new(0, [ParamGroup::Head, ParamGroup::Keys].into_iter().collect(), Tick::ZERO, Tick::from_seconds_f64(1.0));
        take.samples
            .push(crate::take::Sample { t: Tick::ZERO, frame: crate::group::Frame { keys: ["a".to_owned()].into_iter().collect(), ..Default::default() } });
        s.add_take(t, take).unwrap();
        s.tracks[0].behaviors.push(StackOp::SetSeed { seed: 9 });
        s.add_clip(0, AudioClip::new("music", "song", Tick::from_seconds_f64(0.5))).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Scene>(&json).unwrap(), s);
    }
}
