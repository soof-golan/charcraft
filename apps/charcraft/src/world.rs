//! What the document refers to but does not hold: the puppets with their pictures, the recorded sound and where
//! everything came from.
//!
//! [`World::open`] loads a `.puppet`, a `.chproj` (project file or folder), a PSD, AI or SVG file, or a saved
//! `.charcraft` document, and builds the matching [`RecordDoc`] and rig log. [`World::sync`] replays the document's
//! rig log onto the puppets whenever it changed (an edit, an undo, a redo), keeping a pristine copy of each edited
//! puppet to start over from.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anim_artwork::Limits;
use anim_audio::PcmBuffer;
use anim_core::{FrameRate, Size, Tick, Vec2};
use base64::Engine as _;
use charcraft_behaviors::Rig;
use charcraft_maker::Spec;
use charcraft_project::Project;
use charcraft_puppet::{ImportIssue, Puppet};
use charcraft_record::{Cast, CastMember, ImportOptions, MediaStore, PuppetTrack, RecordDoc, Scene, TakePolicy, import_scene_with, load_sounds, wav_bytes};
use serde::{Deserialize, Serialize};

use crate::demo;
use crate::session::{Delivery, ProjectMeta, RigEdit, RigLog, Skeleton};

/// Most artwork versions (one per re-import) the world keeps for undo.
const MAX_REVISIONS: usize = 8;

/// The cast key of the demo puppet.
pub const DEMO_KEY: &str = "demo";
/// The `format` field of a saved document.
pub const FORMAT: &str = "charcraft-app";
/// Largest saved document read, in bytes.
pub const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024 * 1024;

/// Loading or saving failed.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WorldError(pub String);

impl WorldError {
    fn of(context: &str, e: impl std::fmt::Display) -> Self {
        Self(format!("{context}: {e}"))
    }
}

/// What kind of thing a document was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// The built-in demo puppet.
    Demo,
    /// A `.puppet` file.
    Puppet,
    /// A `.chproj` project.
    Project,
    /// A PSD, AI, SVG or PDF imported as a new puppet.
    Artwork,
    /// A new project with no puppets yet.
    Empty,
    /// Puppets from the Puppet Maker only.
    Made,
}

/// One line of the import warnings list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WarningRow {
    /// The puppet it is about (`None` for the document).
    pub puppet: Option<String>,
    /// A short label for the kind of problem.
    pub kind: String,
    /// The layer path it points at, when it points at one.
    pub layer: Option<String>,
    /// What is wrong.
    pub message: String,
}

/// The puppets, the sound and the origin of the open document.
#[derive(Debug)]
pub struct World {
    /// The puppets by cast key.
    pub cast: Cast,
    /// Recorded and imported sound.
    pub media: MediaStore,
    /// The file the document was made from (`None` for the demo).
    pub source: Option<PathBuf>,
    /// What it is.
    pub kind: SourceKind,
    /// A title for the window.
    pub title: String,
    /// Things that did not load.
    pub warnings: Vec<String>,
    /// Counts up whenever the puppets change, so the renderer knows to take a new copy.
    pub generation: u64,
    /// The artwork file changed on disk since it was read (see [`Self::check_source`]).
    pub stale: bool,
    pristine: BTreeMap<String, CastMember>,
    applied: Vec<RigEdit>,
    /// The artwork version the cast was built from (see [`RigLog::revision`]).
    revision: u32,
    /// The puppets read from the artwork at each version, so undoing a re-import can bring them back.
    snapshots: BTreeMap<u32, BTreeMap<String, CastMember>>,
    /// Every Puppet Maker puppet delivered, whether or not the document has it right now (undo and redo).
    made_members: BTreeMap<String, CastMember>,
    /// The Puppet Maker puppets that are in the cast.
    present_made: BTreeSet<String>,
    source_mtime: Option<SystemTime>,
    issues: Vec<(String, ImportIssue)>,
}

/// A freshly opened document.
#[derive(Debug)]
pub struct Opened {
    /// The puppets and sound.
    pub world: World,
    /// The scene and recording settings.
    pub rec: RecordDoc,
    /// The rig log (empty skeleton edits).
    pub rig: RigLog,
    /// Puppet names and imports (empty for a fresh document).
    pub meta: ProjectMeta,
}

