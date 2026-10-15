//! [`Project`]: a Character Animator project folder or `.puppet` zip, opened, decoded, edited and saved back.
//!
//! The project keeps three things apart so that saving stays lossless:
//!
//! - the **store** ([`RopeStore`]): every object and ref, rebuilt from `projectdata.rope`;
//! - the **rope bytes**: exactly what is in the file, plus the records of this session appended at the end
//!   (Character Animator itself only ever appends), so history that was not touched is never re-encoded;
//! - the **package**: every other file (media, ref files, the marker), kept as lazy references into the source.
//!
//! Editing means committing a new tree: [`Project::commit`] moves `HEAD` like Character Animator does, and a save
//! moves `refs/heads/main` to it.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anim_archive::names::is_safe_relative;
use anim_archive::{ArchiveWriter, Container, ContainerKind, DirectoryWriter, ZipWriter};
use charcraft_ltab::LValue;
use charcraft_rope::{CommitInfo, Limits, LogEntry, Oid, Record, RefTarget, RopeStore, StoreSummary, encode_records, parse_record};
use serde::Serialize;

use crate::classed::ClassedDoc;
use crate::error::{Error, Result};
use crate::manifest::ArchiveInfo;
use crate::model::Model;
use crate::package::Package;
use crate::scaffold::{self, DEFAULT_BUILD};

/// What the project was stored as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProjectKind {
    /// A `.puppet` zip: one puppet with its media and an `_archive_info` manifest.
    Puppet,
    /// A project folder with a `.chproj` marker beside `Ch Data/` and `Ch Media/`.
    Folder,
}

/// The identity written into commits, in Character Animator's format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    /// Project format and application version (`190 25.0`).
    pub build: String,
    /// `mac`, `win` or `linux`.
    pub platform: String,
    /// The operating system version.
    pub os: String,
    /// The user id shown in parentheses.
    pub user: String,
}

impl Default for Author {
    fn default() -> Self {
        let platform = match std::env::consts::OS {
            "macos" => "mac",
            "windows" => "win",
            other => other,
        };
        Self { build: DEFAULT_BUILD.to_owned(), platform: platform.to_owned(), os: "0".to_owned(), user: "0".to_owned() }
    }
}

impl Author {
    /// `190 25.0 mac 15.3.1 user (19625) <red@monster>`.
    #[must_use]
    pub fn who(&self) -> String {
        format!("{} {} {} user ({}) <red@monster>", self.build, self.platform, self.os, self.user)
    }
}

/// One change to the files of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Create or replace a file.
    Put(String, Vec<u8>),
    /// Delete a file, or a whole folder.
    Remove(String),
}

/// A project or puppet file.
#[derive(Debug)]
pub struct Project {
    kind: ProjectKind,
    name: String,
    path: Option<PathBuf>,
    package: Package,
    store: RopeStore,
    rope: Vec<u8>,
    rope_disk_len: usize,
    log_mark: usize,
    manifest: Option<ArchiveInfo>,
    author: Author,
    committed: bool,
    warnings: Vec<String>,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
}

/// The length of the longest prefix of `bytes` that is a whole number of intact records.
fn valid_prefix(bytes: &[u8], limits: &Limits) -> usize {
    let mut offset = 0_usize;
    while offset < bytes.len() {
        match parse_record(bytes, offset, limits, false) {
            Ok((_, next)) if next > offset => offset = next,
            _ => break,
        }
    }
    offset
}

fn encode_log_tail(store: &RopeStore, from: usize) -> Vec<u8> {
    let records: Vec<Record> = store
        .log
        .get(from..)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| match entry {
            LogEntry::Object(id) => store.objects.get(id).map(|o| Record::Object { id: *id, kind: o.kind, data: o.data.clone() }),
            LogEntry::SetRef { name, target } => Some(Record::SetRef { name: name.clone(), target: target.clone() }),
            LogEntry::DeleteRef(name) => Some(Record::DeleteRef { name: name.clone() }),
            LogEntry::Unknown { kind, typ, payload } => Some(Record::Unknown { kind: *kind, typ: *typ, payload: payload.clone() }),
        })
        .collect();
    encode_records(&records)
}

