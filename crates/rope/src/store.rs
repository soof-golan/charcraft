//! The in-memory object store: objects, refs and the log that produced them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::git::{Commit, Tag, TreeEntry, encode_tree, parse_tree};
use crate::oid::Oid;
use crate::record::{Limits, ObjectKind, Record, RefTarget, encode_records, records};

/// A stored git object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// Its type.
    pub kind: ObjectKind,
    /// Its decompressed content.
    pub data: Vec<u8>,
}

/// One event of the history, in the order it happened. Object content lives in [`RopeStore::objects`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEntry {
    /// An object was stored.
    Object(Oid),
    /// A ref was set.
    SetRef {
        /// Ref name.
        name: String,
        /// New target.
        target: RefTarget,
    },
    /// A ref was deleted.
    DeleteRef(String),
    /// A record this crate does not understand.
    Unknown {
        /// Record kind byte.
        kind: u8,
        /// Record type byte.
        typ: u8,
        /// Raw payload.
        payload: Vec<u8>,
    },
}

/// A file in a tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    /// Path with `/` separators.
    pub path: String,
    /// Unix mode from the tree.
    pub mode: u32,
    /// Blob id.
    pub id: Oid,
    /// Size of the blob, if it is in the store.
    pub size: Option<u64>,
}

/// Who and when, for a new commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// `name <email>`.
    pub who: String,
    /// Seconds since the epoch.
    pub time: i64,
    /// Zone such as `+0200`.
    pub zone: String,
    /// The commit message.
    pub message: String,
}

/// Everything a project's `repo.noindex` holds: objects, refs and the append-only log.
///
/// The fields are public so callers can inspect them directly; use [`RopeStore::put_object`],
/// [`RopeStore::set_ref`] and [`RopeStore::delete_ref`] to change a store so that the log stays complete.
#[derive(Debug, Clone, Default)]
pub struct RopeStore {
    /// Every object by id.
    pub objects: BTreeMap<Oid, Object>,
    /// Current refs by name, including `HEAD`.
    pub refs: BTreeMap<String, RefTarget>,
    /// The history of changes, oldest first. [`RopeStore::to_rope_bytes`] writes exactly this.
    pub log: Vec<LogEntry>,
    /// Non-fatal problems met while loading (a damaged tail, a bad loose object).
    pub warnings: Vec<String>,
    limits: Limits,
    total_bytes: u64,
}

impl RopeStore {
    /// An empty store with default limits.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty store with the given limits.
    #[must_use]
    pub fn with_limits(limits: Limits) -> Self {
        Self { limits, ..Self::default() }
    }

    /// The limits in force.
    #[must_use]
    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Reads a rope, keeping every record before the first damaged one; the damage is reported in
    /// [`RopeStore::warnings`]. This is what you want for a file whose last append was interrupted.
    ///
    /// # Errors
    ///
    /// Only [`Error::LimitExceeded`] when the objects add up to more than `limits.max_total_bytes`.
    pub fn from_rope_bytes(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let mut store = Self::with_limits(*limits);
        store.load_rope(bytes, false)?;
        Ok(store)
    }

    /// Reads a rope and fails on the first damaged record.
    ///
    /// # Errors
    ///
    /// The first record error (see [`crate::parse_record`]).
    pub fn from_rope_bytes_strict(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let mut store = Self::with_limits(*limits);
        store.load_rope(bytes, true)?;
        Ok(store)
    }

    pub(crate) fn load_rope(&mut self, bytes: &[u8], strict: bool) -> Result<()> {
        for item in records(bytes, self.limits, true) {
            match item {
                Ok((_, record)) => self.apply(record)?,
                Err(error) if strict => return Err(error),
                Err(error) => {
                    self.warnings.push(format!("rope damaged, ignoring the rest: {error}"));
                    break;
                }
            }
        }
        Ok(())
    }

