//! The user's setup.
//!
//! Preferences, keyboard shortcuts, which panels are open, saved workspaces, recent files, autosave
//! and the starter gallery. Preferences are stored with `craft-settings` (`config.toml` in the platform config folder,
//! or `CHARCRAFT_CONFIG_DIR`), the same way the other Craft apps do.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anim_session::craft_settings::Settings;
use anim_session::{CommonSettings, Shortcut};
use serde::{Deserialize, Serialize};

use crate::state::Mode;

/// Which side panels are open and how wide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelState {
    /// The Project panel (puppets, scenes, audio, properties).
    pub project: bool,
    /// The Project panel sits on the right instead of the left.
    pub project_right: bool,
    /// Its width in points.
    pub project_width: f32,
}

impl Default for PanelState {
    fn default() -> Self {
        Self { project: true, project_right: false, project_width: 270.0 }
    }
}

/// A named arrangement: the mode and the panels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    /// The mode it opens in.
    pub mode: Mode,
    /// The panels it shows.
    pub panels: PanelState,
}

/// The built-in workspaces, in menu order. Saved ones with the same name replace them.
#[must_use]
pub fn builtin_workspaces() -> Vec<(&'static str, Workspace)> {
    let panels = |project| PanelState { project, ..PanelState::default() };
    vec![
        ("Rig", Workspace { mode: Mode::Rig, panels: panels(true) }),
        ("Record", Workspace { mode: Mode::Record, panels: panels(false) }),
        ("Stream", Workspace { mode: Mode::Stream, panels: panels(false) }),
        ("Project", Workspace { mode: Mode::Record, panels: panels(true) }),
    ]
}

/// Everything the Preferences window edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "independent settings")]
pub struct Prefs {
    /// Recent files, undo limits and the autosave interval (shared with `AniCraft`).
    pub common: CommonSettings,
    /// Dark theme.
    pub dark: bool,
    /// Interface zoom, 1.0 is normal.
    pub zoom: f32,
    /// Show the start workspace when the app starts without a file.
    pub start_screen: bool,
    /// The puppet mirrors the performer by default.
    pub mirror: bool,
    /// Playback loops by default.
    pub looping: bool,
    /// The panels.
    pub panels: PanelState,
    /// Workspaces the user saved, by name.
    pub workspaces: BTreeMap<String, Workspace>,
    /// Shortcuts the user changed: command id to chord text; an empty text turns the shortcut off.
    pub shortcuts: BTreeMap<String, String>,
    /// The MIDI controller each Controls slider follows, and the device that was open.
    pub midi: crate::midi::MidiPrefs,
    /// Grid, guides and snapping settings.
    pub grid: crate::grid::GridSettings,
}

impl Default for Prefs {
    fn default() -> Self {
        let common = CommonSettings { autosave_minutes: 5, ..CommonSettings::default() };
        Self {
            common,
            dark: true,
            zoom: 1.0,
            start_screen: true,
            mirror: true,
            looping: true,
            panels: PanelState::default(),
            workspaces: BTreeMap::new(),
            shortcuts: BTreeMap::new(),
            midi: crate::midi::MidiPrefs::default(),
            grid: crate::grid::GridSettings::default(),
        }
    }
}

impl Settings for Prefs {
    const APP_ID: &'static str = "charcraft";
    const SCHEMA_VERSION: u32 = 1;
}

// --- shortcuts -------------------------------------------------------------------------------------------------------

/// One command the keyboard can trigger.
#[derive(Debug, Clone, Copy)]
pub struct ShortcutDef {
    /// Stable id (the settings file stores overrides under it).
    pub id: &'static str,
    /// What the Preferences window calls it.
    pub label: &'static str,
    /// The section it is listed under.
    pub group: &'static str,
    /// The chord it has until changed; empty for none.
    pub default: &'static str,
}

const fn def(id: &'static str, label: &'static str, group: &'static str, default: &'static str) -> ShortcutDef {
    ShortcutDef { id, label, group, default }
}

