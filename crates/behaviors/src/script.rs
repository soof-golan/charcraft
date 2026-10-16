//! Scripted inputs: a timeline of events that plays into a stack. Tests, tools and the MCP surface use it to
//! try behaviors without a camera, microphone or mouse.

use std::collections::BTreeMap;

use anim_core::{FrameRate, Point, Tick};
use anim_lipsync::Viseme as VoiceViseme;
use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::input::{FaceInput, Inputs, Pointer};
use crate::stack::BehaviorStack;

/// Most frames one script may play.
pub const MAX_FRAMES: u32 = 200_000;

/// Something that happens at a frame. State changes stick until another event changes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ScriptEvent {
    /// The camera sees this face from now on.
    Face {
        /// Frame the event happens on.
        frame: u32,
        /// The face.
        face: FaceInput,
    },
    /// The camera loses the face.
    NoFace {
        /// Frame the event happens on.
        frame: u32,
    },
    /// The voice makes a mouth shape (`None`: the microphone goes quiet).
    Voice {
        /// Frame the event happens on.
        frame: u32,
        /// The shape.
        viseme: Option<VoiceViseme>,
    },
    /// The scene's own audio makes a mouth shape (`None`: it goes quiet).
    SceneVoice {
        /// Frame the event happens on.
        frame: u32,
        /// The shape.
        viseme: Option<VoiceViseme>,
    },
    /// A pointer is at a place, down or up.
    Pointer {
        /// Frame the event happens on.
        frame: u32,
        /// Pointer id (the mouse is 0).
        id: u32,
        /// Stage x.
        x: f64,
        /// Stage y.
        y: f64,
        /// Button held.
        down: bool,
    },
    /// A pointer leaves the stage.
    PointerGone {
        /// Frame the event happens on.
        frame: u32,
        /// Pointer id.
        id: u32,
    },
    /// A key goes down or up.
    Key {
        /// Frame the event happens on.
        frame: u32,
        /// The key (`a`, `space`, `left`).
        key: String,
        /// True for down.
        down: bool,
    },
}

impl ScriptEvent {
    /// The frame the event happens on.
    #[must_use]
    pub const fn frame(&self) -> u32 {
        match self {
            Self::Face { frame, .. }
            | Self::NoFace { frame }
            | Self::Voice { frame, .. }
            | Self::SceneVoice { frame, .. }
            | Self::Pointer { frame, .. }
            | Self::PointerGone { frame, .. }
            | Self::Key { frame, .. } => *frame,
        }
    }
}

const fn default_fps() -> f64 {
    30.0
}

/// A scripted run: how long, at what rate, and what happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    /// Frames per second.
    #[serde(default = "default_fps")]
    pub fps: f64,
    /// Number of frames to play.
    pub frames: u32,
    /// The events, in any order.
    #[serde(default)]
    pub events: Vec<ScriptEvent>,
}

impl Script {
    /// A silent script with nothing happening.
    #[must_use]
    pub const fn new(fps: f64, frames: u32) -> Self {
        Self { fps, frames, events: Vec::new() }
    }

    /// Adds an event.
    #[must_use]
    pub fn with(mut self, event: ScriptEvent) -> Self {
        self.events.push(event);
        self
    }

