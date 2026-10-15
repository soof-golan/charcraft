//! The value model: [`LValue`] and table [`Key`]s.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use anim_core::num;

use crate::error::{Error, Result};
use crate::number::{Number, format_g17, parse_number_text};

/// One Lua value as it appears in a Character Animator metadata file.
///
/// Tables keep their entries in file order (Character Animator writes them sorted, but we never assume that) and
/// keep duplicate keys, so a document always prints back the way it was read. Lookups follow Lua semantics: when a
/// key appears twice the later entry wins.
///
/// Equality is structural: floats compare by bit pattern *and* source text, so `0.2` and `0.20000000000000001` are
/// different documents even though they are the same number.
#[derive(Debug, Clone)]
pub enum LValue {
    /// `nil`.
    Nil,
    /// `true` or `false`.
    Bool(bool),
    /// An integer literal that fits `i64` (`30`, `-20`, `0x10`).
    Int(i64),
    /// Any other number, with the exact text it was written with (`0.20000000000000001`, `1e+20`, `-0`, `inf`).
    /// Build new ones with [`LValue::float`] so the text is what Character Animator would print.
    Num {
        /// The numeric value.
        value: f64,
        /// The literal; printed verbatim, so files round-trip byte for byte.
        text: String,
    },
    /// A string. Lua strings are byte strings; this is the case where the bytes are valid UTF-8 (all names, paths
    /// and ids in practice, including Hebrew, Arabic and every other script).
    Str(String),
    /// A string whose bytes are *not* valid UTF-8 (only possible through `\ddd` escapes or a non-UTF-8 file). Build
    /// with [`LValue::from_bytes`], which picks [`LValue::Str`] whenever the bytes are valid UTF-8.
    Bytes(Vec<u8>),
    /// A table: ordered `(key, value)` entries.
    Table(Vec<(Key, Self)>),
}

/// The key of a table entry.
#[derive(Debug, Clone)]
pub enum Key {
    /// A string key, written `name = ...` when `name` is an identifier and `["any string"] = ...` otherwise.
    Str(String),
    /// An integer key, written `[3] = ...`.
    Int(i64),
    /// A non-integer number key, written `[1.5] = ...`.
    Num {
        /// The numeric value.
        value: f64,
        /// The literal as written.
        text: String,
    },
    /// A boolean key, written `[true] = ...`.
    Bool(bool),
    /// No explicit key: an array-style value. Its Lua index is its rank among the positional entries, from 1.
    Pos,
}

impl PartialEq for LValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Num { value: a, text: ta }, Self::Num { value: b, text: tb }) => a.to_bits() == b.to_bits() && ta == tb,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Table(a), Self::Table(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for LValue {}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Num { value: a, text: ta }, Self::Num { value: b, text: tb }) => a.to_bits() == b.to_bits() && ta == tb,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Pos, Self::Pos) => true,
            _ => false,
        }
    }
}

impl Eq for Key {}

impl Key {
    /// A number key printed the way Character Animator prints numbers; integral values become [`Key::Int`].
    #[must_use]
    pub fn float(value: f64) -> Self {
        match LValue::float(value) {
            LValue::Int(int) => Self::Int(int),
            LValue::Num { value, text } => Self::Num { value, text },
            _ => Self::Pos,
        }
    }

    /// The string of a [`Key::Str`].
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The order Character Animator writes keys in: positional entries first (in their own order), then integers,
    /// other numbers, booleans and finally strings by byte value (`"Trim_Ah"` sorts before `"faceTracker"`).
    #[must_use]
    pub fn writer_order(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Str(a), Self::Str(b)) => a.as_bytes().cmp(b.as_bytes()),
            (Self::Int(a), Self::Int(b)) => a.cmp(b),
            (Self::Num { value: a, .. }, Self::Num { value: b, .. }) => a.total_cmp(b),
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }

    const fn rank(&self) -> u8 {
        match self {
            Self::Pos => 0,
            Self::Int(_) => 1,
            Self::Num { .. } => 2,
            Self::Bool(_) => 3,
            Self::Str(_) => 4,
        }
    }
}