/// Every shortcut the app has.
pub const SHORTCUTS: &[ShortcutDef] = &[
    def("file.new", "New project", "File", "Cmd+N"),
    def("file.open", "Open or import", "File", "Cmd+O"),
    def("file.save", "Save", "File", "Cmd+S"),
    def("file.save_as", "Save as", "File", "Cmd+Shift+S"),
    def("file.import_puppet", "Import puppet", "File", "Cmd+I"),
    def("file.revert", "Revert", "File", ""),
    def("edit.undo", "Undo", "Edit", "Cmd+Z"),
    def("edit.redo", "Redo", "Edit", "Cmd+Shift+Z"),
    def("mode.start", "Start workspace", "Window", ""),
    def("mode.rig", "Rig mode", "Window", "Cmd+1"),
    def("mode.record", "Record mode", "Window", "Cmd+2"),
    def("mode.stream", "Stream mode", "Window", "Cmd+3"),
    def("window.project", "Project panel", "Window", "Cmd+Alt+P"),
    def("window.start", "Start workspace", "Window", ""),
    def("app.preferences", "Preferences", "Window", "Cmd+Comma"),
    def("transport.play", "Play or pause", "Record", "Space"),
    def("transport.record", "Record or stop", "Record", "R"),
    def("transport.home", "Go to start", "Record", "Home"),
    def("transport.end", "Go to end", "Record", "End"),
    def("transport.back", "Step back one frame", "Record", "Left"),
    def("transport.forward", "Step forward one frame", "Record", "Right"),
    def("transport.back10", "Step back ten frames", "Record", "Shift+Left"),
    def("transport.forward10", "Step forward ten frames", "Record", "Shift+Right"),
    def("stream.hud", "Stream overlay", "Stream", "H"),
];

/// The chord a command has now, if any.
#[must_use]
pub fn chord(prefs: &Prefs, id: &str) -> Option<Shortcut> {
    let def = SHORTCUTS.iter().find(|d| d.id == id)?;
    let text = prefs.shortcuts.get(id).map_or(def.default, String::as_str);
    text.parse().ok()
}

/// The chord as text for menus and the Preferences window; empty when there is none.
#[must_use]
pub fn chord_text(prefs: &Prefs, id: &str) -> String {
    chord(prefs, id).map(|c| c.to_string()).unwrap_or_default()
}

/// The shortcut a key press makes.
#[must_use]
pub fn from_press(key: egui::Key, modifiers: egui::Modifiers) -> Shortcut {
    let mut s = Shortcut::key(key.name());
    s.command = modifiers.command;
    s.shift = modifiers.shift;
    s.alt = modifiers.alt;
    s
}

/// The egui modifiers and key of a shortcut.
#[must_use]
pub fn to_egui(shortcut: &Shortcut) -> Option<(egui::Modifiers, egui::Key)> {
    let key = egui::Key::from_name(&shortcut.key)?;
    let mut modifiers = egui::Modifiers::NONE;
    if shortcut.command {
        modifiers |= egui::Modifiers::COMMAND;
    }
    if shortcut.shift {
        modifiers |= egui::Modifiers::SHIFT;
    }
    if shortcut.alt {
        modifiers |= egui::Modifiers::ALT;
    }
    Some((modifiers, key))
}

/// The command ids whose chords are in use twice (a changed shortcut that now clashes).
#[must_use]
pub fn clashes(prefs: &Prefs) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for (i, a) in SHORTCUTS.iter().enumerate() {
        let Some(ca) = chord(prefs, a.id) else { continue };
        for b in SHORTCUTS.iter().skip(i.saturating_add(1)) {
            if chord(prefs, b.id).as_ref() == Some(&ca) {
                out.push((a.id, b.id));
            }
        }
    }
    out
}

// --- autosave and recovery ---------------------------------------------------------------------------------------------

/// A file name for a title: letters, digits, space, dash and underscore only.
#[must_use]
pub fn safe_name(title: &str) -> String {
    let cleaned: String = title.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).take(80).collect();
    let cleaned = cleaned.trim().to_owned();
    if cleaned.is_empty() { "Untitled".to_owned() } else { cleaned }
}

/// Where the autosave of a document with this title goes.
#[must_use]
pub fn autosave_path(dir: &Path, title: &str) -> PathBuf {
    dir.join(format!("{}.autosave.charcraft", safe_name(title)))
}

/// Autosaved documents waiting in the folder, newest first, with their age in seconds.
#[must_use]
pub fn recoverable(dir: &Path) -> Vec<(PathBuf, u64)> {
    let now = std::time::SystemTime::now();
    let mut found: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".autosave.charcraft"))
        .filter_map(|e| {
            let age = e.metadata().ok()?.modified().ok().and_then(|m| now.duration_since(m).ok()).map_or(0, |d| d.as_secs());
            Some((e.path(), age))
        })
        .collect();
    found.sort_by_key(|(_, age)| *age);
    found
}

/// How long ago, for a list.
#[must_use]
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

// --- the starter gallery -----------------------------------------------------------------------------------------------

/// Where a starter comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StarterSource {
    /// The puppet drawn in code.
    Demo,
    /// One of the Puppet Maker's built-in templates, by id.
    Template(&'static str),
    /// A `.puppet` the user keeps in the starters folder.
    File(PathBuf),
}

