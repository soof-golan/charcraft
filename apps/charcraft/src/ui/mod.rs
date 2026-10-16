//! The window: a top bar with the modes and menus, one of the three mode views, a status bar and the file prompt.
//!
//! Everything here is `fn(&mut View, &Ctx) -> Vec<Action>`: widgets read the state, push actions, and the app
//! applies the actions to the session afterwards.

use std::path::{Path, PathBuf};

use anim_ui::theme::visual;
use egui::{RichText, Ui};

use crate::export::Format;
use crate::state::Mode;
use serde_json::json;

use crate::view::{Action, Ctx, Prompt, SaveKind, View, WorkspaceAction};
use crate::workspace::{builtin_workspaces, chord_text};

pub mod edit;
pub mod layers;
pub mod panels;
pub mod prefs;
pub mod project;
pub mod record;
pub mod rig;
pub mod rigtools;
pub mod start;
pub mod stream;
pub mod timeline;

/// Draws the whole window and returns what the user asked for.
pub fn draw(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    if view.mode != Mode::Stream || view.hud || view.start {
        top_bar(ui, view, ctx, &mut actions);
        status_bar(ui, view, ctx);
    }
    if view.start {
        start::show(ui, view, ctx, &mut actions);
    } else {
        if view.mode != Mode::Stream && view.panels.project {
            project::show(ui, view, ctx, &mut actions);
        }
        match view.mode {
            Mode::Rig => rig::show(ui, view, ctx, &mut actions),
            Mode::Record => record::show(ui, view, ctx, &mut actions),
            Mode::Stream => stream::show(ui, view, ctx, &mut actions),
        }
    }
    prompt(ui, view, ctx, &mut actions);
    prefs::show(ui, view, &mut actions);
    actions
}

/// The keyboard shortcut that switches to a mode, shown as the tab's tooltip.
const fn mode_shortcut(mode: Mode) -> &'static str {
    match mode {
        Mode::Rig => "Ctrl/Cmd+1",
        Mode::Record => "Ctrl/Cmd+2",
        Mode::Stream => "Ctrl/Cmd+3",
    }
}

