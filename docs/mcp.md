# CharCraft over MCP

CharCraft speaks the Model Context Protocol so an agent can open puppets, rig them, perform them, render pictures and
export video. The generic machinery (tool generation, undo, jobs, path confinement, the control protocol) lives in
`anim-mcp` (anicraft-libs, `crates/mcp`); CharCraft adds its commands and hooks in `apps/charcraft`
(`session.rs`, `hooks.rs`, `control.rs`).

## Three ways in

| How | Command | Window | Screenshots |
| --- | --- | --- | --- |
| Headless | `charcraft-cli mcp [FILE]` | none | no (answers "no window") |
| Bridge to the running app | `charcraft --control 7870` then `charcraft-cli mcp --bridge 7870` | yes | yes |
| App as the MCP server | `charcraft --mcp-stdio [--headless] [FILE]` | optional | with a window |

`.mcp.json` in the repository root registers the first two. The app writes a token file (default
`charcraft-control-PORT.token` in the temp folder, or `--token-file`) and prints the address; the bridge reads it.
`--control 0` picks a free port. Only 127.0.0.1 is listened on and every connection must present the token.

Without a FILE the built-in demo puppet is loaded, so everything works with no artwork on disk.

## Files and confinement

With an agent attached, paths are confined: reads and writes work in the working directory and the temp folder, reads
also in the folder of the opened file. Widen with `--allow-read DIR`, `--allow-write DIR` (repeatable) or
`--unrestricted`. Relative paths and `..` are refused.

## Tools

Tools come from the command registry, one per command (`rig.edit` is `rig_edit`), plus fixed tools and one
`query_*` tool per query.

Fixed: `session_status`, `list_commands`, `run_command`, `run_batch` (atomic, one undo step), `inspect_document`,
`get_property`, `set_property`, `list_queries`, `query`, `render_frame`, `screenshot`, `list_export_formats`,
`export`, `undo`, `redo`, `open_document`, `new_document`, `save_document`, `journal`.

Documents: `open_document` reads `.puppet`, `.chproj` (file or folder), `.psd`, `.ai`, `.svg` (artwork becomes a new
puppet with the usual tag guessing) and saved `.charcraft` documents; `save_document` writes `.charcraft`.

Commands:

* `scene_*`: stage size, frame rate, length, background, export/import the scene as JSON.
* `track_add`, `track_set`, `track_remove`, `track_behavior`: puppets on the stage and per-track behavior edits
  (`set_param` with the parameter name as Character Animator shows it, `set_enabled`, `add`, `remove`, `move`).
* `rig_edit`, `rig_layers`: tags, handles (Fixed, Draggable, Dangle, Attach, Stick, Walk), origins, key triggers,
  swap sets, layer visibility.
* `record_arm`, `record_punch`, `record_script`, `record_inputs`, `take_*`, `playback_poses`, `playback_inputs`:
  record a scripted or explicit performance into takes, list, rename, enable, move and delete them, read back poses.
* `puppet_reimport`: reads the artwork file again (or `path`, a new version of it) and rebuilds the puppets, keeping the
  rig: tags, handles, origins, triggers and swap sets follow their layers by path or, when a layer moved, by name;
  edits whose layers are gone are dropped and listed. `puppet_edit_original` opens the artwork in the system's
  application for it. `puppet_make` builds a rigged puppet with the Puppet Maker (`template` and/or a partial `spec`:
  head, skin, hair_style, hair, eyes, iris, lashes, ears, cheeks, lips, shirt, body) and puts it on a new track;
  `puppet_templates` lists the starter puppets. Each is one undo step.
* `lipsync_record`: reads a sound file (WAV, MP3, FLAC, Ogg, AAC), finds the mouth shapes, records them as a voice
  take and places the sound on an audio track. One undo step.
* `puppet_rename`, `puppet_duplicate`, `puppet_remove`, `puppet_import`: the project panel. Names, copies and
  removals are undoable; `puppet_import` reads a `.puppet` (or the puppets of a project) into the open project and adds
  a track for it (`add_track: false` skips that). Imported puppets are copied into the project when it is saved.
* `project_save` (a project folder or `.chproj` name; with a `.puppet` path or `format: "puppet"` and `puppet` it writes
  one puppet), `puppet_save`, `project_revert`, `project_new`: file work. These run on the host's next tick; the answer
  is `last_job` in `query_world_info`. A saved project is the opened one copied with every record kept, plus imported
  puppets, new puppet names, and the scene, takes and rig edits in sidecar files beside it. Puppets made from artwork
  (PSD, AI, SVG) have no project data to write: save those as `.charcraft`.
