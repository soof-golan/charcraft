//! The tool surface end to end: an `anim-session` session over a `RecordDoc`, driven through the MCP server the
//! way a client would drive it (JSON-RPC in, JSON out).
#![allow(
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]
#![allow(clippy::assert_is_empty, reason = "test code")]

use anim_core::{FrameRate, Size, Tick};
use anim_session::{McpServer, Session};
use charcraft_record::{RecordDoc, Scene, registry};
use serde_json::{Value, json};

fn session() -> Session<RecordDoc> {
    let scene = Scene::new("Demo", Size::new(1280.0, 720.0), FrameRate::FPS_30, Tick::from_seconds_f64(4.0));
    Session::new(RecordDoc::new(scene), registry().unwrap())
}

fn server() -> McpServer {
    McpServer::new("charcraft-record", "0.1.0")
}

fn rpc(method: &str, params: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

/// Calls a tool and returns its structured result, panicking with the error text when it failed.
fn ok(s: &mut Session<RecordDoc>, name: &str, args: &Value) -> Value {
    let reply = server().handle(s, &rpc("tools/call", &json!({ "name": name, "arguments": args }))).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{name} failed: {}", reply["result"]["content"][0]["text"]);
    reply["result"]["structuredContent"].clone()
}

fn failure(s: &mut Session<RecordDoc>, name: &str, args: &Value) -> String {
    let reply = server().handle(s, &rpc("tools/call", &json!({ "name": name, "arguments": args }))).unwrap();
    assert_eq!(reply["result"]["isError"], true, "{name} should have failed: {reply}");
    reply["result"]["content"][0]["text"].as_str().unwrap().to_owned()
}

fn performance(roll: f64, voice_at: u32) -> Value {
    json!({
        "fps": 30,
        "frames": 90,
        "events": [
            { "event": "face", "frame": 0, "face": { "head_roll": roll, "eyebrow_left": 0.5, "mouth_open": 0.3 } },
            { "event": "voice", "frame": voice_at, "viseme": "Ah" },
            { "event": "key", "frame": 10, "key": "space", "down": true },
        ]
    })
}

#[test]
fn the_server_offers_every_command_as_a_tool_with_a_schema() {
    let mut s = session();
    let init = server().handle(&mut s, &rpc("initialize", &json!({ "protocolVersion": "2025-06-18" }))).unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "charcraft-record");
    let list = server().handle(&mut s, &rpc("tools/list", &Value::Null)).unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "scene_info",
        "scene_set",
        "scene_export",
        "scene_import",
        "track_add",
        "track_remove",
        "track_set",
        "track_behavior",
        "record_arm",
        "record_punch",
        "record_state",
        "record_script",
        "record_inputs",
        "take_list",
        "take_set",
        "take_remove",
        "take_move",
        "audio_add_clip",
        "audio_set_clip",
        "audio_remove_clip",
        "audio_set_track",
        "playback_inputs",
        "playback_poses",
        "edit_undo",
        "edit_redo",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}: {names:?}");
    }
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        assert!(!tool["description"].as_str().unwrap().is_empty(), "{}", tool["name"]);
    }
    let by_name = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap();
    assert_eq!(by_name("playback_poses")["annotations"]["readOnlyHint"], true);
    assert_eq!(by_name("record_script")["annotations"]["readOnlyHint"], false);
    assert_eq!(by_name("record_script")["inputSchema"]["required"], json!(["track", "script"]));
    assert_eq!(by_name("record_arm")["inputSchema"]["properties"]["groups"]["items"]["enum"][0], "head");
}

