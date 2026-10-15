//! Character Animator's project store.
//!
//! A `.puppet` zip and a project folder both hold `Ch Data/repo.noindex/`: a bare git repository whose objects and
//! refs are ALSO mirrored in `projectdata.rope`, an append-only log that Character Animator treats as the source of
//! truth. This crate reads and writes that log (see `FORMAT.md` for the byte layout), rebuilds the object store with
//! every id verified, resolves refs and walks `HEAD` to the project's files.
//!
//! ```no_run
//! use charcraft_rope::RopeStore;
//!
//! # fn main() -> Result<(), charcraft_rope::Error> {
//! let store = RopeStore::open("Boy.puppet")?;
//! for (path, bytes) in store.head_tree()? {
//!     println!("{path}: {} bytes", bytes.len());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Everything read is treated as hostile: lengths are capped by [`Limits`], decompression is bounded, and no input
//! can make the reader panic.

mod blob;
mod error;
mod git;
mod num;
mod oid;
mod open;
mod record;
mod store;

pub use blob::{ClassTable, class_name, parse_class_table, strip_class_names};
pub use error::{Error, Result};
pub use git::{Commit, Tag, TreeEntry, encode_tree, parse_tree};
pub use oid::Oid;
pub use record::{COMPRESS_ABOVE, Limits, MAGIC, ObjectKind, Record, Records, RefTarget, encode_record, encode_records, parse_record, records};
pub use store::{CommitInfo, FileEntry, LogEntry, Object, RopeStore, StoreSummary};
