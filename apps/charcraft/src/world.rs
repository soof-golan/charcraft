//! What the document refers to but does not hold: the puppets with their pictures, the recorded sound and where
//! everything came from.
//!
//! [`World::open`] loads a `.puppet`, a `.chproj` (project file or folder), a PSD, AI or SVG file, or a saved
//! `.charcraft` document, and builds the matching [`RecordDoc`] and rig log. [`World::sync`] replays the document's
//! rig log onto the puppets whenever it changed (an edit, an undo, a redo), keeping a pristine copy of each edited
//! puppet to start over from.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anim_artwork::Limits;
use anim_audio::PcmBuffer;
use anim_core::{FrameRate, Size, Tick, Vec2};
use base64::Engine as _;
use charcraft_behaviors::Rig;
use charcraft_project::Project;
use charcraft_puppet::Puppet;
use charcraft_record::{Cast, CastMember, ImportOptions, MediaStore, PuppetTrack, RecordDoc, Scene, TakePolicy, import_scene_with, load_sounds, wav_bytes};
use serde::{Deserialize, Serialize};

use crate::demo;
use crate::session::{RigEdit, RigLog, Skeleton};

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
    pristine: BTreeMap<String, CastMember>,
    applied: Vec<RigEdit>,
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
}

/// A saved document.
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    format: String,
    version: u32,
    source: Option<String>,
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
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default()
}

/// A scene with one puppet standing in the middle, scaled down to fit when it is taller than the stage.
fn single_scene(name: &str, key: &str, member: &CastMember) -> Scene {
    let mut scene = Scene::new(name, Size::new(1280.0, 720.0), FrameRate::FPS_30, Tick::from_seconds_f64(8.0));
    let mut track = PuppetTrack::new(name, key);
    let size = member.puppet.size;
    let fit = if size.height > 0.0 { (scene.size.height * 0.94 / size.height).min(1.0) } else { 1.0 };
    track.placement.scale = Vec2::new(fit, fit);
    track.placement.anchor = Vec2::new(size.width / 2.0, size.height / 2.0) + member.skin - member.handle;
    track.placement.position = Vec2::new(-member.handle.x, -member.handle.y);
    scene.background = Some(anim_core::Rgba8 { r: 236, g: 238, b: 244, a: 255 });
    // Tracks fail only past the track limit.
    let _ = scene.add_track(track);
    scene
}

impl World {
    fn new(cast: Cast, kind: SourceKind, source: Option<PathBuf>, title: String) -> Self {
        Self { cast, media: MediaStore::new(), source, kind, title, warnings: Vec::new(), generation: 1, pristine: BTreeMap::new(), applied: Vec::new() }
    }

    fn finish(self, scene: Scene) -> Opened {
        let mut rig = RigLog::default();
        for key in self.cast.keys() {
            if let Some(member) = self.cast.get(key) {
                rig.skeletons.insert(key.to_owned(), Skeleton::of(&member.puppet));
            }
        }
        let mut rec = RecordDoc::new(scene);
        rec.templates = self.cast.templates();
        Opened { world: self, rec, rig }
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
        let sidecar = charcraft_record::sidecar_path(path);
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
        opened.world.title = path.file_stem().map_or_else(|| "Document".to_owned(), |s| s.to_string_lossy().into_owned());
        Ok(opened)
    }

    /// The document as the bytes of a `.charcraft` file.
    ///
    /// # Errors
    /// [`WorldError`] if the document cannot be written as JSON.
    pub fn save_bytes(&self, rec: &RecordDoc, rig: &RigLog) -> Result<Vec<u8>, WorldError> {
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
            scene: first,
            more_scenes: scenes.collect(),
            active_scene: rec.active_scene(),
            arming: rec.arming.groups.iter().copied().collect(),
            rig: rig.edits.clone(),
            media,
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

    /// Brings the puppets in line with the rig log. Returns whether anything changed.
    pub fn sync(&mut self, rig: &RigLog) -> bool {
        let extends = rig.edits.len() >= self.applied.len() && rig.edits.iter().zip(&self.applied).all(|(a, b)| a == b);
        if extends && rig.edits.len() == self.applied.len() {
            return false;
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
        true
    }
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
        let Opened { world, rec, mut rig } = World::demo().unwrap();
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
        let Opened { world, mut rec, rig } = World::demo().unwrap();
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
