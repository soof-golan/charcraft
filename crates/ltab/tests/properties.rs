//! Property tests: round trips for generated documents and no-panic guarantees for hostile input.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration test helpers"
)]

use charcraft_ltab::{Error, Key, LValue, Limits, format_g17, parse, parse_bytes, parse_with, to_string_compact, to_string_pretty};
use proptest::collection::vec;
use proptest::prelude::*;

#[expect(clippy::disallowed_types, reason = "proptest's prop_oneof! expands to std::sync::Arc")]
fn arb_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[A-Za-z_][A-Za-z0-9_]{0,10}",
        3 => vec(any::<char>(), 0..10).prop_map(String::from_iter),
        1 => Just("שלום עולם".to_owned()),
        1 => Just("مرحبا \u{202e}abc\u{200f} 123".to_owned()),
        1 => Just("tab\there \"quoted\" back\\slash\nnewline\r\u{0}\u{1f}\u{7f}".to_owned()),
    ]
}

#[expect(clippy::disallowed_types, reason = "proptest's prop_oneof! expands to std::sync::Arc")]
fn arb_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        4 => any::<f64>().prop_filter("NaN has no canonical bits", |v| !v.is_nan()),
        2 => (-1_000_000_i32..1_000_000).prop_map(|n| f64::from(n) / 1000.0),
        1 => Just(0.2),
        1 => Just(-0.0),
        1 => Just(f64::INFINITY),
        1 => Just(f64::NEG_INFINITY),
        1 => Just(9_007_199_254_740_993.0),
    ]
}

#[expect(clippy::disallowed_types, reason = "proptest's prop_oneof! expands to std::sync::Arc")]
fn arb_scalar() -> impl Strategy<Value = LValue> {
    prop_oneof![
        1 => Just(LValue::Nil),
        1 => any::<bool>().prop_map(LValue::Bool),
        2 => any::<i64>().prop_map(LValue::Int),
        2 => (-100_i64..100).prop_map(LValue::Int),
        3 => arb_f64().prop_map(LValue::float),
        3 => arb_text().prop_map(LValue::Str),
        1 => vec(any::<u8>(), 0..8).prop_map(LValue::from_bytes),
        1 => Just(LValue::from_number_text("nan").unwrap()),
        1 => Just(LValue::from_number_text("-nan").unwrap()),
    ]
}

#[expect(clippy::disallowed_types, reason = "proptest's prop_oneof! expands to std::sync::Arc")]
fn arb_key() -> impl Strategy<Value = Key> {
    prop_oneof![
        4 => arb_text().prop_map(Key::Str),
        2 => any::<i64>().prop_map(Key::Int),
        1 => arb_f64().prop_map(Key::float),
        1 => any::<bool>().prop_map(Key::Bool),
        3 => Just(Key::Pos),
    ]
}

fn arb_value() -> impl Strategy<Value = LValue> {
    arb_scalar().prop_recursive(4, 64, 6, |inner| vec((arb_key(), inner), 0..6).prop_map(LValue::Table))
}

/// Strings built from the characters that matter to the lexer, to reach deep parser states.
fn arb_ltab_ish() -> impl Strategy<Value = String> {
    vec(
        prop_oneof![
            Just("{"),
            Just("}"),
            Just("["),
            Just("]"),
            Just("="),
            Just(","),
            Just(";"),
            Just("\""),
            Just("'"),
            Just("\\"),
            Just("-"),
            Just("--"),
            Just("[["),
            Just("]]"),
            Just("[=["),
            Just("]=]"),
            Just("0x"),
            Just("1e"),
            Just("."),
            Just("nil"),
            Just("true"),
            Just("inf"),
            Just("\n"),
            Just(" "),
            Just("\\u{"),
            Just("\\x"),
            Just("\\z"),
            Just("12"),
            Just("a"),
            Just("שלום"),
            Just("\u{feff}"),
        ],
        0..40,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn pretty_output_parses_back_to_the_same_value(value in arb_value()) {
        let text = to_string_pretty(&value);
        let back = parse(&text).unwrap_or_else(|error| panic!("{error}\n{text}"));
        prop_assert_eq!(&back, &value);
        // Printing is a fixed point: the text of a parsed document is the text it came from.
        prop_assert_eq!(to_string_pretty(&back), text);
    }

    #[test]
    fn compact_output_parses_back_to_the_same_value(value in arb_value()) {
        let text = to_string_compact(&value);
        let back = parse(&text).unwrap_or_else(|error| panic!("{error}\n{text}"));
        prop_assert_eq!(back, value);
    }

    #[test]
    fn pretty_and_compact_agree(value in arb_value()) {
        prop_assert_eq!(parse(&to_string_pretty(&value)).unwrap(), parse(&to_string_compact(&value)).unwrap());
    }

    #[test]
    fn sorting_keys_is_idempotent_and_keeps_the_content(value in arb_value()) {
        let mut once = value.clone();
        once.sort_keys();
        let mut twice = once.clone();
        twice.sort_keys();
        prop_assert_eq!(&once, &twice);
        // Same text length: nothing was lost or added.
        prop_assert_eq!(to_string_compact(&once).len(), to_string_compact(&value).len());
    }

    #[test]
    fn g17_round_trips_every_finite_double(bits in any::<u64>()) {
        let value = f64::from_bits(bits);
        prop_assume!(value.is_finite());
        let text = format_g17(value);
        let back: f64 = text.parse().unwrap();
        prop_assert_eq!(back.to_bits(), value.to_bits(), "{}", text);
        // %.17g prints at most 17 significant digits.
        let mantissa = text.split('e').next().unwrap();
        let significant: String = mantissa.chars().filter(char::is_ascii_digit).collect();
        prop_assert!(significant.trim_start_matches('0').len() <= 17, "{}", text);
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in vec(any::<u8>(), 0..300)) {
        let _ = parse_bytes(&bytes);
    }

    #[test]
    fn ltab_like_text_never_panics(text in arb_ltab_ish()) {
        let limits = Limits { max_depth: 8, max_entries: 100, ..Limits::default() };
        let _ = parse_with(&text, &limits);
        let _ = parse(&text);
    }

    #[test]
    fn damaged_documents_never_panic(value in arb_value(), cut in any::<prop::sample::Index>(), flip in any::<prop::sample::Index>(), byte in any::<u8>()) {
        let mut bytes = to_string_pretty(&value).into_bytes();
        if !bytes.is_empty() {
            let at = flip.index(bytes.len());
            bytes[at] = byte;
            let end = cut.index(bytes.len() + 1);
            let _ = parse_bytes(&bytes);
            bytes.truncate(end);
            let _ = parse_bytes(&bytes);
        }
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash(depth in 0_usize..6000) {
        let text = "{".repeat(depth);
        prop_assert!(parse(&text).is_err());
        let closed = format!("{}{}", "{".repeat(depth), "}".repeat(depth));
        match parse(&closed) {
            Ok(_) => prop_assert!(depth <= 128),
            Err(error) => prop_assert!(depth > 128 || depth == 0, "{error}"),
        }
    }
}

#[test]
fn limits_apply_to_generated_documents() {
    let big = format!("{{{}}}", "1,".repeat(1000));
    assert!(parse(&big).is_ok());
    let few = Limits { max_entries: 999, ..Limits::default() };
    assert!(matches!(parse_with(&big, &few), Err(Error::TooManyEntries { limit: 999 })));
}
