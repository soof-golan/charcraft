//! Parsing and building of git's commit, tag and tree objects.

use std::fmt::Write as _;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::oid::Oid;
use crate::record::{Limits, ObjectKind};

/// A parsed commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    /// Root tree.
    pub tree: Oid,
    /// Parents, first parent first.
    pub parents: Vec<Oid>,
    /// The `author` line without its keyword: `name <email> seconds +zone`.
    pub author: String,
    /// The `committer` line without its keyword.
    pub committer: String,
    /// Everything after the blank line. Character Animator writes the editing command's name here, wrapped in dots
    /// (`.FIRST COMMIT FOR VERSION.`).
    pub message: String,
}

impl Commit {
    /// Seconds since the epoch from the committer line, if it parses.
    #[must_use]
    pub fn commit_time(&self) -> Option<i64> {
        let mut words = self.committer.rsplit(' ');
        let _zone = words.next()?;
        words.next()?.parse().ok()
    }

    /// Parses commit content.
    ///
    /// # Errors
    ///
    /// [`Error::BadObject`] when the headers or the tree id are malformed.
    pub fn parse(id: Oid, data: &[u8]) -> Result<Self> {
        let bad = |reason| Error::BadObject { id, reason };
        let text = String::from_utf8_lossy(data);
        let (head, message) = text.split_once("\n\n").map_or((&*text, ""), |(h, m)| (h, m));
        let mut tree = None;
        let mut parents = Vec::new();
        let mut author = String::new();
        let mut committer = String::new();
        for line in head.lines() {
            if line.starts_with(' ') {
                continue; // continuation of a multi-line header such as gpgsig
            }
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            match key {
                "tree" => tree = Some(Oid::from_hex(value).map_err(|_| bad("bad tree id"))?),
                "parent" => parents.push(Oid::from_hex(value).map_err(|_| bad("bad parent id"))?),
                "author" => value.clone_into(&mut author),
                "committer" => value.clone_into(&mut committer),
                _ => {}
            }
        }
        Ok(Self { tree: tree.ok_or_else(|| bad("commit has no tree"))?, parents, author, committer, message: message.to_owned() })
    }

    /// Serialises the commit the way git (and Character Animator) lays it out.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = format!("tree {}\n", self.tree);
        for parent in &self.parents {
            let _ = writeln!(text, "parent {parent}");
        }
        let _ = write!(text, "author {}\ncommitter {}\n\n{}", self.author, self.committer, self.message);
        text.into_bytes()
    }
}

/// A parsed annotated tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tag {
    /// The tagged object.
    pub object: Oid,
    /// Its type.
    pub kind: ObjectKind,
    /// Tag name (`v190/recent/1`).
    pub name: String,
    /// The `tagger` line, if any.
    pub tagger: Option<String>,
    /// Message after the blank line. Character Animator keeps an ltab record here (`TF1:{ sName = ..., }`).
    pub message: String,
}

impl Tag {
    /// Parses tag content.
    ///
    /// # Errors
    ///
    /// [`Error::BadObject`] when the object line or type is malformed.
    pub fn parse(id: Oid, data: &[u8]) -> Result<Self> {
        let bad = |reason| Error::BadObject { id, reason };
        let text = String::from_utf8_lossy(data);
        let (head, message) = text.split_once("\n\n").map_or((&*text, ""), |(h, m)| (h, m));
        let mut object = None;
        let mut kind = None;
        let mut name = String::new();
        let mut tagger = None;
        for line in head.lines() {
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            match key {
                "object" => object = Some(Oid::from_hex(value).map_err(|_| bad("bad tag object id"))?),
                "type" => kind = Some(ObjectKind::from_name(value).ok_or_else(|| bad("bad tag type"))?),
                "tag" => value.clone_into(&mut name),
                "tagger" => tagger = Some(value.to_owned()),
                _ => {}
            }
        }
        Ok(Self {
            object: object.ok_or_else(|| bad("tag has no object"))?,
            kind: kind.ok_or_else(|| bad("tag has no type"))?,
            name,
            tagger,
            message: message.to_owned(),
        })
    }

    /// Serialises the tag.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = format!("object {}\ntype {}\ntag {}\n", self.object, self.kind.name(), self.name);
        if let Some(tagger) = &self.tagger {
            let _ = writeln!(text, "tagger {tagger}");
        }
        text.push('\n');
        text.push_str(&self.message);
        text.into_bytes()
    }
}

/// One entry of a tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeEntry {
    /// Unix mode: `0o100644` file, `0o100755` executable, `0o040000` directory, `0o120000` symlink.
    pub mode: u32,
    /// File or directory name.
    pub name: String,
    /// The blob or subtree.
    pub id: Oid,
}

impl TreeEntry {
    /// Whether the entry is a subtree.
    #[must_use]
    pub const fn is_tree(&self) -> bool {
        self.mode & 0o170_000 == 0o040_000
    }

    /// Whether the entry is a gitlink (submodule); there is no object to follow.
    #[must_use]
    pub const fn is_gitlink(&self) -> bool {
        self.mode & 0o170_000 == 0o160_000
    }

