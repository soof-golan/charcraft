//! Tests against real Character Animator files. They read `ANICRAFT_SAMPLES` and pass trivially when it is unset.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration test helpers"
)]

use std::path::PathBuf;

use anim_archive::Container;
use charcraft_ltab::LValue;
use charcraft_project::{ClassedDoc, Project, ProjectKind};
use charcraft_rope::class_name;

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from).filter(|p| p.is_dir())
}

fn sample(relative: &str) -> Option<PathBuf> {
    let path = samples()?.join(relative);
    path.exists().then_some(path)
}

fn all_sources() -> Vec<(&'static str, PathBuf)> {
    ["Boy.puppet", "blood_cells.puppet", "chproj_full/news anchors"].into_iter().filter_map(|name| sample(name).map(|p| (name, p))).collect()
}

#[test]
fn both_puppets_load_and_list_their_content() {
    for name in ["Boy.puppet", "blood_cells.puppet"] {
        let Some(path) = sample(name) else { return };
        let project = Project::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(project.kind(), ProjectKind::Puppet, "{name}");
        assert!(project.warnings().is_empty(), "{name}: {:?}", project.warnings());
        let manifest = project.manifest().expect("manifest");
        let root = manifest.root_puppet_ppath().expect("root puppet").trim_start_matches('/').to_owned();
        assert_eq!(manifest.kind(), Some("puppet"));
        assert!(manifest.unique_id().is_some_and(|id| id.len() == 36));

        let model = project.model().unwrap();
        assert!(model.problems.is_empty(), "{name}: {:?}", model.problems);
        let puppet = model.puppet(&root).unwrap_or_else(|| panic!("{name}: root {root} missing"));
        assert!(puppet.is_root_in_puppet_file);
        assert!(puppet.guid.is_some());
        assert!(!puppet.behaviors.is_empty(), "{name} has behaviors");
        for bound in &puppet.behaviors {
            assert!(bound.behavior.starts_with("Adobe."), "{name}: {}", bound.behavior);
        }
        let defaults = manifest.behavior_defaults();
        assert!(defaults.len() >= 5, "{name}: {} behavior defaults", defaults.len());
        assert!(defaults.iter().all(|d| !d.params.is_empty()));
        let blinker = manifest.behavior_default("Adobe.Blinker.js", "rate").and_then(LValue::as_i64);
        assert_eq!(blinker, Some(30), "{name}");

        let tree = model.layer_tree(&root).expect("layer tree");
        let layers = tree.flatten();
        assert!(layers.len() >= 10, "{name}: {} layers", layers.len());
        assert!(layers.iter().any(|n| n.tags.iter().any(|t| t.name.starts_with("Adobe.LipSynch."))), "{name} has lip sync mouths");
        assert!(layers.iter().all(|n| !n.missing), "{name}: every child puppet resolves");
        assert_eq!(model.media.len(), 1, "{name}: one artwork file");
        assert!(model.media.iter().all(|m| m.file.is_some()), "{name}: artwork is in Ch Media");
        assert!(model.puppets.iter().filter(|p| p.is_part).count() >= 10);

        let summary = project.summary().unwrap();
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("Adobe.Blinker.js"));
    }
}

#[test]
fn puppets_save_and_load_again_unchanged() {
    for name in ["Boy.puppet", "blood_cells.puppet"] {
        let Some(path) = sample(name) else { return };
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join(name);
        let mut project = Project::open(&path).unwrap();
        let model = project.model().unwrap();
        project.save_puppet(&out).unwrap();

        let again = Project::open(&out).unwrap();
        assert!(again.warnings().is_empty(), "{name}: {:?}", again.warnings());
        assert_eq!(again.model().unwrap(), model, "{name}: model");
        assert_eq!(again.store().objects, project.store().objects, "{name}: objects");
        assert_eq!(again.store().refs, project.store().refs, "{name}: refs");
        assert_eq!(again.manifest().unwrap().to_bytes(), Container::open(&path).unwrap().read("_archive_info").unwrap(), "{name}: manifest bytes");

        // Every entry, in order, byte for byte (the rope too: nothing was appended, so it is not re-encoded).
        let before = Container::open(&path).unwrap();
        let after = Container::open(&out).unwrap();
        let names = |c: &Container| c.entries().iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&before), names(&after), "{name}: entry list");
        for entry in before.files() {
            assert_eq!(before.read_entry(entry).unwrap(), after.read(&entry.name).unwrap(), "{name}: {}", entry.name);
        }
    }
}

