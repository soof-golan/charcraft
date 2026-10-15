//! Error types.

use std::fmt;

use thiserror::Error;

/// Everything that can go wrong reading, converting or editing an ltab document.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum Error {
    /// The text is not valid ltab; `line` and `column` are 1-based and point at the offending character.
    #[error("line {line}, column {column}: {kind}")]
    Syntax {
        /// 1-based line of the problem.
        line: usize,
        /// 1-based column (in characters) of the problem.
        column: usize,
        /// What was wrong.
        kind: SyntaxError,
    },
    /// The input is larger than [`Limits::max_input_bytes`](crate::Limits::max_input_bytes).
    #[error("input is {len} bytes; the limit is {limit}")]
    InputTooLarge {
        /// Size of the input.
        len: usize,
        /// The configured limit.
        limit: usize,
    },
    /// Tables are nested deeper than [`Limits::max_depth`](crate::Limits::max_depth).
    #[error("line {line}, column {column}: tables are nested more than {limit} levels deep")]
    TooDeep {
        /// 1-based line of the table that is too deep.
        line: usize,
        /// 1-based column of the table that is too deep.
        column: usize,
        /// The configured limit.
        limit: usize,
    },
    /// The document has more table entries than [`Limits::max_entries`](crate::Limits::max_entries).
    #[error("the document has more than {limit} table entries")]
    TooManyEntries {
        /// The configured limit.
        limit: usize,
    },
    /// A path or key was used on a value that is not a table.
    #[error("`{path}` is not inside a table")]
    NotATable {
        /// The path or key that was being resolved.
        path: String,
    },
    /// A message from serde while converting between typed data and [`LValue`](crate::LValue)s.
    #[error("{0}")]
    Custom(String),
}

/// The kinds of syntax error the parser reports.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SyntaxError {
    /// The input ended while more was required.
    #[error("unexpected end of input, expected {expected}")]
    UnexpectedEof {
        /// What would have been acceptable.
        expected: &'static str,
    },
    /// A character that cannot appear here.
    #[error("unexpected {found:?}, expected {expected}")]
    Unexpected {
        /// The offending character.
        found: char,
        /// What would have been acceptable.
        expected: &'static str,
    },
    /// A quoted string was never closed.
    #[error("unterminated string")]
    UnterminatedString,
    /// A long string or long comment (`[[ ... ]]`) was never closed.
    #[error("unterminated long string or comment")]
    UnterminatedLongBracket,
    /// A malformed escape sequence inside a string.
    #[error("invalid escape sequence: {0}")]
    BadEscape(&'static str),
    /// A number literal that does not parse.
    #[error("malformed number `{0}`")]
    BadNumber(String),
    /// A bare word that is not `true`, `false`, `nil`, `inf` or `nan`.
    #[error("unknown identifier `{0}`")]
    UnknownIdentifier(String),
    /// A name or key that is not valid UTF-8.
    #[error("{0} is not valid UTF-8")]
    InvalidUtf8(&'static str),
    /// A table key of a kind that is not supported.
    #[error("unsupported table key: {0}")]
    BadKey(&'static str),
    /// Something follows the top-level value.
    #[error("unexpected data after the end of the value")]
    TrailingData,
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

/// `Result` with this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