    /// Git's tree sort key: directories compare as if their name ended in `/`.
    fn sort_key(&self) -> Vec<u8> {
        let mut key = self.name.as_bytes().to_vec();
        if self.is_tree() {
            key.push(b'/');
        }
        key
    }
}

/// Parses tree content into entries.
///
/// # Errors
///
/// [`Error::BadObject`] for a malformed entry, a non-UTF-8 name, or more than `limits.max_tree_entries` entries.
pub fn parse_tree(id: Oid, data: &[u8], limits: &Limits) -> Result<Vec<TreeEntry>> {
    let bad = |reason| Error::BadObject { id, reason };
    let mut entries = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let space = rest.iter().position(|b| *b == b' ').ok_or_else(|| bad("tree entry has no mode"))?;
        let mode_text = rest.get(..space).ok_or_else(|| bad("tree entry mode"))?;
        let after_mode = rest.get(space.saturating_add(1)..).ok_or_else(|| bad("tree entry name"))?;
        let nul = after_mode.iter().position(|b| *b == 0).ok_or_else(|| bad("tree entry has no name end"))?;
        let name_bytes = after_mode.get(..nul).ok_or_else(|| bad("tree entry name"))?;
        let id_start = nul.saturating_add(1);
        let entry_id = after_mode.get(id_start..).and_then(Oid::from_slice).ok_or_else(|| bad("tree entry id is truncated"))?;
        rest = after_mode.get(id_start.saturating_add(20)..).unwrap_or(&[]);
        let mut mode = 0_u32;
        if mode_text.is_empty() {
            return Err(bad("empty tree entry mode"));
        }
        for digit in mode_text {
            let value = char::from(*digit).to_digit(8).ok_or_else(|| bad("tree entry mode is not octal"))?;
            mode = mode.checked_mul(8).and_then(|m| m.checked_add(value)).ok_or_else(|| bad("tree entry mode overflows"))?;
        }
        let name = String::from_utf8(name_bytes.to_vec()).map_err(|_| bad("tree entry name is not UTF-8"))?;
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(bad("unsafe tree entry name"));
        }
        entries.push(TreeEntry { mode, name, id: entry_id });
        if entries.len() > limits.max_tree_entries {
            return Err(bad("too many tree entries"));
        }
    }
    Ok(entries)
}

/// Serialises entries (sorted into git's order) as tree content.
#[must_use]
pub fn encode_tree(entries: &[TreeEntry]) -> Vec<u8> {
    let mut sorted: Vec<&TreeEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.sort_key());
    let mut out = Vec::new();
    for entry in sorted {
        out.extend_from_slice(format!("{:o} ", entry.mode).as_bytes());
        out.extend_from_slice(entry.name.as_bytes());
        out.push(0);
        out.extend_from_slice(entry.id.as_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &[u8] = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor user <red@monster> 1748875463 +0500\ncommitter user <red@monster> 1748875463 +0500\n\n.FIRST COMMIT FOR VERSION.";

    #[test]
    fn commit_parses_and_reserialises() {
        let id = Oid::of(ObjectKind::Commit, COMMIT);
        let commit = Commit::parse(id, COMMIT).expect("commit");
        assert_eq!(commit.tree, Oid::EMPTY_TREE);
        assert_eq!(commit.parents.len(), 0);
        assert_eq!(commit.commit_time(), Some(1_748_875_463));
        assert_eq!(commit.message, ".FIRST COMMIT FOR VERSION.");
        assert_eq!(commit.to_bytes(), COMMIT);
    }

    #[test]
    fn commit_without_tree_is_an_error() {
        assert!(Commit::parse(Oid::EMPTY_TREE, b"author x\n\nhi").is_err());
    }

    #[test]
    fn tag_round_trips() {
        let data = b"object 3ab42442dbee9f866a157d563c9e0cb80aebdbf8\ntype commit\ntag v190/recent/1\ntagger 190 24.0 mac <red@monster> 1720210954 +0300\n\nTF1:{\n}\n";
        let tag = Tag::parse(Oid::EMPTY_TREE, data).expect("tag");
        assert_eq!(tag.kind, ObjectKind::Commit);
        assert_eq!(tag.name, "v190/recent/1");
        assert_eq!(tag.to_bytes(), data);
    }

    #[test]
    fn tree_round_trips_in_git_order() {
        let blob = Oid::of(ObjectKind::Blob, b"x");
        let entries = vec![
            TreeEntry { mode: 0o100_644, name: "b.txt".into(), id: blob },
            TreeEntry { mode: 0o040_000, name: "a".into(), id: Oid::EMPTY_TREE },
            TreeEntry { mode: 0o100_644, name: "a.txt".into(), id: blob },
        ];
        let bytes = encode_tree(&entries);
        let parsed = parse_tree(Oid::EMPTY_TREE, &bytes, &Limits::default()).expect("tree");
        let names: Vec<_> = parsed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "a", "b.txt"]);
        assert!(parsed.get(1).is_some_and(TreeEntry::is_tree));
    }

    #[test]
    fn hostile_trees_are_errors() {
        for data in [&b"100644"[..], b"100644 name", b"100644 name\0short", b"9 x\0", b" \0"] {
            assert!(parse_tree(Oid::EMPTY_TREE, data, &Limits::default()).is_err() || data.is_empty());
        }
    }
}