/// A saved document.
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    format: String,
    version: u32,
    source: Option<String>,
    /// What the document was made from (`None` in documents saved before Puppet Maker existed).
    #[serde(default)]
    kind: Option<SourceKind>,
    /// Puppet Maker puppets by cast key: the specs, not the pictures.
    #[serde(default)]
    made: BTreeMap<String, Spec>,
    /// How many times the artwork was re-imported.
    #[serde(default)]
    revision: u32,
    /// The first scene of the project.
    scene: Scene,
    /// The scenes after the first, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    more_scenes: Vec<Scene>,
    /// Which scene was open.
    #[serde(default)]
    active_scene: usize,
    arming: Vec<charcraft_record::ParamGroup>,
    rig: Vec<RigEdit>,
    /// WAV files (32-bit float) by media name, base64.
    media: BTreeMap<String, String>,
    /// Puppet names and the puppets imported from other files.
    #[serde(default)]
    project: ProjectMeta,
}

/// A puppet read from a file, ready to join the cast.
#[derive(Debug)]
pub struct LoadedPuppet {
    /// The cast key.
    pub key: String,
    /// The puppet with its behaviors.
    pub member: CastMember,
    /// The name the file gives it.
    pub name: String,
    /// What did not load.
    pub warnings: Vec<String>,
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default()
}

/// A scene with one puppet standing in the middle, scaled down to fit when it is taller than the stage.
fn single_scene(name: &str, key: &str, member: &CastMember) -> Scene {
    let mut scene = empty_scene(name);
    // Tracks fail only past the track limit.
    let _ = scene.add_track(placed_track(&scene, name, key, member));
    scene
}

/// A scene with the standard stage and no tracks.
fn empty_scene(name: &str) -> Scene {
    let mut scene = Scene::new(name, Size::new(1280.0, 720.0), FrameRate::FPS_30, Tick::from_seconds_f64(8.0));
    scene.background = Some(anim_core::Rgba8 { r: 236, g: 238, b: 244, a: 255 });
    scene
}

/// A track for a puppet, standing in the middle of the stage and scaled down to fit when it is taller than it.
#[must_use]
pub fn placed_track(scene: &Scene, name: &str, key: &str, member: &CastMember) -> PuppetTrack {
    placement_track(name, key, member, scene.size, 0)
}

/// How far apart puppets stand on a stage, in stage pixels.
const TRACK_SPACING: f64 = 330.0;

/// A track with a puppet standing on the stage, scaled down to fit when it is taller than the stage. The first
/// track is in the middle; further ones stand to the right and left of it in turn.
#[must_use]
pub fn placement_track(name: &str, key: &str, member: &CastMember, stage: Size, index: usize) -> PuppetTrack {
    let mut track = PuppetTrack::new(name, key);
    let size = member.puppet.size;
    let fit = if size.height > 0.0 { (stage.height * 0.94 / size.height).min(1.0) } else { 1.0 };
    let steps = u32::try_from(index.div_ceil(2)).map_or(0.0, f64::from);
    let offset = if index.is_multiple_of(2) { -steps * TRACK_SPACING } else { steps * TRACK_SPACING };
    track.placement.scale = Vec2::new(fit, fit);
    track.placement.anchor = Vec2::new(size.width / 2.0, size.height / 2.0) + member.skin - member.handle;
    track.placement.position = Vec2::new(-member.handle.x + offset, -member.handle.y);
    track
}

impl World {
    fn new(cast: Cast, kind: SourceKind, source: Option<PathBuf>, title: String) -> Self {
        Self {
            cast,
            media: MediaStore::new(),
            source,
            kind,
            title,
            warnings: Vec::new(),
            generation: 1,
            stale: false,
            pristine: BTreeMap::new(),
            applied: Vec::new(),
            revision: 0,
            snapshots: BTreeMap::new(),
            made_members: BTreeMap::new(),
            present_made: BTreeSet::new(),
            source_mtime: None,
            issues: Vec::new(),
        }
    }

    fn finish(mut self, scene: Scene) -> Opened {
        let mut rig = RigLog::default();
        let mut base = BTreeMap::new();
        for key in self.cast.keys() {
            if let Some(member) = self.cast.get(key) {
                rig.skeletons.insert(key.to_owned(), Skeleton::of(&member.puppet));
                base.insert(key.to_owned(), member.clone());
            }
        }
        self.snapshots.insert(0, base);
        rig.source = self.source.clone().filter(|_| !matches!(self.kind, SourceKind::Demo | SourceKind::Made));
        self.source_mtime = rig.source.as_deref().and_then(modified);
        self.refresh_issues();
        let mut rec = RecordDoc::new(scene);
        rec.templates = self.cast.templates();
        Opened { world: self, rec, rig, meta: ProjectMeta::default() }
    }

