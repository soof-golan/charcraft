//! The printer: Character Animator's layout (tabs, `key = value,` lines, every entry comma-terminated).
//!
//! ```text
//! {
//! <TAB>iVersion = 1,
//! <TAB>tBehaviorDefaults = {
//! <TAB><TAB>["Adobe.Blinker.js"] = {
//! <TAB><TAB><TAB>rate = 30,
//! <TAB><TAB>},
//! <TAB>},
//! }
//! ```

use std::fmt::Write as _;

use crate::number::{format_g17, parse_number_text};
use crate::value::{Key, LValue};

/// Lua's reserved words; a string key with one of these names is written `["end"] = ...` so the text stays valid Lua.
const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
    "true", "until", "while",
];

/// Line ending used by [`to_string_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Newline {
    /// `\n` (what the Mac files we have seen use).
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
}

impl Newline {
    /// The line ending as text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }

    /// The line ending the first line break of `text` uses ([`Newline::Lf`] if there is none), so a file read from
    /// disk can be written back with the same endings.
    #[must_use]
    pub fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(index) if index.checked_sub(1).and_then(|before| text.as_bytes().get(before)) == Some(&b'\r') => Self::CrLf,
            _ => Self::Lf,
        }
    }
}

/// Options for [`to_string_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrettyOptions {
    /// Line ending.
    pub newline: Newline,
}

/// Prints `value` in Character Animator's file layout.
///
/// One tab per nesting level, one `key = value,` line per entry (the last one comma-terminated too), nested tables
/// opened with `{` on the key's line, and a final newline. A table without entries prints as `{` newline `}` at the
/// table's indentation. Entries are written in the order they are stored (see [`LValue::sort_keys`]).
///
/// Reading a file Character Animator wrote and printing it again reproduces it byte for byte, because numbers keep
/// their source text.
#[must_use]
pub fn to_string_pretty(value: &LValue) -> String {
    to_string_with(value, &PrettyOptions::default())
}

/// [`to_string_pretty`] with explicit [`PrettyOptions`].
#[must_use]
pub fn to_string_with(value: &LValue, options: &PrettyOptions) -> String {
    let newline = options.newline.as_str();
    let mut writer = Writer { out: String::new(), newline: Some(newline) };
    writer.value(value, 0);
    writer.out.push_str(newline);
    writer.out
}

/// Prints `edited` the way `original` was written.
///
/// If `original` already holds exactly `edited`, it comes back byte for byte, whatever its layout, comments or
/// escapes were (so files nobody changed are never rewritten differently, even where our idea of Character
/// Animator's layout is wrong). Otherwise `edited` is printed with [`to_string_with`], using the line ending that
/// `original` uses.
#[must_use]
pub fn reprint(original: &str, edited: &LValue) -> String {
    if crate::parse(original).is_ok_and(|current| current == *edited) {
        return original.to_owned();
    }
    to_string_with(edited, &PrettyOptions { newline: Newline::detect(original) })
}

/// Prints `value` on one line without any optional whitespace: `{rate=30,["a b"]="x",1}`.
#[must_use]
pub fn to_string_compact(value: &LValue) -> String {
    let mut writer = Writer { out: String::new(), newline: None };
    writer.value(value, 0);
    writer.out
}

struct Writer {
    out: String,
    /// The line ending in pretty mode, `None` for the compact form.
    newline: Option<&'static str>,
}

impl Writer {
    fn value(&mut self, value: &LValue, depth: usize) {
        match value {
            LValue::Nil => self.out.push_str("nil"),
            LValue::Bool(true) => self.out.push_str("true"),
            LValue::Bool(false) => self.out.push_str("false"),
            LValue::Int(int) => self.out.push_str(&int.to_string()),
            LValue::Num { value, text } => self.number(*value, text),
            LValue::Str(text) => self.string(text),
            LValue::Bytes(bytes) => self.byte_string(bytes),
            LValue::Table(entries) => self.table(entries, depth),
        }
    }

    fn number(&mut self, value: f64, text: &str) {
        // A hand-built `Num` with unusable text must still print something the parser reads back.
        if parse_number_text(text).is_some() {
            self.out.push_str(text);
        } else {
            self.out.push_str(&format_g17(value));
        }
    }

