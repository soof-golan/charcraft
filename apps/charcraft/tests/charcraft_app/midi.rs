//! MIDI through the agent door: devices, learn mode, a note shows a layer, a controller moves a Controls slider.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "integration test helpers")]

use anim_mcp::{Confinement, HeadlessSession, HostInfo, Request, spawn_headless};
use charcraft_app::hooks::make_host;
use charcraft_app::live::{LiveHandle, Sources};
use charcraft_app::midi::MidiBank;
use charcraft_app::session::{Links, registry};
use charcraft_app::world::World;
use charcraft_midi::{MidiHandle, MockBackend};
use serde_json::json;

use crate::common::{query, query_with, run, wait_for};

/// A headless session with a pretend keyboard and the demo microphone and camera.
fn session() -> HeadlessSession {
    spawn_headless(|| {
        let live = LiveHandle::spawn_with(|| {}, Sources::demo());
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(live.link(), ui);
        // The actor lives until the test process ends.
        std::mem::forget(live);
        let mut host = make_host(
            World::demo().unwrap(),
            links,
            registry().unwrap(),
            Confinement::unrestricted(),
            HostInfo { name: "charcraft".into(), version: "test".into() },
        );
        host.hooks_mut().midi = MidiBank::new(MidiHandle::spawn(Box::new(MockBackend::new(&["Test Keys"])), || {}));
        host
    })
    .unwrap()
}

#[test]
fn a_device_is_listed_connected_and_closed() {
    let session = session();
    let b = session.backend();
    run(b, "midi.refresh", json!({})).unwrap();
    let state = wait_for("the device list", || query(b, "midi_state"), |s| s["ports"] == json!(["Test Keys"]));
    assert!(state["connected"].is_null());
    run(b, "midi.connect", json!({ "device": "Test Keys" })).unwrap();
    wait_for("the connection", || query(b, "midi_state"), |s| s["connected"] == "Test Keys");
    run(b, "midi.connect", json!({ "device": "No Such Box" })).unwrap();
    let failed = wait_for("the refusal", || query(b, "midi_state"), |s| s["error"].is_string());
    assert!(failed["error"].as_str().unwrap().contains("No Such Box"));
    assert!(failed["connected"].is_null());
    assert!(run(b, "midi.connect", json!({ "device": "" })).is_err(), "an empty name is refused");
    session.shutdown();
}

#[test]
fn learning_a_note_binds_the_layer_and_the_held_note_is_a_trigger() {
    let session = session();
    let b = session.backend();
    run(b, "midi.learn", json!({ "layer": "Head/Fringe" })).unwrap();
    let state = wait_for("learn mode", || query(b, "midi_state"), |s| s["learn"]["kind"] == "trigger");
    assert_eq!(state["learn"]["layer"], "Head/Fringe");
    run(b, "midi.send", json!({ "type": "note_on", "channel": 2, "note": 60, "velocity": 99 })).unwrap();
    wait_for("the binding", || query(b, "midi_state"), |s| s["learn"].is_null());
    let layer = wait_for("the rig edit", || query_with(b, "puppet_layer", json!({ "layer": "Head/Fringe" })), |l| l["layer"]["midi_trigger"].is_object());
    assert_eq!(layer["layer"]["midi_trigger"], json!({ "channel": 2, "note": 60 }));
    assert_eq!(query(b, "midi_state")["held"], json!([]), "the learned note is not also played");

    run(b, "midi.send", json!({ "type": "note_on", "channel": 2, "note": 60, "velocity": 80 })).unwrap();
    let held = wait_for("the held note", || query(b, "midi_state"), |s| !s["held"].as_array().unwrap().is_empty());
    assert_eq!(held["held"], json!(["midi:2:60", "midi:60"]));
    assert!(held["history"][0].as_str().unwrap().starts_with("Note on C4"));
    run(b, "midi.send", json!({ "type": "note_off", "channel": 2, "note": 60 })).unwrap();
    wait_for("the release", || query(b, "midi_state"), |s| s["held"].as_array().unwrap().is_empty());

    // The edit is one undo step like any other rig edit.
    b.call(Request::Undo {}).unwrap();
    assert!(query_with(b, "puppet_layer", json!({ "layer": "Head/Fringe" }))["layer"]["midi_trigger"].is_null());
    session.shutdown();
}

