//! Where landmarks come from: the [`FaceSource`] trait, a scripted/recorded source for tests and for machines
//! without a camera, and the camera + landmark-model combination.

use std::time::{Duration, Instant};

use anim_core::Tick;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::frame::Frame;
use crate::landmarks::Landmarks;
use crate::synthetic::{SyntheticFace, demo_faces};

/// Most lines [`ScriptedSource::from_json_lines`] reads, so a hostile recording cannot exhaust memory.
pub const MAX_RECORDING_FRAMES: usize = 1_000_000;

/// One camera frame's result: when it was captured and the face found in it, if any.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Capture time since the source started.
    pub time: Tick,
    /// The face landmarks, or `None` when no face was found.
    pub landmarks: Option<Landmarks>,
    /// A small copy of the camera image for the live preview (`None` for scripted sources and recordings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Frame>,
}

/// Widest preview image a camera source attaches to its observations, in pixels.
pub const PREVIEW_WIDTH: u32 = 320;

impl Observation {
    /// An observation without a preview image.
    #[must_use]
    pub const fn new(time: Tick, landmarks: Option<Landmarks>) -> Self {
        Self { time, landmarks, preview: None }
    }
}

/// A stream of observations.
///
/// `next` blocks until the next one (a camera waits for its next frame) and returns `Ok(None)` when the stream has
/// ended. A source is created and used on the actor's own thread, so it need not be `Send`.
pub trait FaceSource {
    /// A short description for status displays (camera name, recording name).
    fn name(&self) -> String;

    /// The next observation.
    ///
    /// # Errors
    /// Whatever the underlying device or model reports.
    fn next(&mut self) -> Result<Option<Observation>>;
}

/// A scripted or recorded source: replays a fixed list of observations.
#[derive(Clone, Debug)]
pub struct ScriptedSource {
    name: String,
    frames: Vec<Observation>,
    next: usize,
    looping: bool,
    interval: Option<Duration>,
    loops: i64,
    clock: Option<Instant>,
}

impl ScriptedSource {
    /// Replays `frames` once, as fast as they are asked for.
    #[must_use]
    pub fn new(name: impl Into<String>, frames: Vec<Observation>) -> Self {
        Self { name: name.into(), frames, next: 0, looping: false, interval: None, loops: 0, clock: None }
    }

    /// A source from synthetic faces at `fps` frames per second of source time.
    #[must_use]
    pub fn from_faces(name: impl Into<String>, faces: &[SyntheticFace], fps: u32) -> Self {
        let step = Tick::from_units(1, i64::from(fps.max(1)));
        let frames = (0_i64..).zip(faces).map(|(i, face)| Observation::new(Tick(step.0.saturating_mul(i)), Some(face.landmarks()))).collect();
        Self::new(name, frames)
    }

    /// A talking, blinking, head-turning demo face: `frames` frames per loop at `fps`, looping forever and paced
    /// in real time like a camera.
    #[must_use]
    pub fn demo(frames: usize, fps: u32) -> Self {
        Self::from_faces("demo face", &demo_faces(frames), fps).looping().paced(Duration::from_secs(1).checked_div(fps.max(1)).unwrap_or_default())
    }

    /// Start over when the frames run out instead of ending.
    #[must_use]
    pub const fn looping(mut self) -> Self {
        self.looping = true;
        self
    }

    /// Wait `interval` between frames, like a camera does.
    #[must_use]
    pub const fn paced(mut self, interval: Duration) -> Self {
        self.interval = Some(interval);
        self
    }

