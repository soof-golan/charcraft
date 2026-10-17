//! Behavior presets: named sets of a behavior's parameters, kept per user like Character Animator's behavior menu does.
//!
//! One JSON file per behavior id (`Adobe.Blinker.js.json`) in the presets folder holds `{ "name": { parameters } }`.
//! Saving a preset reads the parameters the track's behavior has now; loading one lays them over the behavior as an
//! ordinary undoable edit. Reset (every parameter back to the default) is the `reset_params` stack edit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anim_session::{CommandError, CommandSpec, param, param_opt};
use charcraft_behaviors::api::{StackOp, stack_info};
use charcraft_record::track_stack;
use serde_json::{Value, json};

use crate::session::AppDoc;

/// Longest preset name.
const MAX_NAME: usize = 120;
/// Most presets of one behavior.
const MAX_PRESETS: usize = 500;
/// Largest preset file read.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// Where presets are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetLibrary {
    dir: PathBuf,
}

impl PresetLibrary {
    /// A library in a folder (made when the first preset is saved).
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The user's library: `behavior-presets` in the config folder (`CHARCRAFT_CONFIG_DIR` when set).
    #[must_use]
    pub fn user() -> Self {
        let base = std::env::var_os("CHARCRAFT_CONFIG_DIR").map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join("charcraft"))).unwrap_or_default();
        Self::at(base.join("behavior-presets"))
    }

    /// The folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, behavior: &str) -> PathBuf {
        let safe: String = behavior.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '_' }).collect();
        self.dir.join(format!("{safe}.json"))
    }

    fn read(&self, behavior: &str) -> BTreeMap<String, Value> {
        let path = self.file(behavior);
        let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_FILE);
        if !small {
            return BTreeMap::new();
        }
        std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    fn write(&self, behavior: &str, presets: &BTreeMap<String, Value>) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let path = self.file(behavior);
        let bytes = serde_json::to_vec_pretty(presets).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
        std::fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The preset names of a behavior, in alphabetical order.
    #[must_use]
    pub fn names(&self, behavior: &str) -> Vec<String> {
        self.read(behavior).into_keys().collect()
    }

    /// One preset's parameters.
    #[must_use]
    pub fn get(&self, behavior: &str, name: &str) -> Option<Value> {
        self.read(behavior).remove(name)
    }

    /// Saves (or replaces) a preset.
    ///
    /// # Errors
    /// A sentence when the folder or file cannot be written or the behavior has too many presets.
    pub fn save(&self, behavior: &str, name: &str, params: Value) -> Result<(), String> {
        let mut all = self.read(behavior);
        if !all.contains_key(name) && all.len() >= MAX_PRESETS {
            return Err(format!("a behavior can have {MAX_PRESETS} presets"));
        }
        all.insert(name.to_owned(), params);
        self.write(behavior, &all)
    }

    /// Deletes a preset; false when there was none.
    ///
    /// # Errors
    /// A sentence when the file cannot be written.
    pub fn delete(&self, behavior: &str, name: &str) -> Result<bool, String> {
        let mut all = self.read(behavior);
        let had = all.remove(name).is_some();
        if had {
            self.write(behavior, &all)?;
        }
        Ok(had)
    }

    /// Every behavior's presets (for the window).
    #[must_use]
    pub fn all(&self) -> BTreeMap<String, Vec<String>> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return BTreeMap::new() };
        let mut out = BTreeMap::new();
        for entry in entries.flatten().take(256) {
            let path = entry.path();
            let (Some(stem), true) = (path.file_stem().and_then(|s| s.to_str()), path.extension().is_some_and(|e| e == "json")) else { continue };
            let names: Vec<String> = std::fs::metadata(&path)
                .is_ok_and(|m| m.len() <= MAX_FILE)
                .then(|| std::fs::read(&path).ok())
                .flatten()
                .and_then(|b| serde_json::from_slice::<BTreeMap<String, Value>>(&b).ok())
                .map(|m| m.into_keys().collect())
                .unwrap_or_default();
            if !names.is_empty() {
                out.insert(stem.to_owned(), names);
            }
        }
        out
    }
}

fn name_ok(name: &str) -> Result<String, CommandError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(CommandError::invalid_params(format!("a preset name is 1 to {MAX_NAME} characters without control characters")));
    }
    Ok(name.to_owned())
}

/// The id and parameters of the behavior at `index` of a track, as the scene plays it.
fn slot(doc: &AppDoc, track: usize, index: usize) -> Result<(String, Value), CommandError> {
    let t = doc.rec.scene.tracks.get(track).ok_or_else(|| CommandError::invalid_params(format!("no track {track}")))?;
    let stack = track_stack(&doc.rec.scene, &doc.rec.templates, track, t).map_err(|e| CommandError::failed(e.to_string()))?;
    let info = stack_info(&stack);
    let b = info.behaviors.into_iter().nth(index).ok_or_else(|| CommandError::invalid_params(format!("track {track} has no behavior {index}")))?;
    Ok((b.id, b.params))
}

fn preset_save(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let (track, index): (usize, usize) = (param(params, "track")?, param(params, "index")?);
    let name = name_ok(&param::<String>(params, "name")?)?;
    let (behavior, values) = slot(doc, track, index)?;
    doc.links.presets.save(&behavior, &name, values).map_err(CommandError::failed)?;
    Ok(json!({ "behavior": behavior, "name": name, "presets": doc.links.presets.names(&behavior) }))
}