#[test]
fn puppet_edits_survive_a_save() {
    let Some(path) = sample("Boy.puppet") else { return };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("Boy-edited.puppet");
    let mut project = Project::open(&path).unwrap();
    let before = Container::open(&path).unwrap().read("Ch Data/repo.noindex/projectdata.rope").unwrap();
    let root = project.manifest().unwrap().root_puppet_ppath().unwrap().trim_start_matches('/').to_owned();
    project.set_behavior_param(&root, "Adobe.Blinker.js", "rate", LValue::Int(55)).unwrap();
    project.rename_puppet(&root, "Edited boy").unwrap();
    project.save_puppet(&out).unwrap();

    let after = Container::open(&out).unwrap().read("Ch Data/repo.noindex/projectdata.rope").unwrap();
    assert_eq!(&after[..before.len()], &before[..], "the original history is untouched");
    assert!(after.len() > before.len());
    let again = Project::open(&out).unwrap();
    let model = again.model().unwrap();
    let puppet = model.puppet(&root).unwrap();
    assert_eq!(puppet.display_name(), "Edited boy");
    assert_eq!(model.effective_param(&root, "Adobe.Blinker.js", "rate").and_then(|v| v.as_i64()), Some(55));
    assert_eq!(again.heads().0, again.heads().1);
}

#[test]
fn the_news_anchors_project_loads_from_folder_marker_and_zip() {
    let Some(folder) = sample("chproj_full/news anchors") else { return };
    let by_folder = Project::open(&folder).unwrap();
    let by_marker = Project::open(folder.join("news anchors.chproj")).unwrap();
    assert_eq!(by_folder.kind(), ProjectKind::Folder);
    assert_eq!(by_folder.name(), "news anchors");
    assert_eq!(by_folder.model().unwrap(), by_marker.model().unwrap());

    let model = by_folder.model().unwrap();
    assert!(model.problems.is_empty(), "{:?}", model.problems);
    assert_eq!(model.scenes.len(), 3);
    assert_eq!(model.root_puppets().count(), 4);
    assert_eq!(model.media.len(), 6);
    assert!(model.behaviors.len() >= 50);
    assert!(model.media.iter().all(|m| m.file.is_some()), "gathered media is found next to the project");
    let blinker = model.behavior("Adobe.Blinker.js").expect("blinker");
    assert_eq!(blinker.name.as_deref(), Some("Auto Blink"));
    assert_eq!(blinker.param("rate").and_then(|p| p.default.as_i64()), Some(30));
    assert!(blinker.tags.iter().any(|t| t.id == "Adobe.Face.LeftBlink"));
    assert!(blinker.js_len > 1000);

    let scene = model.scene("anchors_01").expect("scene");
    assert_eq!((scene.width, scene.height), (1920, 1080));
    assert!((scene.fps - 24.0).abs() < 1e-9);
    assert!((scene.duration - 144.75).abs() < 1e-9);
    assert_eq!(scene.frame_count(), 3474);
    assert_eq!(scene.tracks.len(), 6);
    assert!(scene.tracks.iter().any(|t| t.kind.as_deref() == Some("trackKind_audio")));
    let male = scene.tracks.iter().flat_map(|t| &t.items).find(|i| i.source.as_deref() == Some("/ProjectItems/male_anchor")).expect("male anchor on a track");
    assert!(male.behaviors.iter().any(|b| b.behavior == "Adobe.LipSync2.js"));
    assert!(scene.behavior_instances().any(|b| !b.takes.is_empty()), "takes are recorded");
    assert_ne!(scene.recordings.len(), 0);

    let male = model.puppet("ProjectItems/male_anchor").unwrap();
    assert_eq!(male.trigger_groups.len(), 1);
    assert_eq!(male.hot_takes.len(), 1);
    let tree = model.layer_tree("ProjectItems/male_anchor").unwrap();
    assert!(tree.flatten().len() > 5);
    assert!(tree.flatten().iter().all(|n| !n.missing));

    let audio = model.media.iter().find(|m| m.file_name.as_deref() == Some("news_song_vocals_mixdown.wav")).expect("audio");
    assert!(audio.duration.is_some_and(|d| (d - 103.796_5).abs() < 0.01));
    assert_eq!(audio.channels, Some(2));
    assert!(audio.transcript.as_deref().is_some_and(|t| t.starts_with("Good evening")));

    if let Some(zip) = sample("chproj_zip/news_anchors.zip") {
        let by_zip = Project::open(&zip).unwrap();
        assert_eq!(by_zip.model().unwrap().scenes.len(), 3);
    }
}

