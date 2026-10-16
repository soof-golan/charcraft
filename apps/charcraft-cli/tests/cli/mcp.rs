//! `charcraft-cli mcp` as a real MCP server over stdio: JSON-RPC from the first `initialize`, the tools generated
//! from the commands and queries, rendering that returns PNGs, lip sync of a synthesized sound, exporting a video that
//! is demuxed again, saving and opening, path confinement, and the bridge to an app that serves the control channel.
//! None of it needs sample files or a display.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "test code"
)]

use crate::common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::{decode_png, demux, speechlike_wav};
use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

impl Client {
    fn spawn(args: &[&str]) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_charcraft-cli"))
            .arg("mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self::connect(child)
    }

    fn connect(mut child: Child) -> Self {
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut client = Self { child, stdin, lines, next_id: 1 };
        let init =
            client.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }));
        assert_eq!(init["serverInfo"]["name"], "charcraft");
        assert!(init["instructions"].as_str().unwrap().contains("lipsync_record"));
        client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        client
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(120)).expect("no reply from charcraft-cli mcp");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout must carry only JSON-RPC, got {line:?}: {e}"));
            if message.get("id") == Some(&json!(id)) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    /// Calls a tool; returns the structured content and whether it was an error.
    fn tool(&mut self, name: &str, args: Value) -> (Value, bool) {
        let result = self.request("tools/call", json!({ "name": name, "arguments": args }));
        (result["structuredContent"].clone(), result["isError"] == true)
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let (value, is_error) = self.tool(name, args);
        assert!(!is_error, "{name} failed: {value}");
        value
    }

    /// The command's own answer (tools that run commands wrap it in `result`).
    fn run(&mut self, name: &str, args: Value) -> Value {
        self.ok(name, args)["result"].clone()
    }

    fn images(&mut self, name: &str, args: Value) -> Vec<Vec<u8>> {
        use base64::Engine as _;
        let result = self.request("tools/call", json!({ "name": name, "arguments": args }));
        assert_ne!(result["isError"], true, "{result}");
        result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "image")
            .map(|c| base64::engine::general_purpose::STANDARD.decode(c["data"].as_str().unwrap()).unwrap())
            .collect()
    }

    fn render(&mut self, options: Value) -> common::Pixels {
        let images = self.images("render_frame", json!({ "scale": 0.5, "options": options }));
        assert_eq!(images.len(), 1);
        assert_eq!(&images[0][..8], b"\x89PNG\r\n\x1a\n");
        decode_png(&images[0])
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

#[test]
fn tools_are_generated_from_the_commands_and_queries() {
    let mut client = Client::spawn(&[]);
    let tools = client.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "session_status",
        "open_document",
        "save_document",
        "render_frame",
        "screenshot",
        "export",
        "run_batch",
        "undo",
        "rig_edit",
        "rig_layers",
        "track_add",
        "track_behavior",
        "record_script",
        "take_list",
        "playback_poses",
        "audio_add_clip",
        "lipsync_record",
        "scene_info",
        "query_puppet_summary",
        "query_puppet_layer",
        "query_behaviors_catalog",
        "query_behaviors_stack",
    ] {
        assert!(names.contains(&expected), "{expected} is missing from {names:?}");
    }
    let rig = tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == "rig_edit").unwrap();
    assert_eq!(rig["inputSchema"]["required"], json!(["op"]));
    let formats = client.ok("list_export_formats", json!({}));
    let ids: Vec<&str> = formats["formats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(&ids[..7], ["mp4", "mov", "gif", "png-sequence", "png", "wav", "after-effects"]);
    for preset in ["h264-match-source", "prores-4444-alpha", "gif-web", "audio-wav", "after-effects-package"] {
        assert!(ids.contains(&preset), "preset {preset} is missing from {ids:?}");
    }
}

