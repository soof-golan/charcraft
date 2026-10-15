//! Rope records: framing, decoding and encoding. See `FORMAT.md` for the byte layout.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::GzBuilder;
use flate2::read::GzDecoder;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::num::{read_num, write_num, write_trailer};
use crate::oid::Oid;

/// The 14 bytes every record starts with: `"Rope10LMm"` and the forward number `0xC3FFCF09`.
pub const MAGIC: [u8; 14] = *b"Rope10LMm\x04\xc3\xff\xcf\x09";

/// Objects larger than this are stored gzip-compressed (Character Animator stores 505 bytes raw and 520 compressed).
pub const COMPRESS_ABOVE: usize = 512;

/// gzip header OS byte Character Animator writes (`0x13`), with XFL 2 (best compression).
const GZIP_OS: u8 = 0x13;

/// Caps that bound what a hostile or damaged rope can make us allocate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest accepted record payload in bytes.
    pub max_record_len: u64,
    /// Largest accepted decompressed object.
    pub max_object_size: u64,
    /// Largest accepted sum of decompressed object sizes in one store.
    pub max_total_bytes: u64,
    /// Largest accepted number of records.
    pub max_records: usize,
    /// Longest accepted ref name or target in bytes.
    pub max_name_len: usize,
    /// Deepest accepted tree nesting when walking files.
    pub max_tree_depth: usize,
    /// Most entries accepted in one tree.
    pub max_tree_entries: usize,
    /// Most files accepted when flattening a tree.
    pub max_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        const MIB: u64 = 1024 * 1024;
        Self {
            max_record_len: 1024 * MIB,
            max_object_size: 1024 * MIB,
            max_total_bytes: 8 * 1024 * MIB,
            max_records: 10_000_000,
            max_name_len: 4096,
            max_tree_depth: 64,
            max_tree_entries: 1_000_000,
            max_files: 1_000_000,
        }
    }
}

/// The four git object types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    /// A commit (`tree`, `parent`, `author`, `committer`, message).
    Commit,
    /// A directory listing.
    Tree,
    /// File content.
    Blob,
    /// An annotated tag.
    Tag,
}

impl ObjectKind {
    /// The record type code: commit 1, tree 2, blob 3, tag 4.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Commit => 1,
            Self::Tree => 2,
            Self::Blob => 3,
            Self::Tag => 4,
        }
    }

    /// Maps a record type code back.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Commit),
            2 => Some(Self::Tree),
            3 => Some(Self::Blob),
            4 => Some(Self::Tag),
            _ => None,
        }
    }

    /// The git type name used in the hashed header.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Tree => "tree",
            Self::Blob => "blob",
            Self::Tag => "tag",
        }
    }

    /// Maps a git type name back.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "commit" => Some(Self::Commit),
            "tree" => Some(Self::Tree),
            "blob" => Some(Self::Blob),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

/// What a ref points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RefTarget {
    /// An object id.
    Direct(Oid),
    /// Another ref's name (`HEAD` -> `refs/heads/main`).
    Symbolic(String),
}

/// One entry of the append-only log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A git object (kind 0). `data` is the decompressed content.
    Object {
        /// The object's id.
        id: Oid,
        /// Its type.
        kind: ObjectKind,
        /// Its content.
        data: Vec<u8>,
    },
    /// A ref created or moved (kind 1).
    SetRef {
        /// Ref name, for example `refs/heads/main`.
        name: String,
        /// What it now points at.
        target: RefTarget,
    },
    /// A ref removed (kind 2).
    DeleteRef {
        /// Ref name.
        name: String,
    },
    /// A record of a kind or type this crate does not know. It is kept verbatim so that writing it back is lossless.
    Unknown {
        /// Record kind byte.
        kind: u8,
        /// Record type byte.
        typ: u8,
        /// The undecoded payload.
        payload: Vec<u8>,
    },
}

impl Record {
    /// An object record whose id is computed from the content.
    #[must_use]
    pub fn object(kind: ObjectKind, data: Vec<u8>) -> Self {
        Self::Object { id: Oid::of(kind, &data), kind, data }
    }
}