    /// A document with no puppets yet (Puppet Maker puppets are added to it).
    fn blank(title: &str) -> Opened {
        Self::new(Cast::new(), SourceKind::Made, None, title.to_owned()).finish(empty_scene(title))
    }

    /// Adds a Puppet Maker puppet to the cast, the rig log and the templates, and (when asked) to a new track.
    fn include_made(&mut self, rig: &mut RigLog, rec: &mut RecordDoc, key: &str, spec: Spec, add_track: bool) -> Result<(), WorldError> {
        let puppet = charcraft_maker::puppet(&spec).map_err(|e| WorldError::of("building the puppet", e))?;
        let member = CastMember::new(puppet);
        rig.skeletons.insert(key.to_owned(), Skeleton::of(&member.puppet));
        if add_track {
            let track = placement_track(&spec.name, key, &member, rec.scene.size, rec.scene.tracks.len());
            rec.scene.add_track(track).map_err(|e| WorldError::of("adding a track", e))?;
        }
        rec.templates.insert(key.to_owned(), member.template.clone());
        self.cast.insert(key, member.clone());
        self.made_members.insert(key.to_owned(), member);
        self.present_made.insert(key.to_owned());
        rig.made.insert(key.to_owned(), spec);
        self.generation = self.generation.saturating_add(1);
        self.refresh_issues();
        Ok(())
    }

    /// A new document holding one starter puppet of the Puppet Maker.
    ///
    /// # Errors
    /// [`WorldError`] for an unknown template.
    pub fn from_template(id: &str) -> Result<Opened, WorldError> {
        let template = charcraft_maker::template(id).ok_or_else(|| {
            WorldError(format!("no starter puppet {id:?}; the starter puppets are {:?}", charcraft_maker::templates().iter().map(|t| t.id).collect::<Vec<_>>()))
        })?;
        Self::from_spec(template.spec)
    }

    /// A new document holding one Puppet Maker puppet.
    ///
    /// # Errors
    /// [`WorldError`] if the puppet cannot be built.
    pub fn from_spec(spec: Spec) -> Result<Opened, WorldError> {
        let mut opened = Self::blank(&spec.name);
        let key = spec.name.clone();
        opened.world.include_made(&mut opened.rig, &mut opened.rec, &key, spec, true)?;
        Ok(opened)
    }

    /// The built-in demo document.
    ///
    /// # Errors
    /// [`WorldError`] if the demo puppet cannot be built (it can always be built).
    pub fn demo() -> Result<Opened, WorldError> {
        let puppet = demo::puppet().map_err(|e| WorldError::of("building the demo puppet", e))?;
        let member = CastMember::new(puppet);
        let scene = single_scene("Demo", DEMO_KEY, &member);
        let mut cast = Cast::new();
        cast.insert(DEMO_KEY, member);
        Ok(Self::new(cast, SourceKind::Demo, None, "Demo puppet".to_owned()).finish(scene))
    }

    /// A new project with a stage and no puppets: the starting point of File > New Project.
    #[must_use]
    pub fn empty(name: &str) -> Opened {
        Self::new(Cast::new(), SourceKind::Empty, None, name.to_owned()).finish(empty_scene(name))
    }

    /// Reads the root puppets of a `.puppet` or project without opening it as a document.
    ///
    /// # Errors
    /// [`WorldError`] when the file is not a project or holds no puppet that can be built.
    pub fn load_members(path: &Path) -> Result<Vec<LoadedPuppet>, WorldError> {
        let project = Project::open(path).map_err(|e| WorldError::of(&format!("opening {}", path.display()), e))?;
        let model = project.model().map_err(|e| WorldError::of("reading the project", e))?;
        let roots: Vec<(String, String)> = project.manifest().and_then(|m| m.root_puppet_ppath()).map_or_else(
            || model.root_puppets().map(|p| (p.path.trim_start_matches('/').to_owned(), p.display_name().to_owned())).collect(),
            |root| {
                let key = root.trim_start_matches('/').to_owned();
                let name = model.puppet(&key).map_or_else(|| key.clone(), |p| p.display_name().to_owned());
                vec![(key, name)]
            },
        );
        let (cast, mut warnings) = Cast::from_project(&project, &model, roots.iter().map(|(k, _)| k.as_str()));
        let mut out = Vec::new();
        for (key, name) in roots {
            if let Some(member) = cast.get(&key) {
                out.push(LoadedPuppet { key, member: member.clone(), name, warnings: std::mem::take(&mut warnings) });
            }
        }
        if out.is_empty() {
            return Err(WorldError(format!("{} holds no puppet that can be built ({})", path.display(), warnings.join("; "))));
        }
        Ok(out)
    }