#[test]
fn the_puppet_tree_tags_and_behaviors_can_be_read_and_edited() {
    let mut client = Client::spawn(&[]);
    let summary = client.ok("query_puppet_summary", json!({}));
    assert_eq!(summary["puppet"], "demo");
    let outline = client.ok("query_puppet_outline", json!({}))["outline"].as_str().unwrap().to_owned();
    assert!(outline.contains("Head") && outline.contains("Nose"), "{outline}");
    let before = client.ok("query_puppet_layer", json!({ "layer": "Head/Nose" }));

    // Tags and handles: edit, read back, undo.
    client.run("rig_edit", json!({ "op": "add_tag", "layer": "Head/Nose", "tag": "Smile" }));
    let after = client.ok("query_puppet_layer", json!({ "layer": "Head/Nose" }));
    assert_ne!(before, after, "the tag shows on the layer");
    assert!(after.to_string().contains("Smile"), "{after}");
    let handle = client.run("rig_edit", json!({ "op": "add_handle", "group": "Head", "name": "Chin", "kind": "Draggable", "position": [320.0, 400.0] }));
    assert_eq!(handle["key"], "Head/Chin");
    assert!(client.ok("query_puppet_summary", json!({})).to_string().contains("Chin"));
    let (err, is_error) = client.tool("rig_edit", json!({ "op": "add_tag", "layer": "No such layer", "tag": "x" }));
    assert!(is_error, "{err}");
    client.ok("undo", json!({}));
    client.ok("undo", json!({}));
    assert_eq!(client.ok("query_puppet_layer", json!({ "layer": "Head/Nose" })), before);

    // Behaviors by their Character Animator names, with parameters; a track edit changes one and undoes.
    let catalog = client.ok("query_behaviors_catalog", json!({}));
    let names: Vec<&str> = catalog["behaviors"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
    for expected in ["Face", "Lip Sync", "Auto Blink", "Breathe", "Triggers", "Dragger", "Physics"] {
        assert!(names.contains(&expected), "{expected} is not in {names:?}");
    }
    let stack = client.ok("query_behaviors_stack", json!({ "track": 0 }));
    let slots = stack["stack"]["behaviors"].as_array().unwrap();
    let (index, blinker) = slots.iter().enumerate().find(|(_, b)| b["name"] == "Auto Blink").expect("the stack has Auto Blink");
    // Parameters are a name to value map, named as Character Animator shows them.
    let (param, value) = blinker["params"].as_object().unwrap().iter().find(|(_, v)| v.is_number()).expect("Auto Blink has a numeric parameter");
    let (param, value) = (param.clone(), value.as_f64().unwrap());
    client.run("track_behavior", json!({ "track": 0, "op": { "op": "set_param", "index": index, "name": param, "value": value + 1.0 } }));
    let stack = client.ok("query_behaviors_stack", json!({ "track": 0 }));
    assert_eq!(stack["stack"]["behaviors"][index]["params"][&param].as_f64(), Some(value + 1.0), "{param}");
    client.run("track_behavior", json!({ "track": 0, "op": { "op": "set_enabled", "index": index, "enabled": false } }));
    let stack = client.ok("query_behaviors_stack", json!({ "track": 0 }));
    assert_eq!(stack["stack"]["behaviors"][index]["enabled"], false, "{param}");
    let (err, is_error) =
        client.tool("track_behavior", json!({ "track": 0, "op": { "op": "set_param", "index": index, "name": "No Such Parameter", "value": 1 } }));
    assert!(is_error, "{err}");
}

#[test]
fn mouth_shapes_poses_triggers_and_times_render_to_pngs() {
    let mut client = Client::spawn(&[]);
    let neutral = client.render(json!({ "puppet": "demo" }));
    assert!(neutral.drawn() > 5_000, "the puppet is drawn: {}", neutral.drawn());
    // Every mouth shape draws something, and the open ones differ from the closed one.
    let ah = client.render(json!({ "viseme": "Ah" }));
    let oh = client.render(json!({ "viseme": "Oh" }));
    assert!(neutral.differing(&ah) > 50);
    assert!(ah.differing(&oh) > 50);
    // A hand-made pose: the head turned moves pixels; a blink closes the eyes.
    let tilted = client.render(json!({ "pose": { "head": { "rotation": 20.0 } } }));
    assert!(neutral.differing(&tilted) > 200);
    let blink = client.render(json!({ "blink": true }));
    assert!(neutral.differing(&blink) > 20);
    // A transparent background has a clear corner.
    let clear = client.render(json!({ "puppet": "demo", "transparent": true }));
    assert_eq!(clear.rgba[3], 0, "the corner is clear");
    assert!(clear.rgba.chunks(4).any(|p| p[3] == 255), "the puppet is solid");
    assert_eq!(neutral.rgba[3], 255, "the default background is opaque");
    // The scene at a time: the same frame by time and by frame number.
    let by_frame = client.images("render_frame", json!({ "frame": 15, "scale": 0.25 }));
    let by_time = client.images("render_frame", json!({ "scale": 0.25, "options": { "time_seconds": 0.5 } }));
    assert_eq!(by_frame, by_time);
    // Nonsense is refused with a reason, not drawn.
    for bad in [json!({ "viseme": "Nonsense" }), json!({ "pose": 3 }), json!({ "time_seconds": -2.0 }), json!({ "puppet": "nobody" })] {
        let (err, is_error) = client.tool("render_frame", json!({ "options": bad }));
        assert!(is_error, "{bad} should be refused");
        assert!(err["error"]["message"].is_string(), "{err}");
    }
    let (_, past_end) = client.tool("render_frame", json!({ "frame": 100_000 }));
    assert!(past_end);
}

#[test]
fn a_scripted_performance_becomes_a_take_with_the_mouth_shapes_in_it() {
    let mut client = Client::spawn(&[]);
    client.run("scene_set", json!({ "duration_seconds": 2.0 }));
    client.run("record_arm", json!({ "groups": ["mouth", "voice"], "armed": true, "only": true }));
    let recorded = client.run(
        "record_script",
        json!({ "track": 0, "script": { "fps": 30.0, "frames": 60, "events": [
            { "event": "voice", "frame": 0, "viseme": "Neutral" },
            { "event": "voice", "frame": 15, "viseme": "Oh" },
            { "event": "voice", "frame": 40, "viseme": "Neutral" },
        ] } }),
    );
    assert!(recorded["take"].is_object(), "{recorded}");
    let takes = client.run("take_list", json!({ "track": 0 }));
    assert!(!takes["takes"].as_array().unwrap().is_empty(), "{takes}");
    let poses = client.run("playback_poses", json!({ "track": 0, "from_frame": 0, "count": 60, "step": 10 }));
    let text = poses.to_string();
    assert!(text.contains("Oh"), "the played back poses carry the recorded mouth shape: {text}");
    // One undo takes the take away.
    client.ok("undo", json!({}));
    let takes = client.run("take_list", json!({ "track": 0 }));
    assert!(takes["takes"].as_array().unwrap().is_empty(), "{takes}");
}

#[test]
fn lip_sync_of_a_synthesized_sound_then_export_an_mp4_that_demuxes() {
    let dir = tempfile::tempdir().unwrap();
    let wav = speechlike_wav(dir.path());
    let mut client = Client::spawn(&["--allow-read", &text(dir.path()), "--allow-write", &text(dir.path())]);
    client.run("scene_set", json!({ "width": 320, "height": 180, "duration_seconds": 2.0 }));
    let synced = client.run("lipsync_record", json!({ "track": 0, "path": text(&wav) }));
    assert_eq!(synced["media"], "hello there");
    assert!(synced["keys"].as_u64().unwrap() >= 2, "{synced}");
    assert!(synced["seconds"].as_f64().unwrap() > 1.9);
    let scene = client.run("scene_info", json!({}));
    assert_eq!(scene["audio"][0]["clips"].as_array().unwrap().len(), 1, "{scene}");
    // The take drives the mouth: some played back pose has a mouth shape other than the closed one.
    let poses = client.run("playback_poses", json!({ "track": 0, "from_frame": 0, "count": 60, "step": 2 }));
    let shapes: std::collections::BTreeSet<&str> = poses["poses"].as_array().unwrap().iter().filter_map(|p| p["viseme"].as_str()).collect();
    assert!(shapes.len() >= 3 && shapes.contains("Neutral"), "the voice moves the mouth through several shapes: {shapes:?}");

    let mp4 = dir.path().join("talk.mp4");
    let done = client.ok("export", json!({ "format": "mp4", "path": text(&mp4) }));
    assert_eq!(done["frames"], 61, "two seconds at 30 fps include both end frames: {done}");
    let bytes = std::fs::read(&mp4).unwrap();
    let tracks = demux(&bytes);
    let video = tracks.iter().find(|t| t.handler == "vide").expect("a video track");
    assert_eq!((video.width, video.height, video.samples), (320, 180, 61), "{tracks:?}");
    assert_eq!(video.codec, "avc1");
    assert!((video.seconds() - 2.0).abs() < 0.1, "{}", video.seconds());
    let audio = tracks.iter().find(|t| t.handler == "soun").expect("the sound is in the file");
    assert_eq!(audio.codec, "mp4a");
    assert!(audio.samples > 20 && audio.seconds() > 1.5, "{audio:?}");

    // A PNG sequence of the same scene, and one PNG.
    let frames = dir.path().join("frames");
    let done = client.ok("export", json!({ "format": "png-sequence", "path": text(&frames), "options": { "frames": 3 } }));
    assert_eq!(done["frames"], 3, "{done}");
    assert_eq!(common::read_png(&frames.join("frame_00001.png")).width, 320);
    let (err, is_error) = client.tool("export", json!({ "format": "avi", "path": text(&dir.path().join("x.avi")) }));
    assert!(is_error, "{err}");

    // A GIF through a preset (half size, 15 fps), the sound alone, and the After Effects package.
    let gif = dir.path().join("talk.gif");
    let done = client.ok("export", json!({ "format": "gif-web", "path": text(&gif), "options": { "frames": 30 } }));
    assert_eq!((done["frames"].as_u64().unwrap(), done["width"].as_u64().unwrap()), (15, 160), "{done}");
    assert_eq!(&std::fs::read(&gif).unwrap()[..6], b"GIF89a");
    let wav = dir.path().join("talk.wav");
    client.ok("export", json!({ "format": "wav", "path": text(&wav) }));
    assert_eq!(&std::fs::read(&wav).unwrap()[..4], b"RIFF");
    let package = dir.path().join("talk-ae");
    let done = client.ok("export", json!({ "format": "after-effects", "path": text(&package), "options": { "frames": 6, "scale": 0.5 } }));
    assert_eq!(done["audio"], true, "{done}");
    for file in ["talk-ae.mov", "audio.wav", "export.json", "REEXPORT.txt", "frames/frame_00006.png"] {
        assert!(package.join(file).is_file(), "{file} is missing");
    }
}

#[test]
fn documents_save_open_and_import_artwork() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::spawn(&["--allow-read", &text(dir.path()), "--allow-write", &text(dir.path())]);
    client.run("rig_edit", json!({ "op": "add_tag", "layer": "Head/Nose", "tag": "Smile" }));
    client.run("scene_set", json!({ "duration_seconds": 3.0 }));
    let saved = dir.path().join("work.charcraft");
    client.ok("save_document", json!({ "path": text(&saved) }));
    assert!(saved.is_file());
    client.ok("new_document", json!({}));
    assert_eq!(client.run("scene_info", json!({}))["frames"], 240, "a new document starts from the demo scene");
    client.ok("open_document", json!({ "path": text(&saved) }));
    assert_eq!(client.run("scene_info", json!({}))["frames"], 90);
    assert!(client.ok("query_puppet_layer", json!({ "layer": "Head/Nose" })).to_string().contains("Smile"), "the rig edit survived the file");

    // Artwork becomes a new puppet.
    let svg = dir.path().join("toy.svg");
    std::fs::write(
        &svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><g id="Head"><circle id="face" cx="100" cy="90" r="70" fill="#f5c9a5"/>
<g id="Mouth"><ellipse id="Neutral" cx="100" cy="125" rx="20" ry="3" fill="#a33"/><ellipse id="Oh" cx="100" cy="128" rx="9" ry="11" fill="#a33"/></g></g></svg>"##,
    )
    .unwrap();
    client.ok("open_document", json!({ "path": text(&svg) }));
    let info = client.ok("query_world_info", json!({}));
    assert_eq!(info["kind"], "artwork", "{info}");
    let picture = client.render(json!({ "viseme": "Oh" }));
    assert!(picture.drawn() > 1_000);
    let (err, is_error) = client.tool("open_document", json!({ "path": text(&dir.path().join("missing.puppet")) }));
    assert!(is_error, "{err}");
}