    fn table(&mut self, entries: &[(Key, LValue)], depth: usize) {
        self.out.push('{');
        let inner = depth.saturating_add(1);
        if let Some(newline) = self.newline {
            self.out.push_str(newline);
            for (key, value) in entries {
                self.indent(inner);
                if self.key(key) {
                    self.out.push_str(" = ");
                }
                self.value(value, inner);
                self.out.push(',');
                self.out.push_str(newline);
            }
            self.indent(depth);
        } else {
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    self.out.push(',');
                }
                if self.key(key) {
                    self.out.push('=');
                }
                self.value(value, inner);
            }
        }
        self.out.push('}');
    }

    fn indent(&mut self, depth: usize) {
        self.out.extend(std::iter::repeat_n('\t', depth));
    }

    /// Writes the key part of an entry; returns whether a `=` and a value follow.
    fn key(&mut self, key: &Key) -> bool {
        match key {
            Key::Pos => return false,
            Key::Str(name) if is_plain_name(name) => self.out.push_str(name),
            Key::Str(name) => {
                self.out.push('[');
                self.string(name);
                self.out.push(']');
            }
            Key::Int(int) => {
                self.out.push('[');
                self.out.push_str(&int.to_string());
                self.out.push(']');
            }
            Key::Num { value, text } => {
                self.out.push('[');
                self.number(*value, text);
                self.out.push(']');
            }
            Key::Bool(flag) => self.out.push_str(if *flag { "[true]" } else { "[false]" }),
        }
        true
    }

    fn string(&mut self, text: &str) {
        self.out.push('"');
        escape_into(&mut self.out, text);
        self.out.push('"');
    }

    fn byte_string(&mut self, bytes: &[u8]) {
        self.out.push('"');
        for chunk in bytes.utf8_chunks() {
            escape_into(&mut self.out, chunk.valid());
            for &byte in chunk.invalid() {
                push_decimal_escape(&mut self.out, byte);
            }
        }
        self.out.push('"');
    }
}

/// True for names written without brackets: ASCII `[A-Za-z_][A-Za-z0-9_]*` that is not a reserved word.
fn is_plain_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_') && !KEYWORDS.contains(&name)
}

/// Escapes the characters that cannot appear raw inside `"..."`. Control characters become 3-digit decimal escapes
/// (`\001`), which never merge with a following digit. Everything else, including every non-ASCII character, is
/// written as is, so Hebrew, Arabic and bidi control characters pass through untouched.
fn escape_into(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_ascii_control() => push_decimal_escape(out, u8::try_from(control).unwrap_or_default()),
            other => out.push(other),
        }
    }
}

