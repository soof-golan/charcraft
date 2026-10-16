//! Takes and scenes for `CharCraft`: record a performance, stack it, play it back, export it.
//!
//! ```text
//!  live inputs ──► Recorder ──► Take ──► PuppetTrack ──► Scene ──► Evaluator ──► PuppetPose ──► SceneFrames
//!  (face, voice,    arm groups,   timestamped   stacked     tracks,    behaviors     per track     (FrameSource)
//!   pointer, keys,  punch range   samples of    takes,      audio,     step once     and frame     + audio mix
//!   audio)                        the groups    blends      size, fps  per frame
//! ```
//!
//! * [`group`]: [`ParamGroup`], the parts of the inputs armed and overridden separately (head, brows, eyes,
//!   mouth, voice, pointer, keys), and the [`Frame`] a sample stores.
//! * [`take`]: a [`Take`] is samples of some groups over a stretch of the scene. [`resolve`] layers a track's takes
//!   over empty inputs: a later take overrides an earlier one *per group* inside its range, with blend in and
//!   out. The face interpolates between samples, everything else holds.
//! * [`recorder`]: [`Recorder`] captures the armed groups of the same [`Inputs`](charcraft_behaviors::Inputs) the
//!   behaviors run on, inside an optional punch-in range, plus microphone chunks.
//! * [`scene`]: [`Scene`], [`PuppetTrack`], [`AudioTrack`]. Plain serde, cheap to clone; puppets and sound live
//!   outside ([`Cast`], [`MediaStore`]).
//! * [`playback`]: [`Evaluator`] gives each track's [`PuppetPose`](charcraft_puppet::PuppetPose) at a frame by
//!   stepping the behavior stack once per frame. Playback is deterministic (seeded) and scrubbing is exact.
//! * [`stage`]: [`Cast`] holds the puppets; [`SceneFrames`] implements [`anim_export::FrameSource`].
//! * [`audio`]: the scene's sound mixed for export ([`mix_scene`], [`scene_soundtrack`] for `anim-video`).
//! * [`store`]: the sidecar JSON next to the project, because the project's own take files are read-only to us.
//! * [`import`]: tracks, placements and sound clips out of a Character Animator project.
//! * [`doc`] and [`commands`]: a [`RecordDoc`] plus the `anim-session` command registry. Plugging that registry
//!   into `anim_session::McpServer` is the whole MCP server; see [`commands`] for the tools.

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: playback must be bit-identical on every CPU")]
#![cfg_attr(
    test,
    allow(
        clippy::arithmetic_side_effects,
        clippy::float_cmp,
        clippy::cast_precision_loss,
        clippy::default_trait_access,
        clippy::iter_on_single_items,
        clippy::assert_is_empty,
        clippy::unnecessary_wraps,
        reason = "test code"
    )
)]

pub mod audio;
pub mod camera;
pub mod commands;
pub mod doc;
pub mod edit_commands;
pub mod error;
pub mod group;
pub mod import;
pub mod keyframes;
pub mod playback;
pub mod recorded;
pub mod recorder;
pub mod scene;
pub mod speech;
pub mod stage;
pub mod store;
pub mod take;
pub mod timing;

pub use audio::{MediaStore, mix_scene, scene_soundtrack, valid_media_name, video_track, wav_bytes};
pub use camera::{Camera, ZoomOnFace};
pub use commands::{commands as all_commands, register, registry};
pub use doc::{Committed, RecordDoc};
pub use error::{Error, Result};
pub use group::{Frame, ParamGroup, apply_group};
pub use import::{ImportOptions, Imported, TakePolicy, import_scene, import_scene_with, load_sounds};
pub use keyframes::{Ease, Key, Lane};
pub use playback::{Evaluator, Templates, track_inputs, track_stack};
pub use recorder::{Arming, RecordedAudio, Recorder, Recording};
pub use scene::{AudioClip, AudioTrack, Marker, Placement, PuppetTrack, Scene};
pub use speech::voice_from_audio;
pub use stage::{Cast, CastMember, Layout, SceneFrames, render_frame};
pub use store::{SceneFile, load, save, sidecar_path};
pub use take::{Sample, Take, resolve};
