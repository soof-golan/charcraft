//! Opening a store from a `.puppet` zip, a project folder, a `repo.noindex` folder or a bare `projectdata.rope`.

use std::io::Read;
use std::path::Path;

use anim_archive::{Container, Entry};
use flate2::read::ZlibDecoder;

use crate::error::{Error, Result};
use crate::oid::Oid;
use crate::record::{Limits, ObjectKind, Record, RefTarget};
use crate::store::RopeStore;

const ROPE_NAME: &str = "projectdata.rope";

impl RopeStore {
    /// Opens the store of a Character Animator file with default limits. `path` may be:
    ///
    /// - a `.puppet` zip (or any zip holding `Ch Data/repo.noindex/`),
    /// - a project or extracted-puppet folder, the `.chproj` marker file inside one, or the `repo.noindex` folder,
    /// - a bare `projectdata.rope` file (no loose objects or ref files are read then).
    ///
    /// # Errors
    ///
    /// [`Error::NoStore`] when no repository is found; archive and I/O errors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, &Limits::default())
    }

    /// As [`RopeStore::open`] with explicit limits.
    ///
    /// # Errors
    ///
    /// As [`RopeStore::open`].
    pub fn open_with(path: impl AsRef<Path>, limits: &Limits) -> Result<Self> {
        let path = path.as_ref();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.eq_ignore_ascii_case(ROPE_NAME) && path.is_file() {
            let bytes = std::fs::read(path).map_err(|e| Error::io(format!("reading {}", path.display()), e))?;
            return Self::from_rope_bytes(&bytes, limits);
        }
        let is_marker = path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("chproj"));
        let root = if is_marker { path.parent().unwrap_or(path) } else { path };
        let container = Container::open(root)?;
        Self::from_container_with(&container, limits)
    }

    /// Reads the store out of an already opened container.
    ///
    /// # Errors
    ///
    /// As [`RopeStore::open`].
    pub fn from_container(container: &Container) -> Result<Self> {
        Self::from_container_with(container, &Limits::default())
    }

    /// As [`RopeStore::from_container`] with explicit limits.
    ///
    /// # Errors
    ///
    /// As [`RopeStore::open`].
    pub fn from_container_with(container: &Container, limits: &Limits) -> Result<Self> {
        let (prefix, rope_entry) = locate(container).ok_or_else(|| Error::NoStore(container.path().map_or_else(String::new, |p| p.display().to_string())))?;
        let mut store = Self::with_limits(*limits);
        let mut loose: Vec<(Oid, &Entry)> = Vec::new();
        let mut ref_files: Vec<(String, &Entry)> = Vec::new();
        for entry in container.files() {
            let Some(rel) = entry.path().strip_prefix(prefix.as_str()) else {
                continue;
            };
            if rel == "HEAD" {
                ref_files.push(("HEAD".to_owned(), entry));
            } else if rel.starts_with("refs/") {
                ref_files.push((rel.to_owned(), entry));
            } else if let Some(id) = loose_id(rel) {
                loose.push((id, entry));
            }
        }
        loose.sort_by_key(|(id, _)| *id);
        ref_files.sort_by(|a, b| a.0.cmp(&b.0));
        let max_loose = limits.max_object_size;
        for (id, entry) in loose {
            if entry.size > max_loose {
                store.warnings.push(format!("loose object {id} is too large, skipped"));
                continue;
            }
            let bytes = match container.read_entry(entry) {
                Ok(bytes) => bytes,
                Err(error) => {
                    store.warnings.push(format!("loose object {id}: {error}"));
                    continue;
                }
            };
            match decode_loose(id, &bytes, limits) {
                Some(record) => store.apply(record)?,
                None => store.warnings.push(format!("loose object {id} is damaged, skipped")),
            }
        }
        for (name, entry) in ref_files {
            if entry.size > 4096 {
                store.warnings.push(format!("ref file {name} is too large, skipped"));
                continue;
            }
            let bytes = container.read_entry(entry)?;
            match parse_ref_file(&bytes) {
                Some(target) => store.set_ref(&name, target),
                None => store.warnings.push(format!("ref file {name} is not a ref, skipped")),
            }
        }
        let rope = container.read_entry(rope_entry)?;
        store.load_rope(&rope, false)?;
        Ok(store)
    }
}

