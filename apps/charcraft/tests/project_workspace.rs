//! The project and workspace features through the same door an agent uses: commands and queries on a headless
//! session. Tests that need Adobe sample files read `ANICRAFT_SAMPLES` and return when it is unset.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "integration test helpers")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anim_mcp::{Backend, Confinement, HeadlessSession, HostInfo, Request, spawn_headless};
use charcraft_app::hooks::make_host;
use charcraft_app::live::LiveLink;
use charcraft_app::session::{Links, registry};
use charcraft_app::world::{SourceKind, World};
use serde_json::{Value, json};

fn headless() -> HeadlessSession {
    spawn_headless(|| {
        let opened = World::demo().unwrap();
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui);
        make_host(opened, links, registry().unwrap(), Confinement::unrestricted(), HostInfo { name: "charcraft".into(), version: "test".into() })
    })
    .unwrap()
}

fn run(backend: &Backend, id: &str, params: Value) -> Result<Value, String> {
    backend.call(Request::CommandRun { id: id.into(), params, merge_key: None }).map(|r| r.value["result"].clone()).map_err(|e| e.to_string())
}

fn query(backend: &Backend, name: &str) -> Value {
    backend.call(Request::Query { name: name.into(), params: json!({}) }).unwrap().value
}

/// Waits for the job a command queued to say something different from `before`.
fn job_result(backend: &Backend, before: &Value) -> String {
    let start = Instant::now();
    loop {
        let now = query(backend, "world_info")["last_job"].clone();
        if !now.is_null() && now != *before {
            return now.as_str().unwrap().to_owned();
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the host never finished the job");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from).filter(|p| p.is_dir())
}

#[test]
fn a_new_project_starts_empty_and_the_panel_query_lists_nothing() {
    let session = headless();
    let backend = session.backend();
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "project.new", json!({ "name": "Show" })).unwrap();
    assert_eq!(job_result(backend, &before), "New project Show");
    let info = query(backend, "project_info");
    assert_eq!(info["title"], "Show");
    assert_eq!(info["kind"], "empty");
    assert_eq!(info["puppets"].as_array().map(Vec::len), Some(0));
    assert_eq!(info["scenes"][0]["tracks"].as_array().map(Vec::len), Some(0));
    // Nothing to write: a project of no puppets is refused with a reason.
    let dir = tempfile::tempdir().unwrap();
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "project.save", json!({ "path": dir.path().join("Empty").display().to_string() })).unwrap();
    assert!(job_result(backend, &before).starts_with("Save failed"), "{}", query(backend, "world_info")["last_job"]);
    session.shutdown();
}

#[test]
fn an_artwork_document_says_why_it_cannot_be_a_project_and_revert_reads_the_saved_file() {
    let dir = tempfile::tempdir().unwrap();
    let session = headless();
    let backend = session.backend();
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "project.save", json!({ "path": dir.path().join("Demo").display().to_string() })).unwrap();
    let said = job_result(backend, &before);
    assert!(said.starts_with("Save failed") && said.contains("artwork"), "{said}");

    // A .charcraft document keeps names; revert goes back to the saved ones.
    let file = dir.path().join("demo.charcraft");
    backend.call(Request::Save { path: Some(file.display().to_string()) }).unwrap();
    run(backend, "puppet.rename", json!({ "puppet": "demo", "name": "Renamed" })).unwrap();
    assert_eq!(query(backend, "project_info")["puppets"][0]["name"], "Renamed");
    assert_eq!(query(backend, "project_info")["dirty"], true);
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "project.revert", json!({})).unwrap();
    assert!(job_result(backend, &before).starts_with("Reverted"));
    assert_eq!(query(backend, "project_info")["puppets"][0]["name"], "demo");
    assert_eq!(query(backend, "project_info")["dirty"], false);

    // Names survive a .charcraft round trip.
    run(backend, "puppet.rename", json!({ "puppet": "demo", "name": "Kept" })).unwrap();
    backend.call(Request::Save { path: None }).unwrap();
    backend.call(Request::Open { path: file.display().to_string() }).unwrap();
    assert_eq!(query(backend, "project_info")["puppets"][0]["name"], "Kept");
    session.shutdown();
}

