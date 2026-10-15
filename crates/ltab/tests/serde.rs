//! The serde bridge: typed structs, enums and collections to and from ltab documents.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "integration test helpers")]

use std::collections::BTreeMap;

use charcraft_ltab::{Error, Key, LValue, from_lvalue, from_str, parse, to_lvalue, to_string, to_string_pretty};
use serde::{Deserialize, Serialize};

/// A hand-written document in Character Animator's layout with keys already in its sort order.
const DOCUMENT: &str = "{\n\
\tbContainsArtworkCache = false,\n\
\tiVersion = 1,\n\
\trootPuppetPPath = \"/ProjectItems/דמות 1\",\n\
\tsType = \"puppet\",\n\
\ttBehaviorDefaults = {\n\
\t\t[\"Vendor.Blink.js\"] = {\n\
\t\t\tlength = 80,\n\
\t\t\trate = 30,\n\
\t\t},\n\
\t\t[\"Vendor.Drag.js\"] = {\n\
\t\t\thandleReleaseDuration = 0.20000000000000001,\n\
\t\t\tshowTargets = false,\n\
\t\t\ttilt = -20,\n\
\t\t},\n\
\t},\n\
}\n";

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Param {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ArchiveInfo {
    #[serde(rename = "bContainsArtworkCache")]
    contains_artwork_cache: bool,
    #[serde(rename = "iVersion")]
    version: u32,
    #[serde(rename = "rootPuppetPPath")]
    root_puppet_path: String,
    #[serde(rename = "sType")]
    kind: String,
    #[serde(rename = "tBehaviorDefaults")]
    behavior_defaults: BTreeMap<String, BTreeMap<String, Param>>,
}

#[test]
fn a_typed_model_reads_a_character_animator_document() {
    let info: ArchiveInfo = from_str(DOCUMENT).unwrap();
    assert!(!info.contains_artwork_cache);
    assert_eq!(info.version, 1);
    assert_eq!(info.root_puppet_path, "/ProjectItems/דמות 1");
    assert_eq!(info.kind, "puppet");
    let drag = info.behavior_defaults.get("Vendor.Drag.js").unwrap();
    assert_eq!(drag.get("handleReleaseDuration"), Some(&Param::Float(0.2)));
    assert_eq!(drag.get("showTargets"), Some(&Param::Bool(false)));
    assert_eq!(drag.get("tilt"), Some(&Param::Int(-20)));
    assert_eq!(info.behavior_defaults.get("Vendor.Blink.js").unwrap().get("rate"), Some(&Param::Int(30)));
}

#[test]
fn a_typed_model_writes_the_same_bytes_back() {
    let info: ArchiveInfo = from_str(DOCUMENT).unwrap();
    assert_eq!(to_string(&info).unwrap(), DOCUMENT);
}

#[test]
fn documents_pass_through_serde_unchanged() {
    let document = parse(DOCUMENT).unwrap();
    let again: LValue = from_lvalue(&document).unwrap();
    assert_eq!(again, document);
    assert_eq!(to_lvalue(&document).unwrap(), document);
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Shape {
    Empty,
    Circle(f64),
    Rect { w: i32, h: i32 },
    Polyline(i64, i64, i64),
    Named(String),
}

#[test]
fn enums_use_strings_and_one_entry_tables() {
    let shapes = vec![Shape::Empty, Shape::Circle(2.5), Shape::Rect { w: 3, h: 4 }, Shape::Polyline(1, 2, 3), Shape::Named("x".into())];
    let doc = to_lvalue(&shapes).unwrap();
    assert_eq!(doc.to_string(), "{\"Empty\",{Circle=2.5},{Rect={w=3,h=4}},{Polyline={1,2,3}},{Named=\"x\"}}");
    let back: Vec<Shape> = from_lvalue(&doc).unwrap();
    assert_eq!(back, shapes);
    assert_eq!(from_str::<Shape>("{ Rect = { h = 1, w = 2 } }").unwrap(), Shape::Rect { w: 2, h: 1 });
    assert_eq!(from_str::<Shape>("\"Empty\"").unwrap(), Shape::Empty);
    assert_eq!(from_str::<Shape>("{ Empty = nil }").unwrap(), Shape::Empty);
    assert_eq!(from_str::<Shape>("{ Empty = {} }").unwrap(), Shape::Empty);
    assert!(from_str::<Shape>("\"Triangle\"").is_err());
    assert!(from_str::<Shape>("{ Circle = 1, Empty = nil }").is_err());
    assert!(from_str::<Shape>("5").is_err());
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Everything {
    flag: bool,
    small: i8,
    word: u16,
    big: u64,
    min: i64,
    float: f64,
    single: f32,
    text: String,
    letter: char,
    maybe: Option<i32>,
    missing: Option<String>,
    list: Vec<i32>,
    pair: (i32, String),
    nested: BTreeMap<String, Vec<f64>>,
    by_number: BTreeMap<i64, String>,
    bytes: Vec<u8>,
}

fn everything() -> Everything {
    Everything {
        flag: true,
        small: -5,
        word: 65535,
        big: u64::MAX,
        min: i64::MIN,
        float: 0.1,
        single: 0.1,
        text: "שלום \"עולם\"\n\t\u{1}".to_owned(),
        letter: 'ש',
        maybe: Some(7),
        missing: None,
        list: vec![3, 1, 2],
        pair: (1, "one".to_owned()),
        nested: BTreeMap::from([("a".to_owned(), vec![0.5, 1.0, 1e-9]), ("b".to_owned(), vec![])]),
        by_number: BTreeMap::from([(2, "two".to_owned()), (10, "ten".to_owned())]),
        bytes: vec![0, 255, 7],
    }
}

#[test]
fn every_kind_of_field_round_trips_through_values_and_text() {
    let original = everything();
    let doc = to_lvalue(&original).unwrap();
    assert_eq!(from_lvalue::<Everything>(&doc).unwrap(), original);
    let text = to_string(&original).unwrap();
    assert_eq!(from_str::<Everything>(&text).unwrap(), original);
    // None is not stored at all (Lua has no stored nil); floats print like Character Animator prints them.
    assert!(doc.get("missing").is_none());
    assert_eq!(from_str::<()>("nil").unwrap(), ());
    assert_eq!(doc.get_path("float"), Some(&LValue::Num { value: 0.1, text: "0.10000000000000001".to_owned() }));
    assert_eq!(doc.get_path("single"), Some(&LValue::Num { value: 0.1, text: "0.1".to_owned() }));
    assert_eq!(doc.get_path("big").and_then(LValue::as_f64), Some(1.844_674_407_370_955_2e19));
    assert_eq!(doc.get_path("min"), Some(&LValue::Int(i64::MIN)));
    assert!(text.contains("\n\tbig = 18446744073709551615,\n"), "{text}");
}

#[test]
fn text_output_is_sorted_and_stable() {
    let text = to_string(&everything()).unwrap();
    let keys: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with('\t') && !l.starts_with("\t\t") && !l.starts_with("\t}"))
        .filter_map(|l| l.trim_start().split(' ').next())
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted);
    assert_eq!(to_string(&everything()).unwrap(), text);
}

#[test]
fn integers_accept_whole_floats_and_floats_accept_integers() {
    assert_eq!(from_str::<i32>("5.0").unwrap(), 5);
    assert_eq!(from_str::<u8>("1e2").unwrap(), 100);
    assert_eq!(from_str::<f64>("5").unwrap().to_bits(), 5.0_f64.to_bits());
    assert!(from_str::<i32>("5.5").is_err());
    assert!(from_str::<u8>("256").is_err());
    assert!(from_str::<u8>("-1").is_err());
    assert!(from_str::<i8>("true").is_err());
    assert_eq!(from_str::<u64>("18446744073709551615").unwrap(), u64::MAX);
    assert_eq!(from_str::<i128>("5").unwrap(), 5);
}

#[test]
fn sequences_are_lua_sequences() {
    assert_eq!(from_str::<Vec<i32>>("{ 1, 2, 3 }").unwrap(), [1, 2, 3]);
    assert_eq!(from_str::<Vec<String>>("{ [2] = \"b\", [1] = \"a\" }").unwrap(), ["a", "b"]);
    assert_eq!(from_str::<Vec<i32>>("{}").unwrap(), Vec::<i32>::new());
    assert_eq!(from_str::<(i32, i32)>("{ 1, 2 }").unwrap(), (1, 2));
    assert!(from_str::<(i32, i32)>("{ 1, 2, 3 }").is_err());
    assert!(from_str::<(i32, i32, i32)>("{ 1, 2 }").is_err());
    assert!(from_str::<Vec<i32>>("{ [1] = 1, [3] = 3 }").is_err());
    assert!(from_str::<Vec<i32>>("{ a = 1 }").is_err());
    assert!(from_str::<Vec<i32>>("5").is_err());
    assert_eq!(from_str::<[u8; 3]>("{ 1, 2, 3 }").unwrap(), [1, 2, 3]);
}

#[test]
fn maps_and_structs_read_integer_and_string_keys() {
    assert_eq!(from_str::<BTreeMap<i64, bool>>("{ [1] = true, [2] = false }").unwrap(), BTreeMap::from([(1, true), (2, false)]));
    assert_eq!(from_str::<BTreeMap<i64, bool>>("{ [\"1\"] = true }").unwrap(), BTreeMap::from([(1, true)]));
    assert_eq!(from_str::<BTreeMap<i64, i32>>("{ 10, 20 }").unwrap(), BTreeMap::from([(1, 10), (2, 20)]));
    assert_eq!(from_str::<BTreeMap<String, i32>>("{ a = 1, [\"b c\"] = 2 }").unwrap().len(), 2);
    assert!(from_str::<BTreeMap<String, i32>>("5").is_err());
}

#[derive(Debug, PartialEq, Deserialize)]
struct Strict {
    a: i32,
    #[serde(default)]
    b: Vec<i32>,
    c: Option<String>,
}

#[test]
fn missing_and_unknown_fields() {
    assert_eq!(from_str::<Strict>("{ a = 1 }").unwrap(), Strict { a: 1, b: vec![], c: None });
    assert_eq!(from_str::<Strict>("{ a = 1, extra = { deeply = { nested = 1 } }, c = nil }").unwrap(), Strict { a: 1, b: vec![], c: None });
    let error = from_str::<Strict>("{ b = {} }").unwrap_err();
    assert!(error.to_string().contains("missing field `a`"), "{error}");
    let error = from_str::<Strict>("{ a = \"x\" }").unwrap_err();
    assert!(error.to_string().contains("invalid type"), "{error}");
    assert!(matches!(error, Error::Custom(_)));
    assert!(matches!(from_str::<Strict>("{ a = "), Err(Error::Syntax { .. })));
}

#[test]
fn right_to_left_keys_and_values_survive_the_bridge() {
    let map = BTreeMap::from([("שלום".to_owned(), "עולם".to_owned()), ("مفتاح".to_owned(), "قيمة \u{200f}12".to_owned())]);
    let text = to_string(&map).unwrap();
    assert!(text.contains("[\"שלום\"] = \"עולם\""), "{text}");
    assert_eq!(from_str::<BTreeMap<String, String>>(&text).unwrap(), map);
}

#[test]
fn unsupported_map_keys_are_rejected() {
    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    struct Point(i32, i32);
    let map = BTreeMap::from([(Point(1, 2), 3)]);
    assert!(matches!(to_lvalue(&map), Err(Error::Custom(_))));
}

#[test]
fn lvalue_is_serializable_to_json() {
    let doc =
        parse("{ a = 1, b = { 1, 2.5, \"x\", true }, [\"c d\"] = { [3] = 1, [2.5] = 2, [false] = 3, \"p\" }, e = nil, f = 0.20000000000000001 }").unwrap();
    let json = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "a": 1,
            "b": [1, 2.5, "x", true],
            "c d": { "3": 1, "2.5": 2, "false": 3, "1": "p" },
            "e": null,
            "f": 0.2,
        })
    );
}

