//! The project panel's commands.
//!
//! Rename, rename, duplicate, remove and import puppets, save the project or one puppet, revert
//! and start a new project. Like every command they are undoable (or transient) and reachable over MCP.
//!
//! A command only sees the document, so work that needs the puppets (reading a `.puppet`, writing files) is read here
//! where it can be checked, and handed to the hooks as a [`WorldJob`] for their next tick.

use std::path::{Path, PathBuf};

use anim_session::{CommandError, CommandSpec, param, param_opt};
use charcraft_record::CastMember;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::session::{AppDoc, ImportedPuppet, SaveFormat, Skeleton, WorldJob};
use crate::world::{World, placed_track};

/// Longest name accepted.
const MAX_NAME: usize = 256;

fn name_ok(what: &str, name: &str) -> Result<String, CommandError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(CommandError::invalid_params(format!("{what} must be 1 to {MAX_NAME} characters without control characters")));
    }
    Ok(name.to_owned())
}

fn need_puppet(doc: &AppDoc, key: &str) -> Result<(), CommandError> {
    if doc.rig.skeletons.contains_key(key) {
        Ok(())
    } else {
        Err(CommandError::invalid_params(format!("no puppet {key:?} in the project (cast keys: {:?})", doc.rig.skeletons.keys().collect::<Vec<_>>())))
    }
}

/// `base`, or `base 2`, `base 3`... when taken.
fn unique_key(doc: &AppDoc, base: &str) -> String {
    if !doc.rig.skeletons.contains_key(base) {
        return base.to_owned();
    }
    (2_u32..=10_000).map(|n| format!("{base} {n}")).find(|k| !doc.rig.skeletons.contains_key(k)).unwrap_or_else(|| format!("{base} copy"))
}

fn send(doc: &AppDoc, job: WorldJob) -> Result<(), CommandError> {
    doc.links.jobs.send(job).map_err(|_| CommandError::failed("the project has no host to do that"))
}

// --- puppets -----------------------------------------------------------------------------------------------------------

fn puppet_rename(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let key: String = param(params, "puppet")?;
    let name = name_ok("name", &param::<String>(params, "name")?)?;
    need_puppet(doc, &key)?;
    doc.meta.names.insert(key.clone(), name.clone());
    Ok(json!({ "puppet": key, "name": name }))
}

fn puppet_duplicate(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let key: String = param(params, "puppet")?;
    need_puppet(doc, &key)?;
    let name = match param_opt::<String>(params, "name")? {
        Some(name) => name_ok("name", &name)?,
        None => format!("{} copy", doc.meta.display_name(&key)),
    };
    let copy = unique_key(doc, &name);
    if let Some(skeleton) = doc.rig.skeletons.get(&key).cloned() {
        doc.rig.skeletons.insert(copy.clone(), skeleton);
    }
    let edits: Vec<_> = doc
        .rig
        .edits
        .iter()
        .filter(|e| e.puppet == key)
        .cloned()
        .map(|mut e| {
            e.puppet.clone_from(&copy);
            e
        })
        .collect();
    if doc.rig.edits.len().saturating_add(edits.len()) > crate::session::MAX_RIG_EDITS {
        return Err(CommandError::failed("too many rig edits to copy"));
    }
    doc.rig.edits.extend(edits);
    if let Some(template) = doc.rec.templates.get(&key).cloned() {
        doc.rec.templates.insert(copy.clone(), template);
    }
    doc.meta.names.insert(copy.clone(), name.clone());
    send(doc, WorldJob::ClonePuppet { from: key.clone(), to: copy.clone() })?;
    Ok(json!({ "puppet": copy, "name": name, "copied_from": key }))
}

fn puppet_remove(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let key: String = param(params, "puppet")?;
    need_puppet(doc, &key)?;
    if let Some(track) = doc.rec.scene.tracks.iter().find(|t| t.puppet == key) {
        return Err(CommandError::failed(format!("the track {:?} plays this puppet; remove the track first", track.name)));
    }
    doc.rig.skeletons.remove(&key);
    doc.rig.edits.retain(|e| e.puppet != key);
    doc.rec.templates.remove(&key);
    doc.meta.names.remove(&key);
    doc.meta.imports.retain(|i| i.key != key);
    Ok(json!({ "removed": key, "puppets": doc.rig.skeletons.len() }))
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ImportParams {
    path: String,
    name: Option<String>,
    add_track: bool,
}

impl Default for ImportParams {
    fn default() -> Self {
        Self { path: String::new(), name: None, add_track: true }
    }
}

fn puppet_import(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let p: ImportParams = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    if p.path.is_empty() {
        return Err(CommandError::invalid_params("`path` is required (a .puppet or a project folder or .chproj)"));
    }
    let name = p.name.as_deref().map(|n| name_ok("name", n)).transpose()?;
    let absolute = std::path::absolute(&p.path).unwrap_or_else(|_| PathBuf::from(&p.path));
    let loaded = World::load_members(&absolute).map_err(|e| CommandError::failed(e.to_string()))?;
    if let Some(taken) = loaded.iter().find(|l| doc.rig.skeletons.contains_key(&l.key)) {
        return Err(CommandError::failed(format!("{} is already in the project", taken.name)));
    }
    let mut keys = Vec::new();
    let mut warnings = Vec::new();
    for (index, item) in loaded.into_iter().enumerate() {
        let shown = if index == 0 { name.clone().unwrap_or_else(|| item.name.clone()) } else { item.name.clone() };
        doc.rig.skeletons.insert(item.key.clone(), Skeleton::of(&item.member.puppet));
        doc.rec.templates.insert(item.key.clone(), item.member.template.clone());
        doc.meta.names.insert(item.key.clone(), shown.clone());
        doc.meta.imports.push(ImportedPuppet { key: item.key.clone(), path: absolute.display().to_string() });
        if p.add_track {
            let track = placed_track(&doc.rec.scene, &shown, &item.key, &item.member);
            doc.rec.scene.add_track(track).map_err(|e| CommandError::failed(e.to_string()))?;
        }
        warnings.extend(item.warnings);
        keys.push(item.key.clone());
        send(doc, WorldJob::AddPuppet { key: item.key, member: Box::<CastMember>::new(item.member) })?;
    }
    Ok(json!({ "puppets": keys, "track_added": p.add_track, "warnings": warnings }))
}

// --- files ---------------------------------------------------------------------------------------------------------------

fn format_of(path: &Path) -> SaveFormat {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("puppet")) { SaveFormat::Puppet } else { SaveFormat::Chproj }
}