#[test]
fn duplicate_and_remove_change_the_project_listing_and_undo() {
    let session = headless();
    let backend = session.backend();
    let done = run(backend, "puppet.duplicate", json!({ "puppet": "demo", "name": "Twin" })).unwrap();
    assert_eq!(done["puppet"], "Twin");
    let info = query(backend, "project_info");
    let names: Vec<&str> = info["puppets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["demo", "Twin"]);
    // The copy renders on its own: the job put it in the cast.
    let start = Instant::now();
    loop {
        let world = query(backend, "world_info");
        if world["puppets"].as_array().is_some_and(|p| p.iter().any(|k| k == "Twin")) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "the copy never reached the cast");
        std::thread::sleep(Duration::from_millis(20));
    }
    run(backend, "puppet.remove", json!({ "puppet": "Twin" })).unwrap();
    assert_eq!(query(backend, "project_info")["puppets"].as_array().map(Vec::len), Some(1));
    backend.call(Request::Undo {}).unwrap();
    assert_eq!(query(backend, "project_info")["puppets"].as_array().map(Vec::len), Some(2));
    session.shutdown();
}

#[test]
fn a_puppet_file_imports_saves_into_a_project_and_comes_back() {
    let Some(samples) = samples() else { return };
    let boy = samples.join("Boy.puppet");
    if !boy.exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let session = headless();
    let backend = session.backend();
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "project.new", json!({ "name": "Show" })).unwrap();
    job_result(backend, &before);

    let done = run(backend, "puppet.import", json!({ "path": boy.display().to_string() })).unwrap();
    let key = done["puppets"][0].as_str().unwrap().to_owned();
    assert_eq!(done["track_added"], true);
    assert!(run(backend, "puppet.import", json!({ "path": boy.display().to_string() })).is_err(), "twice is refused");
    run(backend, "puppet.rename", json!({ "puppet": key, "name": "Hero" })).unwrap();
    let info = query(backend, "project_info");
    assert_eq!(info["puppets"][0]["name"], "Hero");
    assert_eq!(info["puppets"][0]["tracks"], json!([0]));
    assert!(info["puppets"][0]["layers"].as_u64().unwrap() > 10);

    // Save the project: a folder Character Animator opens, with the scene beside it.
    let folder = dir.path().join("Show.chproj");
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "project.save", json!({ "path": folder.display().to_string() })).unwrap();
    let said = job_result(backend, &before);
    assert!(said.starts_with("Saved project"), "{said}");
    let project_dir = dir.path().join("Show");
    assert!(project_dir.is_dir());
    assert!(charcraft_record::sidecar_path(&project_dir).is_file(), "the scene sits beside the project");
    let reopened = charcraft_project::Project::open(&project_dir).unwrap();
    let model = reopened.model().unwrap();
    assert_eq!(model.puppet(&key).map(|p| p.display_name().to_owned()), Some("Hero".to_owned()), "the new name is in the project data");

    // Open it again as the app does: the puppet, its name and the track are back.
    let back = World::open(&project_dir).unwrap();
    assert_eq!(back.world.kind, SourceKind::Project);
    assert_eq!(back.rec.scene.tracks.len(), 1);
    assert_eq!(back.meta.display_name(&key), "Hero");
    assert!(back.rig.skeletons.contains_key(&key));

    // One puppet out as a .puppet.
    let file = dir.path().join("Hero.puppet");
    let before = query(backend, "world_info")["last_job"].clone();
    run(backend, "puppet.save", json!({ "puppet": key, "path": file.display().to_string() })).unwrap();
    assert!(job_result(backend, &before).starts_with("Saved puppet"));
    let as_puppet = charcraft_project::Project::open(&file).unwrap();
    assert_eq!(as_puppet.kind(), charcraft_project::ProjectKind::Puppet);
    let reopened = World::open(&file).unwrap();
    assert_eq!(reopened.world.kind, SourceKind::Puppet);
    session.shutdown();
}

#[test]
fn an_opened_project_saves_to_a_new_folder_and_opens_again() {
    let Some(samples) = samples() else { return };
    let source = samples.join("chproj_full").join("news anchors");
    if !source.exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let opened = World::open(&source).unwrap();
    let tracks = opened.rec.scene.tracks.len();
    let puppets = opened.rig.skeletons.len();
    let path = source;
    let session = spawn_headless(move || {
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui);
        make_host(
            World::open(&path).unwrap(),
            links,
            registry().unwrap(),
            Confinement::unrestricted(),
            HostInfo { name: "charcraft".into(), version: "test".into() },
        )
    })
    .unwrap();
    let backend = session.backend();
    let before = query(backend, "world_info")["last_job"].clone();
    let target = dir.path().join("Copy");
    run(backend, "project.save", json!({ "path": target.display().to_string(), "format": "chproj" })).unwrap();
    let said = job_result(backend, &before);
    assert!(said.starts_with("Saved project"), "{said}");
    let back = World::open(&target).unwrap();
    assert_eq!(back.rec.scene.tracks.len(), tracks);
    assert_eq!(back.rig.skeletons.len(), puppets);
    session.shutdown();
}
