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
* `lipsync_record`: reads a sound file (WAV, MP3, FLAC, Ogg, AAC), finds the mouth shapes, records them as a voice
  take and places the sound on an audio track. One undo step.
* `audio_*`: clips on audio tracks.
* `tracking_*`, `mic_*`: live camera and microphone (app only; fail cleanly without them). `ui_set` drives the
  window (mode rig/record/stream, selection, playhead, playing).

Queries: `query_puppet_summary` (layers, groups, tags, handles, mouth shapes, swap sets), `query_puppet_outline`,
`query_puppet_layer`, `query_behaviors_catalog` and `query_behaviors_stack` (behaviors with parameters, as the track
plays them), `query_world_info`, `query_view_state`. Scenes and takes come from `scene_info` and `take_list`.

Rendering. `render_frame` returns a PNG as image content. With no options it draws a scene frame (`frame`, or
`options.time_seconds`). Any of these options draws one puppet in a hand-made pose instead: `puppet`, `viseme`
(Ah, D, Ee, F, L, M, Oh, R, S, Uh, W-Oo, Smile, Surprised, Neutral), `blink`, `pose` (any part of a puppet pose, for
example `{"head":{"rotation":10}}`), `triggers` (keys that are on), `swaps` (swap set to layer label),
`transparent`. `screenshot` returns the real window (bridge or windowed app).

Export. `export` with `mp4` (H.264 and AAC), `mov` (ProRes 4444 with alpha), `png-sequence` or `png`; long exports
report progress and can be cancelled.

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