    /// Adds a puppet to the cast (a project panel import or duplicate).
    pub fn add_member(&mut self, key: &str, member: CastMember) {
        self.cast.insert(key, member);
        self.generation = self.generation.saturating_add(1);
    }

    /// Opens a file: a puppet, a project, artwork or a saved document.
    ///
    /// # Errors
    /// [`WorldError`] naming the problem.
    pub fn open(path: &Path) -> Result<Opened, WorldError> {
        Self::open_scene(path, None, TakePolicy::default())
    }

    /// Opens a file like [`World::open`]; for a project, `scene` picks the scene (the first by default) and `takes`
    /// says what to do with its recorded takes.
    ///
    /// # Errors
    /// [`WorldError`] naming the problem, such as a scene the project does not have.
    pub fn open_scene(path: &Path, scene: Option<&str>, takes: TakePolicy) -> Result<Opened, WorldError> {
        if path.is_dir() {
            return Self::open_project(path, scene, takes);
        }
        match extension(path).as_str() {
            "charcraft" => Self::open_saved(path),
            "puppet" | "chproj" => Self::open_project(path, scene, takes),
            "psd" | "psb" | "ai" | "svg" | "pdf" | "eps" => Self::open_artwork(path),
            _ => Err(WorldError(format!("{}: not a .puppet, .chproj, .psd, .ai, .svg or .charcraft file", path.display()))),
        }
    }

    fn open_artwork(path: &Path) -> Result<Opened, WorldError> {
        let tree = anim_artwork::import_path(path, &Limits::default()).map_err(|e| WorldError::of(&format!("reading artwork {}", path.display()), e))?;
        let puppet = Puppet::from_artwork(&tree).map_err(|e| WorldError::of("building a puppet from the artwork", e))?;
        let title = path.file_stem().map_or_else(|| "Puppet".to_owned(), |s| s.to_string_lossy().into_owned());
        let member = CastMember::new(puppet);
        let scene = single_scene(&title, &title, &member);
        let mut cast = Cast::new();
        cast.insert(&title, member);
        let mut world = Self::new(cast, SourceKind::Artwork, Some(path.to_owned()), title);
        world.warnings.extend(tree.warnings);
        Ok(world.finish(scene))
    }

