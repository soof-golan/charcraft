//! Writing the document as a Character Animator project or as one `.puppet`.
//!
//! The project crate keeps a project lossless, so a saved project is the opened one (copied to the new folder with
//! every record kept) plus what this app adds: puppets imported from other files are copied in, renamed puppets get
//! their new display name, and the scene (tracks, takes, audio clips), the rig edits and the sounds go to sidecar files
//! beside the folder, because the project's own take files are read-only to us.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use charcraft_project::Project;
use charcraft_record::sidecar_path;
use serde::{Deserialize, Serialize};

use crate::session::{AppDoc, ProjectMeta, RigEdit};
use crate::world::{SourceKind, World};

/// What the rig sidecar holds: the edits made with this app and the names given to puppets.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RigSidecar {
    /// Rig edits, oldest first.
    pub edits: Vec<RigEdit>,
    /// Names and imports.
    pub meta: ProjectMeta,
}

/// Where the rig sidecar of a project lives, beside the scene sidecar.
#[must_use]
pub fn rig_sidecar_path(project: &Path) -> PathBuf {
    let scene = sidecar_path(project);
    let name = scene.file_name().map(|n| n.to_string_lossy().replace(".record.json", ".record.rig.json")).unwrap_or_default();
    scene.with_file_name(name)
}

/// The folder a project save writes: a folder path as given, or the folder named like a `.chproj` path (an existing
/// marker file means the folder it sits in).
#[must_use]
pub fn project_folder(dest: &Path) -> PathBuf {
    let is_chproj = dest.extension().is_some_and(|e| e.eq_ignore_ascii_case("chproj"));
    if !is_chproj || dest.is_dir() {
        return dest.to_path_buf();
    }
    if dest.is_file() { dest.parent().map_or_else(|| dest.to_path_buf(), Path::to_path_buf) } else { dest.with_extension("") }
}

