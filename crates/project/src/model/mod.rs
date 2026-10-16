//! The typed view of a project: puppets, scenes, media and behavior definitions decoded from the files at `HEAD`.
//!
//! Decoding never fails as a whole. A file that cannot be understood is listed in [`Model::problems`] and
//! everything else is still decoded. The view is read-only; edits go through [`crate::Project`], which changes the
//! stored text and keeps every table it does not understand.

pub mod behavior;
pub mod media;
pub mod puppet;
pub mod scene;
mod tree;
mod util;

use std::collections::BTreeMap;

use charcraft_rope::class_name;
use serde::Serialize;

use crate::classed::ClassedDoc;
use crate::package::Package;

pub use behavior::{BehaviorDef, ParamDef, TagDef};
pub use media::{Bounds, MediaFile, MediaItem, MediaKind};
pub use puppet::{
    Binding, BindingKind, BoundBehavior, Handle, HotTake, KeyTrigger, Location, PuppetItem, ReplacementKind, ReplacementSet, Tag, Transform, Trigger,
    TriggerGroup,
};
pub use scene::{BehaviorInstance, Keyframe, KeyframeBucket, Marker, RecordingRef, Scene, Take, TimeItem, Track, TrackItem};
pub use tree::{LayerKind, LayerNode};

/// A recorded event graph (`.arf`) kept in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordingFile {
    /// The store path (`.Recordings/Recordings/2024-07-10/take 01_44_11 pm.arf`).
    pub path: String,
    /// The class (`EventGraphFile_000`).
    pub class: String,
    /// Size of the stored text in bytes.
    pub size: usize,
    /// The path the alias recorded on the author's machine.
    pub alias_path: Option<String>,
    /// The recording as found in `Ch Media/`.
    pub file: Option<MediaFile>,
}

/// Everything decoded from a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Model {
    /// Every puppet file, parts included.
    pub puppets: Vec<PuppetItem>,
    /// Scenes of a project (empty for a `.puppet`).
    pub scenes: Vec<Scene>,
    /// Artwork and audio items.
    pub media: Vec<MediaItem>,
    /// The behavior definitions in `$Behaviors/`.
    pub behaviors: Vec<BehaviorDef>,
    /// Event graphs recorded in scenes.
    pub recordings: Vec<RecordingFile>,
    /// The files under `Ch Media/` in the package.
    pub media_files: Vec<MediaFile>,
    /// Files at `HEAD` that are neither of the above (UI state, folder data, markers).
    pub other_files: usize,
    /// Files that could not be decoded, with the reason.
    pub problems: Vec<String>,
}

fn scene_dir(path: &str) -> Option<&str> {
    path.strip_suffix("/_sceneData")
}

impl Model {
    /// Decodes the files of a tree. `package` supplies the media files next to the store.
    #[must_use]
    pub fn decode(files: &BTreeMap<String, &[u8]>, package: &Package) -> Self {
        let mut model = Self::default();
        let mut on_disk = media::MediaIndex::default();
        for entry in package.files() {
            if entry.name().starts_with("Ch Media/") {
                let file = MediaFile { path: entry.name().to_owned(), size: entry.size() };
                on_disk.insert(file.clone());
                model.media_files.push(file);
            }
        }
        let mut scenes: BTreeMap<String, Scene> = BTreeMap::new();
        for (path, blob) in files {
            let Some(class) = class_name(blob) else {
                model.other_files = model.other_files.saturating_add(1);
                continue;
            };
            let class = class.to_owned();
            let in_items = path.starts_with("ProjectItems/");
            let is_media = media::media_kind(&class);
            let wanted = class == "Behavior"
                || (in_items && class == "Puppet")
                || class == "SceneData"
                || is_media.is_some()
                || (class == "EventGraphFile_000" && path.starts_with(".Recordings/"));
            if !wanted {
                model.other_files = model.other_files.saturating_add(1);
                continue;
            }
            let doc = match ClassedDoc::parse(blob) {
                Ok(doc) => doc,
                Err(error) => {
                    model.problems.push(format!("{path}: {error}"));
                    continue;
                }
            };
            if class == "Behavior" && path.starts_with("$Behaviors/") {
                model.behaviors.push(behavior::decode(path, &doc.value));
            } else if class == "Puppet" {
                model.puppets.push(puppet::decode(path, &doc));
            } else if class == "SceneData" {
                if let Some(dir) = scene_dir(path) {
                    scenes.insert(dir.to_owned(), scene::decode_scene(dir, &doc.value));
                }
            } else if let Some(kind) = is_media {
                model.media.push(media::decode(path, &class, kind, &doc.value, &on_disk));
            } else {
                let (alias_path, file_name) = media::alias_of(&doc.value);
                let file = on_disk.find(alias_path.as_deref(), file_name.as_deref()).cloned();
                model.recordings.push(RecordingFile { path: path.clone(), class, size: blob.len(), alias_path, file });
            }
        }
        model.fill_scenes(files, scenes);
        model
    }