/// Finds the repository prefix (`"Ch Data/repo.noindex/"`, or `""`) and its rope entry.
fn locate(container: &Container) -> Option<(String, &Entry)> {
    let mut best: Option<(String, &Entry)> = None;
    for entry in container.files() {
        let path = entry.path();
        let prefix = if path == ROPE_NAME { Some(String::new()) } else { path.strip_suffix(ROPE_NAME).filter(|p| p.ends_with('/')).map(str::to_owned) };
        let Some(prefix) = prefix else { continue };
        let preferred = prefix.contains("repo.noindex");
        match &best {
            Some((current, _)) if current.contains("repo.noindex") || !preferred => {}
            _ => best = Some((prefix, entry)),
        }
    }
    best
}

/// `objects/ab/cdef...` (38 hex digits after the two-digit folder) to an id.
fn loose_id(rel: &str) -> Option<Oid> {
    let rest = rel.strip_prefix("objects/")?;
    let (fan, tail) = rest.split_once('/')?;
    if fan.len() != 2 || tail.len() != 38 {
        return None;
    }
    Oid::from_hex(&format!("{fan}{tail}")).ok()
}

/// Reads a git loose object: zlib of `"<type> <len>\0<data>"`; the hash must match its file name.
fn decode_loose(expected: Oid, bytes: &[u8], limits: &Limits) -> Option<Record> {
    let mut raw = Vec::new();
    ZlibDecoder::new(bytes).take(limits.max_object_size.saturating_add(64)).read_to_end(&mut raw).ok()?;
    let nul = raw.iter().position(|b| *b == 0)?;
    let header = std::str::from_utf8(raw.get(..nul)?).ok()?;
    let (kind, len) = header.split_once(' ')?;
    let kind = ObjectKind::from_name(kind)?;
    let data = raw.get(nul.checked_add(1)?..)?;
    if len.parse::<usize>().ok()? != data.len() || Oid::of(kind, data) != expected {
        return None;
    }
    Some(Record::Object { id: expected, kind, data: data.to_vec() })
}

/// `ref: name` or 40 hex digits.
fn parse_ref_file(bytes: &[u8]) -> Option<RefTarget> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if let Some(name) = text.strip_prefix("ref:") {
        let name = name.trim();
        return (!name.is_empty()).then(|| RefTarget::Symbolic(name.to_owned()));
    }
    Oid::from_hex(text).ok().map(RefTarget::Direct)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;

    fn loose_bytes(kind: &str, data: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(format!("{kind} {}\0", data.len()).as_bytes()).expect("write");
        enc.write_all(data).expect("write");
        enc.finish().expect("finish")
    }

    #[test]
    fn loose_objects_decode_and_verify() {
        let data = b"hello\n";
        let id = Oid::of(ObjectKind::Blob, data);
        let bytes = loose_bytes("blob", data);
        let record = decode_loose(id, &bytes, &Limits::default()).expect("decodes");
        assert_eq!(record, Record::Object { id, kind: ObjectKind::Blob, data: data.to_vec() });
        assert!(decode_loose(Oid::EMPTY_TREE, &bytes, &Limits::default()).is_none());
        assert!(decode_loose(id, b"junk", &Limits::default()).is_none());
    }

    #[test]
    fn ref_files_parse() {
        assert_eq!(parse_ref_file(b"ref: refs/heads/main\n"), Some(RefTarget::Symbolic("refs/heads/main".into())));
        assert_eq!(parse_ref_file(b"4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"), Some(RefTarget::Direct(Oid::EMPTY_TREE)));
        assert_eq!(parse_ref_file(b"nonsense"), None);
    }

    #[test]
    fn loose_names_are_recognised() {
        assert!(loose_id("objects/8e/6613c7043d13e3997d12a61575e31f44a5628d").is_some());
        assert!(loose_id("objects/info/x").is_none());
        assert!(loose_id("objects/pack/pack-1.idx").is_none());
    }
}