fn preset_load(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let (track, index): (usize, usize) = (param(params, "track")?, param(params, "index")?);
    let name = name_ok(&param::<String>(params, "name")?)?;
    let (behavior, _) = slot(doc, track, index)?;
    let values = doc
        .links
        .presets
        .get(&behavior, &name)
        .ok_or_else(|| CommandError::invalid_params(format!("no preset {name:?} for {behavior} (it has {:?})", doc.links.presets.names(&behavior))))?;
    charcraft_record::commands::edit_track_behaviors(&mut doc.rec, track, Some(StackOp::SetParams { index, params: values }))?;
    Ok(json!({ "behavior": behavior, "name": name }))
}

fn preset_delete(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let behavior: String = param(params, "behavior")?;
    let name = name_ok(&param::<String>(params, "name")?)?;
    let removed = doc.links.presets.delete(&behavior, &name).map_err(CommandError::failed)?;
    Ok(json!({ "behavior": behavior, "name": name, "removed": removed }))
}

fn presets_list(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let behavior: Option<String> = param_opt(params, "behavior")?;
    Ok(match behavior {
        Some(b) => json!({ "behavior": b, "presets": doc.links.presets.names(&b) }),
        None => json!({ "presets": doc.links.presets.all() }),
    })
}

fn behavior_reset(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let (track, index): (usize, usize) = (param(params, "track")?, param(params, "index")?);
    let (behavior, _) = slot(doc, track, index)?;
    charcraft_record::commands::edit_track_behaviors(&mut doc.rec, track, Some(StackOp::ResetParams { index }))?;
    Ok(json!({ "behavior": behavior, "reset": true }))
}

/// The behavior preset and reset commands.
#[must_use]
pub fn commands() -> Vec<CommandSpec<AppDoc>> {
    vec![
        CommandSpec::new("behavior.reset", "Reset behavior parameters", behavior_reset)
            .describe("Put every parameter of the behavior at 'index' of track 'track' back to its default (an undoable edit; the behavior stays on or off).")
            .with_schema(r#"{"type":"object","required":["track","index"],"properties":{"track":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0}},"additionalProperties":false}"#),
        CommandSpec::new("behavior.preset_save", "Save behavior preset", preset_save)
            .transient()
            .describe("Save the parameters the behavior at 'index' of track 'track' has now as a named preset of that behavior (kept for the user, shared by all projects; an existing name is replaced).")
            .with_schema(r#"{"type":"object","required":["track","index","name"],"properties":{"track":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0},"name":{"type":"string"}},"additionalProperties":false}"#),
        CommandSpec::new("behavior.preset_load", "Load behavior preset", preset_load)
            .describe("Apply a saved preset of a behavior to the behavior at 'index' of track 'track' (an undoable edit). behavior.presets lists the names.")
            .with_schema(r#"{"type":"object","required":["track","index","name"],"properties":{"track":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0},"name":{"type":"string"}},"additionalProperties":false}"#),
        CommandSpec::new("behavior.preset_delete", "Delete behavior preset", preset_delete)
            .transient()
            .describe("Delete a saved preset of a behavior ('behavior' is its id, such as Adobe.Blinker.js).")
            .with_schema(r#"{"type":"object","required":["behavior","name"],"properties":{"behavior":{"type":"string"},"name":{"type":"string"}},"additionalProperties":false}"#),
        CommandSpec::new("behavior.presets", "List behavior presets", presets_list)
            .transient()
            .describe("The saved presets: of one behavior id, or of every behavior that has any.")
            .with_schema(r#"{"type":"object","properties":{"behavior":{"type":"string"}},"additionalProperties":false}"#),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_save_load_list_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let lib = PresetLibrary::at(dir.path().join("p"));
        assert!(lib.names("Adobe.Blinker.js").is_empty());
        lib.save("Adobe.Blinker.js", "Slow", json!({ "rate": 10 })).unwrap();
        lib.save("Adobe.Blinker.js", "\u{5de}\u{5d1}\u{5d9}", json!({ "rate": 90 })).unwrap();
        lib.save("Adobe.Blinker.js", "Slow", json!({ "rate": 12 })).unwrap();
        assert_eq!(lib.get("Adobe.Blinker.js", "Slow"), Some(json!({ "rate": 12 })));
        assert_eq!(lib.names("Adobe.Blinker.js").len(), 2);
        assert_eq!(lib.all().get("Adobe.Blinker.js").map(Vec::len), Some(2));
        assert!(lib.delete("Adobe.Blinker.js", "Slow").unwrap());
        assert!(!lib.delete("Adobe.Blinker.js", "Slow").unwrap());
        assert_eq!(lib.names("Adobe.Blinker.js"), ["\u{5de}\u{5d1}\u{5d9}"]);
    }

    #[test]
    fn a_hostile_behavior_id_stays_inside_the_folder_and_garbage_files_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lib = PresetLibrary::at(dir.path());
        lib.save("../../evil", "x", json!({})).unwrap();
        assert!(lib.file("../../evil").starts_with(dir.path()));
        std::fs::write(lib.file("Adobe.Blinker.js"), b"not json").unwrap();
        assert!(lib.names("Adobe.Blinker.js").is_empty());
    }

    #[test]
    fn names_are_checked() {
        assert!(name_ok("  ").is_err());
        assert!(name_ok("a\nb").is_err());
        assert_eq!(name_ok(" Calm ").unwrap(), "Calm");
    }
}