/// A menu item with its shortcut written on it.
fn item(ui: &mut Ui, view: &View, label: &str, id: &str) -> bool {
    let mut button = egui::Button::new(label);
    let chord = chord_text(&view.prefs, id);
    if !chord.is_empty() {
        button = button.shortcut_text(chord);
    }
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn file_menu(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    if item(ui, view, "New project", "file.new") {
        actions.push(Action::Run("project.new", json!({})));
    }
    if ui.button("New demo puppet").clicked() {
        actions.push(Action::New);
        ui.close();
    }
    if item(ui, view, "Open or import...", "file.open") {
        view.prompt = Some((Prompt::Open, default_dir()));
    }
    ui.menu_button("Open recent", |ui| {
        let recent: Vec<PathBuf> = view.prefs.common.recent.to_vec();
        if recent.is_empty() {
            ui.weak("Nothing opened yet");
        }
        for path in recent.iter().take(12) {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
            if ui.add_enabled(path.exists(), egui::Button::new(visual(&name).into_owned())).on_hover_text(path.display().to_string()).clicked() {
                actions.push(Action::Open(path.clone()));
                ui.close();
            }
        }
        if !recent.is_empty() {
            ui.separator();
            if ui.button("Clear recent").clicked() {
                actions.push(Action::Workspace(WorkspaceAction::ClearRecent));
                ui.close();
            }
        }
    });
    if item(ui, view, "Import puppet...", "file.import_puppet") {
        view.prompt = Some((Prompt::ImportPuppet, default_dir()));
    }
    ui.separator();
    if item(ui, view, "Save", "file.save") {
        actions.push(Action::Workspace(WorkspaceAction::Save));
    }
    if item(ui, view, "Save as...", "file.save_as") {
        open_save_prompt(view, ctx.world.kind, &ctx.world.title);
    }
    if ui.add_enabled(ctx.doc_path.is_some() || ctx.world.source.is_some(), egui::Button::new("Revert")).on_hover_text("Read the saved file again").clicked() {
        actions.push(Action::Workspace(WorkspaceAction::Revert));
        ui.close();
    }
    ui.separator();
    if ui.button("Export video or images...").clicked() {
        view.prompt = Some((Prompt::Export, default_dir() + "scene.mp4"));
        ui.close();
    }
    ui.separator();
    if item(ui, view, "Preferences...", "app.preferences") {
        view.prefs_open = true;
    }
    if ui.button("Quit").clicked() {
        actions.push(Action::Quit);
    }
}

/// Opens the Save As prompt with the kind and name the document suggests.
pub fn open_save_prompt(view: &mut View, kind: crate::world::SourceKind, title: &str) {
    let project_like = matches!(kind, crate::world::SourceKind::Project | crate::world::SourceKind::Puppet);
    view.save_kind = if project_like { SaveKind::Project } else { SaveKind::Document };
    let name = crate::workspace::safe_name(title);
    view.prompt = Some((Prompt::Save, format!("{}{name}{}", default_dir(), view.save_kind.suffix())));
}

/// The folder prompts start in, with a trailing separator.
#[must_use]
pub fn default_dir_text() -> String {
    default_dir()
}

fn window_menu(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    let mut panels = view.panels.clone();
    if ui.checkbox(&mut panels.project, "Project panel").on_hover_text(chord_text(&view.prefs, "window.project")).changed() {
        actions.push(Action::Workspace(WorkspaceAction::Panels(panels)));
    }
    if ui.button(if view.panels.project_right { "Project panel on the left" } else { "Project panel on the right" }).clicked() {
        let mut panels = view.panels.clone();
        panels.project_right = !panels.project_right;
        actions.push(Action::Workspace(WorkspaceAction::Panels(panels)));
        ui.close();
    }
    ui.separator();
    ui.menu_button("Workspace", |ui| {
        for (name, _) in builtin_workspaces() {
            if ui.button(name).clicked() {
                actions.push(Action::Workspace(WorkspaceAction::ApplyWorkspace(name.to_owned())));
                ui.close();
            }
        }
        let saved: Vec<String> = view.prefs.workspaces.keys().filter(|k| !builtin_workspaces().iter().any(|(b, _)| b == k)).cloned().collect();
        if !saved.is_empty() {
            ui.separator();
        }
        for name in saved {
            ui.horizontal(|ui| {
                if ui.button(visual(&name).into_owned()).clicked() {
                    actions.push(Action::Workspace(WorkspaceAction::ApplyWorkspace(name.clone())));
                    ui.close();
                }
                if ui.small_button("\u{00D7}").on_hover_text("Forget this workspace").clicked() {
                    actions.push(Action::Workspace(WorkspaceAction::DeleteWorkspace(name.clone())));
                }
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut view.new_workspace).hint_text("Name").desired_width(110.0));
            if ui.add_enabled(!view.new_workspace.trim().is_empty(), egui::Button::new("Save current")).clicked() {
                actions.push(Action::Workspace(WorkspaceAction::SaveWorkspace(view.new_workspace.trim().to_owned())));
                view.new_workspace.clear();
                ui.close();
            }
        });
    });
    ui.separator();
    if item(ui, view, "Start workspace", "window.start") {
        actions.push(Action::Workspace(WorkspaceAction::Start(true)));
    }
    if item(ui, view, "Preferences...", "app.preferences") {
        view.prefs_open = true;
    }
}

fn top_bar(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::Panel::top("top_bar").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("CharCraft").strong().size(15.0));
            ui.add_space(6.0);
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| file_menu(ui, view, ctx, actions));
                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(ctx.can_undo, egui::Button::new("Undo")).clicked() {
                        actions.push(Action::Undo);
                        ui.close();
                    }
                    if ui.add_enabled(ctx.can_redo, egui::Button::new("Redo")).clicked() {
                        actions.push(Action::Redo);
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    let mut dark = view.dark;
                    if ui.checkbox(&mut dark, "Dark theme").changed() {
                        actions.push(Action::Dark(dark));
                    }
                    ui.checkbox(&mut view.hud, "Stream overlay");
                });
                ui.menu_button("Window", |ui| window_menu(ui, view, actions));
                // The mode switch and title live inside the menu bar: it claims the rest of the row, so anything added
                // after it is laid out past the window's right edge.
                ui.separator();
                for mode in Mode::ALL {
                    if ui.selectable_label(view.mode == mode, RichText::new(mode.label()).strong()).on_hover_text(mode_shortcut(mode)).clicked() {
                        actions.push(Action::Mode(mode));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let title = format!("{}{}", ctx.world.title, if ctx.dirty { " *" } else { "" });
                    ui.label(RichText::new(visual(&title).into_owned()).weak());
                });
            });
        });
    });
}

fn status_bar(ui: &mut Ui, view: &View, ctx: &Ctx<'_>) {
    egui::Panel::bottom("status_bar").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(visual(&view.status).into_owned()).weak().size(12.0));
            if let Some((fraction, message)) = &view.exporting {
                ui.add(egui::ProgressBar::new(*fraction).desired_width(160.0).text(message.clone()));
            }
            if let Some(done) = &view.last_export {
                ui.label(RichText::new(visual(done).into_owned()).size(12.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{} warnings", ctx.world.warnings.len())).weak().size(11.0));
            });
        });
    });
}