/// Appends `len` as a forward number followed by `bytes`.
fn put_str(out: &mut Vec<u8>, bytes: &[u8]) {
    write_num(out, u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    out.extend_from_slice(bytes);
}

/// Gzip-compresses like Character Animator does (best compression, OS byte 0x13).
fn gzip(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = GzBuilder::new().operating_system(GZIP_OS).write(Vec::new(), Compression::best());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

/// Gunzips `data`, which must expand to exactly `expected` bytes.
fn gunzip(data: &[u8], expected: u64) -> Option<Vec<u8>> {
    let capacity = usize::try_from(expected.min(1 << 20)).ok()?;
    let mut out = Vec::with_capacity(capacity);
    GzDecoder::new(data).take(expected.saturating_add(1)).read_to_end(&mut out).ok()?;
    (u64::try_from(out.len()).ok()? == expected).then_some(out)
}

/// Encodes one record, appending it to `out`.
pub fn encode_record(record: &Record, out: &mut Vec<u8>) {
    let (kind, typ, payload) = match record {
        Record::Object { id, kind, data } => {
            let mut payload = Vec::with_capacity(data.len().saturating_add(32));
            payload.extend_from_slice(id.as_bytes());
            let raw = u64::try_from(data.len()).unwrap_or(u64::MAX);
            let packed = if data.len() > COMPRESS_ABOVE { gzip(data) } else { None };
            match packed {
                Some(packed) if packed.len() < data.len() => {
                    write_num(&mut payload, raw);
                    write_num(&mut payload, u64::try_from(packed.len()).unwrap_or(u64::MAX));
                    payload.extend_from_slice(&packed);
                }
                _ => {
                    write_num(&mut payload, raw);
                    write_num(&mut payload, raw);
                    payload.extend_from_slice(data);
                }
            }
            (0, kind.code(), payload)
        }
        Record::SetRef { name, target } => {
            let mut payload = Vec::new();
            put_str(&mut payload, name.as_bytes());
            match target {
                RefTarget::Direct(id) => {
                    payload.extend_from_slice(id.as_bytes());
                    (1, 1, payload)
                }
                RefTarget::Symbolic(to) => {
                    put_str(&mut payload, to.as_bytes());
                    (1, 2, payload)
                }
            }
        }
        Record::DeleteRef { name } => {
            let mut payload = Vec::new();
            put_str(&mut payload, name.as_bytes());
            (2, 0, payload)
        }
        Record::Unknown { kind, typ, payload } => (*kind, *typ, payload.clone()),
    };
    let start = out.len();
    out.extend_from_slice(&MAGIC);
    out.push(kind);
    out.push(typ);
    write_num(out, u64::try_from(payload.len()).unwrap_or(u64::MAX));
    out.extend_from_slice(&payload);
    let total = out.len().saturating_sub(start);
    write_trailer(out, u64::try_from(total).unwrap_or(u64::MAX));
}

/// Encodes a sequence of records into a fresh rope.
#[must_use]
pub fn encode_records<'a>(records: impl IntoIterator<Item = &'a Record>) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        encode_record(record, &mut out);
    }
    out
}

fn read_string(payload: &[u8], pos: &mut usize, limits: &Limits, offset: usize) -> Result<String> {
    let bad = |reason| Error::BadRecord { offset, reason };
    let len = read_num(payload, pos).ok_or_else(|| bad("truncated string length"))?;
    if len > u64::try_from(limits.max_name_len).unwrap_or(u64::MAX) {
        return Err(Error::LimitExceeded { offset, what: "ref name length" });
    }
    let len = usize::try_from(len).map_err(|_| bad("string length"))?;
    let end = pos.checked_add(len).ok_or_else(|| bad("string length"))?;
    let bytes = payload.get(*pos..end).ok_or_else(|| bad("truncated string"))?;
    *pos = end;
    String::from_utf8(bytes.to_vec()).map_err(|_| bad("ref name is not UTF-8"))
}

fn decode_object(kind: ObjectKind, payload: &[u8], limits: &Limits, verify: bool, offset: usize) -> Result<Record> {
    let bad = |reason| Error::BadRecord { offset, reason };
    let id = Oid::from_slice(payload).ok_or_else(|| bad("object record shorter than an id"))?;
    let mut pos = 20;
    let raw = read_num(payload, &mut pos).ok_or_else(|| bad("truncated object size"))?;
    let stored = read_num(payload, &mut pos).ok_or_else(|| bad("truncated stored size"))?;
    let body = payload.get(pos..).ok_or_else(|| bad("truncated object"))?;
    if u64::try_from(body.len()).ok() != Some(stored) {
        return Err(bad("stored size does not match the record"));
    }
    if raw > limits.max_object_size {
        return Err(Error::LimitExceeded { offset, what: "object size" });
    }
    let data = if raw == stored { body.to_vec() } else { gunzip(body, raw).ok_or_else(|| bad("object is not a gzip stream of the declared size"))? };
    if verify {
        let computed = Oid::of(kind, &data);
        if computed != id {
            return Err(Error::IdMismatch { offset, declared: id, computed });
        }
    }
    Ok(Record::Object { id, kind, data })
}

