//! Character Animator's recorded takes: `.arf` event graphs.
//!
//! Every take Character Animator records (the face, the voice, a dragged handle) is stored as an `.arf` file under
//! `Ch Media/Recordings/<date>/`, and the transcript behind a Lip Sync take as `Visemes_*.arf` under
//! `Ch Media/Recordings/Transcript/`. The project only holds an alias to the file. This crate reads and writes them;
//! `FORMAT.md` has the full byte-level notes, reverse engineered from the 23 recordings of the `news anchors` sample
//! for interoperability.
//!
//! ```text
//! bytes ──chunk::parse──► Chunk tree ──EventGraph::from_bytes──► frames of Bezier curves + channel table
//!                                                                      │ tracks()
//!                                                                      ▼
//!                                                         Track { name, Role, Curve }  ──► value_at(seconds)
//! ```
//!
//! * [`chunk`]: the container, exact in both directions.
//! * [`EventGraph`]: frames, footer and channel table; `to_bytes(from_bytes(x)) == x` for every sample recording.
//! * [`Curve`] and [`Segment`]: the value of a channel over time, with jumps and holds.
//! * [`Role`]: what a channel is (a face value, a viseme, the voice amplitude, a draggable handle value).
//! * [`decimal`]: the `%.36g` spelling of times.
//!
//! Everything read is treated as hostile: lengths are checked against the data, nesting and counts are bounded, and
//! no input can make the reader panic.

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: curves must evaluate bit-identically on every CPU")]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::float_cmp,
        clippy::cast_precision_loss,
        clippy::unwrap_used,
        clippy::unreadable_literal,
        reason = "test code"
    )
)]

pub mod channel;
pub mod chunk;
pub mod curve;
pub mod decimal;
mod error;
mod graph;

pub use channel::{Axis, FaceParam, HandleSpace, HandleValue, Role, face_channel, handle_channel};
pub use chunk::Chunk;
pub use curve::{Curve, Segment};
pub use error::{Error, Result};
pub use graph::{ChannelEntry, EventGraph, FRAME_SECONDS, Frame, FrameCurve, KIND_BEZIER, MAX_FRAMES, Track};