#[test]
fn bad_learn_and_send_requests_are_refused_with_a_reason() {
    let session = session();
    let b = session.backend();
    assert!(run(b, "midi.learn", json!({ "layer": "Nowhere" })).unwrap_err().contains("Nowhere"));
    assert!(run(b, "midi.learn", json!({ "control": "volume" })).is_err());
    assert!(run(b, "midi.learn", json!({ "layer": "Head/Fringe", "control": "smile" })).is_err());
    assert!(run(b, "midi.send", json!({ "type": "note_on", "channel": 17, "note": 60, "velocity": 90 })).is_err());
    assert!(run(b, "midi.send", json!({ "type": "control_change", "channel": 1, "controller": 200, "value": 1 })).is_err());
    assert!(run(b, "rig.edit", json!({ "op": "set_midi_trigger", "layer": "Head/Fringe", "note": 128 })).is_err());
    session.shutdown();
}

#[test]
fn a_controller_moves_the_slider_it_is_bound_to() {
    let session = session();
    let b = session.backend();
    run(b, "midi.learn", json!({ "control": "mic_gain" })).unwrap();
    wait_for("learn mode", || query(b, "midi_state"), |s| s["learn"]["kind"] == "control");
    run(b, "midi.send", json!({ "type": "control_change", "channel": 1, "controller": 74, "value": 10 })).unwrap();
    let state = wait_for("the binding", || query(b, "midi_state"), |s| s["bindings"]["mic_gain"] == "ch 1 CC 74");
    assert!(state["learn"].is_null());

    // Full travel is the top of the slider: the microphone actor gets gain 8.
    run(b, "midi.send", json!({ "type": "control_change", "channel": 1, "controller": 74, "value": 127 })).unwrap();
    wait_for("the microphone gain", || run(b, "mic.status", json!({})).unwrap(), |s| (s["gain"].as_f64().unwrap() - 8.0).abs() < 1e-6);
    assert!((query(b, "midi_state")["controls"]["mic_gain"].as_f64().unwrap() - 8.0).abs() < 1e-6);
    // Another controller or a note leaves it alone.
    run(b, "midi.send", json!({ "type": "control_change", "channel": 1, "controller": 75, "value": 0 })).unwrap();
    run(b, "midi.send", json!({ "type": "control_change", "channel": 1, "controller": 74, "value": 0 })).unwrap();
    wait_for("the lowest gain", || run(b, "mic.status", json!({})).unwrap(), |s| (s["gain"].as_f64().unwrap() - 0.1).abs() < 1e-6);

    // The slider can be set directly, and an unbound slider forgets its controller.
    run(b, "controls.set", json!({ "control": "mic_gain", "value": 2.0 })).unwrap();
    assert!((run(b, "mic.status", json!({})).unwrap()["gain"].as_f64().unwrap() - 2.0).abs() < 1e-6);
    assert!(run(b, "controls.set", json!({ "control": "mic_gain", "value": 99.0 })).is_err());
    run(b, "midi.bind_cc", json!({ "control": "mic_gain" })).unwrap();
    wait_for("the unbinding", || query(b, "midi_state"), |s| s["bindings"].as_object().unwrap().is_empty());
    run(b, "midi.bind_cc", json!({ "control": "smile", "controller": 7, "channel": 3 })).unwrap();
    wait_for("the explicit binding", || query(b, "midi_state"), |s| s["bindings"]["smile"] == "ch 3 CC 7");
    session.shutdown();
}
