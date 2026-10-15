//! The files around the store: a `.puppet` zip or a project folder as an ordered list of entries.
//!
//! The package never loads media into memory until it is asked to: entries read from disk stay lazy references
//! into the source [`Container`]. Saving writes every entry back (names, order, directory entries and timestamps
//! kept), substituting the rope log and the manifest, which the [`crate::Project`] owns.

use std::collections::BTreeSet;

use anim_archive::{ArchiveWriter, Container, DosDateTime, Entry, NewEntry};

use crate::error::{Error, Result};

/// Where the bytes of an entry come from.
#[derive(Debug, Clone)]
enum Content {
    /// A file of the container the package was opened from.
    Source(Entry),
    /// Bytes held in memory (new or edited files).
    Bytes(Vec<u8>),
    /// The project's `projectdata.rope`, supplied by the project when saving (the entry it was read from, if any).
    Rope(Option<Entry>),
    /// The `_archive_info` manifest, supplied by the project when saving (the entry it was read from, if any).
    Manifest(Option<Entry>),
    /// A directory.
    Dir,
}

/// One file or directory of the package.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    name: String,
    raw_name: Option<Vec<u8>>,
    modified: Option<DosDateTime>,
    content: Content,
}

impl PackageEntry {
    /// The normalised name, `/` separated, with a trailing `/` for directories.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// True for a directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        matches!(self.content, Content::Dir)
    }

    /// The size in bytes when it is known without reading (zero for directories and the project-owned files).
    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.content {
            Content::Source(entry) => entry.size,
            Content::Bytes(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            Content::Rope(_) | Content::Manifest(_) | Content::Dir => 0,
        }
    }

    /// True when the bytes are held in memory because they were added or edited.
    #[must_use]
    pub const fn is_modified(&self) -> bool {
        matches!(self.content, Content::Bytes(_))
    }
}

/// The ordered files of a `.puppet` zip or project folder.
#[derive(Debug, Default)]
pub struct Package {
    source: Option<Container>,
    entries: Vec<PackageEntry>,
}

pub const ROPE_FILE: &str = "projectdata.rope";
/// The manifest file name at the root of a `.puppet`.
pub const MANIFEST_FILE: &str = "_archive_info";

impl Package {
    /// An empty package.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Indexes a container. The rope log and the `_archive_info` manifest are marked as project-owned.
    #[must_use]
    pub fn from_container(container: Container) -> Self {
        let mut entries = Vec::with_capacity(container.entries().len());
        let mut rope_taken = false;
        for entry in container.entries() {
            let raw_name = (!entry.name_encoding.is_unicode()).then(|| entry.raw_name.clone());
            let lower = entry.name.to_ascii_lowercase();
            let content = if entry.is_dir {
                Content::Dir
            } else if !rope_taken && lower.contains("repo.noindex/") && lower.ends_with(ROPE_FILE) {
                rope_taken = true;
                Content::Rope(Some(entry.clone()))
            } else if entry.name == MANIFEST_FILE {
                Content::Manifest(Some(entry.clone()))
            } else {
                Content::Source(entry.clone())
            };
            entries.push(PackageEntry { name: entry.name.clone(), raw_name, modified: entry.modified, content });
        }
        Self { source: Some(container), entries }
    }

    /// The container the package was read from, when it still has one.
    #[must_use]
    pub const fn source(&self) -> Option<&Container> {
        self.source.as_ref()
    }

    /// Every entry in file order.
    #[must_use]
    pub fn entries(&self) -> &[PackageEntry] {
        &self.entries
    }

    /// The files only (no directories).
    pub fn files(&self) -> impl Iterator<Item = &PackageEntry> {
        self.entries.iter().filter(|e| !e.is_dir())
    }

