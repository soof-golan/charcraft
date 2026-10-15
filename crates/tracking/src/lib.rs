//! Webcam face tracking for `CharCraft`, as an actor.
//!
//! # Data flow
//!
//! ```text
//!  camera (nokhwa)      face detector + landmark model (ONNX Runtime)      maths                    puppet
//!  ───────────────      ─────────────────────────────────────────────      ─────                    ──────
//!  FrameSource ──► Frame ──► LandmarkBackend ──► Landmarks (478 points) ──► measure ──► FaceSolver ──► FaceInput
//!        └──────────── TrackedCamera: one FaceSource ──────────────────┘     calibration + smoothing
//! ```
//!
//! * [`actor`]: [`actor::TrackingActor`] owns a [`source::FaceSource`] and a [`solver::FaceSolver`] on its own
//!   thread. The UI sends [`actor::Command`]s (start with a camera, stop, calibrate, tune) and receives
//!   [`actor::Event`]s, chiefly one [`input::FaceInput`] per camera frame. No locks, no shared state.
//! * [`source`]: the [`source::FaceSource`] trait and [`source::ScriptedSource`], a scripted or recorded source for
//!   tests and for machines without a camera.
//! * [`measure`], [`solver`], [`input`]: the maths from landmarks to a puppet-ready [`input::FaceInput`]: head pose
//!   from the head's own axes, brows, lids, pupils and mouth measured in that frame and in eye distances, neutral-face
//!   calibration, output ranges and One-Euro smoothing.
//! * [`model`]: the registry of open-licensed models (name, licence, URL, SHA-256) and the store that downloads them
//!   into the platform cache directory on first use and verifies them. Weights are never committed.
//! * `camera` (feature `camera`): enumerates webcams and opens one at a requested resolution through nokhwa.
//! * `onnx` (feature `onnx` or `onnx-dynamic`): `MediaPipe` `BlazeFace` and Face Mesh V2 on ONNX Runtime, linked
//!   from downloaded binaries or loaded at run time. `pipeline` holds the runtime-independent geometry.
//! * `live` (features `camera` and an ONNX one): [`actor::SourceFactory`] for the real webcam and models.
//! * [`synthetic`]: a parametric face in the landmark layout, for tests, demos and development without a camera.
//!
//! # Tools
//!
//! Every public data type is `serde`-serialisable, and [`tools`] packages the actor as a small set of JSON tools
//! (status, latest face, start, stop, calibrate, tune) with schemas, ready for the app's MCP layer to list and route.

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: results must be bit-identical on every CPU")]
#![allow(clippy::many_single_char_names, clippy::similar_names, reason = "geometry code follows the usual (x, y, u, v, a, b) notation")]
#![cfg_attr(test, allow(clippy::arithmetic_side_effects, clippy::assert_is_empty, reason = "test code"))]

pub mod actor;
#[cfg(feature = "camera")]
pub mod camera;
mod cast;
pub mod error;
pub mod frame;
pub mod input;
pub mod landmarks;
#[cfg(all(feature = "camera", feature = "onnx-runtime"))]
pub mod live;
pub mod measure;
pub mod model;
#[cfg(feature = "onnx-runtime")]
pub mod onnx;
pub mod pipeline;
pub mod solver;
pub mod source;
pub mod synthetic;
pub mod tools;

pub use error::{Error, Result};