    fn open_project(path: &Path, scene: Option<&str>, takes: TakePolicy) -> Result<Opened, WorldError> {
        let project = Project::open(path).map_err(|e| WorldError::of(&format!("opening {}", path.display()), e))?;
        let model = project.model().map_err(|e| WorldError::of("reading the project", e))?;
        let title = path.file_stem().map_or_else(|| "Project".to_owned(), |s| s.to_string_lossy().into_owned());
        let is_puppet = extension(path) == "puppet";
        let mut warnings = Vec::new();
        let imported = if is_puppet {
            None
        } else {
            match import_scene_with(&project, &model, scene, &ImportOptions { takes }) {
                Ok(imported) => Some(imported).filter(|i| !i.scene.tracks.is_empty()),
                // A scene asked for by name is an error when it is not there; the default one may be missing.
                Err(e) if scene.is_some() => return Err(WorldError::of("importing the scene", e)),
                Err(_) => None,
            }
        };
        let (cast, scene, media) = if let Some(imported) = imported {
            warnings.extend(imported.warnings.clone());
            let (cast, cast_warnings) = Cast::from_project(&project, &model, imported.puppets.iter().map(String::as_str));
            warnings.extend(cast_warnings);
            let mut media = MediaStore::new();
            warnings.extend(load_sounds(&project, &imported, &mut media));
            (cast, imported.scene, media)
        } else {
            let paths: Vec<&str> = model.root_puppets().map(|p| p.path.as_str()).collect();
            let (cast, cast_warnings) = Cast::from_project(&project, &model, paths.iter().copied());
            warnings.extend(cast_warnings);
            let key = paths
                .iter()
                .find(|p| cast.get(p).is_some())
                .copied()
                .ok_or_else(|| WorldError(format!("{} holds no puppet that can be built ({})", path.display(), warnings.join("; "))))?;
            let member = cast.get(key).cloned().ok_or_else(|| WorldError("lost the puppet".to_owned()))?;
            let scene = single_scene(&title, key, &member);
            (cast, scene, MediaStore::new())
        };
        let kind = if is_puppet { SourceKind::Puppet } else { SourceKind::Project };
        let mut world = Self::new(cast, kind, Some(path.to_owned()), title);
        world.media = media;
        world.warnings = warnings;
        let mut opened = world.finish(scene);
        // Takes recorded earlier with this app live in a sidecar beside the project.
        let root = if path.is_file() && extension(path) == "chproj" { path.parent().unwrap_or(path) } else { path };
        let sidecar = charcraft_record::sidecar_path(root);
        if let Some(rig) = crate::projectio::load_rig_sidecar(root) {
            match rig {
                Ok(rig) => {
                    opened.rig.edits = rig.edits;
                    opened.meta = rig.meta;
                }
                Err(e) => opened.world.warnings.push(e),
            }
        }
        if sidecar.is_file() {
            match charcraft_record::load(&sidecar) {
                Ok((scene, media)) => {
                    opened.rec.scene = scene;
                    for name in media.names() {
                        if let Some(buffer) = media.get(name) {
                            let _ = opened.world.media.insert(name, buffer.clone());
                        }
                    }
                }
                Err(e) => opened.world.warnings.push(format!("{}: {e}", sidecar.display())),
            }
        }
        // Puppets saved into the project from other files are in it now; the scene may play ones the project's
        // own scene does not.
        opened.meta.imports.clear();
        let wanted: Vec<String> = opened.rec.scene.tracks.iter().map(|t| t.puppet.clone()).filter(|k| opened.world.cast.get(k).is_none()).collect();
        if !wanted.is_empty() {
            let (extra, extra_warnings) = Cast::from_project(&project, &model, wanted.iter().map(String::as_str));
            opened.world.warnings.extend(extra_warnings);
            for key in wanted {
                if let Some(member) = extra.get(&key).cloned() {
                    opened.rig.skeletons.insert(key.clone(), Skeleton::of(&member.puppet));
                    opened.rec.templates.insert(key.clone(), member.template.clone());
                    opened.world.cast.insert(&key, member);
                }
            }
        }
        Ok(opened)
    }

