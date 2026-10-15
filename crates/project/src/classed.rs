//! Class-tagged tables that keep their class names, so an edited blob can be written back.
//!
//! Every project file is `Class{ ... }` with the class repeated on nested objects (`IDArray_001{`, `PPath{`,
//! `SkinBinding{`). `charcraft_rope::parse_class_table` strips the names; [`ClassedDoc`] strips them too but
//! remembers, for the n-th table of the document in pre-order, which class it had. [`ClassedDoc::to_bytes`] prints
//! the table and puts the names back, and writes `inf` and `nan` the way Character Animator does (`1 / 0`, `0 / 0`).

use std::collections::BTreeMap;

use charcraft_ltab::{LValue, to_string_pretty};
use charcraft_rope::parse_class_table;

use crate::error::{Error, Result};

/// A parsed project file: the value and the class of every table in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassedDoc {
    /// The document. Edit it freely; table positions decide which class a table is written with.
    pub value: LValue,
    classes: Vec<Option<String>>,
}

const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The class of `node` according to a map from [`ClassedDoc::classes_by_address`].
#[must_use]
pub fn class_of<'a>(map: &BTreeMap<usize, &'a str>, node: &LValue) -> Option<&'a str> {
    map.get(&std::ptr::from_ref(node).addr()).copied()
}

/// Number of tables in a value (pre-order positions are `0..count`).
fn count_tables(value: &LValue) -> usize {
    let mut count = 0_usize;
    let mut stack = vec![value];
    while let Some(next) = stack.pop() {
        if let Some(entries) = next.as_table() {
            count = count.saturating_add(1);
            stack.extend(entries.iter().map(|(_, v)| v));
        }
    }
    count
}

/// For every `{` outside a string, in order, the class name that directly precedes it.
fn scan_classes(text: &[u8]) -> Vec<Option<String>> {
    let mut classes = Vec::new();
    let mut quote: Option<u8> = None;
    let mut i = 0_usize;
    while let Some(&byte) = text.get(i) {
        if let Some(q) = quote {
            if byte == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if byte == q {
                quote = None;
            }
            i = i.saturating_add(1);
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'{' => {
                let mut start = i;
                while start.checked_sub(1).and_then(|p| text.get(p)).is_some_and(|b| is_ident(*b)) {
                    start = start.saturating_sub(1);
                }
                let word = text.get(start..i).unwrap_or_default();
                let name = word.first().is_some_and(|b| is_ident_start(*b)).then(|| String::from_utf8_lossy(word).into_owned());
                classes.push(name);
            }
            _ => {}
        }
        i = i.saturating_add(1);
    }
    classes
}

impl ClassedDoc {
    /// Parses a class-tagged blob.
    ///
    /// # Errors
    ///
    /// [`Error::Rope`] when the text is not a valid table, [`Error::ClassMismatch`] when the class names cannot be
    /// matched to the tables (only for hostile text).
    pub fn parse(blob: &[u8]) -> Result<Self> {
        let table = parse_class_table(blob)?;
        let classes = scan_classes(blob);
        let tables = count_tables(&table.value);
        if classes.len() != tables {
            return Err(Error::ClassMismatch { path: String::new(), classes: classes.len(), tables });
        }
        Ok(Self { value: table.value, classes })
    }

    /// A document built from a value, with `root` as the class of the outermost table and nothing else tagged.
    #[must_use]
    pub fn new(root: &str, value: LValue) -> Self {
        let mut classes = vec![None; count_tables(&value)];
        if let Some(first) = classes.first_mut() {
            *first = Some(root.to_owned());
        }
        Self { value, classes }
    }

    /// The class of the outermost table.
    #[must_use]
    pub fn class(&self) -> Option<&str> {
        self.classes.first().and_then(|c| c.as_deref())
    }

    /// The class of the n-th table in pre-order.
    #[must_use]
    pub fn class_at(&self, table_index: usize) -> Option<&str> {
        self.classes.get(table_index).and_then(|c| c.as_deref())
    }

    /// How many tables the document has.
    #[must_use]
    pub const fn table_count(&self) -> usize {
        self.classes.len()
    }