    /// The inputs of every frame.
    ///
    /// # Errors
    ///
    /// [`Error::BadScript`] when the rate or length is out of range.
    pub fn inputs(&self) -> Result<Vec<Inputs>> {
        if !(self.fps.is_finite() && self.fps > 0.0 && self.fps <= 1000.0) {
            return Err(Error::BadScript(format!("fps {} is out of range", self.fps)));
        }
        if self.frames > MAX_FRAMES {
            return Err(Error::BadScript(format!("{} frames is more than {MAX_FRAMES}", self.frames)));
        }
        let rate = FrameRate::from_f64(self.fps);
        let mut events: Vec<&ScriptEvent> = self.events.iter().collect();
        events.sort_by_key(|e| e.frame());
        let mut pending = events.into_iter().peekable();

        let mut face = None;
        let mut viseme = None;
        let mut scene_viseme = None;
        let mut pointers: BTreeMap<u32, Pointer> = BTreeMap::new();
        let mut keys = std::collections::BTreeSet::new();
        let mut out = Vec::with_capacity(usize::try_from(self.frames).unwrap_or(0));
        for frame in 0..self.frames {
            while let Some(e) = pending.next_if(|e| e.frame() <= frame) {
                match e {
                    ScriptEvent::Face { face: f, .. } => face = Some(*f),
                    ScriptEvent::NoFace { .. } => face = None,
                    ScriptEvent::Voice { viseme: v, .. } => viseme = *v,
                    ScriptEvent::SceneVoice { viseme: v, .. } => scene_viseme = *v,
                    ScriptEvent::Pointer { id, x, y, down, .. } => {
                        pointers.insert(*id, Pointer { id: *id, position: Point::new(*x, *y), down: *down });
                    }
                    ScriptEvent::PointerGone { id, .. } => {
                        pointers.remove(id);
                    }
                    ScriptEvent::Key { key, down, .. } => {
                        let key = key.to_lowercase();
                        if *down {
                            keys.insert(key);
                        } else {
                            keys.remove(&key);
                        }
                    }
                }
            }
            out.push(Inputs {
                time: rate.tick_of(i64::from(frame)),
                face,
                viseme,
                scene_viseme,
                pointers: pointers.values().copied().collect(),
                keys: keys.clone(),
                ..Inputs::default()
            });
        }
        Ok(out)
    }
}

/// One simulated frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimFrame {
    /// Frame number.
    pub frame: u32,
    /// Its time.
    pub time: Tick,
    /// The pose the stack produced.
    pub pose: PuppetPose,
}

/// Plays a script into a stack from a fresh start and returns the pose of every `every`th frame (all of them
/// when `every` is 0 or 1). The last frame is always included.
///
/// # Errors
///
/// [`Error::BadScript`].
pub fn simulate(stack: &mut BehaviorStack, script: &Script, every: u32) -> Result<Vec<SimFrame>> {
    let inputs = script.inputs()?;
    stack.reset();
    let every = every.max(1);
    let last = script.frames.saturating_sub(1);
    let mut out = Vec::new();
    for (frame, input) in (0_u32..).zip(&inputs) {
        let pose = stack.step(input);
        if frame.checked_rem(every) == Some(0) || frame == last {
            out.push(SimFrame { frame, time: input.time, pose });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_stick_until_changed() {
        let script = Script::new(10.0, 6)
            .with(ScriptEvent::Key { frame: 1, key: "Left".into(), down: true })
            .with(ScriptEvent::Key { frame: 4, key: "left".into(), down: false })
            .with(ScriptEvent::Voice { frame: 2, viseme: Some(VoiceViseme::Oh) });
        let i = script.inputs().unwrap();
        assert_eq!(i.len(), 6);
        assert!(!i[0].key_down("left") && i[1].key_down("left") && i[3].key_down("left") && !i[4].key_down("left"));
        assert_eq!((i[1].viseme, i[2].viseme, i[5].viseme), (None, Some(VoiceViseme::Oh), Some(VoiceViseme::Oh)));
        assert_eq!(i[5].time, Tick::from_seconds_f64(0.5));
    }

    #[test]
    fn hostile_scripts_are_refused() {
        assert!(Script::new(f64::NAN, 3).inputs().is_err());
        assert!(Script::new(30.0, u32::MAX).inputs().is_err());
        assert!(Script::new(0.0, 3).inputs().is_err());
    }

    #[test]
    fn scripts_survive_json() {
        let s = Script::new(24.0, 10).with(ScriptEvent::Pointer { frame: 2, id: 0, x: 1.0, y: 2.0, down: true }).with(ScriptEvent::NoFace { frame: 3 });
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Script>(&json).unwrap(), s);
    }
}
