//! What the blobs hold: Character Animator's class-tagged Lua tables.
//!
//! Every file in a project's tree is text in the shape `ClassName{ key = value, ... }`, a Lua table constructor
//! preceded by the name of the class it deserialises into (`Puppet{`, `Skin{`, `Behavior{`, `ProjectUIData_001{`),
//! and nested objects carry their class the same way (`_aBoundBehaviors = IDArray_001{ ... }`). [`parse_class_table`]
//! strips those names with a string-aware scan and hands the rest to [`charcraft_ltab`].

use charcraft_ltab::{LValue, Limits as LtabLimits};
use serde::Serialize;

use crate::error::Result;

/// A parsed class-tagged table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassTable {
    /// The root class name (`Puppet`).
    pub class: String,
    /// The root table.
    pub value: LValue,
    /// How many nested objects carried a class name that was stripped.
    pub nested_classes: usize,
}

const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Character Animator writes infinity as the Lua expression `1 / 0` (and NaN as `0 / 0`), which a data parser does
/// not evaluate. Returns the length of the expression at `at` and the literal that stands for it.
fn division_literal(text: &[u8], at: usize) -> Option<(usize, &'static [u8])> {
    let before = at.checked_sub(1).and_then(|p| text.get(p)).copied();
    if before.is_some_and(|b| is_ident(b) || b == b'.') {
        return None;
    }
    let literal: &'static [u8] = if text.get(at) == Some(&b'1') { b"inf" } else { b"nan" };
    let mut j = at.saturating_add(1);
    if !text.get(j).is_some_and(|b| b.is_ascii_whitespace() || *b == b'/') {
        return None;
    }
    while text.get(j).is_some_and(u8::is_ascii_whitespace) {
        j = j.saturating_add(1);
    }
    if text.get(j) != Some(&b'/') {
        return None;
    }
    j = j.saturating_add(1);
    while text.get(j).is_some_and(u8::is_ascii_whitespace) {
        j = j.saturating_add(1);
    }
    if text.get(j) != Some(&b'0') {
        return None;
    }
    j = j.saturating_add(1);
    if text.get(j).is_some_and(|b| is_ident(*b) || *b == b'.') {
        return None;
    }
    Some((j.saturating_sub(at), literal))
}

/// Removes every `Name` that directly precedes a `{` outside strings and comments. Returns the cleaned text, the
/// root class name (when the text starts with one) and the number of names removed.
#[must_use]
pub fn strip_class_names(text: &[u8]) -> (Vec<u8>, Option<String>, usize) {
    let mut out = Vec::with_capacity(text.len());
    let mut root = None;
    let mut removed = 0_usize;
    let mut i = 0_usize;
    let mut quote: Option<u8> = None;
    while let Some(&byte) = text.get(i) {
        if let Some(q) = quote {
            out.push(byte);
            if byte == b'\\' {
                if let Some(&next) = text.get(i.saturating_add(1)) {
                    out.push(next);
                }
                i = i.saturating_add(2);
                continue;
            }
            if byte == q {
                quote = None;
            }
            i = i.saturating_add(1);
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            quote = Some(byte);
            out.push(byte);
            i = i.saturating_add(1);
            continue;
        }
        if (byte == b'1' || byte == b'0')
            && let Some((len, word)) = division_literal(text, i)
        {
            out.extend_from_slice(word);
            i = i.saturating_add(len);
            continue;
        }
        let at_word_start = is_ident_start(byte) && !i.checked_sub(1).and_then(|p| text.get(p)).is_some_and(|b| is_ident(*b));
        if at_word_start {
            let mut end = i;
            while text.get(end).is_some_and(|b| is_ident(*b)) {
                end = end.saturating_add(1);
            }
            let word = text.get(i..end).unwrap_or_default();
            if text.get(end) == Some(&b'{') {
                if out.iter().all(u8::is_ascii_whitespace) && root.is_none() {
                    root = Some(String::from_utf8_lossy(word).into_owned());
                }
                removed = removed.saturating_add(1);
            } else {
                out.extend_from_slice(word);
            }
            i = end;
            continue;
        }
        out.push(byte);
        i = i.saturating_add(1);
    }
    (out, root, removed)
}