    /// Applies one record to the store (the log gets it too). Objects already present are ignored.
    ///
    /// # Errors
    ///
    /// [`Error::LimitExceeded`] when the store would exceed `limits.max_total_bytes`.
    pub fn apply(&mut self, record: Record) -> Result<()> {
        match record {
            Record::Object { id, kind, data } => {
                if self.objects.contains_key(&id) {
                    return Ok(());
                }
                let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
                let total = self.total_bytes.saturating_add(size);
                if total > self.limits.max_total_bytes {
                    return Err(Error::LimitExceeded { offset: 0, what: "total object bytes" });
                }
                self.total_bytes = total;
                self.objects.insert(id, Object { kind, data });
                self.log.push(LogEntry::Object(id));
            }
            Record::SetRef { name, target } => {
                self.refs.insert(name.clone(), target.clone());
                self.log.push(LogEntry::SetRef { name, target });
            }
            Record::DeleteRef { name } => {
                self.refs.remove(&name);
                self.log.push(LogEntry::DeleteRef(name));
            }
            Record::Unknown { kind, typ, payload } => self.log.push(LogEntry::Unknown { kind, typ, payload }),
        }
        Ok(())
    }

    /// Stores an object and returns its id (idempotent).
    ///
    /// # Errors
    ///
    /// See [`RopeStore::apply`].
    pub fn put_object(&mut self, kind: ObjectKind, data: Vec<u8>) -> Result<Oid> {
        let id = Oid::of(kind, &data);
        self.apply(Record::Object { id, kind, data })?;
        Ok(id)
    }

    /// Points a ref at a target.
    pub fn set_ref(&mut self, name: &str, target: RefTarget) {
        self.refs.insert(name.to_owned(), target.clone());
        self.log.push(LogEntry::SetRef { name: name.to_owned(), target });
    }

    /// Removes a ref.
    pub fn delete_ref(&mut self, name: &str) {
        self.refs.remove(name);
        self.log.push(LogEntry::DeleteRef(name.to_owned()));
    }

    /// The records the log stands for, ready to encode.
    #[must_use]
    pub fn to_records(&self) -> Vec<Record> {
        self.log
            .iter()
            .filter_map(|entry| match entry {
                LogEntry::Object(id) => self.objects.get(id).map(|o| Record::Object { id: *id, kind: o.kind, data: o.data.clone() }),
                LogEntry::SetRef { name, target } => Some(Record::SetRef { name: name.clone(), target: target.clone() }),
                LogEntry::DeleteRef(name) => Some(Record::DeleteRef { name: name.clone() }),
                LogEntry::Unknown { kind, typ, payload } => Some(Record::Unknown { kind: *kind, typ: *typ, payload: payload.clone() }),
            })
            .collect()
    }

    /// Encodes the whole log as a fresh `projectdata.rope`.
    #[must_use]
    pub fn to_rope_bytes(&self) -> Vec<u8> {
        encode_records(&self.to_records())
    }

    /// Writes a fresh `projectdata.rope` to `path`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the file cannot be written.
    pub fn write_rope(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        std::fs::write(path, self.to_rope_bytes()).map_err(|e| Error::io(format!("writing {}", path.display()), e))
    }

    // ---- reading -------------------------------------------------------------------------------------------------

    /// An object by id.
    #[must_use]
    pub fn object(&self, id: &Oid) -> Option<&Object> {
        self.objects.get(id)
    }

    fn typed(&self, id: &Oid, kind: ObjectKind) -> Result<&Object> {
        let object = self.objects.get(id).ok_or(Error::MissingObject(*id))?;
        if object.kind == kind { Ok(object) } else { Err(Error::BadObject { id: *id, reason: "object has a different type than expected" }) }
    }

    /// A parsed commit.
    ///
    /// # Errors
    ///
    /// [`Error::MissingObject`] or [`Error::BadObject`].
    pub fn commit(&self, id: &Oid) -> Result<Commit> {
        Commit::parse(*id, &self.typed(id, ObjectKind::Commit)?.data)
    }

    /// A parsed annotated tag.
    ///
    /// # Errors
    ///
    /// [`Error::MissingObject`] or [`Error::BadObject`].
    pub fn tag(&self, id: &Oid) -> Result<Tag> {
        Tag::parse(*id, &self.typed(id, ObjectKind::Tag)?.data)
    }

    /// The entries of a tree.
    ///
    /// # Errors
    ///
    /// [`Error::MissingObject`] or [`Error::BadObject`].
    pub fn tree(&self, id: &Oid) -> Result<Vec<TreeEntry>> {
        parse_tree(*id, &self.typed(id, ObjectKind::Tree)?.data, &self.limits)
    }

