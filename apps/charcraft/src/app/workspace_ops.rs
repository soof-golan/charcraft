//! The window's side of the workspace: preferences and recent files, autosave and recovery, saving and reverting,
//! workspaces and the keyboard.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anim_mcp::Request;
use anim_session::craft_settings::{Location, SettingsStore};
use egui::Context;
use serde_json::json;

use super::CharCraftApp;
use crate::hooks::DocEvent;
use crate::projectio::{PuppetRow, puppet_rows};
use crate::state::Mode;
use crate::view::{Action, Prompt, WorkspaceAction};
use crate::workspace::{self, Prefs, StarterSource, Workspace, builtin_workspaces};
use crate::world::SourceKind;

/// What the app keeps for the workspace.
pub struct WorkspaceState {
    /// The stored preferences.
    store: SettingsStore<Prefs>,
    /// The project panel's rows and what they were made from (revision, puppets generation, epoch).
    rows: Vec<PuppetRow>,
    rows_key: (u64, u64, u64),
    last_autosave: Instant,
    /// A recovered document that has not been saved yet.
    recovered: bool,
}

impl std::fmt::Debug for WorkspaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceState").field("rows", &self.rows.len()).finish_non_exhaustive()
    }
}

impl WorkspaceState {
    /// Opens the stored preferences (in memory only when `CHARCRAFT_NO_PREFS` is set).
    #[must_use]
    pub fn open() -> Self {
        let location = if std::env::var_os("CHARCRAFT_NO_PREFS").is_some() { Location::Memory } else { Location::Auto };
        let (store, report) = SettingsStore::<Prefs>::open(location);
        if report.has_problems() {
            eprintln!("charcraft: the settings file had problems; defaults are used where it could not be read");
        }
        Self { store, rows: Vec::new(), rows_key: (0, 0, 0), last_autosave: Instant::now(), recovered: false }
    }

    /// The preferences now.
    #[must_use]
    pub fn prefs(&self) -> &Prefs {
        self.store.get()
    }

    fn dir(&self) -> PathBuf {
        self.store.dir().map_or_else(|| std::env::temp_dir().join("charcraft"), Path::to_path_buf)
    }

    fn autosave_dir(&self) -> PathBuf {
        self.dir().join("autosave")
    }

    fn starters_dir(&self) -> PathBuf {
        std::env::var_os("CHARCRAFT_STARTERS").map_or_else(|| self.dir().join("starters"), PathBuf::from)
    }

    /// The puppets of the document for the Project panel.
    #[must_use]
    pub fn rows(&self) -> &[PuppetRow] {
        &self.rows
    }
}

impl CharCraftApp {
    /// Takes the stored preferences into the view and decides whether the start workspace shows.
    pub(super) fn init_workspace(&mut self, ctx: &Context, start: Option<bool>) {
        let prefs = self.ws.prefs().clone();
        self.view.prefs = prefs.clone();
        self.view.panels = prefs.panels.clone();
        self.view.mirror = prefs.mirror;
        self.view.looping = prefs.looping;
        if (prefs.zoom - 1.0).abs() > f32::EPSILON {
            ctx.set_zoom_factor(prefs.zoom.clamp(0.5, 2.5));
        }
        self.apply_history_budget();
        self.refresh_start_info();
        self.view.start = start.unwrap_or(prefs.start_screen || !self.view.start_info.recoverable.is_empty());
    }

    fn apply_history_budget(&mut self) {
        let budget = self.ws.prefs().common.history.budget();
        self.host.session_mut().history_mut().set_budget(budget);
    }

    fn refresh_start_info(&mut self) {
        let dir = self.ws.starters_dir();
        self.view.start_info.starters = workspace::starters(Some(&dir));
        self.view.start_info.recoverable = workspace::recoverable(&self.ws.autosave_dir());
    }

