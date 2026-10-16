//! Character Animator behaviors as deterministic functions of inputs over time.
//!
//! A [`BehaviorStack`] holds [`Behavior`]s in order. Each tick it takes [`Inputs`] (a tracked [`FaceInput`], the
//! voice's mouth shape from `anim-lipsync`, pointers, keys and the clock) and returns a
//! [`PuppetPose`](charcraft_puppet::PuppetPose) that `Puppet::evaluate` draws. Behaviors add to the pose in
//! stack order, so a later one sees what the earlier ones did. State (smoothing, the blink schedule, a drag in
//! progress, a swinging hair lock) lives inside the behaviors, and every random choice comes from a seeded stream:
//! the same seed and inputs always give the same poses.
//!
//! | behavior | id | does |
//! |---|---|---|
//! | [`Face`] | `Adobe.FaceTracker2.js` | head, eyebrows, blink, eye gaze and (optionally) mouth from the face, smoothed |
//! | [`HeadTurner`] | `Adobe.HeadTurner.js` | frontal, quarter, profile, upward and downward views swapped by head yaw and pitch |
//! | [`MotionTrigger`] | `Adobe.MotionTrigger.js` | at rest, moving left, right, up, down and in transition layers swapped by how the puppet moves |
//! | [`CycleLayers`] | `Adobe.CycleLayers.js` | the children of groups tagged Cycle shown in turn, looping, ping-ponging or once |
//! | [`LipSync`] | `Adobe.LipSync2.js` | mouth shape from the voice, hold trims, hand-back to Face after silence, jaw |
//! | [`EyeGaze`] | `Adobe.EyeGaze.js` | eyes follow the mouse, the arrow keys or the tracked gaze, optionally darting |
//! | [`Blinker`] | `Adobe.Blinker.js` | random blinks |
//! | [`Dragger`] | `Adobe.Dragger3.js` | pointers pull `Draggable` handles, which spring back on release |
//! | [`Dynamics`] | `Adobe.RigDynamics.js` | `Dangle` handles swing as damped springs under head and body movement, plus wind |
//! | [`Transform`] | `Adobe.Transform.js` | a fixed move, turn and scale of the group tagged `Transform` |
//! | [`Triggers`] | `Adobe.Triggers.js` | keys switch triggers (hold, latch, timed) and choose swap-set members |
//! | [`Breathe`] | `Charcraft.Breathe.js` | a slow breath on layers tagged `Breathe` |
//! | [`Walk`] | `Charcraft.Walk.js` | basic side-on walk cycle on two keys |
//! | [`ArmIk`] | `Adobe.ArmIK.js` | parameters kept; no solver yet |
//! | [`BodyTracker`] | `Charcraft.BodyTracker.js` | the webcam-tracked body drives arms, legs and torso through two-bone IK |
//! | [`Particles`] | `Charcraft.Particles.js` | layers tagged `Particle` are thrown as copies under gravity, wind and collisions |
//! | [`Replay`] | `Charcraft.Replay.js` | hold a key to record a performance, press another to play it back |
//! | [`Magnets`] | `Charcraft.Magnets.js` | `Magnet` groups pull dragged handles in and hold them |
//! | [`Wiggler`] | `Charcraft.Wiggler.js` | tagged groups shake, sway or pulse |
//! | [`Fader`] | `Charcraft.Fader.js` | layers fade in and out with a key |
//! | [`LookAtCamera`] | `Charcraft.LookAtCamera.js` | eyes turn to the viewer and the head faces front |
//! | [`MotionLines`] | `Charcraft.MotionLines.js` | fast-moving `Trails` layers leave fading copies |
//! | [`Shadow`] | `Charcraft.Shadow.js` | a `Shadow` layer stays on the floor, shrinking as the puppet jumps |
//! | [`LeaderFollower`] | `Charcraft.LeaderFollower.js` | one group repeats another's movement after a delay |
//! | [`LayerPicker`] | `Charcraft.LayerPicker.js` | exactly one of a list of layers shows, chosen by key |
//!
//! Parameter structs spell their fields exactly as Character Animator files do (`handleReleaseDuration`,
//! `Trim_Ah`, `limit_range_cb`) and fill in missing ones with the stock defaults, so
//! [`behaviors::create_from_pairs`] can build a behavior from a project's stored defaults and a puppet's overrides
//! ([`BehaviorStack::from_bound`]).
//!
//! ```
//! use charcraft_behaviors::{BehaviorStack, Inputs, Rig};
//! use anim_core::Tick;
//!
//! let mut stack = BehaviorStack::standard(Rig::default()).with_seed(7);
//! let mut blinked = false;
//! for frame in 0..300 {
//!     let pose = stack.step(&Inputs::at(Tick::from_seconds_f64(f64::from(frame) / 30.0)));
//!     blinked |= pose.blink_left > 0.5;
//! }
//! assert!(blinked, "ten seconds at 30 blinks a minute");
//! ```
//!
//! [`api`] holds the serde tool surface an MCP server maps onto.

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: poses must be bit-identical on every CPU")]
#![cfg_attr(
    test,
    allow(
        clippy::arithmetic_side_effects,
        clippy::float_cmp,
        clippy::many_single_char_names,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        clippy::assert_is_empty,
        clippy::imprecise_flops,
        clippy::redundant_clone,
        clippy::needless_pass_by_value,
        reason = "test code"
    )
)]

pub mod api;
pub mod behavior;
pub mod behaviors;
pub mod error;
pub mod ik;
pub mod input;
pub mod rig;
pub mod script;
pub mod stack;
pub mod util;

pub use behavior::{Behavior, Cx, ParamDoc};
pub use behaviors::{
    ArmIk, Blinker, BodyTracker, Breathe, CycleLayers, Dragger, Dynamics, EyeGaze, Face, Fader, HeadTurner, LayerPicker, LeaderFollower, LipSync, LookAtCamera,
    Magnets, MotionLines, MotionTrigger, Particles, Replay, Shadow, Transform, Triggers, Walk, Wiggler,
};
pub use error::{Error, Result};
pub use input::{BodyInput, FaceInput, HandleTarget, Inputs, Joint, Pointer};
pub use rig::{ChainLink, CycleGroup, Rig, RigAnchor, RigHandle};
pub use script::{Script, ScriptEvent, SimFrame, simulate};
pub use stack::{BehaviorStack, Slot};
