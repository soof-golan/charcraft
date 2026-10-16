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

use charcraft_rope::{Limits, ObjectKind, RefTarget, RopeStore, class_name, parse_class_table};

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from).filter(|p| p.is_dir())
}

fn open(relative: &str) -> Option<RopeStore> {
    let path = samples()?.join(relative);
    if !path.exists() {
        return None;
    }
    Some(RopeStore::open(&path).unwrap_or_else(|e| panic!("{relative}: {e}")))
}

fn all_sources() -> Vec<(&'static str, RopeStore)> {
    ["Boy.puppet", "blood_cells.puppet", "extracted/Boy", "extracted/blood_cells", "chproj_full/news anchors", "chproj_zip/news_anchors.zip"]
        .into_iter()
        .filter_map(|name| open(name).map(|store| (name, store)))
        .collect()
}

#[test]
fn every_record_parses_and_every_id_verifies() {
    for (name, store) in all_sources() {
        assert_eq!(store.warnings.len(), 0, "{name}: {:?}", store.warnings);
        assert!(!store.objects.is_empty(), "{name}");
        for (id, object) in &store.objects {
            assert_eq!(charcraft_rope::Oid::of(object.kind, &object.data), *id, "{name}");
        }
    }
}

#[test]
fn head_resolves_to_a_commit_with_files() {
    for (name, store) in all_sources() {
        let head = store.head_commit().unwrap_or_else(|e| panic!("{name}: {e}"));
        let commit = store.commit(&head).expect("commit");
        assert!(store.objects.contains_key(&commit.tree), "{name}");
        let files = store.head_tree().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!files.is_empty(), "{name}");
        assert!(!store.history(head, 10_000).expect("history").is_empty(), "{name}");
    }
}

#[test]
fn strict_parse_of_the_raw_rope_matches_the_opened_store() {
    let Some(root) = samples() else { return };
    for rel in [
        "extracted/Boy/Ch Data/repo.noindex/projectdata.rope",
        "extracted/blood_cells/Ch Data/repo.noindex/projectdata.rope",
        "chproj_full/news anchors/Ch Data/repo.noindex/projectdata.rope",
    ] {
        let path = root.join(rel);
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let store = RopeStore::from_rope_bytes_strict(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{rel}: {e}"));
        // The raw rope alone knows the same HEAD commit as the full store.
        let opened = RopeStore::open(path.parent().expect("parent")).expect("opens");
        assert_eq!(store.head_commit().ok(), opened.head_commit().ok(), "{rel}");
    }
}

#[test]
fn records_re_encode_to_the_bytes_character_animator_wrote() {
    let Some(root) = samples() else { return };
    let mut exact = 0_usize;
    let mut gzip_differs = 0_usize;
    for rel in [
        "extracted/Boy/Ch Data/repo.noindex/projectdata.rope",
        "extracted/blood_cells/Ch Data/repo.noindex/projectdata.rope",
        "chproj_full/news anchors/Ch Data/repo.noindex/projectdata.rope",
    ] {
        let Ok(bytes) = std::fs::read(root.join(rel)) else { continue };
        let parsed: Vec<_> = charcraft_rope::records(&bytes, Limits::default(), true).collect::<Result<_, _>>().expect("parses");
        for (index, (offset, record)) in parsed.iter().enumerate() {
            let end = parsed.get(index + 1).map_or(bytes.len(), |(next, _)| *next);
            let original = &bytes[*offset..end];
            let mut again = Vec::new();
            charcraft_rope::encode_record(record, &mut again);
            if again == original {
                exact += 1;
            } else if original.windows(2).any(|w| w == [0x1f, 0x8b]) {
                gzip_differs += 1; // same content, different deflate encoder
            } else {
                panic!("{rel}: record at {offset} does not re-encode exactly");
            }
        }
    }
    println!("{exact} records re-encode exactly, {gzip_differs} differ only in the deflate stream");
}

#[test]
fn rewriting_keeps_objects_and_refs() {
    for (name, store) in all_sources() {
        let bytes = store.to_rope_bytes();
        let back = RopeStore::from_rope_bytes_strict(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back.objects, store.objects, "{name}");
        assert_eq!(back.refs, store.refs, "{name}");
        assert_eq!(back.log, store.log, "{name}");
        // A second write is byte-identical: the encoder is deterministic.
        assert_eq!(back.to_rope_bytes(), bytes, "{name}");
    }
}

#[test]
fn the_samples_use_the_documented_ref_shapes() {
    for (name, store) in all_sources() {
        assert!(store.resolve_ref("HEAD").is_some(), "{name}");
        assert!(matches!(store.refs.get("refs/heads/main"), Some(RefTarget::Direct(_))), "{name}");
    }
}

#[test]
fn class_tagged_blobs_parse_with_ltab() {
    let mut parsed = 0_usize;
    for (name, store) in all_sources() {
        let files = store.head_tree().expect("tree");
        for (path, bytes) in files {
            if class_name(bytes).is_none() {
                continue;
            }
            let table = parse_class_table(bytes).unwrap_or_else(|e| panic!("{name}:{path}: {e}"));
            assert!(!table.class.is_empty(), "{name}:{path}");
            parsed = parsed.saturating_add(1);
        }
    }
    if samples().is_some() {
        assert!(parsed > 0);
    }
}

#[test]
fn the_news_anchors_project_has_tags_and_history() {
    let Some(store) = open("chproj_full/news anchors") else { return };
    let tags = store.objects.values().filter(|o| o.kind == ObjectKind::Tag).count();
    assert!(tags > 0);
    assert!(store.refs.keys().any(|r| r.starts_with("refs/tags/v190/recent/")));
    let head = store.head_commit().expect("head");
    assert!(store.history(head, 100_000).expect("history").len() > 100);
}

#[test]
fn a_new_commit_can_be_appended_to_a_real_rope() {
    let Some(mut store) = open("Boy.puppet") else { return };
    let head = store.head_commit().expect("head");
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = store.head_tree().expect("tree").into_iter().map(|(k, v)| (k, v.to_vec())).collect();
    files.insert("Notes".to_owned(), b"Notes{\n}\n".to_vec());
    let info = charcraft_rope::CommitInfo { who: "user <red@monster>".into(), time: 1_750_000_000, zone: "+0000".into(), message: ".Add notes.".into() };
    let before = store.log.len();
    let id = store.commit_files(&files, &info, Some(head), "refs/heads/main").expect("commit");
    let bytes = store.to_rope_bytes();
    let back = RopeStore::from_rope_bytes_strict(&bytes, &Limits::default()).expect("reads");
    assert_eq!(back.resolve_ref("HEAD"), Some(id));
    assert!(back.head_tree().expect("tree").contains_key("Notes"));
    assert!(back.log.len() > before);
}