fn project_save(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let path: String = param(params, "path")?;
    if path.trim().is_empty() {
        return Err(CommandError::invalid_params("`path` is required (a project folder or .chproj, or a .puppet for one puppet)"));
    }
    let path = PathBuf::from(path.trim());
    let format = match param_opt::<String>(params, "format")?.as_deref() {
        Some("chproj") => SaveFormat::Chproj,
        Some("puppet") => SaveFormat::Puppet,
        Some(other) => return Err(CommandError::invalid_params(format!("format must be chproj or puppet, not {other:?}"))),
        None => format_of(&path),
    };
    let puppet = param_opt::<String>(params, "puppet")?;
    if let Some(key) = &puppet {
        need_puppet(doc, key)?;
    }
    send(doc, WorldJob::Save { format, path: path.clone(), puppet })?;
    Ok(json!({ "queued": true, "format": format, "path": path.display().to_string(), "result": "see world_info.last_job or the status line" }))
}

fn puppet_save(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let key: String = param(params, "puppet")?;
    need_puppet(doc, &key)?;
    let path: String = param(params, "path")?;
    if path.trim().is_empty() {
        return Err(CommandError::invalid_params("`path` is required"));
    }
    let path = PathBuf::from(path.trim());
    send(doc, WorldJob::Save { format: SaveFormat::Puppet, path: path.clone(), puppet: Some(key.clone()) })?;
    Ok(json!({ "queued": true, "puppet": key, "path": path.display().to_string() }))
}

fn project_revert(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    send(doc, WorldJob::Revert)?;
    Ok(json!({ "queued": true }))
}

fn project_new(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let name = match param_opt::<String>(params, "name")? {
        Some(name) => name_ok("name", &name)?,
        None => "Untitled".to_owned(),
    };
    send(doc, WorldJob::NewProject { name: name.clone() })?;
    Ok(json!({ "queued": true, "name": name }))
}

const NO_PARAMS: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;