    /// Reads a recording: one JSON [`Observation`] per line (blank lines are skipped).
    ///
    /// # Errors
    /// [`crate::Error::Inference`] naming the first bad line.
    pub fn from_json_lines(name: impl Into<String>, text: &str) -> Result<Self> {
        let mut frames = Vec::new();
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            if frames.len() >= MAX_RECORDING_FRAMES {
                return Err(crate::Error::Inference(format!("recording has more than {MAX_RECORDING_FRAMES} frames")));
            }
            let obs = serde_json::from_str(line).map_err(|e| crate::Error::Inference(format!("recording line {}: {e}", n.saturating_add(1))))?;
            frames.push(obs);
        }
        Ok(Self::new(name, frames))
    }

    /// Writes `observations` as a recording [`ScriptedSource::from_json_lines`] reads back.
    #[must_use]
    pub fn to_json_lines(observations: &[Observation]) -> String {
        observations.iter().filter_map(|o| serde_json::to_string(o).ok()).fold(String::new(), |mut s, line| {
            s.push_str(&line);
            s.push('\n');
            s
        })
    }

    /// Number of frames in one pass.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether there are no frames.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl FaceSource for ScriptedSource {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn next(&mut self) -> Result<Option<Observation>> {
        if self.frames.is_empty() {
            return Ok(None);
        }
        if let Some(interval) = self.interval {
            // Pace against a fixed clock so processing time does not stretch the frame rate.
            let started = *self.clock.get_or_insert_with(Instant::now);
            let due = started.checked_add(interval.saturating_mul(u32::try_from(self.next).unwrap_or(u32::MAX))).unwrap_or(started);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        let Some(obs) = self.frames.get(self.next) else {
            if !self.looping {
                return Ok(None);
            }
            self.next = 0;
            self.loops = self.loops.saturating_add(1);
            self.clock = None;
            return self.next();
        };
        self.next = self.next.saturating_add(1);
        // Looping continues the timeline instead of jumping back to zero.
        let span = self.frames.last().map_or(0, |l| l.time.0).saturating_add(self.frames.get(1).map_or(0, |f| f.time.0));
        let time = Tick(obs.time.0.saturating_add(span.saturating_mul(self.loops)));
        Ok(Some(Observation { time, landmarks: obs.landmarks.clone(), preview: obs.preview.clone() }))
    }
}

/// Something that delivers camera frames (a webcam, a video file, a test pattern).
pub trait FrameSource {
    /// A short description for status displays.
    fn name(&self) -> String;

    /// The next frame and its capture time, blocking until it arrives; `Ok(None)` at the end of the stream.
    ///
    /// # Errors
    /// Whatever the device reports.
    fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>>;
}

impl FrameSource for Box<dyn FrameSource> {
    fn name(&self) -> String {
        (**self).name()
    }

    fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
        (**self).next_frame()
    }
}

/// A [`FrameSource`] that repeats one image `count` times at `fps`: a still photo standing in for a camera, for
/// tests and demos of the whole pipeline.
#[derive(Clone, Debug)]
pub struct StillImage {
    frame: Frame,
    remaining: usize,
    emitted: i64,
    step: Tick,
}

impl StillImage {
    /// `frame` repeated `count` times, stamped at `fps` frames per second.
    #[must_use]
    pub fn new(frame: Frame, count: usize, fps: u32) -> Self {
        Self { frame, remaining: count, emitted: 0, step: Tick::from_units(1, i64::from(fps.max(1))) }
    }
}

impl FrameSource for StillImage {
    fn name(&self) -> String {
        "still image".into()
    }

    fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining = self.remaining.saturating_sub(1);
        let time = Tick(self.step.0.saturating_mul(self.emitted));
        self.emitted = self.emitted.saturating_add(1);
        Ok(Some((time, self.frame.clone())))
    }
}

/// Something that finds a face's landmarks in a frame, remembering the previous frame to follow the face.
pub trait LandmarkBackend {
    /// The landmarks of the face in `frame`, or `None` when there is none.
    ///
    /// # Errors
    /// Inference failures.
    fn landmarks(&mut self, frame: &Frame) -> Result<Option<Landmarks>>;
}

/// A [`FrameSource`] and a [`LandmarkBackend`] together make a [`FaceSource`].
#[derive(Debug)]
pub struct TrackedCamera<F, B> {
    frames: F,
    backend: B,
}

impl<F: FrameSource, B: LandmarkBackend> TrackedCamera<F, B> {
    /// Combines a frame source with a landmark backend.
    #[must_use]
    pub const fn new(frames: F, backend: B) -> Self {
        Self { frames, backend }
    }
}

impl<F: FrameSource, B: LandmarkBackend> FaceSource for TrackedCamera<F, B> {
    fn name(&self) -> String {
        self.frames.name()
    }