fn display_name(package: &Package, path: &Path) -> String {
    if let Some(marker) = package.marker() {
        let stem = marker.trim_end_matches(|c: char| c != '.').trim_end_matches('.');
        let stem = stem.strip_suffix(".tmp").unwrap_or(stem);
        if !stem.is_empty() {
            return stem.to_owned();
        }
    }
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("project").to_owned()
}

impl Project {
    // ---- opening --------------------------------------------------------------------------------------------------

    /// Opens a `.puppet` zip, a project folder, or the `.chproj` marker inside one.
    ///
    /// # Errors
    ///
    /// [`Error::NotAProject`] when there is no project store, plus container and store errors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, &Limits::default())
    }

    /// As [`Project::open`] with explicit store limits.
    ///
    /// # Errors
    ///
    /// As [`Project::open`].
    pub fn open_with(path: impl AsRef<Path>, limits: &Limits) -> Result<Self> {
        let given = path.as_ref();
        let is_marker = given.is_file() && given.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("chproj"));
        let root = if is_marker { given.parent().unwrap_or(given) } else { given };
        let container = Container::open(root)?;
        let container_kind = container.kind();
        let mut warnings: Vec<String> = container.warnings().to_vec();
        let store = match RopeStore::from_container_with(&container, limits) {
            Ok(store) => store,
            Err(charcraft_rope::Error::NoStore(_)) => return Err(Error::NotAProject(root.display().to_string())),
            Err(error) => return Err(error.into()),
        };
        warnings.extend(store.warnings.iter().cloned());
        let package = Package::from_container(container);
        let mut rope = package.read_source_rope()?.unwrap_or_default();
        let valid = valid_prefix(&rope, limits);
        if valid < rope.len() {
            warnings.push(format!("projectdata.rope has {} damaged bytes at its end; they are dropped on save", rope.len().saturating_sub(valid)));
            rope.truncate(valid);
        }
        let manifest = package.read_source_manifest()?.and_then(|bytes| match ArchiveInfo::parse(&bytes) {
            Ok(info) => Some(info),
            Err(error) => {
                warnings.push(format!("_archive_info is not readable: {error}"));
                None
            }
        });
        // A zip without a manifest is a project that was zipped up; it is a project, not a puppet.
        let kind = if manifest.is_some() { ProjectKind::Puppet } else { ProjectKind::Folder };
        if container_kind == ContainerKind::Zip && kind == ProjectKind::Folder {
            warnings.push("a zipped project: saving writes a zip again".to_owned());
        }
        let name = display_name(&package, root);
        let rope_disk_len = rope.len();
        let log_mark = store.log.len();
        Ok(Self {
            kind,
            name,
            path: Some(root.to_path_buf()),
            package,
            store,
            rope,
            rope_disk_len,
            log_mark,
            manifest,
            author: Author::default(),
            committed: false,
            warnings,
        })
    }

    // ---- creating -------------------------------------------------------------------------------------------------

    /// A new, empty project folder called `name` (nothing is written until [`Project::save_folder`]).
    ///
    /// # Errors
    ///
    /// Store errors while building the opening records.
    pub fn new_project(name: &str) -> Result<Self> {
        Self::scaffold(name, ProjectKind::Folder, "origin")
    }

    /// A new `.puppet` called `name` holding one empty root puppet of the same name.
    ///
    /// # Errors
    ///
    /// Store errors while building the opening records.
    pub fn new_puppet(name: &str) -> Result<Self> {
        Self::scaffold(name, ProjectKind::Puppet, name)
    }

    fn scaffold(name: &str, kind: ProjectKind, puppet: &str) -> Result<Self> {
        let author = Author::default();
        let time = now();
        let mut store = RopeStore::new();
        let first = scaffold::opening_records(&mut store, &author.build, time, "+0000")?;
        let is_puppet = kind == ProjectKind::Puppet;
        let package = scaffold::fresh_package(name, is_puppet, first, &author.build);
        let manifest = is_puppet.then(|| ArchiveInfo::new(&format!("/ProjectItems/{puppet}"), &scaffold::make_guid(&format!("{name}/manifest"))));
        let mut project = Self {
            kind,
            name: name.to_owned(),
            path: None,
            package,
            store,
            rope: Vec::new(),
            rope_disk_len: 0,
            log_mark: 0,
            manifest,
            author,
            committed: false,
            warnings: Vec::new(),
        };
        let mut changes = vec![Change::Put(".project/info".to_owned(), scaffold::EMPTY_UI_DATA.to_vec())];
        if is_puppet {
            changes.push(Change::Put(format!("ProjectItems/{puppet}"), scaffold::minimal_puppet(&scaffold::make_guid(&format!("{name}/puppet")), true)?));
        }
        project.commit_at(changes, ".setupSerializedUIObject", time, "+0000")?;
        Ok(project)
    }

    // ---- reading --------------------------------------------------------------------------------------------------

    /// Puppet or project folder.
    #[must_use]
    pub const fn kind(&self) -> ProjectKind {
        self.kind
    }

    /// The name taken from the marker file (`Boy` for `Boy.tmp.chproj`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the project was opened from or last saved to.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The object store.
    #[must_use]
    pub const fn store(&self) -> &RopeStore {
        &self.store
    }

    /// The files around the store (media, marker, repository boilerplate).
    #[must_use]
    pub const fn package(&self) -> &Package {
        &self.package
    }

    /// The `_archive_info` manifest of a `.puppet`.
    #[must_use]
    pub const fn manifest(&self) -> Option<&ArchiveInfo> {
        self.manifest.as_ref()
    }

    /// Mutable access to the manifest.
    pub const fn manifest_mut(&mut self) -> Option<&mut ArchiveInfo> {
        self.manifest.as_mut()
    }

    /// Notes from opening: recovery strategies used, damaged records, unreadable manifests.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The identity new commits are written with.
    #[must_use]
    pub const fn author(&self) -> &Author {
        &self.author
    }

    /// Changes the identity new commits are written with.
    pub fn set_author(&mut self, author: Author) {
        self.author = author;
    }

    /// The files at `HEAD`, path to text.
    ///
    /// # Errors
    ///
    /// Store errors when `HEAD` does not resolve.
    pub fn files(&self) -> Result<std::collections::BTreeMap<String, &[u8]>> {
        Ok(self.store.head_tree()?)
    }

    /// One file at `HEAD`.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] when the path is not in the tree.
    pub fn file(&self, path: &str) -> Result<&[u8]> {
        self.files()?.get(path).copied().ok_or_else(|| Error::Missing(path.to_owned()))
    }

    /// Parses one file at `HEAD` keeping its class names.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] or a parse error.
    pub fn doc(&self, path: &str) -> Result<ClassedDoc> {
        ClassedDoc::parse(self.file(path)?).map_err(|e| match e {
            Error::ClassMismatch { classes, tables, .. } => Error::ClassMismatch { path: path.to_owned(), classes, tables },
            other => other,
        })
    }

    /// Decodes puppets, scenes, media and behaviors.
    ///
    /// # Errors
    ///
    /// Store errors when `HEAD` does not resolve. Files that cannot be decoded are listed in
    /// [`Model::problems`] instead.
    pub fn model(&self) -> Result<Model> {
        Ok(Model::decode(&self.store.head_tree()?, &self.package))
    }

    /// The `HEAD` commit and the saved `main` commit; they differ after unsaved work.
    #[must_use]
    pub fn heads(&self) -> (Option<Oid>, Option<Oid>) {
        (self.store.resolve_ref("HEAD"), self.store.resolve_ref("refs/heads/main"))
    }

    /// Everything a tool needs to describe the project.
    ///
    /// # Errors
    ///
    /// As [`Project::model`].
    pub fn summary(&self) -> Result<ProjectSummary> {
        let model = self.model()?;
        let (head, main) = self.heads();
        let build = self.store.refs.keys().filter_map(|r| r.strip_prefix("refs/heads/v")).max().map(str::to_owned);
        Ok(ProjectSummary {
            kind: self.kind,
            name: self.name.clone(),
            path: self.path.as_ref().map(|p| p.display().to_string()),
            build,
            root_puppet: self.manifest.as_ref().and_then(|m| m.root_puppet_ppath().map(str::to_owned)),
            unique_id: self.manifest.as_ref().and_then(|m| m.unique_id().map(str::to_owned)),
            head,
            main,
            unsaved_commits: self.committed,
            puppets: model
                .puppets
                .iter()
                .map(|p| PuppetSummary {
                    path: p.path.clone(),
                    name: p.display_name().to_owned(),
                    is_part: p.is_part,
                    guid: p.guid.clone(),
                    layers: p.bindings.len(),
                    behaviors: p.behaviors.iter().map(|b| b.behavior.clone()).collect(),
                    tags: p.tag_counts(),
                    triggers: p.trigger_groups.iter().map(|g| g.triggers.len()).sum(),
                })
                .collect(),
            scenes: model
                .scenes
                .iter()
                .map(|s| SceneSummary {
                    name: s.name.clone(),
                    fps: s.fps,
                    width: s.width,
                    height: s.height,
                    duration: s.duration,
                    tracks: s.tracks.len(),
                    takes: s.behavior_instances().map(|b| b.takes.len()).sum(),
                })
                .collect(),
            media: model
                .media
                .iter()
                .map(|m| MediaSummary { path: m.path.clone(), kind: format!("{:?}", m.kind), file_name: m.file_name.clone(), in_package: m.file.is_some() })
                .collect(),
            behaviors_installed: model.behaviors.len(),
            store: self.store.summary(),
            problems: model.problems.clone(),
            warnings: self.warnings.clone(),
        })
    }

    // ---- editing --------------------------------------------------------------------------------------------------

    /// Commits `changes` on top of `HEAD` with the current time. Returns the new commit.
    ///
    /// # Errors
    ///
    /// Store errors (limits, bad paths).
    pub fn commit(&mut self, changes: Vec<Change>, message: &str) -> Result<Oid> {
        self.commit_at(changes, message, now(), "+0000")
    }

    /// As [`Project::commit`] with an explicit time (seconds since the epoch) and zone (`+0200`), for reproducible
    /// output.
    ///
    /// # Errors
    ///
    /// Store errors (limits, bad paths).
    pub fn commit_at(&mut self, changes: Vec<Change>, message: &str, time: i64, zone: &str) -> Result<Oid> {
        let head = self.store.head_commit().ok();
        let mut files: std::collections::BTreeMap<String, Vec<u8>> = match head {
            Some(_) => self.store.head_tree()?.into_iter().map(|(k, v)| (k, v.to_vec())).collect(),
            None => std::collections::BTreeMap::new(),
        };
        for change in changes {
            match change {
                Change::Put(path, bytes) => {
                    files.insert(path, bytes);
                }
                Change::Remove(path) => {
                    let folder = format!("{}/", path.trim_end_matches('/'));
                    files.retain(|name, _| *name != path && !name.starts_with(&folder));
                }
            }
        }
        let info = CommitInfo { who: self.author.who(), time, zone: zone.to_owned(), message: message.to_owned() };
        let id = self.store.commit_files(&files, &info, head, "HEAD")?;
        self.committed = true;
        Ok(id)
    }

    /// Rewrites one file at `HEAD` through its parsed table and commits the result. The closure edits the value;
    /// class names stay where they are, so it must not add or remove tables (use [`ClassedDoc::retag`] for that).
    ///
    /// # Errors
    ///
    /// The closure's error, [`Error::Missing`], parse errors, or [`Error::ClassMismatch`].
    pub fn edit_doc(&mut self, path: &str, message: &str, edit: impl FnOnce(&mut ClassedDoc) -> Result<()>) -> Result<Oid> {
        let mut doc = self.doc(path)?;
        edit(&mut doc)?;
        let bytes = doc.to_bytes().map_err(|e| match e {
            Error::ClassMismatch { classes, tables, .. } => Error::ClassMismatch { path: path.to_owned(), classes, tables },
            other => other,
        })?;
        self.commit(vec![Change::Put(path.to_owned(), bytes)], message)
    }

    /// Sets the display name of the puppet file at `path`.
    ///
    /// # Errors
    ///
    /// As [`Project::edit_doc`].
    pub fn rename_puppet(&mut self, path: &str, name: &str) -> Result<Oid> {
        self.edit_doc(path, ".setUiName", |doc| {
            doc.value.set("_sUiName", LValue::string(name))?;
            Ok(())
        })
    }

    /// Overrides one parameter of a behavior bound to a puppet (`_sparseParams`). Only plain values (numbers,
    /// strings, booleans) can be set this way.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when the puppet does not have the behavior or the value is a table, plus the errors of
    /// [`Project::edit_doc`].
    pub fn set_behavior_param(&mut self, puppet_path: &str, behavior: &str, param: &str, value: LValue) -> Result<Oid> {
        if value.as_table().is_some() {
            return Err(Error::malformed(puppet_path, "only plain values can be set as a behavior parameter"));
        }
        self.edit_doc(puppet_path, ".setBehaviorParam", |doc| {
            let items = doc.value.get_mut("_aBoundBehaviors").and_then(|b| b.get_mut("_aIDArray")).and_then(LValue::as_table_mut);
            let bound = items
                .into_iter()
                .flatten()
                .map(|(_, item)| item)
                .filter_map(|item| item.get_mut("v"))
                .find(|v| v.get_path_str("_behaviorPPath/p").is_some_and(|p| p.rsplit('/').next() == Some(behavior)))
                .ok_or_else(|| Error::malformed(puppet_path, format!("the puppet has no {behavior} behavior")))?;
            let params = bound.get_mut("_sparseParams").ok_or_else(|| Error::malformed(puppet_path, "the behavior has no _sparseParams table"))?;
            params.set(param, value)?;
            params.sort_keys();
            Ok(())
        })
    }

    /// Applies a tag to one layer binding of a puppet. `user_set` is the flag stored beside the tag (see
    /// [`crate::model::puppet::Tag::user_set`]); use `true` for a tag added by hand.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when the binding does not exist, plus the errors of [`Project::edit_doc`].
    pub fn set_layer_tag(&mut self, puppet_path: &str, binding_id: &str, tag: &str, user_set: bool) -> Result<Oid> {
        self.edit_doc(puppet_path, ".setLayerTag", |doc| {
            let items = doc.value.get_mut("_idaBindings").and_then(|b| b.get_mut("_aIDArray")).and_then(LValue::as_table_mut);
            let binding = items
                .into_iter()
                .flatten()
                .map(|(_, item)| item)
                .find(|item| item.get_path_str("id") == Some(binding_id))
                .and_then(|item| item.get_mut("v"))
                .ok_or_else(|| Error::malformed(puppet_path, format!("no binding {binding_id}")))?;
            let tags = binding.get_mut("_tTags").ok_or_else(|| Error::malformed(puppet_path, "the binding has no _tTags table"))?;
            tags.set(tag, LValue::Bool(user_set))?;
            tags.sort_keys();
            Ok(())
        })
    }

    /// Removes a tag from one layer binding of a puppet.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when the binding or the tag does not exist, plus the errors of [`Project::edit_doc`].
    pub fn remove_layer_tag(&mut self, puppet_path: &str, binding_id: &str, tag: &str) -> Result<Oid> {
        self.edit_doc(puppet_path, ".removeLayerTag", |doc| {
            let items = doc.value.get_mut("_idaBindings").and_then(|b| b.get_mut("_aIDArray")).and_then(LValue::as_table_mut);
            let tags = items
                .into_iter()
                .flatten()
                .map(|(_, item)| item)
                .find(|item| item.get_path_str("id") == Some(binding_id))
                .and_then(|item| item.get_mut("v"))
                .and_then(|v| v.get_mut("_tTags"))
                .ok_or_else(|| Error::malformed(puppet_path, format!("no binding {binding_id} with tags")))?;
            tags.remove(tag).ok_or_else(|| Error::malformed(puppet_path, format!("binding {binding_id} has no tag {tag}")))?;
            Ok(())
        })
    }

    /// Adds (or replaces) a file in `Ch Media/Gathered Media/`. Media is not part of the store, so no commit is
    /// made; the file is written by the next save.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when `file_name` has a path in it.
    pub fn add_media(&mut self, file_name: &str, bytes: Vec<u8>) -> Result<()> {
        if file_name.is_empty() || file_name.contains(['/', '\\', '\0']) || file_name == ".." || file_name == "." {
            return Err(Error::malformed(file_name, "a media file name cannot have a path in it"));
        }
        self.package.set(&format!("Ch Media/Gathered Media/{file_name}"), bytes);
        Ok(())
    }

    // ---- saving ---------------------------------------------------------------------------------------------------

    /// Moves `main` to `HEAD` when this session committed, then appends the session's records to the rope bytes.
    fn flush_log(&mut self) {
        if self.committed
            && let Some(head) = self.store.resolve_ref("HEAD")
            && self.store.resolve_ref("refs/heads/main") != Some(head)
        {
            self.store.set_ref("refs/heads/main", RefTarget::Direct(head));
        }
        self.rope.extend(encode_log_tail(&self.store, self.log_mark));
        self.log_mark = self.store.log.len();
        self.committed = false;
    }

    fn manifest_bytes(&self) -> Option<Vec<u8>> {
        self.manifest.as_ref().map(ArchiveInfo::to_bytes)
    }

    /// Saves to the file or folder the project was opened from.
    ///
    /// A zip is rewritten through a temporary file. A folder is updated in place: the new records are appended to
    /// `projectdata.rope` and only edited files are written, so large media is never copied.
    ///
    /// # Errors
    ///
    /// [`Error::Missing`] when the project has no path yet (use [`Project::save_puppet`] or
    /// [`Project::save_folder`]), plus I/O and container errors.
    pub fn save(&mut self) -> Result<()> {
        let path = self.path.clone().ok_or_else(|| Error::Missing("a save path (the project was never opened or saved)".to_owned()))?;
        if path.is_dir() { self.save_folder_in_place(&path) } else { self.save_puppet(&path) }
    }

    /// Saves as a `.puppet` zip at `path` (replacing it atomically) and continues to work on that file.
    ///
    /// # Errors
    ///
    /// I/O and container errors.
    pub fn save_puppet(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        self.flush_log();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("project.puppet");
        let temp = path.with_file_name(format!(".{name}.saving"));
        let manifest = self.manifest_bytes();
        let written = (|| -> Result<()> {
            let file = fs::File::create(&temp).map_err(|e| Error::io(format!("creating {}", temp.display()), e))?;
            let mut writer = ZipWriter::new(BufWriter::new(file));
            self.package.write_to(&mut writer, &self.rope, manifest.as_deref())?;
            let mut inner = writer.finish()?;
            inner.flush().map_err(|e| Error::io(format!("flushing {}", temp.display()), e))
        })();
        if let Err(error) = written {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        fs::rename(&temp, path).map_err(|e| Error::io(format!("replacing {}", path.display()), e))?;
        self.reopen(path)
    }

    /// Saves as a project folder at `dir` (created if needed), copying every file, and continues to work there.
    ///
    /// # Errors
    ///
    /// I/O and container errors.
    pub fn save_folder(&mut self, dir: impl AsRef<Path>) -> Result<()> {
        let dir = dir.as_ref();
        self.flush_log();
        let manifest = self.manifest_bytes();
        let mut writer = DirectoryWriter::create(dir)?;
        self.package.write_to(&mut writer, &self.rope, manifest.as_deref())?;
        writer.finish()?;
        self.rope_disk_len = self.rope.len();
        self.reopen(dir)
    }

    fn save_folder_in_place(&mut self, root: &Path) -> Result<()> {
        self.flush_log();
        let rope_name = self.package.rope_name().ok_or_else(|| Error::Missing("projectdata.rope".to_owned()))?.to_owned();
        let rope_path = root.join(rope_name.split('/').collect::<PathBuf>());
        let on_disk = fs::metadata(&rope_path).ok().and_then(|m| usize::try_from(m.len()).ok());
        match (on_disk, self.rope.get(self.rope_disk_len..)) {
            (Some(len), Some(tail)) if len == self.rope_disk_len => {
                if !tail.is_empty() {
                    let mut file =
                        fs::OpenOptions::new().append(true).open(&rope_path).map_err(|e| Error::io(format!("appending to {}", rope_path.display()), e))?;
                    file.write_all(tail).map_err(|e| Error::io(format!("appending to {}", rope_path.display()), e))?;
                }
            }
            _ => write_atomic(&rope_path, &self.rope)?,
        }
        if let Some(bytes) = self.manifest_bytes() {
            write_atomic(&root.join(crate::package::MANIFEST_FILE), &bytes)?;
        }
        for (name, bytes) in self.package.modified() {
            if !is_safe_relative(name) {
                return Err(Error::malformed(name, "unsafe file name"));
            }
            write_atomic(&root.join(name.split('/').collect::<PathBuf>()), bytes)?;
        }
        self.rope_disk_len = self.rope.len();
        self.reopen(root)
    }

    /// Points the package at what was just written, so lazy reads see the new files.
    fn reopen(&mut self, path: &Path) -> Result<()> {
        let container = Container::open(path)?;
        self.package = Package::from_container(container);
        self.path = Some(path.to_path_buf());
        self.rope_disk_len = self.rope.len();
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(format!("creating {}", parent.display()), e))?;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let temp = path.with_file_name(format!(".{name}.saving"));
    fs::write(&temp, bytes).map_err(|e| Error::io(format!("writing {}", temp.display()), e))?;
    fs::rename(&temp, path).map_err(|e| Error::io(format!("replacing {}", path.display()), e))
}

