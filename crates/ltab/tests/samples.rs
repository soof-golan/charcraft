//! Real Character Animator files. These are Adobe-authored samples that must never be committed: the tests read the
//! directory named by `ANICRAFT_SAMPLES` and pass trivially when it is not set, so CI stays green.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "integration test helpers")]

use std::fs;
use std::path::{Path, PathBuf};

use charcraft_ltab::{parse_bytes, to_string_pretty};

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from).filter(|dir| dir.is_dir())
}

fn archive_infos() -> Vec<PathBuf> {
    let Some(root) = samples() else { return Vec::new() };
    ["Boy", "blood_cells"].iter().map(|name| root.join("extracted").join(name).join("_archive_info")).filter(|path| path.is_file()).collect()
}

#[test]
fn archive_info_files_round_trip_byte_exact() {
    for path in archive_infos() {
        let original = fs::read(&path).unwrap();
        let parsed = parse_bytes(&original).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let printed = to_string_pretty(&parsed);
        assert_eq!(printed.as_bytes(), original.as_slice(), "{} does not re-serialise byte for byte", path.display());
    }
}

#[test]
fn archive_info_values_are_reachable_by_path() {
    for path in archive_infos() {
        let doc = parse_bytes(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(doc.get_path_str("sType"), Some("puppet"));
        assert_eq!(doc.get_path_i64("iVersion"), Some(1));
        assert_eq!(doc.get_path_bool("bContainsArtworkCache"), Some(false));
        assert!(doc.get_path_str("rootPuppetPPath").is_some_and(|p| p.starts_with("/ProjectItems/")));
        assert!(doc.get_path_str("sUniqueID").is_some_and(|id| id.len() == 36));
        assert_eq!(doc.get_path_i64("tBehaviorDefaults/Adobe.Blinker.js/rate"), Some(30));
        assert_eq!(doc.get_path_i64("tBehaviorDefaults/Adobe.Blinker.js/length"), Some(80));
        assert_eq!(doc.get_path_f64("tBehaviorDefaults/Adobe.Dragger3.js/handleReleaseDuration"), Some(0.2));
        assert_eq!(doc.get_path_bool("tBehaviorDefaults/Adobe.ArmIK.js/autoBend"), Some(true));
        assert_eq!(doc.get_path_i64("tBehaviorDefaults/Adobe.FaceTracker2.js/downEyebrowsTilt"), Some(-20));
        assert_eq!(doc.get_path("tBehaviorDefaults").map(|t| t.named().count()), Some(9));
    }
}

#[test]
fn character_animator_writes_keys_sorted() {
    for path in archive_infos() {
        let doc = parse_bytes(&fs::read(&path).unwrap()).unwrap();
        let mut sorted = doc.clone();
        sorted.sort_keys();
        assert_eq!(sorted, doc, "{}: sort_keys must be a no-op on a file Character Animator wrote", path.display());
    }
}

#[test]
fn editing_a_value_changes_only_that_line() {
    for path in archive_infos() {
        let original = String::from_utf8(fs::read(&path).unwrap()).unwrap();
        let mut doc = parse_bytes(original.as_bytes()).unwrap();
        doc.set_path("tBehaviorDefaults/Adobe.Blinker.js/rate", 45).unwrap();
        let edited = to_string_pretty(&doc);
        let changed: Vec<(&str, &str)> = original.lines().zip(edited.lines()).filter(|(a, b)| a != b).collect();
        assert_eq!(changed, [("\t\t\trate = 30,", "\t\t\trate = 45,")]);
        assert_eq!(original.lines().count(), edited.lines().count());
    }
}

/// Hostile-input smoke test: every file in the sample tree (JSON, binary ropes, takes, images, scripts) is fed to
/// the parser. Almost all must be rejected; none may panic, hang or allocate wildly.
#[test]
fn parser_survives_every_sample_file() {
    fn visit(dir: &Path, depth: usize, visited: &mut usize) {
        if depth > 12 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, depth.saturating_add(1), visited);
            } else if entry.metadata().is_ok_and(|m| m.len() <= 4_194_304)
                && let Ok(bytes) = fs::read(&path)
            {
                let _ = parse_bytes(&bytes);
                *visited = visited.saturating_add(1);
            }
        }
    }
    let Some(root) = samples() else { return };
    let mut visited = 0;
    for sub in ["extracted", "chproj_full"] {
        visit(&root.join(sub), 0, &mut visited);
    }
    assert!(visited > 0 || !root.join("extracted").is_dir());
}

#[test]
fn nothing_but_archive_info_looks_like_ltab() {
    // `_archive_info` is the only ltab file in the samples; the project JSON must not parse as ltab by accident.
    let Some(root) = samples() else { return };
    let scene = root.join("chproj_full/news anchors/Ch Data/temp.noindex/compiler/scene.json");
    if let Ok(bytes) = fs::read(scene) {
        assert!(parse_bytes(&bytes).is_err());
    }
}