#[test]
fn json_documents_load_as_lvalues() {
    let value: LValue = serde_json::from_str(r#"{"a":[1,2.5,"x",null],"b":{"c":true},"d":18446744073709551615}"#).unwrap();
    assert_eq!(value.get_path_i64("a/1"), Some(1));
    assert_eq!(value.get_path_f64("a/2"), Some(2.5));
    assert_eq!(value.get_path_str("a/3"), Some("x"));
    assert_eq!(value.get_path("a/4"), Some(&LValue::Nil));
    assert_eq!(value.get_path_bool("b/c"), Some(true));
    assert_eq!(value.get_path("d").and_then(LValue::as_f64), Some(1.844_674_407_370_955_2e19));
    assert_eq!(to_string_pretty(&value).lines().next(), Some("{"));
    assert!(matches!(value.get_path("a"), Some(LValue::Table(items)) if items.iter().all(|(k, _)| *k == Key::Pos)));
}

#[test]
fn serializing_floats_matches_character_animator_text() {
    #[derive(Serialize)]
    struct Floats {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f32,
    }
    let text = to_string(&Floats { a: 0.2, b: 100.0, c: -0.0, d: 1e20, e: f64::INFINITY, f: 1.5 }).unwrap();
    assert_eq!(text, "{\n\ta = 0.20000000000000001,\n\tb = 100,\n\tc = -0,\n\td = 1e+20,\n\te = inf,\n\tf = 1.5,\n}\n");
}