    /// Replaces the document value with one that has the same table layout (same number of tables in the same
    /// order), keeping all classes. Use [`ClassedDoc::value`] directly to edit in place.
    ///
    /// # Errors
    ///
    /// [`Error::ClassMismatch`] when the table counts differ.
    pub fn replace_value(&mut self, value: LValue) -> Result<()> {
        let tables = count_tables(&value);
        if tables != self.classes.len() {
            return Err(Error::ClassMismatch { path: String::new(), classes: self.classes.len(), tables });
        }
        self.value = value;
        Ok(())
    }

    /// Prints the document in Character Animator's layout with class names restored.
    ///
    /// # Errors
    ///
    /// [`Error::ClassMismatch`] when the value was edited so that it has a different number of tables than
    /// classes (call [`ClassedDoc::retag`] after adding or removing tables).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let tables = count_tables(&self.value);
        if tables != self.classes.len() {
            return Err(Error::ClassMismatch { path: String::new(), classes: self.classes.len(), tables });
        }
        Ok(print_with_classes(&to_string_pretty(&self.value), &self.classes))
    }

    /// The class of every tagged table, keyed by the table's address inside [`ClassedDoc::value`]. Look a node up
    /// with [`class_of`]. The map is only valid while the document is not moved or edited.
    #[must_use]
    pub fn classes_by_address(&self) -> BTreeMap<usize, &str> {
        let mut map = BTreeMap::new();
        let mut index = 0_usize;
        let mut stack = vec![&self.value];
        while let Some(next) = stack.pop() {
            if let Some(entries) = next.as_table() {
                if let Some(class) = self.class_at(index) {
                    map.insert(std::ptr::from_ref(next).addr(), class);
                }
                index = index.saturating_add(1);
                for (_, v) in entries.iter().rev() {
                    stack.push(v);
                }
            }
        }
        map
    }

    /// Re-tags the n-th table.
    pub fn set_class_at(&mut self, table_index: usize, class: Option<&str>) {
        if let Some(slot) = self.classes.get_mut(table_index) {
            *slot = class.map(str::to_owned);
        }
    }

    /// Rebuilds the class list after the value gained or lost tables: `tagger` is asked for the class of each table
    /// of the new value (given its pre-order index and its position), and existing tables keep what it returns.
    pub fn retag(&mut self, mut tagger: impl FnMut(usize, &LValue) -> Option<String>) {
        let mut classes = Vec::new();
        let mut stack = vec![&self.value];
        while let Some(next) = stack.pop() {
            if let Some(entries) = next.as_table() {
                classes.push(tagger(classes.len(), next));
                for (_, v) in entries.iter().rev() {
                    stack.push(v);
                }
            }
        }
        self.classes = classes;
    }
}