/// One tile of the start workspace's gallery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Starter {
    /// Its title.
    pub name: String,
    /// One line about it.
    pub about: String,
    /// What opens.
    pub source: StarterSource,
}

/// The starters: the Puppet Maker's built-in templates, then every `.puppet` file in `dir`.
///
/// `dir` is the `starters` folder of the settings, or the folder named by `CHARCRAFT_STARTERS`. Projects and other
/// folders are not starters.
#[must_use]
pub fn starters(dir: Option<&Path>) -> Vec<Starter> {
    let mut out: Vec<Starter> = charcraft_maker::templates()
        .into_iter()
        .map(|t| Starter { name: t.title.to_owned(), about: t.about.to_owned(), source: StarterSource::Template(t.id) })
        .collect();
    let Some(dir) = dir else { return out };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
        if ext == "puppet" && path.is_file() {
            let name = path.file_stem().map_or_else(|| "Puppet".to_owned(), |s| s.to_string_lossy().into_owned());
            let about = "A puppet file".to_owned();
            out.push(Starter { name, about, source: StarterSource::File(path) });
        }
    }
    out.truncate(24);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_chord_parses_and_round_trips_through_egui() {
        let prefs = Prefs::default();
        for def in SHORTCUTS {
            if def.default.is_empty() {
                assert!(chord(&prefs, def.id).is_none());
                continue;
            }
            let c = chord(&prefs, def.id).unwrap_or_else(|| panic!("{} does not parse", def.id));
            let (modifiers, key) = to_egui(&c).unwrap_or_else(|| panic!("{} has no egui key", def.id));
            assert_eq!(from_press(key, modifiers), c, "{}", def.id);
        }
        assert!(clashes(&prefs).is_empty(), "{:?}", clashes(&prefs));
    }

    #[test]
    fn a_changed_shortcut_replaces_the_default_and_clashes_show() {
        let mut prefs = Prefs::default();
        prefs.shortcuts.insert("file.save".into(), "Cmd+Alt+S".into());
        assert_eq!(chord_text(&prefs, "file.save"), "Cmd+Alt+S");
        prefs.shortcuts.insert("file.open".into(), String::new());
        assert!(chord(&prefs, "file.open").is_none(), "an empty text turns it off");
        prefs.shortcuts.insert("file.new".into(), "Cmd+Z".into());
        assert_eq!(clashes(&prefs), [("file.new", "edit.undo")]);
    }

    #[test]
    fn preferences_round_trip_through_the_settings_store() {
        use anim_session::craft_settings::SettingsStore;
        let dir = tempfile::tempdir().unwrap();
        let (mut store, report) = SettingsStore::<Prefs>::open_at(dir.path());
        assert!(!report.has_problems());
        store.update(|p| {
            p.dark = false;
            p.common.recent.push(Path::new("/a/b.puppet"));
            p.shortcuts.insert("file.save".into(), "F2".into());
            p.workspaces.insert("Mine".into(), Workspace { mode: Mode::Stream, panels: PanelState { project: false, ..PanelState::default() } });
        });
        store.flush().unwrap();
        let (back, report) = SettingsStore::<Prefs>::open_at(dir.path());
        assert!(!report.has_problems());
        assert!(!back.get().dark);
        assert_eq!(back.get().common.recent.len(), 1);
        assert_eq!(back.get().shortcuts.get("file.save").map(String::as_str), Some("F2"));
        assert_eq!(back.get().workspaces.get("Mine").map(|w| w.mode), Some(Mode::Stream));
        assert_eq!(back.get().common.autosave_minutes, 5, "defaults survive");
    }

    #[test]
    fn autosave_names_are_safe_and_recovery_lists_newest_first() {
        assert_eq!(safe_name("a/b:c"), "a_b_c");
        assert_eq!(safe_name("  "), "Untitled");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(autosave_path(dir.path(), "One"), b"{}").unwrap();
        std::fs::write(dir.path().join("other.txt"), b"x").unwrap();
        let found = recoverable(dir.path());
        assert_eq!(found.len(), 1);
        assert!(found[0].0.ends_with("One.autosave.charcraft"));
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(300), "5 min ago");
    }

    #[test]
    fn the_gallery_has_the_templates_and_only_puppet_files_from_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Hero.puppet"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("a_project")).unwrap();
        std::fs::write(dir.path().join("Old.chproj"), b"x").unwrap();
        let templates = charcraft_maker::templates().len();
        let list = starters(Some(dir.path()));
        assert_eq!(list.len(), templates + 1);
        assert!(matches!(list[0].source, StarterSource::Template(_)));
        assert_eq!(list[templates].name, "Hero");
        assert_eq!(starters(None).len(), templates);
    }
}