    /// Once per frame: file events from the hooks, stored preferences, the project panel's rows and autosave.
    pub(super) fn pump_workspace(&mut self) {
        let now = Instant::now();
        while let Some(event) = self.host.hooks_mut().events.pop_front() {
            match event {
                DocEvent::Opened(path) | DocEvent::Saved(path) => {
                    let is_autosave = path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".autosave.charcraft"));
                    if !is_autosave {
                        self.ws.store.update(|p| p.common.recent.push(&path));
                        self.view.prefs.common.recent = self.ws.prefs().common.recent.clone();
                    }
                }
                DocEvent::Note(text) => {
                    if text.starts_with("Saved") || text.starts_with("Reverted") {
                        self.ws.recovered = false;
                        self.remove_autosave();
                    }
                    self.view.status = text;
                    self.rev = self.rev.saturating_add(1);
                }
            }
        }
        let _ = self.ws.store.tick(now);
        let key = (self.rev, self.host.hooks_mut().world.generation, self.host.hooks_mut().epoch);
        if key != self.ws.rows_key {
            self.ws.rows_key = key;
            self.ws.rows = puppet_rows(&self.host.hooks_mut().world, &self.doc);
        }
        self.autosave(now);
    }

    fn autosave_file(&mut self) -> PathBuf {
        let title = self.host.hooks_mut().world.title.clone();
        workspace::autosave_path(&self.ws.autosave_dir(), &title)
    }

    fn remove_autosave(&mut self) {
        let file = self.autosave_file();
        if file.is_file() {
            let _ = std::fs::remove_file(file);
        }
    }

    /// Writes the autosave file now. Returns where it went.
    ///
    /// # Errors
    /// A message when the document or the file cannot be written.
    pub fn write_autosave(&mut self) -> Result<PathBuf, String> {
        let file = self.autosave_file();
        let bytes = self.host.hooks_mut().world.save_bytes_with(&self.doc.rec, &self.doc.rig, &self.doc.meta).map_err(|e| e.to_string())?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let temp = file.with_extension("saving");
        std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
        std::fs::rename(&temp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(file)
    }

    fn autosave(&mut self, now: Instant) {
        let minutes = self.ws.prefs().common.autosave_minutes;
        if minutes == 0 || now.duration_since(self.ws.last_autosave) < Duration::from_secs(u64::from(minutes).saturating_mul(60)) {
            return;
        }
        self.ws.last_autosave = now;
        if (self.host.session().is_dirty() || self.ws.recovered)
            && let Err(e) = self.write_autosave()
        {
            self.view.status = format!("Autosave failed: {e}");
        }
    }

    /// Clean exit: an unchanged document needs no autosave.
    pub(super) fn finish_workspace(&mut self) {
        if !self.host.session().is_dirty() && !self.ws.recovered {
            self.remove_autosave();
        }
        let _ = self.ws.store.flush();
    }

    fn set_panels(&mut self, panels: workspace::PanelState) {
        self.view.panels = panels.clone();
        self.ws.store.update(|p| p.panels = panels);
        self.view.prefs.panels = self.ws.prefs().panels.clone();
    }

    /// Saves to the file the document has, or asks where.
    fn save_current(&mut self) {
        if self.host.session().path().is_some() {
            self.view.status = match self.call(Request::Save { path: None }) {
                Ok(_) => {
                    self.ws.recovered = false;
                    self.remove_autosave();
                    "Saved".to_owned()
                }
                Err(e) => format!("Save failed: {e}"),
            };
            return;
        }
        let hooks = self.host.hooks_mut();
        let source = hooks.world.source.clone();
        let kind = hooks.world.kind;
        match source {
            Some(dir) if kind == SourceKind::Project && dir.is_dir() => {
                self.run("project.save", &json!({ "path": dir.display().to_string(), "format": "chproj" }), None);
            }
            _ => {
                self.view.save_kind =
                    if matches!(kind, SourceKind::Project | SourceKind::Puppet) { crate::view::SaveKind::Project } else { crate::view::SaveKind::Document };
                let name = workspace::safe_name(&self.host.hooks_mut().world.title);
                self.view.prompt = Some((Prompt::Save, format!("{}{name}{}", crate::ui::default_dir_text(), self.view.save_kind.suffix())));
            }
        }
    }

    /// Applies a request from the menus, windows and shortcuts.
    pub(super) fn workspace_action(&mut self, ctx: &Context, action: WorkspaceAction) {
        match action {
            WorkspaceAction::Save => self.save_current(),
            WorkspaceAction::Revert => self.run("project.revert", &json!({}), None),
            WorkspaceAction::Start(on) => {
                if on {
                    self.refresh_start_info();
                }
                self.view.start = on;
            }
            WorkspaceAction::Panels(panels) => self.set_panels(panels),
            WorkspaceAction::ApplyWorkspace(name) => {
                let found =
                    self.ws.prefs().workspaces.get(&name).cloned().or_else(|| builtin_workspaces().into_iter().find(|(n, _)| *n == name).map(|(_, w)| w));
                if let Some(w) = found {
                    self.set_mode(w.mode);
                    self.set_panels(w.panels);
                    self.view.start = false;
                    self.view.status = format!("Workspace {name}");
                }
            }
            WorkspaceAction::SaveWorkspace(name) => {
                let w = Workspace { mode: self.view.mode, panels: self.view.panels.clone() };
                self.ws.store.update(|p| {
                    p.workspaces.insert(name.clone(), w);
                });
                self.view.prefs = self.ws.prefs().clone();
                self.view.status = format!("Saved workspace {name}");
            }
            WorkspaceAction::DeleteWorkspace(name) => {
                self.ws.store.update(|p| {
                    p.workspaces.remove(&name);
                });
                self.view.prefs = self.ws.prefs().clone();
            }
            WorkspaceAction::SetPrefs(prefs) => {
                let zoom = prefs.zoom.clamp(0.5, 2.5);
                let changed_zoom = (zoom - self.ws.prefs().zoom).abs() > f32::EPSILON;
                let prefs = *prefs;
                self.ws.store.update(|p| *p = prefs.clone());
                self.view.prefs = self.ws.prefs().clone();
                if changed_zoom {
                    ctx.set_zoom_factor(zoom);
                }
                self.apply_history_budget();
            }
            WorkspaceAction::ClearRecent => {
                self.ws.store.update(|p| p.common.recent.clear());
                self.view.prefs.common.recent = self.ws.prefs().common.recent.clone();
            }
            WorkspaceAction::Recover(path) => {
                self.open(&path);
                self.host.session_mut().set_path(None);
                self.ws.recovered = true;
                self.view.start = false;
                "Recovered unsaved work: save it to keep it".clone_into(&mut self.view.status);
                let _ = std::fs::remove_file(&path);
                self.refresh_start_info();
            }
            WorkspaceAction::Discard(path) => {
                let _ = std::fs::remove_file(&path);
                self.refresh_start_info();
            }
            WorkspaceAction::Starter(index) => {
                let source = self.view.start_info.starters.get(index).map(|s| s.source.clone());
                match source {
                    Some(StarterSource::Demo) => {
                        if let Err(e) = self.call(Request::New {}) {
                            self.view.status = format!("New failed: {e}");
                        }
                        self.view.start = false;
                    }
                    Some(StarterSource::File(path)) => {
                        self.open(&path);
                        self.view.start = false;
                    }
                    None => {}
                }
            }
        }
    }

    /// The actions the keyboard asks for this frame, with the chords from the preferences.
    pub(super) fn workspace_shortcuts(&mut self, ctx: &Context) -> Vec<Action> {
        let mut actions = Vec::new();
        if ctx.egui_wants_keyboard_input() || self.view.capturing.is_some() {
            return actions;
        }
        let mut chords: Vec<(&'static str, egui::Modifiers, egui::Key)> = workspace::SHORTCUTS
            .iter()
            .filter_map(|d| workspace::chord(&self.view.prefs, d.id).and_then(|c| workspace::to_egui(&c)).map(|(m, k)| (d.id, m, k)))
            .collect();
        // The chord with more modifiers first: `Cmd+Shift+Z` must not be taken for `Cmd+Z`.
        chords.sort_by_key(|(_, m, _)| std::cmp::Reverse(u8::from(m.shift).saturating_add(u8::from(m.alt)).saturating_add(u8::from(m.command))));
        let mode = self.view.mode;
        let recording = self.view.recording.is_some();
        let playing = self.view.playing;
        let mut pressed = Vec::new();
        ctx.input_mut(|i| {
            for (id, modifiers, key) in &chords {
                let applies = match *id {
                    "transport.play" | "transport.record" => mode == Mode::Record,
                    "stream.hud" => mode == Mode::Stream,
                    _ => true,
                };
                if applies && i.consume_key(*modifiers, *key) {
                    pressed.push(*id);
                }
            }
        });
        for id in pressed {
            match id {
                "file.new" => actions.push(Action::Run("project.new", json!({}))),
                "file.open" => self.view.prompt = Some((Prompt::Open, crate::ui::default_dir_text())),
                "file.save" => actions.push(Action::Workspace(WorkspaceAction::Save)),
                "file.save_as" => {
                    let kind = self.host.hooks_mut().world.kind;
                    let title = self.host.hooks_mut().world.title.clone();
                    crate::ui::open_save_prompt(&mut self.view, kind, &title);
                }
                "file.import_puppet" => self.view.prompt = Some((Prompt::ImportPuppet, crate::ui::default_dir_text())),
                "file.revert" => actions.push(Action::Workspace(WorkspaceAction::Revert)),
                "edit.undo" => actions.push(Action::Undo),
                "edit.redo" => actions.push(Action::Redo),
                "mode.rig" => actions.push(Action::Mode(Mode::Rig)),
                "mode.record" => actions.push(Action::Mode(Mode::Record)),
                "mode.stream" => actions.push(Action::Mode(Mode::Stream)),
                "window.project" => {
                    let mut panels = self.view.panels.clone();
                    panels.project = !panels.project;
                    actions.push(Action::Workspace(WorkspaceAction::Panels(panels)));
                }
                "window.start" => actions.push(Action::Workspace(WorkspaceAction::Start(!self.view.start))),
                "app.preferences" => self.view.prefs_open = true,
                "transport.play" => actions.push(Action::Play(!playing)),
                "transport.record" => actions.push(if recording { Action::StopRecord } else { Action::StartRecord }),
                "stream.hud" => self.view.hud = !self.view.hud,
                _ => {}
            }
        }
        actions
    }
}
