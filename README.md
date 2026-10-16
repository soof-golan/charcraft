# CharCraft

Open-source live character animation in pure Rust: webcam face tracking, lip sync, and puppets built from PSD or AI
artwork. CharCraft opens Character Animator `.puppet` files and `.chproj` project folders, so existing puppets and
projects keep working.

Part of a family with [AniCraft](https://github.com/soof-golan/anicraft) (2D animation editor) on the shared
[anicraft-libs](https://github.com/soof-golan/anicraft-libs) engine, built on the open-source
[Craft](https://github.com/storytold) crates.

This is version 0.1.0, a pre-release; see [Known gaps](#known-gaps).

## Features

- **Files**: read and write `.puppet`; read `.chproj` project folders (scenes, puppets, media); build a puppet from a
  `.psd` or `.ai` file, with rigging tags read from layer names (`+Head`, `Left Eye`, viseme names, `+Draggable` and
  so on, verbatim, including right-to-left names).
- **Behaviors**: Face, Lip Sync, Eye Gaze, Auto Blink, Dragger, Physics/Dangle, Breathe, Walk, Triggers and Transform,
  with per-scene overrides; ARAP mesh deformation for handles.
- **Lip sync**: speech audio (WAV, MP3, AAC, FLAC, Ogg) turned into the 13 viseme mouth shapes; puppets with only a
  Flappy Chin jaw get the jaw driven instead.
- **Live input**: webcam face tracking (camera and ONNX Runtime, built into the release packages) and microphone, as
  actors that own their devices. With no camera the app says so and offers the demo face as an explicit choice;
  `charcraft --face-file PHOTO` tracks a picture, an animated GIF or a folder of frames instead.
- **Rig, Record and Stream modes** in the app; scenes with recorded and performed takes.
- **Export**: a project scene or a performance to `MP4` (H.264 with AAC), `MOV` (ProRes 4444 with transparency) or a
  `PNG` sequence.
- **MCP**: everything the UI does is a command; `charcraft-cli mcp` serves them headless or as a bridge to the running
  window, with real screenshots. See [docs/mcp.md](docs/mcp.md).

## Install

Download a package from the [releases page](https://github.com/soof-golan/charcraft/releases): Linux `tar.gz`, macOS
(Apple silicon and Intel, an app bundle with camera and microphone usage descriptions) and Windows `zip`, with
`SHA256SUMS`. Every package carries its own ONNX Runtime library (next to the program, or in the app's
`Contents/Frameworks`), so face tracking needs nothing installed; the two small face models are downloaded on first
use and checked against their SHA-256. `charcraft --check-live` runs that whole chain without a camera. The binaries
are unsigned: on macOS right-click the app and choose Open the first time; on Windows choose More info, then Run
anyway in SmartScreen.

## Using it

```sh
charcraft MY.puppet                     # the app (rig mode); --mode record, --screenshot out.png
charcraft                               # the built-in demo puppet

charcraft-cli info MY.puppet            # layers, tags, behaviors, warnings
charcraft-cli render MY.puppet --viseme Ah -o ah.png
charcraft-cli lipsync speech.wav        # visemes as JSON
charcraft-cli animate MY.puppet --audio speech.wav -o out.mp4
charcraft-cli export "My Project" --scene Scene -o scene.mp4
charcraft-cli convert MY.puppet COPY.puppet
charcraft-cli mcp MY.puppet             # MCP server on stdin/stdout
```

## Screenshots

Rendered by the behavior tests from this repository's own synthetic puppet (no Adobe content is used anywhere in this
repository): a head turning to its extremes, nudged in each direction. Each has a written prediction next to it in
`crates/behaviors/tests/fixtures/visual/`.

![head extremes](crates/behaviors/tests/fixtures/visual/head-extremes.png)

![eye gaze limits](crates/behaviors/tests/fixtures/visual/eye-gaze-limits.png)

## Fidelity

Checked against Character Animator puppets and a full project folder (two PSD news anchors, a recorded song, 14
takes): neutral renders match the artwork, mouth shapes open, necks stay attached, performances export to playable
video. Those files are never committed; tests that need them read `ANICRAFT_SAMPLES` and are skipped when it is unset.

## Building

```sh
cargo build -p charcraft-app -p charcraft-cli       # demo face only
cargo build -p charcraft-app --features live        # webcam and ONNX Runtime, as in the release packages
cargo test --workspace
```

Rust 1.99.1 is pinned in `rust-toolchain.toml`. On Linux install the development packages that
`.github/workflows/ci.yml` lists (ALSA, udev, xkbcommon, Wayland, X11, GL, GTK 3, fontconfig), and `libclang` for the
camera backend (its V4L2 bindings are generated with bindgen); `libv4l-dev` is not required. The engine crates come
from `anicraft-libs`, pinned by git rev in `Cargo.toml`.

Face tracking (`charcraft-tracking`) has optional features: `camera` (nokhwa), `onnx` (ONNX Runtime from the binaries
its build downloads; needs glibc 2.38+ on Linux, so Ubuntu 24.04 or newer) and `onnx-dynamic` (loads `libonnxruntime`
at run time, the mode of the `live` build: from `ORT_DYLIB_PATH` when set, else the copy next to the program or in
`Contents/Frameworks`; works on any Linux with glibc 2.27 or newer, Jetson and apps that ship their own runtime). Model
weights are
downloaded into the platform cache on first use and verified by SHA-256; `cargo run -p charcraft-tracking --features
onnx-dynamic --example probe -- PHOTO.jpg OVERLAY.png` runs them on a photo.

## Known gaps

- Recorded takes (`.arf` event graphs) in Character Animator projects are not decoded; scenes are performed from
  their audio instead.
- Puppets without mouth-shape layers only get a moving jaw (when they have a Flappy Chin), no visemes.
- Binaries are unsigned and there is no installer.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [BSD-3-Clause](LICENSE-BSD-3-Clause), at your option. Third-party code
and assets are listed in [ATTRIBUTION.md](ATTRIBUTION.md).

See [ARCHITECTURE.md](ARCHITECTURE.md) for the crate map and [AGENTS.md](AGENTS.md) for the rules every change
follows.

This project is not affiliated with or endorsed by Adobe. Adobe and Character Animator are trademarks of Adobe Inc.;
they are named here only to describe file compatibility.