impl From<&str> for Key {
    fn from(key: &str) -> Self {
        Self::Str(key.to_owned())
    }
}

impl From<String> for Key {
    fn from(key: String) -> Self {
        Self::Str(key)
    }
}

impl From<i64> for Key {
    fn from(key: i64) -> Self {
        Self::Int(key)
    }
}

impl From<Number> for Key {
    fn from(number: Number) -> Self {
        match number {
            Number::Int(int) => Self::Int(int),
            Number::Float { value, text } => Self::Num { value, text },
        }
    }
}

impl From<Number> for LValue {
    fn from(number: Number) -> Self {
        match number {
            Number::Int(int) => Self::Int(int),
            Number::Float { value, text } => Self::Num { value, text },
        }
    }
}

// ---- constructors -------------------------------------------------------------------------------------------------

impl LValue {
    /// An empty table.
    #[must_use]
    pub const fn empty_table() -> Self {
        Self::Table(Vec::new())
    }

    /// A string value.
    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }

    /// A string value from raw bytes: [`LValue::Str`] when they are valid UTF-8, otherwise [`LValue::Bytes`].
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(text) => Self::Str(text),
            Err(error) => Self::Bytes(error.into_bytes()),
        }
    }

    /// A number printed the way Character Animator prints one (`%.17g`): integral values within `i64` become
    /// [`LValue::Int`] (`100`, not `100.0`), everything else a [`LValue::Num`] such as `0.20000000000000001`.
    #[must_use]
    pub fn float(value: f64) -> Self {
        let text = format_g17(value);
        match parse_number_text(&text) {
            Some(Number::Int(int)) => Self::Int(int),
            _ => Self::Num { value, text },
        }
    }

    /// A number from decimal text (`"0.1"`, `"42"`) kept verbatim; `None` if the text is not a number literal.
    #[must_use]
    pub fn from_number_text(text: &str) -> Option<Self> {
        parse_number_text(text).map(Self::from)
    }

    // ---- scalar access --------------------------------------------------------------------------------------------

    /// The Lua type name: `nil`, `boolean`, `number`, `string` or `table`.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Bool(_) => "boolean",
            Self::Int(_) | Self::Num { .. } => "number",
            Self::Str(_) | Self::Bytes(_) => "string",
            Self::Table(_) => "table",
        }
    }

    /// True for `nil`.
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    /// The boolean, if this is one.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The integer: an [`LValue::Int`], or a number whose value is a whole number that fits `i64` (`1e3`, `5.0`).
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(int) => Some(*int),
            Self::Num { value, .. } if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.2e18 => Some(num::round_f64_to_i64(*value)),
            _ => None,
        }
    }

    /// The number as `f64` (integers above 2^53 are rounded).
    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(int) => Some(num::i64_to_f64(*int)),
            Self::Num { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// The string, if this is a valid-UTF-8 string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The bytes of any string, valid UTF-8 or not.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Str(s) => Some(s.as_bytes()),
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// The entries, if this is a table.
    #[must_use]
    pub fn as_table(&self) -> Option<&[(Key, Self)]> {
        match self {
            Self::Table(entries) => Some(entries),
            _ => None,
        }
    }

    /// The entries for editing, if this is a table.
    pub const fn as_table_mut(&mut self) -> Option<&mut Vec<(Key, Self)>> {
        match self {
            Self::Table(entries) => Some(entries),
            _ => None,
        }
    }

    // ---- table access ---------------------------------------------------------------------------------------------

    /// All entries of a table (empty for every other kind of value).
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &(Key, Self)> {
        self.as_table().unwrap_or_default().iter()
    }

    /// The entries that have a string key, in order.
    pub fn named(&self) -> impl Iterator<Item = (&str, &Self)> {
        self.iter().filter_map(|(key, value)| key.as_str().map(|name| (name, value)))
    }

    /// The value stored under the string key `key` (the last one if it appears twice).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        self.iter().rev().find(|(k, _)| k.as_str() == Some(key)).map(|(_, v)| v)
    }

    /// Mutable access to the value under the string key `key` (the last one if it appears twice).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Self> {
        self.as_table_mut()?.iter_mut().rev().find(|(k, _)| k.as_str() == Some(key)).map(|(_, v)| v)
    }

    /// The value at Lua index `index` (1-based): either an explicit `[index] = ...` entry or the `index`-th
    /// positional entry.
    #[must_use]
    pub fn get_index(&self, index: i64) -> Option<&Self> {
        let mut next_positional = 1_i64;
        let mut found = None;
        for (key, value) in self.iter() {
            let logical = match key {
                Key::Pos => {
                    let here = next_positional;
                    next_positional = next_positional.saturating_add(1);
                    Some(here)
                }
                Key::Int(int) => Some(*int),
                _ => None,
            };
            if logical == Some(index) {
                found = Some(value);
            }
        }
        found
    }

    /// Follows a `/`-separated path of keys, for example `"tBehaviorDefaults/Adobe.Blinker.js/rate"`. A segment
    /// that is not a string key but reads as an integer addresses that Lua index. Empty segments are skipped, so a
    /// key that is the empty string or contains `/` needs [`LValue::get_in`].
    #[must_use]
    pub fn get_path(&self, path: &str) -> Option<&Self> {
        path.split('/').filter(|segment| !segment.is_empty()).try_fold(self, |current, segment| current.step(segment))
    }

    /// Like [`LValue::get_path`] with the path already split, so keys may contain `/`.
    #[must_use]
    pub fn get_in<S: AsRef<str>>(&self, segments: &[S]) -> Option<&Self> {
        segments.iter().try_fold(self, |current, segment| current.step(segment.as_ref()))
    }

    fn step(&self, segment: &str) -> Option<&Self> {
        self.get(segment).or_else(|| segment.parse::<i64>().ok().and_then(|index| self.get_index(index)))
    }

    /// The boolean at `path` (see [`LValue::get_path`]).
    #[must_use]
    pub fn get_path_bool(&self, path: &str) -> Option<bool> {
        self.get_path(path).and_then(Self::as_bool)
    }

    /// The integer at `path` (see [`LValue::get_path`] and [`LValue::as_i64`]).
    #[must_use]
    pub fn get_path_i64(&self, path: &str) -> Option<i64> {
        self.get_path(path).and_then(Self::as_i64)
    }

    /// The number at `path` (see [`LValue::get_path`]).
    #[must_use]
    pub fn get_path_f64(&self, path: &str) -> Option<f64> {
        self.get_path(path).and_then(Self::as_f64)
    }

    /// The string at `path` (see [`LValue::get_path`]).
    #[must_use]
    pub fn get_path_str(&self, path: &str) -> Option<&str> {
        self.get_path(path).and_then(Self::as_str)
    }

    /// The values of a table that is a Lua sequence (keys `1..=n`, positional or explicit, in any order), sorted by
    /// index. An empty table is an empty sequence; anything else is `None`.
    #[must_use]
    pub fn sequence(&self) -> Option<Vec<&Self>> {
        let mut indexed = Vec::new();
        let mut next_positional = 1_i64;
        for (key, value) in self.as_table()? {
            let index = match key {
                Key::Pos => {
                    let here = next_positional;
                    next_positional = next_positional.saturating_add(1);
                    here
                }
                Key::Int(int) => *int,
                _ => return None,
            };
            indexed.push((index, value));
        }
        indexed.sort_by_key(|(index, _)| *index);
        let dense = indexed.iter().zip(1_i64..).all(|((index, _), expected)| *index == expected);
        dense.then(|| indexed.into_iter().map(|(_, value)| value).collect())
    }

    // ---- editing --------------------------------------------------------------------------------------------------

    /// Sets the string key `key`: replaces the last entry with that key (returning the old value) or appends one.
    ///
    /// # Errors
    ///
    /// [`Error::NotATable`] if `self` is not a table.
    pub fn set(&mut self, key: &str, value: impl Into<Self>) -> Result<Option<Self>> {
        let entries = self.as_table_mut().ok_or_else(|| Error::NotATable { path: key.to_owned() })?;
        let value = value.into();
        if let Some((_, slot)) = entries.iter_mut().rev().find(|(k, _)| k.as_str() == Some(key)) {
            return Ok(Some(std::mem::replace(slot, value)));
        }
        entries.push((Key::Str(key.to_owned()), value));
        Ok(None)
    }

    /// Sets the value at a `/`-separated path, creating missing intermediate tables (see [`LValue::get_path`]).
    /// Returns the value that was replaced, if any.
    ///
    /// # Errors
    ///
    /// [`Error::NotATable`] if a segment along the path exists but is not a table, or `self` is not a table.
    pub fn set_path(&mut self, path: &str, value: impl Into<Self>) -> Result<Option<Self>> {
        let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
        let Some((last, parents)) = segments.split_last() else {
            return Err(Error::NotATable { path: path.to_owned() });
        };
        let mut current = self;
        for segment in parents {
            if current.get(segment).is_none() {
                current.set(segment, Self::empty_table())?;
            }
            current = current.get_mut(segment).ok_or_else(|| Error::NotATable { path: path.to_owned() })?;
            if current.as_table().is_none() {
                return Err(Error::NotATable { path: path.to_owned() });
            }
        }
        current.set(last, value)
    }

    /// Removes every entry with the string key `key` and returns the last one's value.
    pub fn remove(&mut self, key: &str) -> Option<Self> {
        let entries = self.as_table_mut()?;
        let mut removed = None;
        let mut index = 0;
        while let Some((k, _)) = entries.get(index) {
            if k.as_str() == Some(key) {
                removed = Some(entries.remove(index).1);
            } else {
                index = index.saturating_add(1);
            }
        }
        removed
    }

    /// Appends a positional (array-style) value.
    ///
    /// # Errors
    ///
    /// [`Error::NotATable`] if `self` is not a table.
    pub fn push(&mut self, value: impl Into<Self>) -> Result<()> {
        let entries = self.as_table_mut().ok_or_else(|| Error::NotATable { path: String::new() })?;
        entries.push((Key::Pos, value.into()));
        Ok(())
    }

    /// Sorts the keys of every table in the document the way Character Animator writes them (see
    /// [`Key::writer_order`]). The sort is stable, so positional entries and duplicate keys keep their order.
    pub fn sort_keys(&mut self) {
        if let Self::Table(entries) = self {
            entries.sort_by(|a, b| a.0.writer_order(&b.0));
            for (_, value) in entries.iter_mut() {
                value.sort_keys();
            }
        }
    }
}