    /// The entry called `name` (exact match on the normalised name).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PackageEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// True if a file or directory called `name` exists (a directory may be given with or without its `/`).
    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        let dir = format!("{}/", name.trim_end_matches('/'));
        self.entries.iter().any(|e| e.name == name || e.name == dir)
    }

    /// The name of the `.chproj` marker file in the package root, if there is one.
    #[must_use]
    pub fn marker(&self) -> Option<&str> {
        self.files().map(PackageEntry::name).find(|n| !n.contains('/') && n.to_ascii_lowercase().ends_with(".chproj"))
    }

    /// Reads a file that is not owned by the project (media, ref files, ...).
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] when there is no such file (or it is the rope or the manifest), plus container errors.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self.get(name).ok_or_else(|| Error::Missing(name.to_owned()))?;
        match &entry.content {
            Content::Bytes(bytes) => Ok(bytes.clone()),
            Content::Source(source) => {
                let container = self.source.as_ref().ok_or_else(|| Error::Missing(name.to_owned()))?;
                Ok(container.read_entry(source)?)
            }
            Content::Rope(_) | Content::Manifest(_) | Content::Dir => Err(Error::Missing(name.to_owned())),
        }
    }

    /// The `projectdata.rope` bytes as they are in the source (`None` when the package has no source rope).
    ///
    /// # Errors
    ///
    /// Container errors from reading the entry.
    pub(crate) fn read_source_rope(&self) -> Result<Option<Vec<u8>>> {
        self.read_owned(true)
    }

    /// The `_archive_info` bytes as they are in the source.
    ///
    /// # Errors
    ///
    /// Container errors from reading the entry.
    pub(crate) fn read_source_manifest(&self) -> Result<Option<Vec<u8>>> {
        self.read_owned(false)
    }

    fn read_owned(&self, rope: bool) -> Result<Option<Vec<u8>>> {
        let (Some(container), Some(source)) = (
            self.source.as_ref(),
            self.entries.iter().find_map(|e| match (&e.content, rope) {
                (Content::Rope(Some(entry)), true) | (Content::Manifest(Some(entry)), false) => Some(entry),
                _ => None,
            }),
        ) else {
            return Ok(None);
        };
        Ok(Some(container.read_entry(source)?))
    }

    /// Adds or replaces a file, creating the directory entries above it.
    pub fn set(&mut self, name: &str, bytes: Vec<u8>) {
        self.ensure_dirs(name);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.name == name) {
            entry.content = Content::Bytes(bytes);
            return;
        }
        self.entries.push(PackageEntry { name: name.to_owned(), raw_name: None, modified: None, content: Content::Bytes(bytes) });
    }

    /// Adds a directory entry (and the ones above it) unless it exists.
    pub fn add_dir(&mut self, name: &str) {
        let dir = format!("{}/", name.trim_end_matches('/'));
        self.ensure_dirs(&dir);
        if !self.entries.iter().any(|e| e.name == dir) {
            self.entries.push(PackageEntry { name: dir, raw_name: None, modified: None, content: Content::Dir });
        }
    }

    /// Removes a file, or a directory with everything under it. Returns how many entries went away.
    pub fn remove(&mut self, name: &str) -> usize {
        let dir = format!("{}/", name.trim_end_matches('/'));
        let before = self.entries.len();
        self.entries.retain(|e| e.name != name && e.name != dir && !e.name.starts_with(&dir));
        before.saturating_sub(self.entries.len())
    }

    /// Declares the rope and manifest entries (used when the package is built from nothing).
    pub(crate) fn add_owned(&mut self, name: &str, manifest: bool) {
        self.ensure_dirs(name);
        let content = if manifest { Content::Manifest(None) } else { Content::Rope(None) };
        self.entries.push(PackageEntry { name: name.to_owned(), raw_name: None, modified: None, content });
    }

    /// The name of the rope entry.
    pub(crate) fn rope_name(&self) -> Option<&str> {
        self.entries.iter().find(|e| matches!(e.content, Content::Rope(_))).map(|e| e.name.as_str())
    }

    /// The files whose bytes are held in memory (added or edited), for in-place saves.
    pub(crate) fn modified(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().filter_map(|e| match &e.content {
            Content::Bytes(bytes) => Some((e.name.as_str(), bytes.as_slice())),
            _ => None,
        })
    }

    fn ensure_dirs(&mut self, name: &str) {
        let mut present: BTreeSet<&str> = BTreeSet::new();
        for entry in &self.entries {
            if entry.is_dir() {
                present.insert(entry.name.as_str());
            }
        }
        let mut missing = Vec::new();
        for (index, _) in name.match_indices('/') {
            if let Some(prefix) = name.get(..=index)
                && prefix != name
                && !present.contains(prefix)
            {
                missing.push(prefix.to_owned());
            }
        }
        for dir in missing {
            self.entries.push(PackageEntry { name: dir, raw_name: None, modified: None, content: Content::Dir });
        }
    }

    /// Writes every entry to `writer`, substituting the rope and the manifest.
    ///
    /// # Errors
    ///
    /// Container errors from reading a source entry or writing the destination.
    pub fn write_to<W: ArchiveWriter>(&self, writer: &mut W, rope: &[u8], manifest: Option<&[u8]>) -> Result<()> {
        for entry in &self.entries {
            let owned;
            let new = match &entry.content {
                Content::Dir => NewEntry::dir(&entry.name),
                Content::Rope(_) => NewEntry::file(&entry.name, rope),
                Content::Manifest(_) => NewEntry::file(&entry.name, manifest.unwrap_or_default()),
                Content::Bytes(bytes) => NewEntry::file(&entry.name, bytes),
                Content::Source(source) => {
                    let container = self.source.as_ref().ok_or_else(|| Error::Missing(entry.name.clone()))?;
                    owned = container.read_entry(source)?;
                    NewEntry::file(&entry.name, &owned)
                }
            };
            let new = entry.raw_name.as_ref().map_or(new, |raw| new.with_raw_name(raw));
            writer.add_entry(new.with_modified(entry.modified))?;
        }
        Ok(())
    }
}