#[test]
fn a_session_records_stacks_and_plays_back_takes() {
    let mut s = session();
    ok(&mut s, "scene_set", &json!({ "fps_num": 30, "fps_den": 1, "width": 960, "height": 540, "duration_seconds": 3, "seed": 5 }));
    let added = ok(&mut s, "track_add", &json!({ "puppet": "ProjectItems/Hero", "name": "Hero" }));
    assert_eq!(added["track"], 0);

    // Nothing armed: recording refuses.
    assert!(failure(&mut s, "record_script", &json!({ "track": 0, "script": performance(10.0, 30) })).contains("nothing is armed"));

    let armed = ok(&mut s, "record_arm", &json!({ "groups": ["head", "voice"], "only": true }));
    assert_eq!(armed["armed_groups"], json!(["head", "voice"]));
    let rec = ok(&mut s, "record_script", &json!({ "track": 0, "script": performance(20.0, 30) }));
    assert_eq!(rec["take"]["number"], 1);
    assert_eq!(rec["take"]["groups"], json!(["head", "voice"]));
    assert!(rec["take"]["samples"].as_u64().unwrap() >= 2);

    // A second take of just the brows and keys, starting later: layers per group over the first.
    ok(&mut s, "record_arm", &json!({ "groups": ["brows", "keys"], "only": true }));
    ok(&mut s, "record_script", &json!({ "track": 0, "script": performance(-40.0, 5), "start_seconds": 1.0 }));
    let takes = ok(&mut s, "take_list", &json!({ "track": 0 }));
    assert_eq!(takes["takes"].as_array().unwrap().len(), 2);
    assert_eq!(takes["takes"][1]["start_seconds"], 1.0);

    // At 0.5 s only the first take speaks; at 1.5 s the second adds brows and keys but leaves head and voice alone.
    let early = ok(&mut s, "playback_inputs", &json!({ "track": 0, "time_seconds": 0.5 }));
    assert_eq!(early["face"]["head_roll"], 20.0);
    assert_eq!(early["viseme"], Value::Null);
    assert_eq!(early["keys"], json!([]));
    let late = ok(&mut s, "playback_inputs", &json!({ "track": 0, "time_seconds": 1.5 }));
    assert_eq!(late["face"]["head_roll"], 20.0, "the head still comes from take 1");
    assert_eq!(late["face"]["eyebrow_left"], 0.5, "the brows come from take 2");
    assert_eq!(late["viseme"], "Ah");
    assert_eq!(late["keys"], json!(["space"]));

    // Playback is deterministic and the head settles towards the recorded roll.
    let args = json!({ "track": 0, "from_frame": 0, "count": 4, "step": 30 });
    let a = ok(&mut s, "playback_poses", &args);
    let b = ok(&mut s, "playback_poses", &args);
    assert_eq!(a, b);
    let poses = a["poses"].as_array().unwrap();
    assert_eq!(poses.len(), 4);
    assert!(poses[2]["head_rotation_degrees"].as_f64().unwrap().abs() > 1.0, "{poses:?}");
    assert_eq!(poses[2]["viseme"], "Ah");
    let full = ok(&mut s, "playback_poses", &json!({ "track": 0, "from_frame": 60, "count": 1, "detail": "full" }));
    assert!(full["poses"][0]["pose"]["head"]["rotation"].as_f64().unwrap().abs() > 0.01);

    // Taking the second take out of play and fading it.
    ok(&mut s, "take_set", &json!({ "track": 0, "take": 1, "blend_in_seconds": 0.5 }));
    ok(&mut s, "take_set", &json!({ "track": 0, "take": 1, "enabled": false }));
    let off = ok(&mut s, "playback_inputs", &json!({ "track": 0, "time_seconds": 1.5 }));
    assert_eq!(off["keys"], json!([]));
    ok(&mut s, "take_move", &json!({ "track": 0, "from": 1, "to": 0 }));
    let info = ok(&mut s, "scene_info", &json!({}));
    assert_eq!(info["tracks"][0]["takes"][0]["number"], 2);
    assert_eq!(info["tracks"][0]["takes"].as_array().unwrap().len(), 2);
    ok(&mut s, "take_remove", &json!({ "track": 0, "take": 0 }));
    assert_eq!(ok(&mut s, "take_list", &json!({ "track": 0 }))["takes"].as_array().unwrap().len(), 1);
}