    fn open_saved(path: &Path) -> Result<Opened, WorldError> {
        let size = std::fs::metadata(path).map_err(|e| WorldError::of(&path.display().to_string(), e))?.len();
        if size > MAX_DOCUMENT_BYTES {
            return Err(WorldError(format!("{} is larger than {MAX_DOCUMENT_BYTES} bytes", path.display())));
        }
        let text = std::fs::read_to_string(path).map_err(|e| WorldError::of(&path.display().to_string(), e))?;
        let saved: Saved = serde_json::from_str(&text).map_err(|e| WorldError::of("reading the document", e))?;
        if saved.format != FORMAT {
            return Err(WorldError(format!("{} is not a CharCraft document", path.display())));
        }
        let mut opened = match saved.source.as_deref().map(Path::new).filter(|p| p.exists()) {
            Some(source) => Self::open(source)?,
            None if saved.kind == Some(SourceKind::Made) => Self::blank("Puppet Maker"),
            None => Self::demo()?,
        };
        let mut all = vec![saved.scene];
        all.extend(saved.more_scenes);
        for scene in &mut all {
            scene.sanitize().map_err(|e| WorldError::of("checking the scene", e))?;
        }
        let templates = std::mem::take(&mut opened.rec.templates);
        opened.rec = RecordDoc::with_scenes(all, saved.active_scene).map_err(|e| WorldError::of("checking the scenes", e))?;
        opened.rec.templates = templates;
        let Opened { world, rec, rig, .. } = &mut opened;
        for (key, spec) in saved.made {
            world.include_made(rig, rec, &key, spec, false)?;
        }
        if saved.revision != 0 {
            // The artwork file read just now is the version the document was saved with.
            if let Some(base) = world.snapshots.remove(&0) {
                world.snapshots.insert(saved.revision, base);
            }
            world.revision = saved.revision;
            rig.revision = saved.revision;
        }
        opened.rec.arming.groups = saved.arming.into_iter().collect();
        for (name, data) in saved.media {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| WorldError::of(&format!("sound {name}"), e))
                .and_then(|bytes| anim_audio::decode_bytes(&bytes, Some(&name)).map_err(|e| WorldError::of(&format!("sound {name}"), e)));
            match decoded.and_then(|buffer| opened.world.media.insert(&name, buffer).map_err(|e| WorldError::of(&name, e))) {
                Ok(()) => {}
                Err(e) => opened.world.warnings.push(e.to_string()),
            }
        }
        opened.rig.edits = saved.rig;
        opened.meta = saved.project;
        // Puppets imported from other files are read again from where they came from.
        for import in opened.meta.imports.clone() {
            match Self::load_members(Path::new(&import.path)) {
                Ok(members) => {
                    for loaded in members.into_iter().filter(|m| m.key == import.key) {
                        opened.rig.skeletons.insert(loaded.key.clone(), Skeleton::of(&loaded.member.puppet));
                        opened.rec.templates.insert(loaded.key.clone(), loaded.member.template.clone());
                        opened.world.cast.insert(&loaded.key, loaded.member);
                    }
                }
                Err(e) => opened.world.warnings.push(format!("imported puppet {}: {e}", import.key)),
            }
        }
        opened.world.title = path.file_stem().map_or_else(|| "Document".to_owned(), |s| s.to_string_lossy().into_owned());
        Ok(opened)
    }

    /// The document as the bytes of a `.charcraft` file.
    ///
    /// # Errors
    /// [`WorldError`] if the document cannot be written as JSON.
    pub fn save_bytes(&self, rec: &RecordDoc, rig: &RigLog) -> Result<Vec<u8>, WorldError> {
        self.save_bytes_with(rec, rig, &ProjectMeta::default())
    }

    /// As [`Self::save_bytes`], with the puppet names and imports of the document.
    ///
    /// # Errors
    /// [`WorldError`] if the document cannot be written as JSON.
    pub fn save_bytes_with(&self, rec: &RecordDoc, rig: &RigLog, meta: &ProjectMeta) -> Result<Vec<u8>, WorldError> {
        let wanted: std::collections::BTreeSet<&str> = rec.scenes().flat_map(|s| &s.audio).flat_map(|t| &t.clips).map(|c| c.media.as_str()).collect();
        let mut scenes = rec.scenes().cloned();
        let first = scenes.next().unwrap_or_else(|| rec.scene.clone());
        let media = wanted
            .into_iter()
            .filter_map(|name| self.media.get(name).map(|b: &PcmBuffer| (name.to_owned(), base64::engine::general_purpose::STANDARD.encode(wav_bytes(b)))))
            .collect();
        let saved = Saved {
            format: FORMAT.to_owned(),
            version: 1,
            source: self.source.as_ref().map(|p| p.to_string_lossy().into_owned()),
            kind: Some(self.kind),
            made: rig.made.clone(),
            revision: rig.revision,
            scene: first,
            more_scenes: scenes.collect(),
            active_scene: rec.active_scene(),
            arming: rec.arming.groups.iter().copied().collect(),
            rig: rig.edits.clone(),
            media,
            project: meta.clone(),
        };
        serde_json::to_vec_pretty(&saved).map_err(|e| WorldError::of("writing the document", e))
    }

    /// The cast key of the puppet a track plays, or the first puppet.
    #[must_use]
    pub fn key_for_track(&self, rec: &RecordDoc, track: usize) -> Option<String> {
        rec.scene.tracks.get(track).map(|t| t.puppet.clone()).filter(|k| self.cast.get(k).is_some()).or_else(|| self.cast.keys().next().map(str::to_owned))
    }

    fn update(&mut self, key: &str, change: impl FnOnce(&mut CastMember)) {
        if let Some(mut member) = self.cast.get(key).cloned() {
            change(&mut member);
            self.cast.insert(key, member);
        }
    }

    /// Takes in a puppet a command built.
    pub fn deliver(&mut self, delivery: Delivery) {
        match delivery {
            Delivery::Made { key, member } => {
                self.made_members.insert(key, *member);
            }
            Delivery::Reimport { from, revision, cast, warnings } => {
                let mut base = self.snapshots.get(&from).cloned().unwrap_or_else(|| {
                    self.cast.keys().filter(|k| !self.present_made.contains(*k)).filter_map(|k| self.cast.get(k).map(|m| (k.to_owned(), m.clone()))).collect()
                });
                for key in cast.keys() {
                    if let Some(member) = cast.get(key) {
                        base.insert(key.to_owned(), member.clone());
                    }
                }
                self.snapshots.insert(revision, base);
                while self.snapshots.len() > MAX_REVISIONS {
                    // Undo reaches back this far and no further.
                    let oldest = self.snapshots.keys().copied().find(|v| *v != revision && *v != self.revision);
                    match oldest {
                        Some(v) => self.snapshots.remove(&v),
                        None => break,
                    };
                }
                self.warnings.extend(warnings);
                self.source_mtime = self.source.as_deref().and_then(modified);
                self.stale = false;
            }
        }
    }

    /// Looks at the artwork file's modification time. True when it changed since the artwork was read.
    pub fn check_source(&mut self) -> bool {
        if matches!(self.kind, SourceKind::Demo | SourceKind::Made) {
            return false;
        }
        let now = self.source.as_deref().and_then(modified);
        if now.is_some() && now != self.source_mtime {
            self.stale = true;
        }
        self.stale
    }

    /// Records that the artwork file as it is now has been read.
    pub fn acknowledge_source(&mut self) {
        self.source_mtime = self.source.as_deref().and_then(modified);
        self.stale = false;
    }

    fn refresh_issues(&mut self) {
        self.issues = self
            .cast
            .keys()
            .filter_map(|k| self.cast.get(k).map(|m| (k, m)))
            .flat_map(|(k, m)| m.puppet.import_report().into_iter().map(move |i| (k.to_owned(), i)))
            .collect();
    }

    /// The import warnings: what the loaders reported and what the layer names and structure of each puppet give
    /// away (duplicate names, a face without eyes, handle tags on plain groups, ...).
    #[must_use]
    pub fn warning_rows(&self) -> Vec<WarningRow> {
        let mut rows: Vec<WarningRow> = self
            .warnings
            .iter()
            .filter(|w| !self.issues.iter().any(|(_, i)| &i.message == *w))
            .map(|w| WarningRow { puppet: None, kind: "Document".to_owned(), layer: None, message: w.clone() })
            .collect();
        rows.extend(self.issues.iter().map(|(k, i)| WarningRow {
            puppet: Some(k.clone()),
            kind: i.kind.label().to_owned(),
            layer: i.layer.clone(),
            message: i.message.clone(),
        }));
        rows
    }

    /// Rebuilds the cast when the document moved to another artwork version or has other Puppet Maker puppets.
    fn reconcile(&mut self, rig: &RigLog) -> bool {
        let wanted: BTreeSet<String> = rig.made.keys().cloned().collect();
        if rig.revision == self.revision && wanted == self.present_made {
            return false;
        }
        let base = if let Some(base) = self.snapshots.get(&rig.revision) {
            base.clone()
        } else {
            self.warnings.push(format!("artwork version {} is no longer kept; the puppets stay as they are", rig.revision));
            self.cast.keys().filter(|k| !self.present_made.contains(*k)).filter_map(|k| self.cast.get(k).map(|m| (k.to_owned(), m.clone()))).collect()
        };
        let mut cast = Cast::new();
        for (key, member) in base {
            if !wanted.contains(&key) {
                cast.insert(&key, member);
            }
        }
        for key in &wanted {
            if let Some(member) = self.made_members.get(key) {
                cast.insert(key, member.clone());
            }
        }
        self.cast = cast;
        self.present_made = wanted.into_iter().filter(|k| self.cast.get(k).is_some()).collect();
        self.revision = rig.revision;
        self.pristine.clear();
        self.applied.clear();
        true
    }

    /// Brings the puppets in line with the rig log. Returns whether anything changed.
    pub fn sync(&mut self, rig: &RigLog) -> bool {
        let rebuilt = self.reconcile(rig);
        let extends = rig.edits.len() >= self.applied.len() && rig.edits.iter().zip(&self.applied).all(|(a, b)| a == b);
        if extends && rig.edits.len() == self.applied.len() {
            if rebuilt {
                self.generation = self.generation.saturating_add(1);
                self.refresh_issues();
            }
            return rebuilt;
        }
        if !extends {
            let touched: Vec<String> = self.applied.iter().map(|e| e.puppet.clone()).collect();
            for key in touched {
                if let Some(original) = self.pristine.get(&key).cloned() {
                    self.cast.insert(&key, original);
                }
            }
            self.applied.clear();
        }
        let tail: Vec<RigEdit> = rig.edits.iter().skip(self.applied.len()).cloned().collect();
        for edit in tail {
            if !self.pristine.contains_key(&edit.puppet)
                && let Some(member) = self.cast.get(&edit.puppet)
            {
                self.pristine.insert(edit.puppet.clone(), member.clone());
            }
            let mut failure = None;
            self.update(&edit.puppet, |member| {
                failure = member.puppet.apply(&edit.op).err();
                member.template.set_rig(Rig::from_puppet(&member.puppet));
            });
            if let Some(e) = failure {
                self.warnings.push(format!("rig edit on {} failed: {e}", edit.puppet));
            }
            self.applied.push(edit);
        }
        self.generation = self.generation.saturating_add(1);
        self.refresh_issues();
        true
    }
}