fn decode_payload(kind: u8, typ: u8, payload: &[u8], limits: &Limits, verify: bool, offset: usize) -> Result<Record> {
    let bad = |reason| Error::BadRecord { offset, reason };
    match (kind, typ) {
        (0, code) if ObjectKind::from_code(code).is_some() => {
            let object_kind = ObjectKind::from_code(code).ok_or_else(|| bad("object type"))?;
            decode_object(object_kind, payload, limits, verify, offset)
        }
        (1, 1) => {
            let mut pos = 0;
            let name = read_string(payload, &mut pos, limits, offset)?;
            let rest = payload.get(pos..).ok_or_else(|| bad("truncated ref"))?;
            if rest.len() != 20 {
                return Err(bad("direct ref does not end in a 20 byte id"));
            }
            let id = Oid::from_slice(rest).ok_or_else(|| bad("direct ref id"))?;
            Ok(Record::SetRef { name, target: RefTarget::Direct(id) })
        }
        (1, 2) => {
            let mut pos = 0;
            let name = read_string(payload, &mut pos, limits, offset)?;
            let to = read_string(payload, &mut pos, limits, offset)?;
            if pos != payload.len() {
                return Err(bad("trailing bytes after symbolic ref"));
            }
            Ok(Record::SetRef { name, target: RefTarget::Symbolic(to) })
        }
        (2, 0) => {
            let mut pos = 0;
            let name = read_string(payload, &mut pos, limits, offset)?;
            if pos != payload.len() {
                return Err(bad("trailing bytes after ref deletion"));
            }
            Ok(Record::DeleteRef { name })
        }
        _ => Ok(Record::Unknown { kind, typ, payload: payload.to_vec() }),
    }
}

/// Decodes the record starting at `offset` in `buf`; returns it and the offset of the next record.
///
/// With `verify` set, an object whose content does not hash to its id is an error.
///
/// # Errors
///
/// [`Error::BadMagic`], [`Error::Truncated`], [`Error::BadRecord`], [`Error::LimitExceeded`] or
/// [`Error::IdMismatch`]; never panics, whatever `buf` holds.
pub fn parse_record(buf: &[u8], offset: usize, limits: &Limits, verify: bool) -> Result<(Record, usize)> {
    let rest = buf.get(offset..).ok_or(Error::Truncated { offset })?;
    if rest.get(..MAGIC.len()) != Some(&MAGIC[..]) {
        return if rest.len() < MAGIC.len() && MAGIC.starts_with(rest) { Err(Error::Truncated { offset }) } else { Err(Error::BadMagic { offset }) };
    }
    let truncated = || Error::Truncated { offset };
    let mut pos = offset.checked_add(MAGIC.len()).ok_or_else(truncated)?;
    let kind = *buf.get(pos).ok_or_else(truncated)?;
    let typ = *buf.get(pos.saturating_add(1)).ok_or_else(truncated)?;
    pos = pos.checked_add(2).ok_or_else(truncated)?;
    let len = read_num(buf, &mut pos).ok_or_else(truncated)?;
    if len > limits.max_record_len {
        return Err(Error::LimitExceeded { offset, what: "record length" });
    }
    let len = usize::try_from(len).map_err(|_| Error::LimitExceeded { offset, what: "record length" })?;
    let end = pos.checked_add(len).ok_or_else(truncated)?;
    let payload = buf.get(pos..end).ok_or_else(truncated)?;
    let total = end.checked_sub(offset).ok_or_else(truncated)?;
    let mut expected = Vec::with_capacity(9);
    write_trailer(&mut expected, u64::try_from(total).unwrap_or(u64::MAX));
    let trailer_end = end.checked_add(expected.len()).ok_or_else(truncated)?;
    let trailer = buf.get(end..trailer_end).ok_or_else(truncated)?;
    if trailer != expected.as_slice() {
        return Err(Error::BadRecord { offset, reason: "trailer does not match the record length" });
    }
    let record = decode_payload(kind, typ, payload, limits, verify, offset)?;
    Ok((record, trailer_end))
}

/// Iterator over the records of a rope. Yields `(offset, record)`; after the first error it yields that error and
/// stops.
#[derive(Debug)]
pub struct Records<'a> {
    buf: &'a [u8],
    pos: usize,
    count: usize,
    limits: Limits,
    verify: bool,
    done: bool,
}

/// Iterates the records of `buf`.
#[must_use]
pub const fn records(buf: &[u8], limits: Limits, verify: bool) -> Records<'_> {
    Records { buf, pos: 0, count: 0, limits, verify, done: false }
}