    /// The content of a blob.
    ///
    /// # Errors
    ///
    /// [`Error::MissingObject`] or [`Error::BadObject`].
    pub fn blob(&self, id: &Oid) -> Result<&[u8]> {
        Ok(&self.typed(id, ObjectKind::Blob)?.data)
    }

    /// Follows symbolic refs from `name` to an object id. `None` when a link is missing or the chain loops.
    #[must_use]
    pub fn resolve_ref(&self, name: &str) -> Option<Oid> {
        let mut current = name;
        for _ in 0..16 {
            match self.refs.get(current)? {
                RefTarget::Direct(id) => return Some(*id),
                RefTarget::Symbolic(next) => current = next,
            }
        }
        None
    }

    /// Follows annotated tags until a commit (or tree) is reached.
    ///
    /// # Errors
    ///
    /// [`Error::MissingObject`] or [`Error::BadObject`] when the chain is broken or cyclic.
    pub fn peel(&self, id: Oid) -> Result<Oid> {
        let mut current = id;
        for _ in 0..16 {
            let object = self.objects.get(&current).ok_or(Error::MissingObject(current))?;
            if object.kind != ObjectKind::Tag {
                return Ok(current);
            }
            current = self.tag(&current)?.object;
        }
        Err(Error::BadObject { id, reason: "tag chain is too long" })
    }

    /// The commit `HEAD` ends up at.
    ///
    /// # Errors
    ///
    /// [`Error::UnresolvedRef`] when `HEAD` does not resolve, otherwise as [`RopeStore::peel`].
    pub fn head_commit(&self) -> Result<Oid> {
        let id = self.resolve_ref("HEAD").ok_or_else(|| Error::UnresolvedRef("HEAD".to_owned()))?;
        self.peel(id)
    }