#[test]
fn agents_cannot_reach_outside_the_folders_they_were_given() {
    // The process keeps the package folder and the temp folder; the folder above the package is outside both.
    let allowed = tempfile::tempdir().unwrap();
    let elsewhere = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let mut client = Client::spawn(&["--allow-read", &text(allowed.path()), "--allow-write", &text(allowed.path())]);
    let (err, is_error) = client.tool("open_document", json!({ "path": text(&elsewhere.join("README.md")) }));
    assert!(is_error);
    assert_eq!(err["error"]["kind"], "denied", "{err}");
    let (err, is_error) = client.tool("export", json!({ "format": "png", "path": text(&elsewhere.join("leak.png")) }));
    assert!(is_error && err["error"]["kind"] == "denied", "{err}");
    assert!(!elsewhere.join("leak.png").exists());
    let (err, is_error) = client.tool("lipsync_record", json!({ "path": text(&elsewhere.join("README.md")) }));
    assert!(is_error && err["error"]["kind"] == "denied", "{err}");
    let (err, is_error) = client.tool("save_document", json!({ "path": "relative.charcraft" }));
    assert!(is_error && err["error"]["kind"] == "denied", "{err}");
    // Inside the allowed folder it works.
    client.ok("export", json!({ "format": "png", "path": text(&allowed.path().join("ok.png")) }));
    assert!(allowed.path().join("ok.png").is_file());
}

