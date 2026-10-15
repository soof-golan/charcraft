//! The parser: Lua table constructor syntax over bytes, bounded in depth, size and entry count.

use crate::error::{Error, Result, SyntaxError};
use crate::number::{Number, parse_number_text};
use crate::value::{Key, LValue};

/// Nesting is never allowed beyond this, whatever [`Limits::max_depth`] says, so the recursive parser cannot
/// overflow the stack.
pub const HARD_MAX_DEPTH: usize = 1024;

/// Caps that keep hostile input from exhausting memory or the stack.
///
/// The defaults are far above anything Character Animator writes (its metadata nests a dozen levels and holds a few
/// thousand entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Deepest allowed table nesting; a top-level table has depth 1. Clamped to [`HARD_MAX_DEPTH`].
    pub max_depth: usize,
    /// Largest accepted input in bytes.
    pub max_input_bytes: usize,
    /// Most table entries (counted over the whole document) before the parser gives up.
    pub max_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_depth: 128, max_input_bytes: 33_554_432, max_entries: 1_000_000 }
    }
}

/// Parses one ltab value (normally a table) from `input` with the default [`Limits`].
///
/// Accepts the Lua table-constructor subset Character Animator writes and, for tolerance, the rest of Lua's data
/// syntax: `--` and `--[[ ]]` comments, single-quoted and long (`[[ ]]`) strings, every Lua escape, `;` as a
/// separator, hexadecimal integers, positional values, `[expr] = value` keys, an optional leading `return`, a UTF-8
/// byte-order mark, and the words `inf` and `nan`. Unknown escapes (`\q`) stand for the character itself, as in
/// Lua 5.1, and a quoted string may contain raw newlines.
///
/// # Errors
///
/// [`Error::Syntax`] with the line and column of the first problem, or a limit error for oversized input.
pub fn parse(input: &str) -> Result<LValue> {
    parse_with(input, &Limits::default())
}

/// [`parse`] with explicit [`Limits`].
///
/// # Errors
///
/// As [`parse`].
pub fn parse_with(input: &str, limits: &Limits) -> Result<LValue> {
    parse_bytes_with(input.as_bytes(), limits)
}

/// Parses ltab from raw bytes (for files that may not be UTF-8). String contents that are not valid UTF-8 become
/// [`LValue::Bytes`]; names and keys must be valid UTF-8.
///
/// # Errors
///
/// As [`parse`].
pub fn parse_bytes(input: &[u8]) -> Result<LValue> {
    parse_bytes_with(input, &Limits::default())
}

/// [`parse_bytes`] with explicit [`Limits`].
///
/// # Errors
///
/// As [`parse`].
pub fn parse_bytes_with(input: &[u8], limits: &Limits) -> Result<LValue> {
    if input.len() > limits.max_input_bytes {
        return Err(Error::InputTooLarge { len: input.len(), limit: limits.max_input_bytes });
    }
    let mut parser = Parser { src: input, pos: 0, max_depth: limits.max_depth.min(HARD_MAX_DEPTH), max_entries: limits.max_entries, entries: 0 };
    parser.document()
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    max_depth: usize,
    max_entries: usize,
    entries: usize,
}

const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Identifier bytes: ASCII letters, digits and `_`, plus every byte of a multi-byte UTF-8 character (as `LuaJIT` does).
const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

const fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

/// The words that are numbers: `inf` and `nan` in any case (C's `%g` prints non-finite values that way).
fn number_word(word: &[u8]) -> Option<Number> {
    std::str::from_utf8(word).ok().and_then(parse_number_text)
}