/// Puts class names in front of every `{` (outside strings) and turns `inf`/`nan` into Lua divisions.
fn print_with_classes(text: &str, classes: &[Option<String>]) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len().saturating_add(bytes.len() / 8));
    let mut table = 0_usize;
    let mut quote: Option<u8> = None;
    let mut i = 0_usize;
    while let Some(&byte) = bytes.get(i) {
        if let Some(q) = quote {
            if byte == b'\\' {
                // Character Animator writes a line break inside a string as a backslash followed by the line
                // break itself, and a tab as a plain tab; the table printer uses the `\n` and `\t` escapes.
                match bytes.get(i.saturating_add(1)) {
                    Some(b'n') => out.extend_from_slice(b"\\\n"),
                    Some(b't') => out.push(b'\t'),
                    Some(&next) => {
                        out.push(byte);
                        out.push(next);
                    }
                    None => out.push(byte),
                }
                i = i.saturating_add(2);
                continue;
            }
            out.push(byte);
            if byte == q {
                quote = None;
            }
            i = i.saturating_add(1);
            continue;
        }
        match byte {
            b'"' | b'\'' => {
                quote = Some(byte);
                out.push(byte);
                i = i.saturating_add(1);
            }
            b'{' => {
                if let Some(Some(name)) = classes.get(table) {
                    out.extend_from_slice(name.as_bytes());
                }
                table = table.saturating_add(1);
                out.push(byte);
                i = i.saturating_add(1);
            }
            b if is_ident_start(b) && !i.checked_sub(1).and_then(|p| bytes.get(p)).is_some_and(|p| is_ident(*p)) => {
                let mut end = i;
                while bytes.get(end).is_some_and(|b| is_ident(*b)) {
                    end = end.saturating_add(1);
                }
                let word = bytes.get(i..end).unwrap_or_default();
                let mut after = end;
                while bytes.get(after).is_some_and(u8::is_ascii_whitespace) {
                    after = after.saturating_add(1);
                }
                let is_key = bytes.get(after) == Some(&b'=');
                match word {
                    b"inf" if !is_key => out.extend_from_slice(b"1 / 0"),
                    b"nan" if !is_key => out.extend_from_slice(b"0 / 0"),
                    _ => out.extend_from_slice(word),
                }
                i = end;
            }
            _ => {
                out.push(byte);
                i = i.saturating_add(1);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUPPET: &[u8] =
        b"Puppet{\n\t_aBoundBehaviors = IDArray_001{\n\t\t_aIDArray = {\n\t\t},\n\t\t_lastID = 0,\n\t\t_sPrefix = \"BB\",\n\t},\n\tname = \"Foo{Bar\",\n}\n";

    #[test]
    fn class_names_are_kept_per_table() {
        let doc = ClassedDoc::parse(PUPPET).expect("parses");
        assert_eq!(doc.class(), Some("Puppet"));
        assert_eq!(doc.class_at(1), Some("IDArray_001"));
        assert_eq!(doc.class_at(2), None);
        assert_eq!(doc.table_count(), 3);
    }

    #[test]
    fn untouched_blob_prints_back_identically() {
        let doc = ClassedDoc::parse(PUPPET).expect("parses");
        assert_eq!(doc.to_bytes().expect("prints"), PUPPET);
    }

    #[test]
    fn infinity_round_trips_as_a_division() {
        let text = b"Track{\n\t_maxTrackItems = 1 / 0,\n\tneg = -1 / 0,\n\tnan = 0 / 0,\n\tinf = 3,\n}\n";
        let doc = ClassedDoc::parse(text).expect("parses");
        assert_eq!(doc.value.get_path_f64("_maxTrackItems"), Some(f64::INFINITY));
        assert_eq!(String::from_utf8(doc.to_bytes().expect("prints")).expect("utf8"), String::from_utf8_lossy(text));
    }

    #[test]
    fn edits_keep_the_classes() {
        let mut doc = ClassedDoc::parse(PUPPET).expect("parses");
        doc.value.set_path("_aBoundBehaviors/_lastID", 7_i64).expect("set");
        let text = String::from_utf8(doc.to_bytes().expect("prints")).expect("utf8");
        assert!(text.starts_with("Puppet{"));
        assert!(text.contains("IDArray_001{"));
        assert!(text.contains("_lastID = 7,"));
        let again = ClassedDoc::parse(text.as_bytes()).expect("reparses");
        assert_eq!(again, doc);
    }

    #[test]
    fn braces_inside_strings_are_not_tables() {
        let text = b"A{\n\ts = \"B{x}\\\"C{\",\n}\n";
        let doc = ClassedDoc::parse(text).expect("parses");
        assert_eq!(doc.table_count(), 1);
        assert_eq!(doc.to_bytes().expect("prints"), text);
    }

    #[test]
    fn line_breaks_and_tabs_in_strings_are_written_the_way_ca_writes_them() {
        let text = b"Behavior{\n\t_jsCode = \"define(\\\n\tfunction () {\\\n\t\\\\n\tx = '\\\"';\\\n});\",\n}\n";
        let doc = ClassedDoc::parse(text).expect("parses");
        assert_eq!(doc.value.get_path_str("_jsCode"), Some("define(\n\tfunction () {\n\t\\n\tx = '\"';\n});"));
        assert_eq!(doc.to_bytes().expect("prints"), text);
    }

    #[test]
    fn hostile_text_is_an_error_not_a_panic() {
        for text in [&b"{{{{"[..], b"\"unterminated", b"A{B{C{", b"\\", b"", b"\xff\xfe{"] {
            let _ = ClassedDoc::parse(text);
        }
    }

    #[test]
    fn table_count_mismatch_is_reported() {
        let mut doc = ClassedDoc::parse(PUPPET).expect("parses");
        doc.value.set("extra", LValue::empty_table()).expect("set");
        assert!(matches!(doc.to_bytes(), Err(Error::ClassMismatch { .. })));
        doc.retag(|i, _| (i == 0).then(|| "Puppet".to_owned()));
        assert!(doc.to_bytes().is_ok());
    }
}