    /// Every file reachable from a commit or tree, sorted by path.
    ///
    /// # Errors
    ///
    /// Missing or malformed trees, or exceeding `max_tree_depth` / `max_files`.
    pub fn list_files(&self, commit_or_tree: Oid) -> Result<Vec<FileEntry>> {
        let root = match self.objects.get(&commit_or_tree).map(|o| o.kind) {
            Some(ObjectKind::Tree) => commit_or_tree,
            _ => self.commit(&self.peel(commit_or_tree)?)?.tree,
        };
        let mut out = Vec::new();
        let mut stack = vec![(String::new(), root, 0_usize)];
        while let Some((prefix, tree, depth)) = stack.pop() {
            if depth > self.limits.max_tree_depth {
                return Err(Error::LimitExceeded { offset: 0, what: "tree depth" });
            }
            for entry in self.tree(&tree)? {
                let path = if prefix.is_empty() { entry.name.clone() } else { format!("{prefix}/{}", entry.name) };
                if entry.is_tree() {
                    stack.push((path, entry.id, depth.saturating_add(1)));
                } else if !entry.is_gitlink() {
                    if out.len() >= self.limits.max_files {
                        return Err(Error::LimitExceeded { offset: 0, what: "file count" });
                    }
                    let size = self.objects.get(&entry.id).map(|o| u64::try_from(o.data.len()).unwrap_or(u64::MAX));
                    out.push(FileEntry { path, mode: entry.mode, id: entry.id, size });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Every file of a commit or tree with its content.
    ///
    /// # Errors
    ///
    /// As [`RopeStore::list_files`], plus [`Error::MissingObject`] for a blob that is not stored.
    pub fn files(&self, commit_or_tree: Oid) -> Result<BTreeMap<String, &[u8]>> {
        let mut out = BTreeMap::new();
        for file in self.list_files(commit_or_tree)? {
            out.insert(file.path, self.blob(&file.id)?);
        }
        Ok(out)
    }

    /// The files of the commit `HEAD` points at: path to content.
    ///
    /// # Errors
    ///
    /// As [`RopeStore::head_commit`] and [`RopeStore::files`].
    pub fn head_tree(&self) -> Result<BTreeMap<String, &[u8]>> {
        self.files(self.head_commit()?)
    }

    /// The first-parent history from `from`, newest first, at most `max` commits.
    ///
    /// # Errors
    ///
    /// A missing or malformed commit.
    pub fn history(&self, from: Oid, max: usize) -> Result<Vec<(Oid, Commit)>> {
        let mut out = Vec::new();
        let mut next = Some(self.peel(from)?);
        while let Some(id) = next {
            if out.len() >= max {
                break;
            }
            let commit = self.commit(&id)?;
            next = commit.parents.first().copied().filter(|p| self.objects.contains_key(p));
            out.push((id, commit));
        }
        Ok(out)
    }

    // ---- writing -------------------------------------------------------------------------------------------------

    /// Stores `files` as blobs and trees, creates a commit on top of `parent`, and points `branch` (a full ref name
    /// such as `refs/heads/main`) at it. Returns the commit id.
    ///
    /// # Errors
    ///
    /// A path that is empty, absolute or has `.`/`..` segments is a [`Error::BadObject`]; also see
    /// [`RopeStore::apply`].
    pub fn commit_files(&mut self, files: &BTreeMap<String, Vec<u8>>, info: &CommitInfo, parent: Option<Oid>, branch: &str) -> Result<Oid> {
        let mut root = Dir::default();
        for (path, data) in files {
            let id = self.put_object(ObjectKind::Blob, data.clone())?;
            let segments: Vec<&str> = path.split('/').collect();
            if segments.iter().any(|s| s.is_empty() || *s == "." || *s == "..") || segments.len() > self.limits.max_tree_depth {
                return Err(Error::BadObject { id, reason: "file path is not a clean relative path" });
            }
            root.insert(&segments, id);
        }
        let tree = self.store_dir(&root)?;
        let line = format!("{} {} {}", info.who, info.time, info.zone);
        let commit = Commit { tree, parents: parent.into_iter().collect(), author: line.clone(), committer: line, message: info.message.clone() };
        let id = self.put_object(ObjectKind::Commit, commit.to_bytes())?;
        self.set_ref(branch, RefTarget::Direct(id));
        Ok(id)
    }

    fn store_dir(&mut self, dir: &Dir) -> Result<Oid> {
        let mut entries = Vec::new();
        for (name, node) in &dir.children {
            match node {
                Node::File(id) => entries.push(TreeEntry { mode: 0o100_644, name: name.clone(), id: *id }),
                Node::Dir(sub) => {
                    let id = self.store_dir(sub)?;
                    entries.push(TreeEntry { mode: 0o040_000, name: name.clone(), id });
                }
            }
        }
        self.put_object(ObjectKind::Tree, encode_tree(&entries))
    }
}

#[derive(Default)]
struct Dir {
    children: BTreeMap<String, Node>,
}

enum Node {
    File(Oid),
    Dir(Dir),
}

impl Dir {
    fn insert(&mut self, segments: &[&str], id: Oid) {
        let Some((first, rest)) = segments.split_first() else {
            return;
        };
        if rest.is_empty() {
            self.children.insert((*first).to_owned(), Node::File(id));
            return;
        }
        let node = self.children.entry((*first).to_owned()).or_insert_with(|| Node::Dir(Self::default()));
        if let Node::File(_) = node {
            *node = Node::Dir(Self::default());
        }
        if let Node::Dir(sub) = node {
            sub.insert(rest, id);
        }
    }
}

/// A summary of a store for display and for tool output (it serialises to JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoreSummary {
    /// Counts of each object type.
    pub objects: BTreeMap<String, usize>,
    /// Current refs; direct ones as hex, symbolic ones as `-> name`.
    pub refs: BTreeMap<String, String>,
    /// The commit `HEAD` resolves to.
    pub head: Option<Oid>,
    /// Number of log entries.
    pub log_len: usize,
    /// Load warnings.
    pub warnings: Vec<String>,
}

impl RopeStore {
    /// Summarises the store.
    #[must_use]
    pub fn summary(&self) -> StoreSummary {
        let mut objects = BTreeMap::new();
        for object in self.objects.values() {
            let count = objects.entry(object.kind.name().to_owned()).or_insert(0_usize);
            *count = count.saturating_add(1);
        }
        let refs = self
            .refs
            .iter()
            .map(|(name, target)| {
                let text = match target {
                    RefTarget::Direct(id) => id.to_hex(),
                    RefTarget::Symbolic(to) => format!("-> {to}"),
                };
                (name.clone(), text)
            })
            .collect();
        StoreSummary { objects, refs, head: self.head_commit().ok(), log_len: self.log.len(), warnings: self.warnings.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(message: &str) -> CommitInfo {
        CommitInfo { who: "user <red@monster>".into(), time: 1_700_000_000, zone: "+0000".into(), message: message.into() }
    }

    fn sample() -> RopeStore {
        let mut store = RopeStore::new();
        let mut files = BTreeMap::new();
        files.insert("Puppet".to_owned(), b"Puppet{\n}\n".to_vec());
        files.insert("Behaviors/Adobe.Blinker.js".to_owned(), vec![b'x'; 3000]);
        files.insert("Behaviors/.folderData".to_owned(), b"ProjectUIData_001{\n}\n".to_vec());
        let first = store.commit_files(&files, &info(".one."), None, "refs/heads/main").expect("commit");
        files.insert("Puppet".to_owned(), b"Puppet{\n\ta = 1,\n}\n".to_vec());
        store.commit_files(&files, &info(".two."), Some(first), "refs/heads/main").expect("commit");
        store.set_ref("HEAD", RefTarget::Symbolic("refs/heads/main".into()));
        store
    }

    #[test]
    fn head_tree_lists_files() {
        let store = sample();
        let tree = store.head_tree().expect("tree");
        let names: Vec<_> = tree.keys().map(String::as_str).collect();
        assert_eq!(names, ["Behaviors/.folderData", "Behaviors/Adobe.Blinker.js", "Puppet"]);
        assert_eq!(tree.get("Puppet").copied(), Some(&b"Puppet{\n\ta = 1,\n}\n"[..]));
    }

    #[test]
    fn history_walks_parents() {
        let store = sample();
        let head = store.head_commit().expect("head");
        let history = store.history(head, 10).expect("history");
        let messages: Vec<_> = history.iter().map(|(_, c)| c.message.as_str()).collect();
        assert_eq!(messages, [".two.", ".one."]);
    }

    #[test]
    fn read_write_read_is_stable() {
        let store = sample();
        let bytes = store.to_rope_bytes();
        let back = RopeStore::from_rope_bytes_strict(&bytes, &Limits::default()).expect("reads");
        assert_eq!(back.objects, store.objects);
        assert_eq!(back.refs, store.refs);
        assert_eq!(back.log, store.log);
        assert_eq!(back.to_rope_bytes(), bytes);
    }

    #[test]
    fn a_torn_tail_keeps_the_good_prefix() {
        let store = sample();
        let mut bytes = store.to_rope_bytes();
        let full = RopeStore::from_rope_bytes(&bytes, &Limits::default()).expect("reads");
        assert_eq!(full.warnings.len(), 0);
        bytes.truncate(bytes.len().saturating_sub(5));
        let torn = RopeStore::from_rope_bytes(&bytes, &Limits::default()).expect("reads");
        assert_eq!(torn.warnings.len(), 1);
        assert!(torn.log.len() < store.log.len());
        assert!(RopeStore::from_rope_bytes_strict(&bytes, &Limits::default()).is_err());
    }

    #[test]
    fn unclean_paths_are_refused() {
        let mut store = RopeStore::new();
        let mut files = BTreeMap::new();
        files.insert("../evil".to_owned(), b"x".to_vec());
        assert!(store.commit_files(&files, &info("x"), None, "refs/heads/main").is_err());
    }

    #[test]
    fn deleting_a_ref_is_logged() {
        let mut store = sample();
        store.delete_ref("refs/heads/main");
        assert!(store.resolve_ref("HEAD").is_none());
        let back = RopeStore::from_rope_bytes_strict(&store.to_rope_bytes(), &Limits::default()).expect("reads");
        assert!(back.resolve_ref("HEAD").is_none());
    }

    #[test]
    fn summary_serialises() {
        let json = serde_json::to_string(&sample().summary()).expect("json");
        assert!(json.contains("refs/heads/main"));
    }
}