#[test]
fn a_project_folder_saves_and_loads_again() {
    let Some(folder) = sample("chproj_full/news anchors") else { return };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("copy");
    let mut project = Project::open(&folder).unwrap();
    let model = project.model().unwrap();
    project.save_folder(&out).unwrap();
    assert!(out.join("news anchors.chproj").is_file());

    let again = Project::open(&out).unwrap();
    assert_eq!(again.model().unwrap(), model);
    assert_eq!(again.store().objects, project.store().objects);
    let original = Container::open(&folder).unwrap();
    let copy = Container::open(&out).unwrap();
    assert_eq!(original.files().count(), copy.files().count());
    for entry in original.files() {
        let wanted = original.read_entry(entry).unwrap();
        assert!(wanted == copy.read(&entry.name).unwrap(), "{} differs", entry.name);
    }

    // Edit the copy in place: only the rope grows.
    let mut edited = Project::open(&out).unwrap();
    let rope = out.join("Ch Data/repo.noindex/projectdata.rope");
    let before = std::fs::read(&rope).unwrap();
    edited.set_behavior_param("ProjectItems/male_anchor", "Adobe.Dragger3.js", "range", LValue::Int(42)).unwrap();
    edited.save().unwrap();
    let after = std::fs::read(&rope).unwrap();
    assert_eq!(&after[..before.len()], &before[..]);
    assert!(after.len() > before.len());
    let reread = Project::open(&out).unwrap();
    assert!(reread.warnings().is_empty(), "{:?}", reread.warnings());
    assert_eq!(reread.heads().0, edited.heads().0);
    assert_eq!(reread.heads().0, reread.heads().1, "a save moves main to HEAD");
}

#[test]
fn every_blob_prints_back_with_its_classes() {
    let mut total = 0_usize;
    let mut failures = Vec::new();
    for (name, path) in all_sources() {
        let project = Project::open(&path).unwrap();
        for (file, blob) in project.files().unwrap() {
            if class_name(blob).is_none() {
                continue;
            }
            total += 1;
            let printed = ClassedDoc::parse(blob).and_then(|doc| doc.to_bytes());
            match printed {
                Ok(bytes) if bytes == blob => {}
                Ok(bytes) => {
                    let at = bytes.iter().zip(blob.iter()).position(|(a, b)| a != b).unwrap_or_else(|| bytes.len().min(blob.len()));
                    let around = |data: &[u8]| String::from_utf8_lossy(&data[at.saturating_sub(60)..(at + 60).min(data.len())]).into_owned();
                    failures.push(format!(
                        "{name}: {file}: differs at byte {at} (sizes {} vs {})\n  original: {:?}\n  printed:  {:?}",
                        blob.len(),
                        bytes.len(),
                        around(blob),
                        around(&bytes)
                    ));
                }
                Err(error) => failures.push(format!("{name}: {file}: {error}")),
            }
        }
    }
    assert!(
        failures.is_empty() || total == 0,
        "{} of {total} blobs did not print back byte for byte: {:#?}",
        failures.len(),
        &failures[..failures.len().min(10)]
    );
}