fn default_dir() -> String {
    let dir = std::env::current_dir().ok().or_else(dirs_home).unwrap_or_default();
    let mut text = dir.display().to_string();
    if !text.ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    text
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Entries of the folder the typed path points into, folders first.
fn listing(text: &str) -> Vec<(String, bool)> {
    let path = Path::new(text);
    let dir = if text.ends_with(std::path::MAIN_SEPARATOR) || path.is_dir() { path } else { path.parent().unwrap_or(path) };
    let prefix = if path.is_dir() || text.ends_with(std::path::MAIN_SEPARATOR) {
        String::new()
    } else {
        path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
    };
    let mut out: Vec<(String, bool)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    (!name.starts_with('.') && name.to_lowercase().starts_with(&prefix)).then(|| (name, e.path().is_dir()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    out.truncate(200);
    out
}

/// `text` with its `.charcraft`, `.chproj` or `.puppet` ending swapped for the one `kind` writes.
fn with_suffix(text: &str, kind: SaveKind) -> String {
    let lower = text.to_lowercase();
    let stem = [".charcraft", ".chproj", ".puppet"]
        .iter()
        .find(|s| lower.ends_with(**s))
        .map_or(text, |s| text.get(..text.len().saturating_sub(s.len())).unwrap_or(text));
    format!("{stem}{}", kind.suffix())
}

fn prompt(ui: &Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some((kind, mut text)) = view.prompt.take() else { return };
    let title = match kind {
        Prompt::Open => "Open or import",
        Prompt::Save => "Save as",
        Prompt::Export => "Export",
        Prompt::Audio => "Import audio",
        Prompt::ImportPuppet => "Import puppet",
    };
    let mut keep = true;
    let mut close = false;
    egui::Window::new(title).collapsible(false).resizable(true).default_width(560.0).open(&mut keep).show(ui.ctx(), |ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if kind == Prompt::Save {
            ui.horizontal(|ui| {
                for choice in [SaveKind::Document, SaveKind::Project, SaveKind::Puppet] {
                    if ui.selectable_label(view.save_kind == choice, choice.label()).clicked() {
                        view.save_kind = choice;
                        text = with_suffix(&text, choice);
                    }
                }
            });
            match view.save_kind {
                SaveKind::Document => ui.weak("One file with the scene, takes, rig edits and sound."),
                SaveKind::Project => ui.weak("A Character Animator project folder; takes and rig edits are saved beside it."),
                SaveKind::Puppet => ui.weak("The selected puppet as a .puppet file."),
            };
        }
        if kind == Prompt::Export {
            ui.horizontal(|ui| {
                for format in [Format::Mp4, Format::Mov, Format::PngSequence, Format::Png] {
                    if ui.selectable_label(view.export_format == format, format.id()).clicked() {
                        view.export_format = format;
                    }
                }
            });
        }
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for (name, is_dir) in listing(&text) {
                let label = if is_dir { format!("{name}{}", std::path::MAIN_SEPARATOR) } else { name.clone() };
                if ui.selectable_label(false, visual(&label).into_owned()).clicked() {
                    let base = if text.ends_with(std::path::MAIN_SEPARATOR) || Path::new(&text).is_dir() {
                        PathBuf::from(&text)
                    } else {
                        Path::new(&text).parent().map(Path::to_path_buf).unwrap_or_default()
                    };
                    text = base.join(&name).display().to_string();
                    if is_dir && !text.ends_with(std::path::MAIN_SEPARATOR) {
                        text.push(std::path::MAIN_SEPARATOR);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            let ok_label = match kind {
                Prompt::Open => "Open",
                Prompt::Save => "Save",
                Prompt::Export => "Export",
                Prompt::Audio | Prompt::ImportPuppet => "Import",
            };
            if ui.button(ok_label).clicked() || enter {
                let path = PathBuf::from(text.trim());
                actions.push(match kind {
                    Prompt::Open => Action::Open(path),
                    Prompt::Save => match view.save_kind {
                        SaveKind::Document => Action::Save(path),
                        SaveKind::Project => Action::Run("project.save", json!({ "path": path.display().to_string(), "format": "chproj" })),
                        SaveKind::Puppet => {
                            let key = view.project.selected.clone().or_else(|| ctx.puppet.map(str::to_owned));
                            Action::Run("project.save", json!({ "path": path.display().to_string(), "format": "puppet", "puppet": key }))
                        }
                    },
                    Prompt::Export => Action::Export(view.export_format, path),
                    Prompt::Audio => {
                        Action::Run("audio.import", serde_json::json!({ "path": path.display().to_string(), "start_seconds": view.playhead.max(0.0) }))
                    }
                    Prompt::ImportPuppet => Action::Run("puppet.import", json!({ "path": path.display().to_string() })),
                });
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if keep && !close {
        view.prompt = Some((kind, text));
    }
}