/// The commands of the project panel.
#[must_use]
pub fn commands() -> Vec<CommandSpec<AppDoc>> {
    vec![
        CommandSpec::new("puppet.rename", "Rename puppet", puppet_rename)
            .describe("Give a puppet of the project another display name. 'puppet' is its cast key (see project_info or rig_layers).")
            .with_schema(r#"{"type":"object","required":["puppet","name"],"properties":{"puppet":{"type":"string"},"name":{"type":"string"}},"additionalProperties":false}"#),
        CommandSpec::new("puppet.duplicate", "Duplicate puppet", puppet_duplicate)
            .describe("Copy a puppet of the project, with its rig edits, under a new name. The answer names the new cast key. The copy is not in the scene until a track plays it (track.add).")
            .with_schema(r#"{"type":"object","required":["puppet"],"properties":{"puppet":{"type":"string"},"name":{"type":"string","description":"Default: the name with ' copy'."}},"additionalProperties":false}"#),
        CommandSpec::new("puppet.remove", "Remove puppet from project", puppet_remove)
            .describe("Take a puppet out of the project. Refused while a track plays it.")
            .with_schema(r#"{"type":"object","required":["puppet"],"properties":{"puppet":{"type":"string"}},"additionalProperties":false}"#),
        CommandSpec::new("puppet.import", "Import puppet", puppet_import)
            .describe("Import a .puppet file (or the puppets of a project) into the open project. By default a track is added so it shows in the scene. The imported puppet is copied into the project when the project is saved.")
            .with_schema(r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string","description":"A .puppet, .chproj or project folder."},"name":{"type":"string","description":"Display name of the first puppet."},"add_track":{"type":"boolean","description":"Add a track for it (default true)."}},"additionalProperties":false}"#)
            .in_menu(&["File"]),
        CommandSpec::new("project.save", "Save project", project_save)
            .transient()
            .describe("Write the project as a Character Animator project folder (path: a folder or a .chproj name; takes and rig edits are saved beside it) or, with a .puppet path (or format 'puppet' and 'puppet' naming one), one puppet as a .puppet file. Runs on the host's next tick; the answer is in the status line and the world_info query.")
            .with_schema(r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string"},"format":{"enum":["chproj","puppet"]},"puppet":{"type":"string","description":"Cast key of the puppet to write when format is puppet."}},"additionalProperties":false}"#),
        CommandSpec::new("puppet.save", "Save puppet as .puppet", puppet_save)
            .transient()
            .describe("Write one puppet of the project as a .puppet file Character Animator opens.")
            .with_schema(r#"{"type":"object","required":["puppet","path"],"properties":{"puppet":{"type":"string"},"path":{"type":"string"}},"additionalProperties":false}"#),
        CommandSpec::new("project.revert", "Revert to saved", project_revert)
            .transient()
            .describe("Throw away the changes since the document was opened or saved and read it again.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("project.new", "New project", project_new)
            .transient()
            .describe("Start an empty project with a stage and no puppets; import puppets into it with puppet.import.")
            .with_schema(r#"{"type":"object","properties":{"name":{"type":"string"}},"additionalProperties":false}"#),
    ]
}

#[cfg(test)]
mod tests {
    use anim_session::Session;

    use super::*;
    use crate::live::LiveLink;
    use crate::session::{Links, registry};
    use crate::world::DEMO_KEY;

    fn session() -> (Session<AppDoc>, crossbeam_channel::Receiver<WorldJob>) {
        let opened = World::demo().unwrap();
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui);
        let jobs = links.jobs_in.clone();
        let doc = AppDoc::new(opened.rec, opened.rig, links);
        (Session::new(doc, registry().unwrap()), jobs)
    }

    #[test]
    fn rename_duplicate_and_remove_undo() {
        let (mut s, jobs) = session();
        s.execute("puppet.rename", &json!({ "puppet": DEMO_KEY, "name": "Hero" })).unwrap();
        assert_eq!(s.doc().meta.display_name(DEMO_KEY), "Hero");
        s.undo().unwrap();
        assert_eq!(s.doc().meta.display_name(DEMO_KEY), DEMO_KEY);
        assert!(s.execute("puppet.rename", &json!({ "puppet": DEMO_KEY, "name": "  " })).is_err());
        assert!(s.execute("puppet.rename", &json!({ "puppet": "nobody", "name": "x" })).is_err());

        s.execute("rig.edit", &json!({ "op": "set_visible", "layer": "Head/Nose", "visible": false })).unwrap();
        let done = s.execute("puppet.duplicate", &json!({ "puppet": DEMO_KEY })).unwrap();
        let copy = done["puppet"].as_str().unwrap().to_owned();
        assert_eq!(copy, "demo copy");
        assert_eq!(s.doc().rig.edits.iter().filter(|e| e.puppet == copy).count(), 1, "the rig edits come along");
        assert!(matches!(jobs.try_recv(), Ok(WorldJob::ClonePuppet { from, to }) if from == DEMO_KEY && to == copy));
        assert!(s.execute("puppet.remove", &json!({ "puppet": DEMO_KEY })).is_err(), "a track plays it");
        s.execute("puppet.remove", &json!({ "puppet": copy })).unwrap();
        assert!(!s.doc().rig.skeletons.contains_key(&copy));
        s.undo().unwrap();
        assert!(s.doc().rig.skeletons.contains_key(&copy), "undo brings it back");
    }

    #[test]
    fn saving_and_reverting_queue_jobs() {
        let (mut s, jobs) = session();
        s.execute("project.save", &json!({ "path": "/tmp/x/show.chproj" })).unwrap();
        assert!(matches!(jobs.try_recv(), Ok(WorldJob::Save { format: SaveFormat::Chproj, .. })));
        s.execute("project.save", &json!({ "path": "/tmp/x/hero.puppet", "puppet": DEMO_KEY })).unwrap();
        assert!(matches!(jobs.try_recv(), Ok(WorldJob::Save { format: SaveFormat::Puppet, puppet: Some(_), .. })));
        assert!(s.execute("project.save", &json!({ "path": "/tmp/x", "format": "zip" })).is_err());
        assert!(s.execute("puppet.save", &json!({ "puppet": "nobody", "path": "/tmp/a.puppet" })).is_err());
        s.execute("project.revert", &json!({})).unwrap();
        assert!(matches!(jobs.try_recv(), Ok(WorldJob::Revert)));
        assert!(!s.history().can_undo(), "files are not undoable");
    }

    #[test]
    fn importing_a_missing_file_fails_cleanly() {
        let (mut s, _jobs) = session();
        assert!(s.execute("puppet.import", &json!({ "path": "/nonexistent/none.puppet" })).is_err());
        assert!(s.execute("puppet.import", &json!({})).is_err());
        assert_eq!(s.doc().meta.imports.len(), 0);
    }
}
