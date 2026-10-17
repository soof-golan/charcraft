//! Grid, guides and snapping through the agent door: settings, guides per view, where a dropped handle lands.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "integration test helpers")]

use serde_json::{Value, json};

use crate::common::{headless, query, query_with, run, wait_for};

fn snap(b: &anim_mcp::Backend, view: &str, x: f64, y: f64) -> Value {
    query_with(b, "grid_snap", json!({ "view": view, "x": x, "y": y }))
}

#[test]
fn settings_change_and_report_back() {
    let session = headless();
    let b = session.backend();
    let state = query(b, "grid_state");
    assert_eq!(state["settings"]["show_grid"], false);
    assert_eq!(state["settings"]["snap"], false);
    run(b, "grid.set", json!({ "show_grid": true, "size": 25.0, "snap": true })).unwrap();
    let state = wait_for("the settings", || query(b, "grid_state"), |s| s["settings"]["show_grid"] == true);
    assert_eq!(state["settings"]["size"], 25.0);
    assert_eq!(state["settings"]["snap"], true);
    assert_eq!(state["settings"]["show_guides"], true, "untouched fields stay");
    assert!(run(b, "grid.set", json!({ "size": 0.5 })).is_err(), "too small a grid is refused");
    assert!(run(b, "grid.set", json!({ "spacing": 3 })).is_err(), "unknown fields are refused");
    session.shutdown();
}

#[test]
fn a_dropped_point_lands_on_the_grid_or_a_guide_only_while_snapping_is_on() {
    let session = headless();
    let b = session.backend();
    assert_eq!(snap(b, "rig", 19.0, 41.0)["snapped"], false, "snapping is off at first");
    run(b, "grid.set", json!({ "snap": true, "size": 20.0 })).unwrap();
    wait_for("snapping on", || query(b, "grid_state"), |s| s["settings"]["snap"] == true);
    let p = snap(b, "rig", 19.0, 41.0);
    assert_eq!((p["x"].as_f64(), p["y"].as_f64(), p["snapped"].as_bool()), (Some(20.0), Some(40.0), Some(true)));
    assert_eq!(snap(b, "rig", 30.0, 30.0)["snapped"], false, "between lines nothing catches it");

    run(b, "grid.guide", json!({ "view": "scene", "action": "add", "axis": "vertical", "pos": 333.0 })).unwrap();
    wait_for("the guide", || query(b, "grid_state"), |s| s["guides"]["scene"].as_array().unwrap().len() == 1);
    assert_eq!(snap(b, "scene", 331.0, 100.0)["x"], 333.0);
    assert_eq!(snap(b, "rig", 331.0, 100.0)["x"], 331.0, "guides belong to their view");

    run(b, "grid.set", json!({ "snap_to_guides": false })).unwrap();
    wait_for("guides off", || query(b, "grid_state"), |s| s["settings"]["snap_to_guides"] == false);
    assert_eq!(snap(b, "scene", 331.0, 100.0)["x"], 331.0);
    session.shutdown();
}

#[test]
fn guides_are_added_moved_removed_and_can_be_locked() {
    let session = headless();
    let b = session.backend();
    run(b, "grid.guide", json!({ "action": "add", "axis": "horizontal", "pos": 120.0 })).unwrap();
    run(b, "grid.guide", json!({ "action": "add", "axis": "vertical", "pos": 80.0 })).unwrap();
    let state = wait_for("two guides", || query(b, "grid_state"), |s| s["guides"]["rig"].as_array().unwrap().len() == 2);
    assert_eq!(state["guides"]["rig"][0], json!({ "axis": "horizontal", "pos": 120.0 }));
    run(b, "grid.guide", json!({ "action": "move", "index": 1, "pos": 95.0 })).unwrap();
    wait_for("the move", || query(b, "grid_state"), |s| s["guides"]["rig"][1]["pos"] == 95.0);

    run(b, "grid.set", json!({ "lock_guides": true })).unwrap();
    run(b, "grid.guide", json!({ "action": "remove", "index": 0 })).unwrap();
    run(b, "grid.guide", json!({ "action": "move", "index": 1, "pos": 5.0 })).unwrap();
    // The queue is in order: once the unlock shows, the two edits before it were handled (and refused).
    run(b, "grid.set", json!({ "lock_guides": false })).unwrap();
    let state = wait_for("unlocked", || query(b, "grid_state"), |s| s["settings"]["lock_guides"] == false);
    assert_eq!(state["guides"]["rig"].as_array().unwrap().len(), 2, "a locked guide stays");
    assert_eq!(state["guides"]["rig"][1]["pos"], 95.0);

    run(b, "grid.guide", json!({ "action": "remove", "index": 0 })).unwrap();
    wait_for("the removal", || query(b, "grid_state"), |s| s["guides"]["rig"].as_array().unwrap().len() == 1);
    run(b, "grid.guide", json!({ "action": "clear" })).unwrap();
    wait_for("clear", || query(b, "grid_state"), |s| s["guides"]["rig"].as_array().unwrap().is_empty());

    assert!(run(b, "grid.guide", json!({ "action": "add", "axis": "diagonal", "pos": 1.0 })).is_err());
    assert!(run(b, "grid.guide", json!({ "action": "add", "axis": "vertical", "pos": 1.0e12 })).is_err());
    assert!(run(b, "grid.guide", json!({ "view": "stage", "action": "clear" })).is_err());
    assert!(run(b, "grid.guide", json!({ "action": "spin" })).is_err());
    session.shutdown();
}
