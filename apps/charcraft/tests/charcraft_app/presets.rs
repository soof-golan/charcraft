//! Behavior Reset and presets through the agent door: edit a parameter, reset it, save the edit as a preset, change it
//! again, load the preset back, and see every step in the behaviors stack.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "integration test helpers")]

use anim_mcp::Request;
use serde_json::{Value, json};

use crate::common::{headless_presets, query_with, run};

/// The parameters of the behavior called `id` on track 0 and its index.
fn behavior(b: &anim_mcp::Backend, id: &str) -> (usize, Value) {
    let stack = query_with(b, "behaviors_stack", json!({ "track": 0 }));
    let found = stack["stack"]["behaviors"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap().clone();
    (usize::try_from(found["index"].as_u64().unwrap()).unwrap(), found["params"].clone())
}

const BLINKER: &str = "Adobe.Blinker.js";

#[test]
fn reset_restores_the_defaults_and_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let session = headless_presets(dir.path().join("presets"));
    let b = session.backend();
    let (index, defaults) = behavior(b, BLINKER);
    run(b, "track.behavior", json!({ "track": 0, "op": { "op": "set_param", "index": index, "name": "rate", "value": 77 } })).unwrap();
    assert_eq!(behavior(b, BLINKER).1["rate"], 77.0);
    run(b, "behavior.reset", json!({ "track": 0, "index": index })).unwrap();
    assert_eq!(behavior(b, BLINKER).1, defaults);
    b.call(Request::Undo {}).unwrap();
    assert_eq!(behavior(b, BLINKER).1["rate"], 77.0, "undo brings the edit back");
    assert!(run(b, "behavior.reset", json!({ "track": 0, "index": 99 })).is_err());
    assert!(run(b, "behavior.reset", json!({ "track": 9, "index": 0 })).is_err());
    session.shutdown();
}

#[test]
fn a_preset_is_saved_loaded_listed_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let session = headless_presets(dir.path().join("presets"));
    let b = session.backend();
    let (index, _) = behavior(b, BLINKER);
    run(b, "track.behavior", json!({ "track": 0, "op": { "op": "set_param", "index": index, "name": "rate", "value": 55 } })).unwrap();
    let saved = run(b, "behavior.preset_save", json!({ "track": 0, "index": index, "name": "  Nervous  " })).unwrap();
    assert_eq!(saved["name"], "Nervous");
    assert_eq!(saved["presets"], json!(["Nervous"]));
    assert!(dir.path().join("presets").join("Adobe.Blinker.js.json").is_file());

    run(b, "behavior.reset", json!({ "track": 0, "index": index })).unwrap();
    assert_ne!(behavior(b, BLINKER).1["rate"], 55.0);
    run(b, "behavior.preset_load", json!({ "track": 0, "index": index, "name": "Nervous" })).unwrap();
    assert_eq!(behavior(b, BLINKER).1["rate"], 55.0);

    assert_eq!(run(b, "behavior.presets", json!({ "behavior": BLINKER })).unwrap()["presets"], json!(["Nervous"]));
    assert_eq!(run(b, "behavior.presets", json!({})).unwrap()["presets"][BLINKER], json!(["Nervous"]));
    assert!(run(b, "behavior.preset_load", json!({ "track": 0, "index": index, "name": "Nope" })).unwrap_err().contains("Nervous"));
    assert!(run(b, "behavior.preset_save", json!({ "track": 0, "index": index, "name": " " })).is_err());

    // Presets belong to the behavior: another behavior does not see them.
    let (other, _) = behavior(b, "Charcraft.Breathe.js");
    assert!(run(b, "behavior.preset_load", json!({ "track": 0, "index": other, "name": "Nervous" })).is_err());

    assert_eq!(run(b, "behavior.preset_delete", json!({ "behavior": BLINKER, "name": "Nervous" })).unwrap()["removed"], true);
    assert_eq!(run(b, "behavior.presets", json!({})).unwrap()["presets"], json!({}));
    session.shutdown();
}