impl Iterator for Records<'_> {
    type Item = Result<(usize, Record)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.pos >= self.buf.len() {
            return None;
        }
        if self.count >= self.limits.max_records {
            self.done = true;
            return Some(Err(Error::LimitExceeded { offset: self.pos, what: "record count" }));
        }
        let offset = self.pos;
        match parse_record(self.buf, offset, &self.limits, self.verify) {
            Ok((record, next)) => {
                self.pos = next;
                self.count = self.count.saturating_add(1);
                Some(Ok((offset, record)))
            }
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(record: &Record) -> Record {
        let mut bytes = Vec::new();
        encode_record(record, &mut bytes);
        let (back, next) = parse_record(&bytes, 0, &Limits::default(), true).expect("parses");
        assert_eq!(next, bytes.len());
        back
    }

    #[test]
    fn matches_the_observed_head_record() {
        let record = Record::SetRef { name: "HEAD".into(), target: RefTarget::Symbolic("refs/heads/main".into()) };
        let mut bytes = Vec::new();
        encode_record(&record, &mut bytes);
        let mut expected = b"Rope10LMm\x04\xc3\xff\xcf\x09\x01\x02\x16\x01\x04HEAD\x0frefs/heads/main".to_vec();
        expected.push(0x27);
        assert_eq!(bytes, expected);
        assert_eq!(roundtrip(&record), record);
    }

    #[test]
    fn matches_the_observed_empty_tree_record() {
        let record = Record::object(ObjectKind::Tree, Vec::new());
        let mut bytes = Vec::new();
        encode_record(&record, &mut bytes);
        let mut expected = b"Rope10LMm\x04\xc3\xff\xcf\x09\x00\x02\x16".to_vec();
        expected.extend_from_slice(Oid::EMPTY_TREE.as_bytes());
        expected.extend_from_slice(&[0, 0, 0x27]);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn small_objects_are_raw_and_large_ones_gzip() {
        let small = Record::object(ObjectKind::Blob, vec![b'a'; 100]);
        let mut bytes = Vec::new();
        encode_record(&small, &mut bytes);
        assert!(bytes.windows(2).all(|w| w != [0x1f, 0x8b]));
        let large = Record::object(ObjectKind::Blob, vec![b'a'; 5000]);
        let mut bytes = Vec::new();
        encode_record(&large, &mut bytes);
        assert!(bytes.len() < 500);
        assert!(bytes.windows(2).any(|w| w == [0x1f, 0x8b]));
        assert_eq!(roundtrip(&large), large);
        assert_eq!(roundtrip(&small), small);
    }

    #[test]
    fn unknown_records_survive() {
        let record = Record::Unknown { kind: 9, typ: 3, payload: vec![1, 2, 3] };
        assert_eq!(roundtrip(&record), record);
    }

    #[test]
    fn refs_round_trip() {
        let direct = Record::SetRef { name: "refs/heads/main".into(), target: RefTarget::Direct(Oid::EMPTY_TREE) };
        assert_eq!(roundtrip(&direct), direct);
        let delete = Record::DeleteRef { name: "refs/heads/v190_24.0".into() };
        assert_eq!(roundtrip(&delete), delete);
    }

    #[test]
    fn a_wrong_id_is_detected() {
        let record = Record::Object { id: Oid::EMPTY_TREE, kind: ObjectKind::Blob, data: b"x".to_vec() };
        let mut bytes = Vec::new();
        encode_record(&record, &mut bytes);
        assert!(matches!(parse_record(&bytes, 0, &Limits::default(), true), Err(Error::IdMismatch { .. })));
        assert!(parse_record(&bytes, 0, &Limits::default(), false).is_ok());
    }

    #[test]
    fn damaged_records_are_errors_not_panics() {
        let mut bytes = Vec::new();
        encode_record(&Record::object(ObjectKind::Blob, vec![7; 2000]), &mut bytes);
        for cut in 0..bytes.len() {
            let _ = parse_record(bytes.get(..cut).unwrap_or(&[]), 0, &Limits::default(), true);
        }
        for index in 0..bytes.len() {
            let mut copy = bytes.clone();
            if let Some(byte) = copy.get_mut(index) {
                *byte ^= 0xff;
            }
            let _ = parse_record(&copy, 0, &Limits::default(), true);
        }
    }

    #[test]
    fn oversized_declarations_are_rejected() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&[0, 3, 8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert!(matches!(parse_record(&bytes, 0, &Limits::default(), true), Err(Error::LimitExceeded { .. })));
    }

    #[test]
    fn iterator_stops_after_an_error() {
        let mut bytes = Vec::new();
        encode_record(&Record::object(ObjectKind::Blob, b"one".to_vec()), &mut bytes);
        bytes.extend_from_slice(b"garbage");
        let items: Vec<_> = records(&bytes, Limits::default(), true).collect();
        assert_eq!(items.len(), 2);
        assert!(items.first().is_some_and(Result::is_ok));
        assert!(items.get(1).is_some_and(Result::is_err));
    }
}