* `audio_*`: clips on audio tracks.
* `tracking_*`, `mic_*`: live camera and microphone (app only; fail cleanly without them). `tracking_list_cameras`
  and `tracking_start {index}` pick the camera, `tracking_set_rest_pose {target: face|body|both}` sets the rest pose
  (the body's is the posture a webcam sees through the head: position, distance, turn and tilt), `tracking_preview`
  says whether the live image and landmark points are flowing. `mic_devices`, `mic_set_device {device}` and
  `mic_set_gain {gain}` choose the microphone and its sensitivity.
* `stream_outputs`, `stream_start`, `stream_stop`, `stream_status`: send the performing puppet out of the app.
  `stream_outputs` says what each output needs on this machine, including what to install when it cannot start.
  `stream_start {kind: ndi|virtual_camera|web, name, width, height, fps, background: transparent|green|black|white|#rrggbb}`:
  `ndi` is a network source and needs the NDI runtime (loaded when it starts); `virtual_camera` writes to a
  v4l2loopback device on Linux (no alpha; transparent shows chroma green); `web` serves MJPEG video at
  `/stream.mjpg` and a PNG with alpha at `/frame.png` on `127.0.0.1`, which OBS reads as a Browser or Media source.
  Syphon and Spout are not available in this build; use NDI.
* `ui_set` drives the window (mode rig/record/stream, selection, playhead, playing; in the stream view `triggers`
  holds trigger keys down, `swaps` picks swap set members, `transparent` and `green_screen` set the backdrop,
  `stream_panels` shows the Controls, Triggers and Scenes panels).
* Full-body tracking is `tracking_body_start`, `tracking_body_stop`, `tracking_body_status`, `tracking_body_latest`
  (17 joints in body units against the body rest pose), `tracking_body_set_rest_pose` (stand or sit as the puppet
  does, arms hanging, shoulders in view), `tracking_reset_rest_pose`, `tracking_body_set_smoothing` and
  `tracking_body_mirror`. The body and face trackers share one camera.
* `track_behavior` adds, removes and edits the behaviors of a track by id, including the body and effect behaviors:
  `Charcraft.BodyTracker.js`, `Charcraft.Particles.js`, `Charcraft.Replay.js`, `Charcraft.Magnets.js`,
  `Charcraft.Wiggler.js`, `Charcraft.Fader.js`, `Charcraft.LookAtCamera.js`, `Charcraft.MotionLines.js`,
  `Charcraft.Shadow.js`, `Charcraft.LeaderFollower.js` and `Charcraft.LayerPicker.js` (see `query_behaviors_catalog`
  for their parameters). Scripted runs (`record_script`) take `body` and `no_body` events with joint positions in
  body units.

Queries: `query_puppet_summary` (layers, groups, tags, handles, mouth shapes, swap sets), `query_puppet_outline`,
`query_puppet_layer`, `query_import_warnings` (the import warnings list), `query_behaviors_catalog` and `query_behaviors_stack` (behaviors with parameters, as the track
plays them), `query_world_info`, `query_project_info` (puppets with names, folders, sizes, tracks and import sources; the scene;
audio; the document's file and whether it has unsaved changes), `query_view_state`. Scenes and takes come from
`scene_info` and `take_list`.

Rendering. `render_frame` returns a PNG as image content. With no options it draws a scene frame (`frame`, or
`options.time_seconds`). Any of these options draws one puppet in a hand-made pose instead: `puppet`, `viseme`
(Ah, D, Ee, F, L, M, Oh, R, S, Uh, W-Oo, Smile, Surprised, Neutral), `blink`, `pose` (any part of a puppet pose, for
example `{"head":{"rotation":10}}`), `triggers` (keys that are on), `swaps` (swap set to layer label),
`transparent`. `screenshot` returns the real window (bridge or windowed app).

Export. `export` with `mp4` (H.264 and AAC), `mov` (ProRes 4444 with alpha), `gif`, `png-sequence`, `png`, `wav` (the
sound alone) or `after-effects` (a folder with a PNG sequence, `audio.wav`, a ProRes 4444 MOV, `export.json` and
`REEXPORT.txt`; the closest legitimate stand-in for Dynamic Link). Options: `scale`, `start_frame`, `frames`,
`transparent`, `fps`. `list_export_formats` also lists presets (`h264-half`, `gif-web`, `audio-wav`, ...); a preset id
works as the `format`, and the options given override the preset's. There is no WebM (no pure-Rust VP9 encoder). Long
exports report progress and can be cancelled; a client queues exports by calling `export` again, and the window has an
export queue of its own (File > Export queue).

## Example session

```text
open_document {path: "hero.puppet"}
query_puppet_summary {}
rig_edit {op: "add_tag", layer: "Head/Nose", tag: "Nose"}
render_frame {options: {viseme: "Oh", pose: {head: {rotation: 8}}}}
lipsync_record {path: "line.wav", track: 0}
export {format: "mp4", path: "line.mp4"}
```

## Tests

`apps/charcraft-cli/tests/mcp.rs` runs real JSON-RPC sessions against the binary: tool list, tags/handles/behaviors,
rendering, a scripted performance, lip sync of a synthesized sound followed by an MP4 export that is demuxed again,
save/open/import, confinement, hostile input, and the bridge (against the control channel without a window).
`CHARCRAFT_GUI_TESTS=1 CHARCRAFT_SHOTS=DIR` (with a display and a built `charcraft`) also bridges to the real window
and saves `charcraft-mcp-{rig,record,stream}.png`.