/// `\ddd` with exactly three decimal digits, so the escape never merges with a digit that follows it.
fn push_decimal_escape(out: &mut String, byte: u8) {
    // Writing to a `String` cannot fail.
    let _ = write!(out, "\\{byte:03}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// A hand-written document in Character Animator's layout (invented values, real conventions).
    const DOCUMENT: &str = "{\n\
\tbFlag = false,\n\
\tiVersion = 1,\n\
\tsName = \"/ProjectItems/A puppet\",\n\
\ttDefaults = {\n\
\t\t[\"Vendor.Blink.js\"] = {\n\
\t\t\tlength = 80,\n\
\t\t\trate = 30,\n\
\t\t},\n\
\t\t[\"Vendor.Drag.js\"] = {\n\
\t\t\thandleReleaseDuration = 0.20000000000000001,\n\
\t\t\tleftRightScaleFactor = 0.69999999999999996,\n\
\t\t\tminInterval = 0.10000000000000001,\n\
\t\t\ttilt = -20,\n\
\t\t},\n\
\t},\n\
}\n";

    #[test]
    fn character_animator_layout_round_trips_byte_exact() {
        let value = parse(DOCUMENT).unwrap();
        assert_eq!(to_string_pretty(&value), DOCUMENT);
    }

    #[test]
    fn nested_empty_positional_and_bracketed_entries() {
        let text = "{\n\tempty = {\n\t},\n\tlist = {\n\t\t1,\n\t\t\"two\",\n\t\t{\n\t\t\tx = 3,\n\t\t},\n\t},\n\t[1] = true,\n\t[2.5] = nil,\n\t[false] = 0,\n\t[\"end\"] = 1,\n\t[\"\"] = 2,\n\t[\"9lives\"] = 3,\n}\n";
        let value = parse(text).unwrap();
        assert_eq!(to_string_pretty(&value), text);
    }

    #[test]
    fn non_canonical_input_is_normalised() {
        let value = parse("-- note\n{ b = 2; a = 'x', [\"plain\"] = {}, 0x10 }").unwrap();
        assert_eq!(to_string_pretty(&value), "{\n\tb = 2,\n\ta = \"x\",\n\tplain = {\n\t},\n\t16,\n}\n");
    }

    #[test]
    fn crlf_output_and_detection() {
        let value = parse("{ a = { b = 1 } }").unwrap();
        let crlf = to_string_with(&value, &PrettyOptions { newline: Newline::CrLf });
        assert_eq!(crlf, "{\r\n\ta = {\r\n\t\tb = 1,\r\n\t},\r\n}\r\n");
        assert_eq!(Newline::detect(&crlf), Newline::CrLf);
        assert_eq!(Newline::detect("a\nb\r\n"), Newline::Lf);
        assert_eq!(Newline::detect("\n"), Newline::Lf);
        assert_eq!(Newline::detect("no breaks"), Newline::Lf);
        assert_eq!(parse(&crlf).unwrap(), value);
    }

    #[test]
    fn reprint_keeps_untouched_documents_byte_exact_and_matches_line_endings() {
        let odd = "-- a comment\r\n{ b = 'x';  a = {1, 2,}, ['k'] = \"\\65\" }\r\n";
        let mut value = parse(odd).unwrap();
        assert_eq!(reprint(odd, &value), odd);
        value.set_path("a/new", 5).unwrap();
        let edited = reprint(odd, &value);
        assert_eq!(edited, "{\r\n\tb = \"x\",\r\n\ta = {\r\n\t\t1,\r\n\t\t2,\r\n\t\tnew = 5,\r\n\t},\r\n\tk = \"A\",\r\n}\r\n");
        assert_eq!(parse(&edited).unwrap(), value);
        assert_eq!(reprint("not ltab {{{", &LValue::Int(1)), "1\n");
    }

    #[test]
    fn strings_are_escaped_and_non_ascii_is_raw() {
        let value = LValue::string("a\"b\\c\nd\re\tf\u{1}g\u{7f}1 שלום مرحبا \u{200f}");
        assert_eq!(to_string_compact(&value), "\"a\\\"b\\\\c\\nd\\re\\tf\\001g\\1271 שלום مرحبا \u{200f}\"");
        assert_eq!(parse(&to_string_compact(&value)).unwrap(), value);
    }

    #[test]
    fn non_utf8_strings_use_decimal_escapes() {
        let value = LValue::Bytes(vec![b'a', 0xff, 0xe2, 0x82, b'z']);
        assert_eq!(to_string_compact(&value), "\"a\\255\\226\\130z\"");
        assert_eq!(parse(&to_string_compact(&value)).unwrap(), value);
    }

    #[test]
    fn compact_form() {
        let value = parse("{ rate = 30, [\"a b\"] = \"x\", 1, { }, [3] = nil }").unwrap();
        assert_eq!(to_string_compact(&value), "{rate=30,[\"a b\"]=\"x\",1,{},[3]=nil}");
        assert_eq!(value.to_string(), to_string_compact(&value));
    }

    #[test]
    fn numbers_print_their_source_text() {
        let text = "{\n\ta = 0.20000000000000001,\n\tb = 1e+20,\n\tc = -0,\n\td = 100,\n\te = inf,\n\tf = -nan,\n}\n";
        assert_eq!(to_string_pretty(&parse(text).unwrap()), text);
    }

    #[test]
    fn unusable_number_text_falls_back_to_canonical_printing() {
        let broken = LValue::Num { value: 0.5, text: "not a number".to_owned() };
        assert_eq!(to_string_compact(&broken), "0.5");
        assert_eq!(to_string_compact(&LValue::Num { value: 2.0, text: String::new() }), "2");
    }

    #[test]
    fn top_level_scalars_print_with_a_newline() {
        assert_eq!(to_string_pretty(&LValue::Int(5)), "5\n");
        assert_eq!(to_string_pretty(&LValue::string("x")), "\"x\"\n");
        assert_eq!(to_string_pretty(&LValue::empty_table()), "{\n}\n");
    }

    #[test]
    fn plain_names() {
        for good in ["a", "_", "Trim_Ah", "x9", "tBehaviorDefaults"] {
            assert!(is_plain_name(good), "{good}");
        }
        for bad in ["", "9a", "a b", "a.b", "end", "nil", "true", "שלום", "a-b"] {
            assert!(!is_plain_name(bad), "{bad}");
        }
    }
}