/// Parses a blob that is a class-tagged table.
///
/// # Errors
///
/// [`crate::Error::Ltab`] when the remaining text is not valid ltab. A blob without a class name parses as a plain
/// table with an empty `class`.
pub fn parse_class_table(blob: &[u8]) -> Result<ClassTable> {
    let (clean, root, nested) = strip_class_names(blob);
    let value = charcraft_ltab::parse_bytes_with(&clean, &LtabLimits::default())?;
    Ok(ClassTable { class: root.unwrap_or_default(), value, nested_classes: nested.saturating_sub(1) })
}

/// The class name a blob starts with, without parsing the rest.
#[must_use]
pub fn class_name(blob: &[u8]) -> Option<&str> {
    let start = blob.iter().position(|b| !b.is_ascii_whitespace())?;
    let rest = blob.get(start..)?;
    let end = rest.iter().position(|b| !is_ident(*b))?;
    if end == 0 || rest.get(end) != Some(&b'{') || !rest.first().is_some_and(|b| is_ident_start(*b)) {
        return None;
    }
    std::str::from_utf8(rest.get(..end)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUPPET: &[u8] =
        b"Puppet{\n\t_aBoundBehaviors = IDArray_001{\n\t\t_aIDArray = {\n\t\t},\n\t\t_lastID = 0,\n\t\t_sPrefix = \"BB\",\n\t},\n\tname = \"Foo{Bar\",\n}\n";

    #[test]
    fn class_names_are_stripped_outside_strings_only() {
        let (clean, root, removed) = strip_class_names(PUPPET);
        assert_eq!(root.as_deref(), Some("Puppet"));
        assert_eq!(removed, 2);
        let text = String::from_utf8(clean).expect("utf8");
        assert!(text.starts_with("{\n"));
        assert!(text.contains("= {\n\t\t_aIDArray"));
        assert!(text.contains("\"Foo{Bar\""));
    }

    #[test]
    fn a_puppet_blob_parses() {
        let table = parse_class_table(PUPPET).expect("parses");
        assert_eq!(table.class, "Puppet");
        assert_eq!(table.nested_classes, 1);
        assert_eq!(table.value.get_path_str("name"), Some("Foo{Bar"));
        assert_eq!(table.value.get_path_str("_aBoundBehaviors/_sPrefix"), Some("BB"));
    }

    #[test]
    fn empty_marker_files_parse() {
        let table = parse_class_table(b"ProjectUIData_001{\n}\n").expect("parses");
        assert_eq!(table.class, "ProjectUIData_001");
        assert!(table.value.as_table().is_some_and(<[_]>::is_empty));
    }

    #[test]
    fn infinity_written_as_a_division_is_understood() {
        let text = b"Track{\n\t_maxTrackItems = 1 / 0,\n\tnot = 10 / 0.5,\n\tneg = -1 / 0,\n\tnan = 0 / 0,\n}\n";
        let (clean, _, _) = strip_class_names(text);
        let clean = String::from_utf8(clean).expect("utf8");
        assert!(clean.contains("_maxTrackItems = inf,"));
        assert!(clean.contains("neg = -inf,"));
        assert!(clean.contains("nan = nan,"));
        assert!(clean.contains("10 / 0.5"));
        let table = parse_class_table(b"Track{\n\t_max = 1 / 0,\n}\n").expect("parses");
        assert_eq!(table.value.get_path_f64("_max"), Some(f64::INFINITY));
    }

    #[test]
    fn class_name_peeks() {
        assert_eq!(class_name(PUPPET), Some("Puppet"));
        assert_eq!(class_name(b"{ a = 1 }"), None);
        assert_eq!(class_name(b"hello"), None);
    }

    #[test]
    fn hostile_text_does_not_panic() {
        for text in [&b"{{{{"[..], b"\"unterminated", b"A{B{C{", b"\\", b"", b"\xff\xfe{"] {
            let _ = parse_class_table(text);
        }
    }
}
