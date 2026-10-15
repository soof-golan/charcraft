//! The Lua-table text format Character Animator uses for metadata.
//!
//! `_archive_info` (the manifest inside every `.puppet` archive) and other small metadata blobs are written as a
//! Lua table constructor, with Hungarian-prefixed keys, tabs for indentation and every number printed with C's
//! `%.17g`:
//!
//! ```text
//! {
//! <TAB>bContainsArtworkCache = false,
//! <TAB>iVersion = 1,
//! <TAB>rootPuppetPPath = "/ProjectItems/A puppet",
//! <TAB>tBehaviorDefaults = {
//! <TAB><TAB>["Adobe.Blinker.js"] = {
//! <TAB><TAB><TAB>rate = 30,
//! <TAB><TAB>},
//! <TAB><TAB>["Adobe.Dragger3.js"] = {
//! <TAB><TAB><TAB>handleReleaseDuration = 0.20000000000000001,
//! <TAB><TAB>},
//! <TAB>},
//! }
//! ```
//!
//! This crate owns that format and nothing else:
//!
//! - [`parse`] reads it into an [`LValue`] tree. The reader accepts the whole data subset of Lua 5.1 to 5.4
//!   (comments, long strings, every escape, `;` separators, hex integers, `[expr] = value` keys), is bounded by
//!   [`Limits`], and never panics on hostile input.
//! - [`to_string_pretty`] writes the tree in Character Animator's layout. Numbers keep the text they were read
//!   with and strings and keys are written the way CA writes them, so a file that CA wrote reads and writes back
//!   **byte for byte**.
//! - [`LValue`] has typed accessors and path lookup (`get_path("tBehaviorDefaults/Adobe.Blinker.js/rate")`), and
//!   `from_lvalue`/`to_lvalue` bridge to any `serde` type.
//!
//! ```
//! use charcraft_ltab::{parse, to_string_pretty};
//!
//! let text = "{\n\tiVersion = 1,\n\ttDefaults = {\n\t\t[\"Adobe.Blinker.js\"] = {\n\t\t\trate = 30,\n\t\t},\n\t},\n}\n";
//! let doc = parse(text)?;
//! assert_eq!(doc.get_path_i64("tDefaults/Adobe.Blinker.js/rate"), Some(30));
//! assert_eq!(to_string_pretty(&doc), text);
//! # Ok::<(), charcraft_ltab::Error>(())
//! ```
//!
//! Typed models read and write through `serde` ([`from_str`], [`from_lvalue`], [`to_string`], [`to_lvalue`]).
//! [`to_string`] sorts keys the way Character Animator does and prints numbers the way it does (`0.2` as
//! `0.20000000000000001`, `100.0` as `100`):
//!
//! ```
//! use charcraft_ltab::{from_str, to_string};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize)]
//! struct Blinker {
//!     length: i64,
//!     rate: f64,
//!     randomness: Option<f64>,
//! }
//!
//! let blinker: Blinker = from_str("{ length = 80, rate = 0.20000000000000001 }")?;
//! assert_eq!(blinker, Blinker { length: 80, rate: 0.2, randomness: None });
//! assert_eq!(to_string(&blinker)?, "{\n\tlength = 80,\n\trate = 0.20000000000000001,\n}\n");
//! # Ok::<(), charcraft_ltab::Error>(())
//! ```
//!
//! Strings are UTF-8 and pass through untouched, so right-to-left names (Hebrew, Arabic) and bidi control
//! characters survive a round trip; there is no text shaping at this level.
//!
//! What is *not* known about the real format (no sample contains it) is written the way a Lua-aware emitter would:
//! an empty table prints as `{` newline `}`, array-style values print positionally, strings escape control
//! characters as 3-digit decimals, and keys that are Lua reserved words are bracketed. The reader accepts every
//! variant, so only byte-exact re-serialisation of such files could differ.

mod de;
mod error;
mod number;
mod parse;
mod ser;
mod value;
mod write;

pub use de::{from_lvalue, from_str};
pub use error::{Error, Result, SyntaxError};
pub use number::{Number, format_g17, parse_number_text};
pub use parse::{HARD_MAX_DEPTH, Limits, parse, parse_bytes, parse_bytes_with, parse_with};
pub use ser::{to_lvalue, to_string};
pub use value::{Key, LValue};
pub use write::{Newline, PrettyOptions, reprint, to_string_compact, to_string_pretty, to_string_with};