    fn fill_scenes(&mut self, files: &BTreeMap<String, &[u8]>, mut scenes: BTreeMap<String, Scene>) {
        for (path, blob) in files {
            let Some((dir, rest)) = scenes
                .keys()
                .find(|dir| path.starts_with(&format!("{dir}/")))
                .cloned()
                .and_then(|dir| path.strip_prefix(&format!("{dir}/")).map(|r| (dir, r.to_owned())))
            else {
                continue;
            };
            let Some(scene) = scenes.get_mut(&dir) else { continue };
            let class = class_name(blob);
            if let Some(file) = rest.strip_prefix("_idaTracks/").filter(|f| f.starts_with("idaTracks") && !f.contains('/')) {
                match ClassedDoc::parse(blob) {
                    Ok(doc) => scene.tracks.extend(scene::decode_track(file, &doc.value)),
                    Err(error) => self.problems.push(format!("{path}: {error}")),
                }
            } else if let Some(file) = rest.strip_prefix("_idaMarkers/").filter(|f| f.starts_with("idaMarkers") && !f.contains('/')) {
                if let Ok(doc) = ClassedDoc::parse(blob)
                    && let Some(marker) = scene::decode_marker(file, &doc.value)
                {
                    scene.markers.push(marker);
                }
            } else if class == Some("RecordingRef_001")
                && rest.starts_with("_amKeyframeBuckets/")
                && let Some(bucket_dir) = path.strip_suffix("/recordingRef")
            {
                let prefix = format!("{bucket_dir}/kfba/");
                let buckets = files.keys().filter(|p| p.strip_prefix(&prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))).count();
                match ClassedDoc::parse(blob) {
                    Ok(doc) => scene.recordings.push(scene::decode_recording_ref(path, &doc.value, buckets)),
                    Err(error) => self.problems.push(format!("{path}: {error}")),
                }
            }
        }
        for scene in scenes.values_mut() {
            scene.tracks.sort_by_key(|t| t.index);
        }
        self.scenes = scenes.into_values().collect();
    }

    /// The puppet stored at `path` (`ProjectItems/Hero`).
    #[must_use]
    pub fn puppet(&self, path: &str) -> Option<&PuppetItem> {
        self.puppets.iter().find(|p| p.path == path)
    }

    /// The top-level puppets: the ones directly under `ProjectItems/` (not the rig parts).
    pub fn root_puppets(&self) -> impl Iterator<Item = &PuppetItem> {
        self.puppets.iter().filter(|p| !p.is_part)
    }

    /// A scene by display name.
    #[must_use]
    pub fn scene(&self, name: &str) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.name == name)
    }

    /// The recording a take points to (`/.Recordings/Recordings/2024-07-10/take 01_44_11 pm.arf`; the leading `/` is
    /// optional).
    #[must_use]
    pub fn recording(&self, path: &str) -> Option<&RecordingFile> {
        let path = path.trim_start_matches('/');
        self.recordings.iter().find(|r| r.path == path)
    }

    /// A behavior definition by id (`Adobe.Blinker.js`).
    #[must_use]
    pub fn behavior(&self, id: &str) -> Option<&BehaviorDef> {
        self.behaviors.iter().find(|b| b.id == id)
    }

    /// The layer tree of the puppet at `path`: its layers and child puppets, resolved recursively.
    #[must_use]
    pub fn layer_tree(&self, path: &str) -> Option<LayerNode> {
        tree::build(self, path)
    }

    /// The parameter value a behavior has on a puppet: the puppet's own override, or the behavior's default.
    #[must_use]
    pub fn effective_param(&self, puppet_path: &str, behavior: &str, param: &str) -> Option<charcraft_ltab::LValue> {
        let bound = self.puppet(puppet_path)?.behavior(behavior);
        if let Some((_, value)) = bound.and_then(|b| b.params.iter().find(|(k, _)| k == param)) {
            return Some(value.clone());
        }
        self.behavior(behavior)?.param(param).map(|p| p.default.clone())
    }
}