#[test]
fn hostile_calls_get_errors_not_crashes() {
    let mut client = Client::spawn(&[]);
    for (tool, args) in [
        ("record_script", json!({ "track": 0, "script": { "frames": 4_000_000_000_u64 } })),
        ("record_script", json!({ "track": 99, "script": { "frames": 3 } })),
        ("take_move", json!({ "track": 0, "from": 5, "to": 9 })),
        ("scene_set", json!({ "width": -4, "height": 0 })),
        ("rig_edit", json!({ "op": "set_origin", "group": "Head", "origin": ["NaN", 0] })),
        ("lipsync_record", json!({ "path": "", "track": 0 })),
        ("query_puppet_layer", json!({})),
    ] {
        let result = client.request("tools/call", json!({ "name": tool, "arguments": args }));
        assert_eq!(result["isError"], true, "{tool} {args} should be refused: {result}");
    }
    // Absurd placements are cleaned up or refused, and the scene still renders afterwards.
    let _ = client.tool("track_add", json!({ "puppet": "demo", "placement": { "scale": [1e300, 1e300], "rotation": 1e300, "opacity": 7 } }));
    let _ = client.tool("scene_set", json!({ "width": 100_000, "height": 100_000 }));
    let _ = client.tool("render_frame", json!({ "frame": 0, "scale": 16.0 }));
    // Still alive and well.
    assert_eq!(client.ok("session_status", json!({}))["app"]["puppets"], json!(["demo"]));
}