#[test]
fn punch_in_limits_what_is_recorded() {
    let mut s = session();
    ok(&mut s, "track_add", &json!({ "puppet": "p" }));
    ok(&mut s, "record_arm", &json!({ "groups": ["head"] }));
    ok(&mut s, "record_punch", &json!({ "start_seconds": 1.0, "end_seconds": 2.0 }));
    let rec = ok(&mut s, "record_script", &json!({ "track": 0, "script": performance(15.0, 30) }));
    assert_eq!(rec["take"]["start_seconds"], 1.0);
    assert!((rec["take"]["duration_seconds"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    assert!(failure(&mut s, "record_punch", &json!({ "start_seconds": 2.0, "end_seconds": 1.0 })).contains("end after"));
    let cleared = ok(&mut s, "record_punch", &json!({}));
    assert_eq!(cleared["punch"], Value::Null);
    // A range the performance never reaches records nothing and says so.
    ok(&mut s, "record_punch", &json!({ "start_seconds": 50.0, "end_seconds": 51.0 }));
    let none = ok(&mut s, "record_script", &json!({ "track": 0, "script": performance(15.0, 30) }));
    assert_eq!(none["take"], Value::Null);
}

#[test]
fn edits_are_undoable_and_failures_change_nothing() {
    let mut s = session();
    ok(&mut s, "track_add", &json!({ "puppet": "p" }));
    ok(&mut s, "track_set", &json!({ "track": 0, "placement": { "position": { "x": 100.0, "y": -20.0 }, "scale": { "x": 0.5, "y": 0.5 } } }));
    assert_eq!(s.doc().scene.tracks[0].placement.position.x, 100.0);
    ok(&mut s, "edit_undo", &Value::Null);
    assert_eq!(s.doc().scene.tracks[0].placement.position.x, 0.0);
    ok(&mut s, "edit_undo", &Value::Null);
    assert!(s.doc().scene.tracks.is_empty());
    ok(&mut s, "edit_redo", &Value::Null);
    assert_eq!(s.doc().scene.tracks.len(), 1);

    let before = s.doc().scene.clone();
    for (tool, args) in [
        ("track_set", json!({ "track": 7, "enabled": false })),
        ("record_arm", json!({ "groups": ["elbows"] })),
        ("record_script", json!({ "track": 0, "script": { "fps": 0, "frames": 5 } })),
        ("record_script", json!({ "track": 0, "script": { "fps": 30, "frames": 5 }, "start_seconds": 1e300 })),
        ("scene_set", json!({ "fps_num": 0 })),
        ("scene_set", json!({ "width": -4 })),
        ("scene_set", json!({ "colour": "red" })),
        ("take_set", json!({ "track": 0, "take": 3, "enabled": true })),
        ("playback_poses", json!({ "track": 0, "detail": "everything" })),
        ("audio_set_clip", json!({ "clip": 0 })),
        ("track_behavior", json!({ "track": 0, "op": { "op": "remove", "index": 500 } })),
    ] {
        failure(&mut s, tool, &args);
        assert_eq!(s.doc().scene, before, "{tool} must leave the scene alone");
    }
}

#[test]
fn track_behavior_edits_change_playback() {
    let mut s = session();
    ok(&mut s, "track_add", &json!({ "puppet": "p" }));
    ok(&mut s, "record_arm", &json!({ "groups": ["head"], "only": true }));
    ok(&mut s, "record_script", &json!({ "track": 0, "script": performance(30.0, 30) }));
    let rotation = |s: &mut Session<RecordDoc>| {
        ok(s, "playback_poses", &json!({ "track": 0, "from_frame": 60, "count": 1 }))["poses"][0]["head_rotation_degrees"].as_f64().unwrap()
    };
    assert!(rotation(&mut s).abs() > 1.0);
    // The standard stack starts with the Face behavior: switch it off for this track.
    ok(&mut s, "track_behavior", &json!({ "track": 0, "op": { "op": "set_enabled", "index": 0, "enabled": false } }));
    assert!(rotation(&mut s).abs() < 1e-9);
    ok(&mut s, "track_behavior", &json!({ "track": 0, "op": null }));
    assert!(rotation(&mut s).abs() > 1.0);
}

#[test]
fn scenes_export_and_import_through_json() {
    let mut s = session();
    ok(&mut s, "track_add", &json!({ "puppet": "p" }));
    ok(&mut s, "record_arm", &json!({ "groups": ["head", "keys", "voice"], "only": true }));
    ok(&mut s, "record_script", &json!({ "track": 0, "script": performance(12.0, 20) }));
    ok(&mut s, "audio_add_clip", &json!({ "media": "music", "start_seconds": 0.5, "gain": 0.8, "pan": -0.5 }));
    ok(&mut s, "audio_set_clip", &json!({ "clip": 0, "duration_seconds": 1.5 }));
    let exported = ok(&mut s, "scene_export", &json!({}));
    assert_eq!(exported["format"], "charcraft-record");
    let scene_before = s.doc().scene.clone();
    ok(&mut s, "scene_set", &json!({ "name": "Other", "transparent": true }));
    assert!(s.doc().scene.background.is_none());
    ok(&mut s, "scene_import", &json!({ "scene": exported["scene"] }));
    assert_eq!(s.doc().scene, scene_before);
    ok(&mut s, "audio_remove_clip", &json!({ "clip": 0 }));
    assert!(s.doc().scene.audio[0].clips.is_empty());
    // Hostile import: a scene file with an impossible frame rate is refused.
    let mut bad = exported["scene"].clone();
    bad["fps"] = json!({ "num": 0, "den": 0 });
    failure(&mut s, "scene_import", &json!({ "scene": bad }));
}