/// When a file or folder was last changed.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use charcraft_puppet::PuppetOp;

    use super::*;

    #[test]
    fn the_demo_opens_with_one_track() {
        let opened = World::demo().unwrap();
        assert_eq!(opened.rec.scene.tracks.len(), 1);
        assert_eq!(opened.world.kind, SourceKind::Demo);
        assert!(opened.rig.skeletons.contains_key(DEMO_KEY));
        assert!(opened.rec.templates.contains_key(DEMO_KEY));
    }

    #[test]
    fn rig_edits_replay_and_undo_from_the_pristine_puppet() {
        let Opened { mut world, mut rig, .. } = World::demo().unwrap();
        let head_tags =
            |w: &World| w.cast.get(DEMO_KEY).and_then(|m| m.puppet.find_path("Head").and_then(|id| m.puppet.layer(id)).map(|l| l.tags.len())).unwrap();
        let before = head_tags(&world);
        rig.edits.push(RigEdit { puppet: DEMO_KEY.into(), op: PuppetOp::AddTag { layer: "Head".into(), tag: "Custom.Tag".into() } });
        assert!(world.sync(&rig));
        assert_eq!(head_tags(&world), before + 1);
        assert!(!world.sync(&rig), "nothing new to apply");
        rig.edits.clear();
        assert!(world.sync(&rig), "undo");
        assert_eq!(head_tags(&world), before);
    }

    #[test]
    fn a_document_survives_save_and_open() {
        let Opened { world, rec, mut rig, .. } = World::demo().unwrap();
        rig.edits.push(RigEdit { puppet: DEMO_KEY.into(), op: PuppetOp::SetVisible { layer: "Head/Nose".into(), visible: false } });
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.charcraft");
        std::fs::write(&file, world.save_bytes(&rec, &rig).unwrap()).unwrap();
        let back = World::open(&file).unwrap();
        assert_eq!(back.rec.scene, rec.scene);
        assert_eq!(back.rig.edits, rig.edits);
        assert!(World::open(&dir.path().join("junk.txt")).is_err());
    }

    #[test]
    fn several_scenes_camera_and_keyframes_survive_save_and_open() {
        let Opened { world, mut rec, rig, .. } = World::demo().unwrap();
        let mut second = rec.scene.clone();
        second.name = "Second".into();
        rec.add_scene(second, None).unwrap();
        rec.select_scene(1).unwrap();
        rec.scene.camera.zoom = 2.0;
        rec.scene.camera.lanes.push({
            let mut lane = charcraft_record::Lane::new("zoom");
            lane.set_key(anim_core::Tick::ZERO, 1.0, None).unwrap();
            lane.set_key(anim_core::Tick::from_seconds_f64(1.0), 3.0, Some(charcraft_record::Ease::EaseInOut)).unwrap();
            lane
        });
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("multi.charcraft");
        std::fs::write(&file, world.save_bytes(&rec, &rig).unwrap()).unwrap();
        let back = World::open(&file).unwrap();
        assert_eq!(back.rec.scene_count(), 2);
        assert_eq!(back.rec.active_scene(), 1);
        assert_eq!(back.rec.scene, rec.scene);
        assert_eq!(back.rec.scene_at(0), rec.scene_at(0));
    }

    #[test]
    fn hostile_documents_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [("a.charcraft", "{"), ("b.charcraft", r#"{"format":"x","version":1}"#), ("c.charcraft", "[]")] {
            let file = dir.path().join(name);
            std::fs::write(&file, text).unwrap();
            assert!(World::open(&file).is_err(), "{name}");
        }
    }
}
