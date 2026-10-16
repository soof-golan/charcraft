# Architecture

Three repositories, one family:

| Repo | What | Depends on |
|---|---|---|
| [`anicraft-libs`](https://github.com/soof-golan/anicraft-libs) | Shared engine crates (`anim-*`) | Craft crates by git rev |
| [`anicraft`](https://github.com/soof-golan/anicraft) | 2D animation editor, file-compatible with Adobe Animate | `anicraft-libs` by git |
| `charcraft` (this repo) | Live character animation, file-compatible with Adobe Character Animator (`.puppet`, `.chproj`) | `anicraft-libs` by git |

The engine crates are described in the `anicraft-libs` ARCHITECTURE.md (display lists, CPU renderer, rig/ARAP
deformation, audio, lip sync, artwork import, export, shared egui widgets, session framework). CharCraft depends on
them with `anim-x = { git = "https://github.com/soof-golan/anicraft-libs", rev = "..." }`, pinned to a release
commit; to pick up newer engine work, change the revs and run `cargo update`.

## Data flow

```
 webcam ─► tracking ─► face parameters ─┐
 mic ────► lip sync ─► visemes ─────────┤
 keyboard / mouse ─► triggers, drag ────┤
                                        ▼
 .puppet / .chproj ─► project ─► puppet model ─► behaviors ─► DisplayList ─► anim-render ─► pixels
 (zip / folder)      (ltab, rope)  (layer tree,    (face, lip sync,   (anim-display)          (window, PNG,
                                    tags, handles)  gaze, physics...)                         video export)
                                        ▲
                       record / playback (takes, scene timeline) ─┘
```

- A **puppet** is a layer tree built from PSD/AI artwork. Layer and group names carry the rigging tags (`+Head`,
  `Mouth`, viseme names, `Left Eye`, `+Draggable`, ...) exactly as Character Animator reads them, so names are kept
  verbatim (any Unicode, including right-to-left scripts).
- **Behaviors** are pure functions of (puppet, parameters, live input, time) producing a pose; parameters live in
  the puppet's behavior defaults and per-scene overrides.
- **Producers** evaluate the posed puppet into an `anim_display::DisplayList`; `anim-render` rasterises it.
- **Live input** (camera, microphone) runs in actors that own their devices and talk over channels.

## charcraft crates (`crates/<name>`, package `charcraft-<name>`)

| Crate | Owns | Key dependencies |
|---|---|---|
| `charcraft-ltab` | The Lua-table text format Character Animator uses for metadata (`_archive_info` and friends): parse, typed access, byte-exact pretty printing, serde bridge | anim-core |
| `charcraft-rope` | The `Rope10LM` log and the git-style object store inside `Ch Data/repo.noindex` | charcraft-ltab |
| `charcraft-project` | `.chproj` folders and `.puppet` zips to and from the project model; scenes, takes, media references | anim-archive, charcraft-rope, charcraft-ltab |
| `charcraft-puppet` | Puppet model: layer tree from artwork, tags from names, visemes, eyes, handles, behaviors and their parameters, rigging | anim-artwork, anim-rig |
| `charcraft-behaviors` | Face, Lip Sync, Eye Gaze, Auto Blink, Dragger, Physics/Dangle, Breathe, Walk, Triggers, Transform | charcraft-puppet, anim-lipsync, anim-rig |
| `charcraft-tracking` | Webcam via nokhwa and face landmarks via ort (ONNX Runtime), as an actor | nokhwa, ort |
| `charcraft-record` | Takes, the scene timeline, playback and recording | charcraft-project, anim-timeline |

Apps live in `apps/*`: `charcraft-app` (eframe app, binary `charcraft`) and `charcraft-cli` (headless convert,
render, export and inspect, used by tests).

## Crate template

```toml
[package]
name = "charcraft-<name>"
description = "..."
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
repository.workspace = true

[lints]
workspace = true
```

Every crate has a crate-level doc comment saying what it owns, unit tests, and (for formats) round-trip tests.
