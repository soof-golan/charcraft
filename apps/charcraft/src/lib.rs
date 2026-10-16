//! `CharCraft`, the desktop app: rig, record and stream puppets that are file-compatible with Adobe Character
//! Animator.
//!
//! ```text
//!   window (eframe)                       actors (threads that own their state, talk over channels)
//!   ───────────────                       ───────────────────────────────────────────────────────
//!   ui(&State) -> Vec<Action>  ──────►    live    camera + face tracker, microphone + lip sync   (live.rs)
//!        │                                render  every puppet's pictures, draws poses to pixels (render.rs)
//!        ▼                                export  scene -> mp4 / mov / png, as an anim-session job (export.rs)
//!   Action = a command of the registry
//!        │
//!        ▼
//!   SessionHost<AppDoc, Hooks>   ◄────── MCP: stdio (--mcp-stdio), loopback control channel (--control-port), or
//!   undoable document, journal           the `anim-mcp` bridge binary, all through the same host   (control.rs)
//! ```
//!
//! * [`session`]: the document ([`session::AppDoc`]: scene, takes and rig edits) and its command registry. Every
//!   user action is a command, so the menus, the shortcuts and the MCP tools are one list.
//! * [`hooks`]: what MCP cannot know: queries about puppets and behaviors, rendering, exporting, opening, saving.
//! * [`world`]: the puppets and sound the document refers to, and how files are opened.
//! * [`compose`], [`render`], [`export`]: from poses to pixels, in the window and for files.
//! * [`live`]: the camera, tracking and microphone actor.
//! * [`demo`]: a puppet drawn in code, so the app and `--smoke` work without any artwork.
//! * [`puppet_cmds`] and [`reimport`]: bringing puppets in (Puppet Maker, starter puppets) and keeping them in step
//!   with their artwork files (re-import, Edit Original).
//! * [`app`] and [`ui`]: the window.

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: poses must be bit-identical on every CPU")]
#![cfg_attr(
    test,
    allow(
        clippy::arithmetic_side_effects,
        clippy::float_cmp,
        clippy::assert_is_empty,
        clippy::unnecessary_wraps,
        clippy::indexing_slicing,
        reason = "test code"
    )
)]

pub mod app;
pub mod compose;
pub mod control;
pub mod demo;
pub mod export;
pub mod hooks;
pub mod live;
pub mod project_cmds;
pub mod projectio;
pub mod puppet_cmds;
pub mod reimport;
pub mod render;
pub mod session;
pub mod smoke;
pub mod state;
pub mod ui;
pub mod view;
pub mod workspace;
pub mod world;