/// A puppet at a glance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PuppetSummary {
    /// Store path.
    pub path: String,
    /// Display name.
    pub name: String,
    /// A rig part rather than a top-level puppet.
    pub is_part: bool,
    /// Guid.
    pub guid: Option<String>,
    /// Number of layer bindings.
    pub layers: usize,
    /// Behavior ids.
    pub behaviors: Vec<String>,
    /// Tags in use and how many layers carry them.
    pub tags: Vec<(String, usize)>,
    /// Number of triggers.
    pub triggers: usize,
}

/// A scene at a glance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneSummary {
    /// Display name.
    pub name: String,
    /// Frames per second.
    pub fps: f64,
    /// Width in pixels.
    pub width: i64,
    /// Height in pixels.
    pub height: i64,
    /// Length in seconds.
    pub duration: f64,
    /// Number of tracks.
    pub tracks: usize,
    /// Number of recorded takes.
    pub takes: usize,
}

/// A media item at a glance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaSummary {
    /// Store path.
    pub path: String,
    /// Artwork, Audio or Other.
    pub kind: String,
    /// The file name.
    pub file_name: Option<String>,
    /// The file is inside the package.
    pub in_package: bool,
}

/// A JSON-friendly description of a whole project, for tools.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectSummary {
    /// Puppet or folder.
    pub kind: ProjectKind,
    /// Name.
    pub name: String,
    /// Where it lives.
    pub path: Option<String>,
    /// The build branch (`190_25.0`).
    pub build: Option<String>,
    /// `rootPuppetPPath` of a `.puppet`.
    pub root_puppet: Option<String>,
    /// `sUniqueID` of a `.puppet`.
    pub unique_id: Option<String>,
    /// The working tip.
    pub head: Option<Oid>,
    /// The saved tip.
    pub main: Option<Oid>,
    /// Commits made this session that a save would persist.
    pub unsaved_commits: bool,
    /// Puppets.
    pub puppets: Vec<PuppetSummary>,
    /// Scenes.
    pub scenes: Vec<SceneSummary>,
    /// Media.
    pub media: Vec<MediaSummary>,
    /// How many `$Behaviors/` definitions are installed.
    pub behaviors_installed: usize,
    /// The store.
    pub store: StoreSummary,
    /// Files that could not be decoded.
    pub problems: Vec<String>,
    /// Notes from opening.
    pub warnings: Vec<String>,
}