// --- the bridge ---------------------------------------------------------------------------------------------------

/// An app without a window: the host and its control channel, pumped on a thread the way the window pumps them once
/// per frame. What a bridge client sees is exactly what it sees from the real app.
struct HeadlessApp {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    token_file: std::path::PathBuf,
}

impl HeadlessApp {
    fn start(dir: &Path) -> Self {
        use anim_mcp::{Confinement, ControlHost, HostInfo};
        use charcraft_app::control::{Control, ControlArgs};
        use charcraft_app::hooks::make_host;
        use charcraft_app::live::LiveLink;
        use charcraft_app::session::{Links, registry};
        use charcraft_app::world::World;

        let token_file = dir.join("control.token");
        let (stop, stopped) = mpsc::channel::<()>();
        let (ready_tx, ready) = mpsc::channel::<()>();
        let file = token_file.clone();
        let thread = std::thread::spawn(move || {
            let opened = World::demo().unwrap();
            let (ui, _no_window) = crossbeam_channel::unbounded();
            let links = Links::new(LiveLink::disconnected(), ui);
            let info = HostInfo { name: "charcraft".into(), version: "test".into() };
            let mut host = make_host(opened, links, registry().unwrap(), Confinement::unrestricted(), info);
            let args = ControlArgs { control_port: Some(0), token_file: Some(file), ..ControlArgs::default() };
            let control = Control::start(&args, || {}).unwrap();
            ready_tx.send(()).unwrap();
            while matches!(stopped.recv_timeout(Duration::from_millis(5)), Err(mpsc::RecvTimeoutError::Timeout)) {
                control.pump(&mut host);
                host.tick();
            }
        });
        ready.recv_timeout(Duration::from_secs(60)).expect("the control channel starts");
        Self { stop: Some(stop), thread: Some(thread), token_file }
    }

    fn port(&self) -> String {
        let (_, addr) = anim_mcp::read_token_file(&self.token_file).unwrap();
        addr.port().to_string()
    }
}