    fn next(&mut self) -> Result<Option<Observation>> {
        let Some((time, frame)) = self.frames.next_frame()? else { return Ok(None) };
        let landmarks = self.backend.landmarks(&frame)?;
        Ok(Some(Observation { time, landmarks, preview: Some(frame.downscaled(PREVIEW_WIDTH)) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(mut s: impl FaceSource, cap: usize) -> Vec<Observation> {
        let mut out = vec![];
        while out.len() < cap {
            match s.next().unwrap() {
                Some(o) => out.push(o),
                None => break,
            }
        }
        out
    }

    #[test]
    fn replays_once_in_order() {
        let faces = demo_faces(5);
        let all = collect(ScriptedSource::from_faces("t", &faces, 30), 100);
        assert_eq!(all.len(), 5);
        assert!(all.windows(2).all(|w| w[0].time < w[1].time));
        assert_eq!(all[1].time, Tick::from_units(1, 30));
    }

    #[test]
    fn looping_continues_the_timeline() {
        let all = collect(ScriptedSource::from_faces("t", &demo_faces(4), 30).looping(), 10);
        assert_eq!(all.len(), 10);
        assert!(all.windows(2).all(|w| w[0].time < w[1].time), "time never jumps back");
    }

    #[test]
    fn an_empty_script_ends_at_once() {
        let mut s = ScriptedSource::new("e", vec![]).looping();
        assert!(s.is_empty() && s.next().unwrap().is_none());
    }

    #[test]
    fn recordings_round_trip_through_json_lines() {
        let src = ScriptedSource::from_faces("t", &demo_faces(3), 30);
        let obs = collect(src, 10);
        let text = ScriptedSource::to_json_lines(&obs);
        assert_eq!(text.lines().count(), 3);
        let back = collect(ScriptedSource::from_json_lines("r", &text).unwrap(), 10);
        assert_eq!(back, obs);
    }

    #[test]
    fn bad_recordings_are_reported_with_a_line_number() {
        let err = ScriptedSource::from_json_lines("r", "\n{not json}\n").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
        assert!(ScriptedSource::from_json_lines("r", "").unwrap().is_empty());
    }

    #[test]
    fn a_frame_with_no_face_is_an_observation_without_landmarks() {
        let mut s = ScriptedSource::new("x", vec![Observation::new(Tick::ZERO, None)]);
        assert_eq!(s.next().unwrap().unwrap().landmarks, None);
        assert!(s.next().unwrap().is_none());
    }

    struct Frames(u32);
    impl FrameSource for Frames {
        fn name(&self) -> String {
            "frames".into()
        }
        fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
            if self.0 == 0 {
                return Ok(None);
            }
            self.0 = self.0.saturating_sub(1);
            Ok(Some((Tick(i64::from(self.0)), Frame::solid(4, 4, [0; 3])?)))
        }
    }
    struct FindsFaceOnOddFrames(bool);
    impl LandmarkBackend for FindsFaceOnOddFrames {
        fn landmarks(&mut self, _: &Frame) -> Result<Option<Landmarks>> {
            self.0 = !self.0;
            Ok(self.0.then(|| SyntheticFace::default().landmarks()))
        }
    }

    #[test]
    fn a_still_image_repeats_with_rising_timestamps() {
        let mut s = StillImage::new(Frame::solid(2, 2, [1, 2, 3]).unwrap(), 3, 30);
        let times: Vec<_> = std::iter::from_fn(|| s.next_frame().unwrap().map(|(t, _)| t)).collect();
        assert_eq!(times, [Tick(0), Tick::from_units(1, 30), Tick::from_units(2, 30)]);
    }

    #[test]
    fn a_camera_and_a_backend_make_a_source() {
        let all = collect(TrackedCamera::new(Frames(4), FindsFaceOnOddFrames(false)), 10);
        assert_eq!(all.len(), 4);
        assert!(all.iter().all(|o| o.preview.is_some()), "a camera source attaches the preview image");
        assert_eq!(all.iter().map(|o| o.landmarks.is_some()).collect::<Vec<_>>(), [true, false, true, false]);
    }
}