fn open(path: &Path) -> Result<Project, String> {
    Project::open(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether a cast key names a puppet of `project`.
fn in_project(project: &Project, key: &str) -> bool {
    project.model().is_ok_and(|m| m.puppet(key).is_some())
}

/// Applies the display names the document gives to the puppets the project holds.
fn apply_names(project: &mut Project, meta: &ProjectMeta) -> Result<(), String> {
    let model = project.model().map_err(|e| e.to_string())?;
    let renames: Vec<(String, String)> = meta
        .names
        .iter()
        .filter(|(key, name)| model.puppet(key).is_some_and(|p| p.display_name() != name.as_str()))
        .map(|(k, n)| (k.clone(), n.clone()))
        .collect();
    for (key, name) in renames {
        project.rename_puppet(&key, &name).map_err(|e| format!("renaming {key}: {e}"))?;
    }
    Ok(())
}

/// Writes the document as a project folder with its sidecars. Returns a line saying what was written.
///
/// # Errors
/// A message when the document holds nothing a project can store, or a file cannot be read or written.
pub fn save_project(world: &World, doc: &AppDoc, dest: &Path) -> Result<String, String> {
    let folder = project_folder(dest);
    let mut project = match (&world.source, world.kind) {
        (Some(source), SourceKind::Project | SourceKind::Puppet) => open(source)?,
        _ => Project::new_project(&world.title).map_err(|e| e.to_string())?,
    };
    let mut seen = std::collections::BTreeSet::new();
    for import in &doc.meta.imports {
        if seen.insert(import.path.clone()) {
            let other = open(Path::new(&import.path))?;
            project.import_from(&other).map_err(|e| format!("{}: {e}", import.path))?;
        }
    }
    let kept: Vec<&String> = doc.rig.skeletons.keys().filter(|k| in_project(&project, k)).collect();
    if kept.is_empty() {
        return Err("none of this document's puppets can be written into a project (they were made from artwork); save it as a .charcraft document, or import a .puppet first".to_owned());
    }
    let dropped: Vec<&String> = doc.rig.skeletons.keys().filter(|k| !kept.contains(k)).collect();
    apply_names(&mut project, &doc.meta)?;
    project.save_folder(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    charcraft_record::save(&sidecar_path(&folder), &doc.rec.scene, &world.media).map_err(|e| format!("the scene: {e}"))?;
    let rig = RigSidecar { edits: doc.rig.edits.clone(), meta: doc.meta.clone() };
    let text = serde_json::to_vec_pretty(&rig).map_err(|e| e.to_string())?;
    std::fs::write(rig_sidecar_path(&folder), text).map_err(|e| format!("the rig edits: {e}"))?;
    let mut line = format!("Saved project {} with {} puppets, {} tracks", folder.display(), kept.len(), doc.rec.scene.tracks.len());
    if !dropped.is_empty() {
        let _ = write!(line, "; left out (made from artwork): {}", dropped.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", "));
    }
    Ok(line)
}

/// Writes one puppet as a `.puppet` file.
///
/// # Errors
/// A message when the puppet was made from artwork (there is no project data to write), or a file cannot be read or
/// written.
pub fn export_puppet(world: &World, doc: &AppDoc, key: &str, dest: &Path) -> Result<String, String> {
    let origin: PathBuf = match doc.meta.imports.iter().find(|i| i.key == key) {
        Some(import) => PathBuf::from(&import.path),
        None => match (&world.source, world.kind) {
            (Some(source), SourceKind::Project | SourceKind::Puppet) => source.clone(),
            _ => return Err(format!("{key} was made from artwork; only puppets read from a .puppet or project can be written as .puppet files")),
        },
    };
    let mut project = open(&origin)?;
    if !in_project(&project, key) {
        return Err(format!("{key} is not a puppet of {}", origin.display()));
    }
    apply_names(&mut project, &doc.meta)?;
    project.make_puppet(key).map_err(|e| e.to_string())?;
    let dest = if dest.extension().is_some_and(|e| e.eq_ignore_ascii_case("puppet")) { dest.to_path_buf() } else { dest.with_extension("puppet") };
    project.save_puppet(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(format!("Saved puppet {}", dest.display()))
}

/// One puppet of the project as the project panel and the `project_info` query show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PuppetRow {
    /// The cast key.
    pub key: String,
    /// The display name.
    pub name: String,
    /// The folders above it in the project (`ProjectItems/Cast/Boy` is in `Cast`).
    pub folders: Vec<String>,
    /// Canvas size in pixels.
    pub size: [f64; 2],
    /// Layer count.
    pub layers: usize,
    /// Independent groups.
    pub groups: usize,
    /// Mouth shapes found.
    pub visemes: usize,
    /// Swap sets.
    pub swap_sets: usize,
    /// Behaviors of the puppet's stack.
    pub behaviors: usize,
    /// Tracks that play it.
    pub tracks: Vec<usize>,
    /// Where it was imported from, when it was imported.
    pub imported_from: Option<String>,
}

/// The folders of a cast key: the segments between the leading `ProjectItems` and the name.
fn folders_of(key: &str) -> Vec<String> {
    let mut parts: Vec<&str> = key.split('/').filter(|p| !p.is_empty()).collect();
    parts.pop();
    if parts.first().is_some_and(|p| *p == "ProjectItems") {
        parts.remove(0);
    }
    parts.into_iter().filter(|p| !p.starts_with('.')).map(str::to_owned).collect()
}

/// The puppets of the document, sorted by folder then name.
#[must_use]
pub fn puppet_rows(world: &World, doc: &AppDoc) -> Vec<PuppetRow> {
    let mut rows: Vec<PuppetRow> = doc
        .rig
        .skeletons
        .iter()
        .map(|(key, skeleton)| {
            let summary = world.cast.get(key).map(|m| (m.puppet.summary(), m.template.slots().len()));
            let (size, groups, visemes, swap_sets, behaviors) =
                summary.as_ref().map_or(([0.0, 0.0], 0, 0, 0, 0), |(s, b)| (s.size, s.groups.len(), s.visemes.len(), s.swap_sets.len(), *b));
            PuppetRow {
                key: key.clone(),
                name: doc.meta.display_name(key),
                folders: folders_of(key),
                size,
                layers: skeleton.layers.len(),
                groups,
                visemes,
                swap_sets,
                behaviors,
                tracks: doc.rec.scene.tracks.iter().enumerate().filter(|(_, t)| t.puppet == *key).map(|(i, _)| i).collect(),
                imported_from: doc.meta.imports.iter().find(|i| i.key == *key).map(|i| i.path.clone()),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.folders.cmp(&b.folders).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    rows
}

/// The `project_info` answer: the project's puppets, scene, audio and where it lives.
#[must_use]
pub fn project_info(world: &World, doc: &AppDoc, path: Option<&Path>, dirty: bool) -> serde_json::Value {
    let scene = &doc.rec.scene;
    serde_json::json!({
        "title": world.title,
        "kind": world.kind,
        "source": world.source.as_ref().map(|p| p.display().to_string()),
        "document_path": path.map(|p| p.display().to_string()),
        "dirty": dirty,
        "puppets": puppet_rows(world, doc),
        "scenes": [{
            "name": scene.name,
            "size": [scene.size.width, scene.size.height],
            "fps": scene.fps_f64(),
            "duration_seconds": scene.duration.seconds(),
            "tracks": scene.tracks.iter().map(|t| serde_json::json!({ "name": t.name, "puppet": t.puppet, "takes": t.takes.len() })).collect::<Vec<_>>(),
        }],
        "audio": scene.audio.iter().map(|a| serde_json::json!({ "name": a.name, "clips": a.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>() })).collect::<Vec<_>>(),
        "sounds": world.media.names().collect::<Vec<_>>(),
        "folders": puppet_rows(world, doc).iter().filter(|r| !r.folders.is_empty()).map(|r| r.folders.join("/")).collect::<std::collections::BTreeSet<_>>(),
    })
}

/// The rig sidecar next to a project, when there is one.
#[must_use]
pub fn load_rig_sidecar(project: &Path) -> Option<Result<RigSidecar, String>> {
    let path = rig_sidecar_path(project);
    path.is_file().then(|| {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_follow_the_path_given() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(project_folder(&dir.path().join("Show.chproj")), dir.path().join("Show"));
        assert_eq!(project_folder(&dir.path().join("Show")), dir.path().join("Show"));
        let marker = dir.path().join("Old.chproj");
        std::fs::write(&marker, b"").unwrap();
        assert_eq!(project_folder(&marker), dir.path());
    }

    #[test]
    fn the_rig_sidecar_sits_beside_the_scene_sidecar() {
        let p = rig_sidecar_path(Path::new("/a/Show"));
        assert!(p.to_string_lossy().ends_with("Show.record.rig.json"), "{}", p.display());
    }

    #[test]
    fn an_artwork_document_cannot_be_written_as_a_project() {
        let opened = World::demo().unwrap();
        let (ui, _rx) = crossbeam_channel::unbounded();
        let doc = AppDoc::new(opened.rec, opened.rig, crate::session::Links::new(crate::live::LiveLink::disconnected(), ui));
        let dir = tempfile::tempdir().unwrap();
        let err = save_project(&opened.world, &doc, &dir.path().join("x")).unwrap_err();
        assert!(err.contains("artwork"), "{err}");
        assert!(export_puppet(&opened.world, &doc, "demo", &dir.path().join("x.puppet")).is_err());
    }
}