impl<'a> Parser<'a> {
    // ---- cursor ---------------------------------------------------------------------------------------------------

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos.saturating_add(offset)).copied()
    }

    const fn bump(&mut self) {
        self.pos = self.pos.saturating_add(1);
    }

    const fn advance(&mut self, count: usize) {
        self.pos = self.pos.saturating_add(count);
    }

    fn starts_with(&self, pattern: &[u8]) -> bool {
        self.src.get(self.pos..).is_some_and(|rest| rest.starts_with(pattern))
    }

    // ---- errors ---------------------------------------------------------------------------------------------------

    /// 1-based line and column (in characters) of the current position.
    fn locate(&self) -> (usize, usize) {
        let before = self.src.get(..self.pos.min(self.src.len())).unwrap_or_default();
        let line = before.split(|&b| b == b'\n').count();
        let current_line = before.rsplit(|&b| b == b'\n').next().unwrap_or_default();
        let column = String::from_utf8_lossy(current_line).chars().count().saturating_add(1);
        (line, column)
    }

    fn syntax(&self, kind: SyntaxError) -> Error {
        let (line, column) = self.locate();
        Error::Syntax { line, column, kind }
    }

    fn syntax_at(&mut self, pos: usize, kind: SyntaxError) -> Error {
        self.pos = pos;
        self.syntax(kind)
    }

    fn current_char(&self) -> char {
        let rest = self.src.get(self.pos..).unwrap_or_default();
        let head = rest.get(..rest.len().min(4)).unwrap_or_default();
        String::from_utf8_lossy(head).chars().next().unwrap_or(char::REPLACEMENT_CHARACTER)
    }

    fn unexpected(&self, expected: &'static str) -> Error {
        if self.peek().is_none() {
            self.syntax(SyntaxError::UnexpectedEof { expected })
        } else {
            self.syntax(SyntaxError::Unexpected { found: self.current_char(), expected })
        }
    }

    fn expect(&mut self, byte: u8, expected: &'static str) -> Result<()> {
        if self.peek() == Some(byte) {
            self.bump();
            Ok(())
        } else {
            Err(self.unexpected(expected))
        }
    }

    // ---- document -------------------------------------------------------------------------------------------------

    fn document(&mut self) -> Result<LValue> {
        if self.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.advance(3);
        }
        self.skip_trivia()?;
        if self.at_keyword(b"return") {
            self.advance(6);
        }
        let value = self.value(0)?;
        self.skip_trivia()?;
        if self.peek() == Some(b';') {
            self.bump();
            self.skip_trivia()?;
        }
        if self.peek().is_some() {
            return Err(self.syntax(SyntaxError::TrailingData));
        }
        Ok(value)
    }

    fn at_keyword(&self, keyword: &[u8]) -> bool {
        self.starts_with(keyword) && !self.peek_at(keyword.len()).is_some_and(is_ident_continue)
    }

    /// Skips whitespace and comments.
    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            match self.peek() {
                Some(byte) if is_space(byte) => self.bump(),
                Some(b'-') if self.peek_at(1) == Some(b'-') => {
                    self.advance(2);
                    self.skip_comment_body()?;
                }
                _ => return Ok(()),
            }
        }
    }

    fn skip_comment_body(&mut self) -> Result<()> {
        if let Some(level) = self.long_bracket_level() {
            self.long_bracket_body(level)?;
        } else {
            while self.peek().is_some_and(|b| b != b'\n' && b != b'\r') {
                self.bump();
            }
        }
        Ok(())
    }

    // ---- values ---------------------------------------------------------------------------------------------------

    fn value(&mut self, depth: usize) -> Result<LValue> {
        self.skip_trivia()?;
        match self.peek() {
            Some(b'{') => self.table(depth),
            Some(quote @ (b'"' | b'\'')) => Ok(LValue::from_bytes(self.quoted(quote)?)),
            Some(b'[') => match self.long_bracket_level() {
                Some(level) => Ok(LValue::from_bytes(self.long_bracket_body(level)?)),
                None => Err(self.unexpected("a value")),
            },
            Some(b'-' | b'.' | b'0'..=b'9') => Ok(self.number()?.into()),
            Some(byte) if is_ident_start(byte) => self.word_value(),
            _ => Err(self.unexpected("a value")),
        }
    }

    fn word_value(&mut self) -> Result<LValue> {
        let start = self.pos;
        let word = self.ident_bytes();
        match word {
            b"true" => Ok(LValue::Bool(true)),
            b"false" => Ok(LValue::Bool(false)),
            b"nil" => Ok(LValue::Nil),
            _ => {
                if let Some(number) = number_word(word) {
                    return Ok(number.into());
                }
                Err(self.syntax_at(start, SyntaxError::UnknownIdentifier(String::from_utf8_lossy(word).into_owned())))
            }
        }
    }

    fn ident_bytes(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_continue) {
            self.bump();
        }
        self.src.get(start..self.pos).unwrap_or_default()
    }

    // ---- tables ---------------------------------------------------------------------------------------------------

    fn table(&mut self, depth: usize) -> Result<LValue> {
        if depth >= self.max_depth {
            let (line, column) = self.locate();
            return Err(Error::TooDeep { line, column, limit: self.max_depth });
        }
        self.bump();
        let mut entries = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                Some(b'}') => {
                    self.bump();
                    return Ok(LValue::Table(entries));
                }
                None => return Err(self.unexpected("`}`")),
                Some(_) => {}
            }
            self.entries = self.entries.saturating_add(1);
            if self.entries > self.max_entries {
                return Err(Error::TooManyEntries { limit: self.max_entries });
            }
            entries.push(self.field(depth.saturating_add(1))?);
            self.skip_trivia()?;
            match self.peek() {
                Some(b',' | b';') => self.bump(),
                Some(b'}') => {}
                _ => return Err(self.unexpected("`,`, `;` or `}`")),
            }
        }
    }

    /// One table field; `depth` is the depth its value would have if it is itself a table.
    fn field(&mut self, depth: usize) -> Result<(Key, LValue)> {
        match self.peek() {
            Some(b'[') if self.long_bracket_level().is_none() => {
                self.bump();
                let key = self.key_expression()?;
                self.skip_trivia()?;
                self.expect(b']', "`]`")?;
                self.skip_trivia()?;
                self.expect(b'=', "`=`")?;
                Ok((key, self.value(depth)?))
            }
            Some(byte) if is_ident_start(byte) => {
                let start = self.pos;
                let name = self.ident_bytes();
                self.skip_trivia()?;
                if self.peek() == Some(b'=') && self.peek_at(1) != Some(b'=') {
                    self.bump();
                    let name = String::from_utf8(name.to_vec()).map_err(|_| self.syntax_at(start, SyntaxError::InvalidUtf8("table key")))?;
                    return Ok((Key::Str(name), self.value(depth)?));
                }
                self.pos = start;
                Ok((Key::Pos, self.value(depth)?))
            }
            _ => Ok((Key::Pos, self.value(depth)?)),
        }
    }

    /// The expression inside `[ ... ]`: a string, a number or a boolean.
    fn key_expression(&mut self) -> Result<Key> {
        self.skip_trivia()?;
        let start = self.pos;
        match self.peek() {
            Some(quote @ (b'"' | b'\'')) => {
                let bytes = self.quoted(quote)?;
                String::from_utf8(bytes).map(Key::Str).map_err(|_| self.syntax_at(start, SyntaxError::InvalidUtf8("table key")))
            }
            Some(b'[') => match self.long_bracket_level() {
                Some(level) => {
                    let bytes = self.long_bracket_body(level)?;
                    String::from_utf8(bytes).map(Key::Str).map_err(|_| self.syntax_at(start, SyntaxError::InvalidUtf8("table key")))
                }
                None => Err(self.unexpected("a string, number or boolean key")),
            },
            Some(b'-' | b'.' | b'0'..=b'9') => Ok(self.number()?.into()),
            Some(byte) if is_ident_start(byte) => {
                let word = self.ident_bytes();
                match word {
                    b"true" => Ok(Key::Bool(true)),
                    b"false" => Ok(Key::Bool(false)),
                    b"nil" => Err(self.syntax_at(start, SyntaxError::BadKey("nil cannot be a key"))),
                    _ => {
                        if let Some(number) = number_word(word) {
                            return Ok(number.into());
                        }
                        Err(self.syntax_at(start, SyntaxError::BadKey("a bare word other than true, false, inf or nan")))
                    }
                }
            }
            _ => Err(self.unexpected("a string, number or boolean key")),
        }
    }

    // ---- numbers --------------------------------------------------------------------------------------------------

    /// A number literal with an optional leading `-`. Reads the whole run of letters, digits and `.` (and a signed
    /// exponent) so `5abc` is one bad token, not `5` followed by garbage.
    fn number(&mut self) -> Result<Number> {
        let start = self.pos;
        let mut text = String::new();
        if self.peek() == Some(b'-') {
            self.bump();
            self.skip_trivia()?;
            text.push('-');
        }
        let token_start = self.pos;
        let hexadecimal = self.starts_with(b"0x") || self.starts_with(b"0X");
        while let Some(byte) = self.peek() {
            let exponent_sign = matches!(byte, b'+' | b'-')
                && self.pos > token_start
                && self
                    .src
                    .get(self.pos.saturating_sub(1))
                    .is_some_and(|&prev| if hexadecimal { matches!(prev, b'p' | b'P') } else { matches!(prev, b'e' | b'E') });
            if byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || exponent_sign {
                self.bump();
            } else {
                break;
            }
        }
        text.push_str(&String::from_utf8_lossy(self.src.get(token_start..self.pos).unwrap_or_default()));
        parse_number_text(&text).ok_or_else(|| self.syntax_at(start, SyntaxError::BadNumber(text)))
    }

    // ---- strings --------------------------------------------------------------------------------------------------

    /// A `"..."` or `'...'` string, returning its bytes after escape processing.
    fn quoted(&mut self, quote: u8) -> Result<Vec<u8>> {
        let start = self.pos;
        self.bump();
        let mut out = Vec::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.syntax_at(start, SyntaxError::UnterminatedString));
            };
            self.bump();
            if byte == quote {
                return Ok(out);
            }
            if byte == b'\\' {
                if self.peek().is_none() {
                    return Err(self.syntax_at(start, SyntaxError::UnterminatedString));
                }
                self.escape(&mut out)?;
            } else {
                out.push(byte);
            }
        }
    }

    /// The escape after a backslash that has just been consumed.
    fn escape(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let Some(kind) = self.peek() else {
            return Ok(());
        };
        self.bump();
        match kind {
            b'a' => out.push(0x07),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(0x0b),
            b'\n' | b'\r' => {
                // A backslash before a line break is that line break; CRLF and LFCR count as one.
                if matches!((kind, self.peek()), (b'\n', Some(b'\r')) | (b'\r', Some(b'\n'))) {
                    self.bump();
                }
                out.push(b'\n');
            }
            b'x' => {
                let high = self.hex_digit()?;
                let low = self.hex_digit()?;
                out.push(high.saturating_mul(16).saturating_add(low));
            }
            b'z' => {
                while self.peek().is_some_and(is_space) {
                    self.bump();
                }
            }
            b'u' => self.unicode_escape(out)?,
            b'0'..=b'9' => self.decimal_escape(kind, out)?,
            // `\\`, `\"`, `\'` and (as in Lua 5.1) any other character: the character itself.
            other => out.push(other),
        }
        Ok(())
    }

    fn hex_digit(&mut self) -> Result<u8> {
        let digit = self.peek().and_then(|b| char::from(b).to_digit(16)).and_then(|d| u8::try_from(d).ok());
        match digit {
            Some(digit) => {
                self.bump();
                Ok(digit)
            }
            None => Err(self.syntax(SyntaxError::BadEscape("expected a hexadecimal digit"))),
        }
    }

    fn decimal_escape(&mut self, first: u8, out: &mut Vec<u8>) -> Result<()> {
        let mut value = u32::from(first.saturating_sub(b'0'));
        for _ in 0..2 {
            let Some(digit) = self.peek().and_then(|b| char::from(b).to_digit(10)) else {
                break;
            };
            self.bump();
            value = value.saturating_mul(10).saturating_add(digit);
        }
        out.push(u8::try_from(value).map_err(|_| self.syntax(SyntaxError::BadEscape("decimal escape above 255")))?);
        Ok(())
    }

    fn unicode_escape(&mut self, out: &mut Vec<u8>) -> Result<()> {
        self.expect(b'{', "`{` after `\\u`")?;
        let mut code = 0_u32;
        let mut digits = 0_usize;
        while let Some(digit) = self.peek().and_then(|b| char::from(b).to_digit(16)) {
            self.bump();
            digits = digits.saturating_add(1);
            code = code.saturating_mul(16).saturating_add(digit);
        }
        self.expect(b'}', "`}` closing the unicode escape")?;
        let character = if (1..=8).contains(&digits) { char::from_u32(code) } else { None };
        let character = character.ok_or_else(|| self.syntax(SyntaxError::BadEscape("unicode escape is not a valid code point")))?;
        let mut buffer = [0_u8; 4];
        out.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        Ok(())
    }

    // ---- long brackets --------------------------------------------------------------------------------------------

    /// If the cursor is at `[`, `=`*n, `[`, returns n without consuming anything.
    fn long_bracket_level(&self) -> Option<usize> {
        if self.peek() != Some(b'[') {
            return None;
        }
        let mut level = 0_usize;
        loop {
            match self.peek_at(level.saturating_add(1)) {
                Some(b'=') => level = level.saturating_add(1),
                Some(b'[') => return Some(level),
                _ => return None,
            }
        }
    }

    /// Consumes a long string or comment of the given level and returns its content (first newline dropped, line
    /// breaks normalised to `\n`, as Lua does).
    fn long_bracket_body(&mut self, level: usize) -> Result<Vec<u8>> {
        let start = self.pos;
        self.advance(level.saturating_add(2));
        if let Some(first @ (b'\n' | b'\r')) = self.peek() {
            self.bump();
            if self.peek().is_some_and(|second| (second == b'\n' || second == b'\r') && second != first) {
                self.bump();
            }
        }
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.syntax_at(start, SyntaxError::UnterminatedLongBracket)),
                Some(b']') if self.at_long_close(level) => {
                    self.advance(level.saturating_add(2));
                    return Ok(out);
                }
                Some(first @ (b'\n' | b'\r')) => {
                    self.bump();
                    if self.peek().is_some_and(|second| (second == b'\n' || second == b'\r') && second != first) {
                        self.bump();
                    }
                    out.push(b'\n');
                }
                Some(byte) => {
                    out.push(byte);
                    self.bump();
                }
            }
        }
    }

    fn at_long_close(&self, level: usize) -> bool {
        (1..=level).all(|offset| self.peek_at(offset) == Some(b'=')) && self.peek_at(level.saturating_add(1)) == Some(b']')
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::arithmetic_side_effects)]

    use super::*;

    fn p(text: &str) -> LValue {
        parse(text).unwrap()
    }

    fn syntax_kind(text: &str) -> SyntaxError {
        match parse(text) {
            Err(Error::Syntax { kind, .. }) => kind,
            other => panic!("expected a syntax error for {text:?}, got {other:?}"),
        }
    }

    #[test]
    fn scalars_parse_at_top_level() {
        assert_eq!(p("nil"), LValue::Nil);
        assert_eq!(p("true"), LValue::Bool(true));
        assert_eq!(p(" false \n"), LValue::Bool(false));
        assert_eq!(p("42"), LValue::Int(42));
        assert_eq!(p("-7"), LValue::Int(-7));
        assert_eq!(p("- 7"), LValue::Int(-7));
        assert_eq!(p("\"hi\""), LValue::string("hi"));
        assert_eq!(p("'hi'"), LValue::string("hi"));
        assert_eq!(p("{}"), LValue::empty_table());
    }

    #[test]
    fn numbers_keep_their_text() {
        let v = p("{ a = 0.20000000000000001, b = -1.5, c = 1e+20, d = 1E5, e = .5, f = 5., g = -0, h = 0x1F, i = inf, j = -nan }");
        assert_eq!(v.get("a"), Some(&LValue::Num { value: 0.2, text: "0.20000000000000001".to_owned() }));
        assert_eq!(v.get("b"), Some(&LValue::Num { value: -1.5, text: "-1.5".to_owned() }));
        assert_eq!(v.get("c"), Some(&LValue::Num { value: 1e20, text: "1e+20".to_owned() }));
        assert_eq!(v.get("d"), Some(&LValue::Num { value: 1e5, text: "1E5".to_owned() }));
        assert_eq!(v.get("e"), Some(&LValue::Num { value: 0.5, text: ".5".to_owned() }));
        assert_eq!(v.get("f"), Some(&LValue::Num { value: 5.0, text: "5.".to_owned() }));
        assert!(matches!(v.get("g"), Some(LValue::Num { value, text }) if value.is_sign_negative() && text == "-0"));
        assert_eq!(v.get("h"), Some(&LValue::Int(31)));
        assert_eq!(v.get("i").and_then(LValue::as_f64), Some(f64::INFINITY));
        assert!(v.get("j").and_then(LValue::as_f64).is_some_and(f64::is_nan));
    }

    #[test]
    fn integers_beyond_i64_become_floats() {
        assert_eq!(p("9223372036854775807"), LValue::Int(i64::MAX));
        assert_eq!(p("-9223372036854775808"), LValue::Int(i64::MIN));
        assert!(matches!(p("9223372036854775808"), LValue::Num { .. }));
    }

    #[test]
    fn keys_in_every_form() {
        let v = p("{ name = 1, [\"two words\"] = 2, ['single'] = 3, [7] = 4, [1.5] = 5, [true] = 6, [ [[long]] ] = 7, 8, 9 }");
        let keys: Vec<&Key> = v.iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            [
                &Key::Str("name".into()),
                &Key::Str("two words".into()),
                &Key::Str("single".into()),
                &Key::Int(7),
                &Key::Num { value: 1.5, text: "1.5".into() },
                &Key::Bool(true),
                &Key::Str("long".into()),
                &Key::Pos,
                &Key::Pos
            ]
        );
    }

    #[test]
    fn separators_whitespace_and_comments_are_tolerated() {
        let v = p("-- header\n{ a = 1; b = 2, --[[ inline ]] c = 3,\n\t-- trailing\n}");
        assert_eq!(v.iter().count(), 3);
        assert_eq!(p("{1,2,3,}").iter().count(), 3);
        assert!(parse("{,}").is_err());
        assert_eq!(p("--[==[ a\n]] b ]==] {x=1}").get_path_i64("x"), Some(1));
        assert_eq!(p("\u{feff}{x=1}").get_path_i64("x"), Some(1));
        assert_eq!(p("return {x=1};").get_path_i64("x"), Some(1));
        assert_eq!(p("{\r\n\tx = 1,\r\n}\r\n").get_path_i64("x"), Some(1));
    }

    #[test]
    fn identifier_value_vs_name_is_decided_by_the_equals_sign() {
        let v = p("{ true, nil, false, end = 1, nan, x = 2 }");
        assert_eq!(v.iter().count(), 6);
        assert_eq!(v.get_index(1), Some(&LValue::Bool(true)));
        assert_eq!(v.get_index(2), Some(&LValue::Nil));
        assert_eq!(v.get("end"), Some(&LValue::Int(1)));
        assert!(v.get_index(4).and_then(LValue::as_f64).is_some_and(f64::is_nan));
    }

    #[test]
    fn string_escapes() {
        assert_eq!(p(r#""a\nb\tc\r\\\"\'\a\b\f\v""#), LValue::string("a\nb\tc\r\\\"'\u{7}\u{8}\u{c}\u{b}"));
        assert_eq!(p(r#""\65\066\0067""#), LValue::string("AB\u{6}7"));
        assert_eq!(p(r#""\x41\x7a""#), LValue::string("Az"));
        assert_eq!(p(r#""\u{5E9}\u{5DC}\u{1F600}""#), LValue::string("של😀"));
        assert_eq!(p("\"a\\\nb\""), LValue::string("a\nb"));
        assert_eq!(p("\"a\\\r\nb\""), LValue::string("a\nb"));
        assert_eq!(p("\"a\\z  \n\t b\""), LValue::string("ab"));
        assert_eq!(p(r#""\q""#), LValue::string("q"));
        assert_eq!(p("\"raw\nnewline\""), LValue::string("raw\nnewline"));
        assert_eq!(p(r#""\255\254""#), LValue::Bytes(vec![0xff, 0xfe]));
        assert_eq!(p(r#""\195\169""#), LValue::string("é"));
    }

    #[test]
    fn right_to_left_text_survives_untouched() {
        let v = p("{ [\"שלום עולם\"] = \"مرحبا بالعالم\", mixed = \"abc \u{5d0}\u{5d1}\u{5d2} 123 \u{202e}x\" }");
        assert_eq!(v.get("שלום עולם"), Some(&LValue::string("مرحبا بالعالم")));
        assert_eq!(v.get_path_str("mixed"), Some("abc אבג 123 \u{202e}x"));
        // Hebrew identifiers are accepted bare, as LuaJIT does.
        assert_eq!(p("{ שלום = 1 }").get_path_i64("שלום"), Some(1));
    }

    #[test]
    fn long_strings() {
        assert_eq!(p("[[hello]]"), LValue::string("hello"));
        assert_eq!(p("[[\nfirst newline dropped]]"), LValue::string("first newline dropped"));
        assert_eq!(p("[==[a]]b]=]c]==]"), LValue::string("a]]b]=]c"));
        assert_eq!(p("[[a\r\nb\rc]]"), LValue::string("a\nb\nc"));
        assert_eq!(p("{ [[x]], y = [=[z]=] }").iter().count(), 2);
    }

    #[test]
    fn errors_carry_positions() {
        let Err(Error::Syntax { line, column, kind }) = parse("{\n\ta = 1,\n\tb = @\n}") else { panic!("expected a syntax error") };
        assert_eq!((line, column), (3, 6));
        assert!(matches!(kind, SyntaxError::Unexpected { found: '@', .. }));
        let Err(Error::Syntax { line, column, .. }) = parse("{ \"שלום\" = ") else { panic!("expected a syntax error") };
        assert_eq!((line, column), (1, 10));
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(matches!(syntax_kind(""), SyntaxError::UnexpectedEof { .. }));
        assert!(matches!(syntax_kind("{"), SyntaxError::UnexpectedEof { .. }));
        assert!(matches!(syntax_kind("{ a = }"), SyntaxError::Unexpected { .. }));
        assert!(matches!(syntax_kind("{ a = 1 b = 2 }"), SyntaxError::Unexpected { .. }));
        assert!(matches!(syntax_kind("{ [1 = 2 }"), SyntaxError::Unexpected { .. }));
        assert!(matches!(syntax_kind("{ [nil] = 2 }"), SyntaxError::BadKey(_)));
        assert!(matches!(syntax_kind("{ [foo] = 2 }"), SyntaxError::BadKey(_)));
        assert!(matches!(syntax_kind("{ [{}] = 2 }"), SyntaxError::Unexpected { .. }));
        assert!(matches!(syntax_kind("\"abc"), SyntaxError::UnterminatedString));
        assert!(matches!(syntax_kind("\"abc\\"), SyntaxError::UnterminatedString));
        assert!(matches!(syntax_kind("[[abc"), SyntaxError::UnterminatedLongBracket));
        assert!(matches!(syntax_kind("--[[ abc"), SyntaxError::UnterminatedLongBracket));
        assert!(matches!(syntax_kind("1 2"), SyntaxError::TrailingData));
        assert!(matches!(syntax_kind("{} {}"), SyntaxError::TrailingData));
        assert!(matches!(syntax_kind("{} }"), SyntaxError::TrailingData));
        assert!(matches!(syntax_kind("foo"), SyntaxError::UnknownIdentifier(_)));
        assert!(matches!(syntax_kind("5abc"), SyntaxError::BadNumber(_)));
        assert!(matches!(syntax_kind("1.2.3"), SyntaxError::BadNumber(_)));
        assert!(matches!(syntax_kind("-"), SyntaxError::BadNumber(_)));
        assert!(matches!(syntax_kind("\"\\x4\""), SyntaxError::BadEscape(_)));
        assert!(matches!(syntax_kind("\"\\400\""), SyntaxError::BadEscape(_)));
        assert!(matches!(syntax_kind("\"\\u{110000}\""), SyntaxError::BadEscape(_)));
        assert!(matches!(syntax_kind("\"\\u{D800}\""), SyntaxError::BadEscape(_)));
        assert!(matches!(syntax_kind("\"\\u{}\""), SyntaxError::BadEscape(_)));
        assert!(matches!(syntax_kind("\"\\u0041\""), SyntaxError::Unexpected { .. }));
        assert!(matches!(syntax_kind("{ \"\\255\" = 1 }"), SyntaxError::Unexpected { .. }));
        assert!(matches!(syntax_kind("{ [\"\\255\"] = 1 }"), SyntaxError::InvalidUtf8(_)));
    }

    #[test]
    fn non_utf8_names_are_rejected_but_non_utf8_strings_are_kept() {
        assert!(matches!(parse_bytes(b"{ a\xff = 1 }"), Err(Error::Syntax { kind: SyntaxError::InvalidUtf8(_), .. })));
        assert_eq!(parse_bytes(b"{ a = \"\xff\" }").unwrap().get("a"), Some(&LValue::Bytes(vec![0xff])));
    }

    #[test]
    fn depth_limit_is_enforced() {
        let nested = |n: usize| format!("{}{}", "{".repeat(n), "}".repeat(n));
        assert!(parse(&nested(128)).is_ok());
        let Err(Error::TooDeep { limit, line, column }) = parse(&nested(129)) else { panic!("expected TooDeep") };
        assert_eq!((limit, line, column), (128, 1, 129));
        let tight = Limits { max_depth: 2, ..Limits::default() };
        assert!(parse_with("{{}}", &tight).is_ok());
        assert!(matches!(parse_with("{{{}}}", &tight), Err(Error::TooDeep { .. })));
        // A huge configured depth is clamped, so deep hostile input errors instead of overflowing the stack.
        let loose = Limits { max_depth: usize::MAX, ..Limits::default() };
        assert!(matches!(parse_with(&nested(5000), &loose), Err(Error::TooDeep { limit: HARD_MAX_DEPTH, .. })));
        assert!(parse_with(&"{".repeat(100_000), &loose).is_err());
    }

    #[test]
    fn size_and_entry_limits_are_enforced() {
        let small = Limits { max_input_bytes: 8, ..Limits::default() };
        assert!(matches!(parse_with("{a=1,b=2,c=3}", &small), Err(Error::InputTooLarge { len: 13, limit: 8 })));
        let few = Limits { max_entries: 3, ..Limits::default() };
        assert!(parse_with("{1,2,3}", &few).is_ok());
        assert!(matches!(parse_with("{1,2,{3,4}}", &few), Err(Error::TooManyEntries { limit: 3 })));
    }

    #[test]
    fn error_messages_read_well() {
        let message = parse("{\n a = ?\n}").unwrap_err().to_string();
        assert_eq!(message, "line 2, column 6: unexpected '?', expected a value");
    }
}