impl<'a> IntoIterator for &'a LValue {
    type Item = &'a (Key, LValue);
    type IntoIter = std::slice::Iter<'a, (Key, LValue)>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_table().unwrap_or_default().iter()
    }
}

// ---- conversions --------------------------------------------------------------------------------------------------

impl From<bool> for LValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for LValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<i32> for LValue {
    fn from(value: i32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<u32> for LValue {
    fn from(value: u32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<f64> for LValue {
    fn from(value: f64) -> Self {
        Self::float(value)
    }
}

impl From<&str> for LValue {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<String> for LValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<Vec<(Key, Self)>> for LValue {
    fn from(entries: Vec<(Key, Self)>) -> Self {
        Self::Table(entries)
    }
}

/// Builds a table from `(key, value)` pairs: `[("rate", 30)].into_iter().collect::<LValue>()`.
impl<K: Into<Key>, V: Into<Self>> FromIterator<(K, V)> for LValue {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self::Table(iter.into_iter().map(|(key, value)| (key.into(), value.into())).collect())
    }
}

impl FromStr for LValue {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        crate::parse(text)
    }
}

/// The compact one-line form (`{rate=30,["a b"]="x"}`); see [`crate::to_string_pretty`] for the file form.
impl fmt::Display for LValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::to_string_compact(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LValue {
        crate::parse("{ a = 1, b = { c = 2.5, [3] = \"x\", \"p\", \"q\" }, [\"d e\"] = true, a = 4 }").unwrap()
    }

    #[test]
    fn later_duplicate_keys_win() {
        assert_eq!(sample().get("a"), Some(&LValue::Int(4)));
    }

    #[test]
    fn paths_reach_nested_values() {
        let v = sample();
        assert_eq!(v.get_path("b/c").and_then(LValue::as_f64), Some(2.5));
        assert_eq!(v.get_path_bool("d e"), Some(true));
        assert_eq!(v.get_path("b/1"), Some(&LValue::string("p")));
        assert_eq!(v.get_path("b/2"), Some(&LValue::string("q")));
        assert_eq!(v.get_path("b/3"), Some(&LValue::string("x")));
        assert_eq!(v.get_path("b/zzz"), None);
        assert_eq!(v.get_path("a/b"), None);
        assert_eq!(v.get_path(""), Some(&v));
        assert_eq!(v.get_in(&["b", "c"]), v.get_path("b/c"));
    }

    #[test]
    fn typed_accessors_convert_between_integers_and_floats() {
        assert_eq!(LValue::Int(3).as_f64(), Some(3.0));
        assert_eq!(LValue::from_number_text("5.0").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(LValue::from_number_text("1e3").and_then(|v| v.as_i64()), Some(1000));
        assert_eq!(LValue::from_number_text("0.5").and_then(|v| v.as_i64()), None);
        assert_eq!(LValue::from_number_text("1e30").and_then(|v| v.as_i64()), None);
        assert_eq!(LValue::from_number_text("inf").and_then(|v| v.as_i64()), None);
        assert_eq!(LValue::Bool(true).as_f64(), None);
        assert_eq!(LValue::string("x").as_bytes(), Some(&b"x"[..]));
        assert_eq!(LValue::Bytes(vec![0xff]).as_str(), None);
        assert_eq!(LValue::Bytes(vec![0xff]).as_bytes(), Some(&[0xff][..]));
        assert_eq!(LValue::Nil.type_name(), "nil");
        assert_eq!(LValue::empty_table().type_name(), "table");
    }

    #[test]
    fn float_constructor_prints_like_character_animator() {
        assert_eq!(LValue::float(100.0), LValue::Int(100));
        assert_eq!(LValue::float(0.2), LValue::Num { value: 0.2, text: "0.20000000000000001".to_owned() });
        assert_eq!(LValue::float(-0.0).to_string(), "-0");
        assert_eq!(LValue::float(1e20).to_string(), "1e+20");
        assert_eq!(LValue::float(f64::INFINITY).to_string(), "inf");
    }

    #[test]
    fn from_bytes_prefers_str() {
        assert_eq!(LValue::from_bytes("שלום".as_bytes().to_vec()), LValue::string("שלום"));
        assert_eq!(LValue::from_bytes(vec![0xe2, 0x82]), LValue::Bytes(vec![0xe2, 0x82]));
    }

    #[test]
    fn set_and_set_path_edit_in_place() {
        let mut v = sample();
        assert_eq!(v.set("a", 9).unwrap(), Some(LValue::Int(4)));
        assert_eq!(v.get("a"), Some(&LValue::Int(9)));
        assert_eq!(v.set("new", "s").unwrap(), None);
        assert_eq!(v.set_path("x/y/z", 1).unwrap(), None);
        assert_eq!(v.get_path_i64("x/y/z"), Some(1));
        assert_eq!(v.set_path("x/y/z", 2).unwrap(), Some(LValue::Int(1)));
        assert!(matches!(v.set_path("a/b", 1), Err(Error::NotATable { .. })));
        assert!(matches!(LValue::Int(1).set("k", 1), Err(Error::NotATable { .. })));
        assert!(matches!(v.set_path("", 1), Err(Error::NotATable { .. })));
    }

    #[test]
    fn remove_drops_every_entry_with_the_key() {
        let mut v = sample();
        assert_eq!(v.remove("a"), Some(LValue::Int(4)));
        assert_eq!(v.get("a"), None);
        assert_eq!(v.remove("a"), None);
        assert_eq!(LValue::Nil.remove("a"), None);
    }

    #[test]
    fn push_appends_positional_values() {
        let mut v = LValue::empty_table();
        v.push(1).unwrap();
        v.push("two").unwrap();
        assert_eq!(v.to_string(), "{1,\"two\"}");
        assert_eq!(v.sequence(), Some(vec![&LValue::Int(1), &LValue::string("two")]));
        assert!(LValue::Nil.push(1).is_err());
    }

    #[test]
    fn sequences_need_dense_indices() {
        assert_eq!(crate::parse("{}").unwrap().sequence(), Some(vec![]));
        assert_eq!(crate::parse("{[2]=\"b\",[1]=\"a\"}").unwrap().sequence().map(|s| s.len()), Some(2));
        assert_eq!(crate::parse("{[1]=\"a\",[3]=\"c\"}").unwrap().sequence(), None);
        assert_eq!(crate::parse("{a=1}").unwrap().sequence(), None);
        assert_eq!(crate::parse("{[1]=\"a\",\"b\"}").unwrap().sequence(), None);
        assert_eq!(LValue::Int(1).sequence(), None);
    }

    #[test]
    fn sort_keys_follows_the_writer_order() {
        let mut v = crate::parse("{ faceTracker = 1, Trim_Ah = 2, [\"b\"] = 3, 10, [5] = 4, 20, [true] = 5, [1.5] = 6, a = { z = 1, y = 2 } }").unwrap();
        v.sort_keys();
        let order: Vec<String> = v.iter().map(|(k, _)| format!("{k:?}")).collect();
        assert_eq!(
            order,
            ["Pos", "Pos", "Int(5)", "Num { value: 1.5, text: \"1.5\" }", "Bool(true)", "Str(\"Trim_Ah\")", "Str(\"a\")", "Str(\"b\")", "Str(\"faceTracker\")"]
        );
        assert_eq!(v.get_path("a").unwrap().to_string(), "{y=2,z=1}");
        assert_eq!(v.get_index(2), Some(&LValue::Int(20)));
    }

    #[test]
    fn collecting_pairs_builds_a_table() {
        let v: LValue = [("rate", 30), ("length", 80)].into_iter().collect();
        assert_eq!(v.to_string(), "{rate=30,length=80}");
        assert_eq!(v.named().map(|(k, _)| k).collect::<Vec<_>>(), ["rate", "length"]);
        assert_eq!((&v).into_iter().count(), 2);
    }

    #[test]
    fn equality_is_structural() {
        let a = LValue::from_number_text("0.2").unwrap();
        let b = LValue::from_number_text("0.20000000000000001").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.as_f64(), b.as_f64());
        let nan = LValue::from_number_text("nan").unwrap();
        assert_eq!(nan, nan.clone());
    }

    #[test]
    fn from_str_parses() {
        assert_eq!("{a=1}".parse::<LValue>().unwrap().get_path_i64("a"), Some(1));
        assert!("{a=".parse::<LValue>().is_err());
    }
}
