//! Hostile and damaged input: nothing may panic, and damage must be reported, not swallowed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration test helpers"
)]

use std::collections::BTreeMap;

use charcraft_rope::{CommitInfo, Limits, MAGIC, ObjectKind, Record, RefTarget, RopeStore, encode_records, parse_class_table, records};
use proptest::prelude::*;

fn valid_rope() -> Vec<u8> {
    let mut store = RopeStore::new();
    let mut files = BTreeMap::new();
    files.insert("Puppet".to_owned(), b"Puppet{\n\tname = \"\xd7\xa9\xd7\x9c\xd7\x95\xd7\x9d\",\n}\n".to_vec());
    files.insert("Big.js".to_owned(), vec![b'z'; 4000]);
    let info = CommitInfo { who: "user <red@monster>".into(), time: 1, zone: "+0000".into(), message: ".x.".into() };
    store.commit_files(&files, &info, None, "refs/heads/main").expect("commit");
    store.set_ref("HEAD", RefTarget::Symbolic("refs/heads/main".into()));
    store.to_rope_bytes()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = RopeStore::from_rope_bytes(&bytes, &Limits::default());
        let _ = RopeStore::from_rope_bytes_strict(&bytes, &Limits::default());
        let _ = parse_class_table(&bytes);
    }

    #[test]
    fn random_bytes_after_a_valid_magic_never_panic(tail in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&tail);
        let _ = RopeStore::from_rope_bytes(&bytes, &Limits::default());
    }

    #[test]
    fn single_byte_damage_never_panics(index in 0_usize..4096, value in any::<u8>()) {
        let mut bytes = valid_rope();
        let at = index % bytes.len();
        if let Some(byte) = bytes.get_mut(at) {
            *byte = value;
        }
        let lenient = RopeStore::from_rope_bytes(&bytes, &Limits::default());
        prop_assert!(lenient.is_ok());
        if let Ok(store) = lenient {
            let _ = store.head_tree();
            let _ = store.summary();
        }
    }

    #[test]
    fn truncation_keeps_a_valid_prefix(cut in 0_usize..4096) {
        let bytes = valid_rope();
        let at = cut % bytes.len();
        let store = RopeStore::from_rope_bytes(bytes.get(..at).unwrap_or_default(), &Limits::default()).expect("lenient");
        for item in records(bytes.get(..at).unwrap_or_default(), Limits::default(), true) {
            if item.is_err() { break; }
        }
        prop_assert!(store.log.len() <= RopeStore::from_rope_bytes(&bytes, &Limits::default()).expect("full").log.len());
    }

    #[test]
    fn objects_round_trip(data in proptest::collection::vec(any::<u8>(), 0..3000), kind in 0_u8..4) {
        let kind = ObjectKind::from_code(kind.saturating_add(1)).expect("kind");
        let record = Record::object(kind, data);
        let bytes = encode_records([&record]);
        let parsed: Vec<_> = records(&bytes, Limits::default(), true).collect::<Result<_, _>>().expect("parses");
        prop_assert_eq!(parsed.len(), 1);
        prop_assert_eq!(parsed.first().map(|(_, r)| r), Some(&record));
    }
}

#[test]
fn a_gzip_bomb_declared_small_is_rejected() {
    // Declares 10 raw bytes but the gzip stream inflates to far more.
    let big = vec![0_u8; 1_000_000];
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut gz, &big).expect("write");
    let gz = gz.finish().expect("finish");
    let mut payload = vec![0_u8; 20];
    payload.push(10);
    payload.push(2);
    payload.extend_from_slice(&u16::try_from(gz.len()).expect("small").to_be_bytes());
    payload.extend_from_slice(&gz);
    let record = Record::Unknown { kind: 0, typ: 3, payload };
    let bytes = encode_records([&record]);
    assert!(RopeStore::from_rope_bytes_strict(&bytes, &Limits::default()).is_err());
}

#[test]
fn a_tiny_object_limit_stops_big_objects() {
    let bytes = valid_rope();
    let limits = Limits { max_object_size: 100, ..Limits::default() };
    assert!(RopeStore::from_rope_bytes_strict(&bytes, &limits).is_err());
    let lenient = RopeStore::from_rope_bytes(&bytes, &limits).expect("lenient");
    assert_eq!(lenient.warnings.len(), 1);
}

#[test]
fn total_size_limit_is_enforced() {
    let bytes = valid_rope();
    let limits = Limits { max_total_bytes: 1000, ..Limits::default() };
    assert!(RopeStore::from_rope_bytes(&bytes, &limits).is_err());
}

#[test]
fn deep_trees_are_bounded() {
    let mut store = RopeStore::with_limits(Limits { max_tree_depth: 4, ..Limits::default() });
    let mut files = BTreeMap::new();
    files.insert("a/b/c/d/e/f/g/h.txt".to_owned(), b"x".to_vec());
    let info = CommitInfo { who: "u <u@x>".into(), time: 0, zone: "+0000".into(), message: String::new() };
    assert!(store.commit_files(&files, &info, None, "refs/heads/main").is_err());
}

#[test]
fn rtl_text_survives_the_store() {
    let rope = valid_rope();
    let store = RopeStore::from_rope_bytes_strict(&rope, &Limits::default()).expect("reads");
    let files = store.head_tree().expect("tree");
    let table = parse_class_table(files.get("Puppet").expect("file")).expect("parses");
    assert_eq!(table.value.get_path_str("name"), Some("שלום"));
}
