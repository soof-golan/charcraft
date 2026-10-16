//! The window: a top bar with the modes and menus, one of the three mode views, a status bar and the file prompt.
//!
//! Everything here is `fn(&mut View, &Ctx) -> Vec<Action>`: widgets read the state, push actions, and the app
//! applies the actions to the session afterwards.

use std::path::{Path, PathBuf};

use anim_ui::theme::visual;
use egui::{RichText, Ui};

use crate::export::Format;
use crate::state::Mode;
use crate::view::{Action, Ctx, Prompt, View};

pub mod panels;
pub mod record;
pub mod rig;
pub mod stream;
pub mod timeline;

/// Draws the whole window and returns what the user asked for.
pub fn draw(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    if view.mode != Mode::Stream || view.hud {
        top_bar(ui, view, ctx, &mut actions);
        status_bar(ui, view, ctx);
    }
    match view.mode {
        Mode::Rig => rig::show(ui, view, ctx, &mut actions),
        Mode::Record => record::show(ui, view, ctx, &mut actions),
        Mode::Stream => stream::show(ui, view, ctx, &mut actions),
    }
    prompt(ui, view, &mut actions);
    actions
}

fn top_bar(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::Panel::top("top_bar").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("CharCraft").strong().size(15.0));
            ui.add_space(6.0);
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New demo puppet").clicked() {
                        actions.push(Action::New);
                        ui.close();
                    }
                    if ui.button("Open or import...").on_hover_text(".puppet, .chproj, .psd, .ai, .svg, .charcraft").clicked() {
                        view.prompt = Some((Prompt::Open, default_dir()));
                        ui.close();
                    }
                    if ui.button("Save...").clicked() {
                        view.prompt = Some((Prompt::Save, default_dir() + "scene.charcraft"));
                        ui.close();
                    }
                    if ui.button("Export video or images...").clicked() {
                        view.prompt = Some((Prompt::Export, default_dir() + "scene.mp4"));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        actions.push(Action::Quit);
                    }
                });
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
            });
            ui.separator();
            for mode in Mode::ALL {
                if ui.selectable_label(view.mode == mode, RichText::new(mode.label()).strong()).clicked() {
                    actions.push(Action::Mode(mode));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let title = format!("{}{}", ctx.world.title, if ctx.dirty { " *" } else { "" });
                ui.label(RichText::new(visual(&title).into_owned()).weak());
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
    let prefix = if path.is_dir() || text.ends_with(std::path::MAIN_SEPARATOR) { String::new() } else { path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default() };
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

fn prompt(ui: &Ui, view: &mut View, actions: &mut Vec<Action>) {
    let Some((kind, mut text)) = view.prompt.take() else { return };
    let title = match kind {
        Prompt::Open => "Open or import",
        Prompt::Save => "Save document",
        Prompt::Export => "Export",
    };
    let mut keep = true;
    let mut close = false;
    egui::Window::new(title).collapsible(false).resizable(true).default_width(560.0).open(&mut keep).show(ui.ctx(), |ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
                    let base = if text.ends_with(std::path::MAIN_SEPARATOR) || Path::new(&text).is_dir() { PathBuf::from(&text) } else { Path::new(&text).parent().map(Path::to_path_buf).unwrap_or_default() };
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
            };
            if ui.button(ok_label).clicked() || enter {
                let path = PathBuf::from(text.trim());
                actions.push(match kind {
                    Prompt::Open => Action::Open(path),
                    Prompt::Save => Action::Save(path),
                    Prompt::Export => Action::Export(view.export_format, path),
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