impl Drop for HeadlessApp {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn bridge_client(port: &str, token_file: &Path) -> Client {
    Client::spawn(&["--bridge", port, "--token-file", &text(token_file)])
}

#[test]
fn the_bridge_forwards_every_tool_to_a_running_app() {
    let dir = tempfile::tempdir().unwrap();
    let app = HeadlessApp::start(dir.path());
    let mut client = bridge_client(&app.port(), &app.token_file);
    let status = client.ok("session_status", json!({}));
    assert_eq!(status["app"]["puppets"], json!(["demo"]));
    assert_eq!(status["app"]["windowed"], false);
    // Edits through the bridge are seen by a second client of the same app.
    client.run("rig_edit", json!({ "op": "add_tag", "layer": "Head/Nose", "tag": "Smile" }));
    let mut other = bridge_client(&app.port(), &app.token_file);
    assert!(other.ok("query_puppet_layer", json!({ "layer": "Head/Nose" })).to_string().contains("Smile"));
    other.ok("undo", json!({}));
    assert!(!client.ok("query_puppet_layer", json!({ "layer": "Head/Nose" })).to_string().contains("Smile"));
    // Renders and exports cross the bridge, progress and all.
    let picture = client.render(json!({ "viseme": "Oh" }));
    assert!(picture.drawn() > 5_000);
    let out = dir.path().join("frame.png");
    client.ok("export", json!({ "format": "png", "path": text(&out) }));
    assert!(out.is_file());
    // No window: the screenshot says so instead of hanging.
    let (err, is_error) = client.tool("screenshot", json!({}));
    assert!(is_error, "{err}");
    assert!(err["error"]["message"].as_str().unwrap().contains("window"), "{err}");
    // Lip sync reads the file on the app's side.
    let wav = speechlike_wav(dir.path());
    let synced = client.run("lipsync_record", json!({ "path": text(&wav) }));
    assert!(synced["keys"].as_u64().unwrap() >= 2);
}

#[test]
fn the_bridge_says_why_it_cannot_connect() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_charcraft-cli"))
        .args(["mcp", "--bridge", "1", "--token-file", &text(&dir.path().join("none.token"))])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout is for protocol messages only");
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(why.contains("could not connect") && why.contains("charcraft --control"), "{why}");
}

/// Kills the child process when dropped.
struct Reaper(Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The real app in a real window, reached through the bridge. Opt in with `CHARCRAFT_GUI_TESTS=1` (it needs a display
/// and a built `charcraft` binary next to the test binaries); screenshots go to `CHARCRAFT_SHOTS` when set.
#[test]
fn bridge_to_the_real_window_takes_screenshots() {
    if std::env::var_os("CHARCRAFT_GUI_TESTS").is_none() || std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipped: set CHARCRAFT_GUI_TESTS=1 and DISPLAY to run the window test");
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let app_binary = exe.parent().and_then(Path::parent).unwrap().join(format!("charcraft{}", std::env::consts::EXE_SUFFIX));
    if !app_binary.is_file() {
        eprintln!("skipped: build the app first ({} is missing)", app_binary.display());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let token_file = dir.path().join("control.token");
    let _app = Reaper(
        Command::new(&app_binary)
            .args(["--control", "0", "--token-file", &text(&token_file), "--demo-takes", "--live-demo"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let started = std::time::Instant::now();
    while !token_file.is_file() {
        assert!(started.elapsed() < Duration::from_secs(120), "the app never opened its control channel");
        std::thread::sleep(Duration::from_millis(200));
    }
    let (_, addr) = anim_mcp::read_token_file(&token_file).unwrap();
    let mut client = bridge_client(&addr.port().to_string(), &token_file);
    let status = client.ok("session_status", json!({}));
    assert_eq!(status["app"]["windowed"], true, "{status}");
    let shots = std::env::var_os("CHARCRAFT_SHOTS").map_or_else(|| dir.path().to_path_buf(), std::path::PathBuf::from);
    std::fs::create_dir_all(&shots).unwrap();
    let mut seen = Vec::new();
    for mode in ["rig", "record", "stream"] {
        client.ok("ui_set", json!({ "mode": mode }));
        std::thread::sleep(Duration::from_millis(1_500));
        let images = client.images("screenshot", json!({}));
        assert_eq!(images.len(), 1);
        let picture = decode_png(&images[0]);
        assert!(picture.width >= 800 && picture.height >= 500, "{}x{}", picture.width, picture.height);
        assert!(picture.drawn() > picture.rgba.len() / 4 / 10, "the window is not blank");
        std::fs::write(shots.join(format!("charcraft-mcp-{mode}.png")), &images[0]).unwrap();
        seen.push(picture);
    }
    assert!(seen[0].differing(&seen[2]) > 1_000, "the modes look different");
    assert_eq!(client.ok("query_view_state", json!({}))["mode"], "stream");
}
