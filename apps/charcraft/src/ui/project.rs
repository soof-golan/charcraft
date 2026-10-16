//! The Project panel.
//!
//! The puppets of the project (in their folders), the scene with its tracks, the audio, and the
//! properties of the selected puppet. Everything it does is a command (rename, duplicate, remove, import, save), so an
//! agent can do the same.

use anim_ui::theme::visual;
use egui::{Color32, RichText, Ui, vec2};
use serde_json::json;

use crate::projectio::PuppetRow;
use crate::view::{Action, Ctx, Prompt, SaveKind, View, WorkspaceAction};

const ACCENT: Color32 = Color32::from_rgb(0x4f, 0x9d, 0xff);

/// Whether a row passes the filter text (name, key or folder, any case).
#[must_use]
pub fn matches(row: &PuppetRow, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || row.name.to_lowercase().contains(&filter)
        || row.key.to_lowercase().contains(&filter)
        || row.folders.join("/").to_lowercase().contains(&filter)
}

/// The Project panel's contents, for a docked panel.
pub fn panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    header(ui, view, actions);
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        puppets(ui, view, ctx, actions);
        scenes(ui, view, ctx, actions);
        audio(ui, ctx);
        properties(ui, view, ctx, actions);
    });
}

/// A small picture of a puppet or scene (the neutral pose, drawn by the render actor), or an empty tile.
fn thumb(ui: &mut Ui, ctx: &Ctx<'_>, name: &str, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, super::theme::WELL);
    if let Some((texture, pixels)) = ctx.thumbs.get(name) {
        let w = anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(pixels[0].max(1)));
        let h = anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(pixels[1].max(1)));
        let scale = (rect.width() / w).min(rect.height() / h);
        let fitted = egui::Rect::from_center_size(rect.center(), vec2(w * scale, h * scale));
        ui.painter().image(texture.id(), fitted, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

fn header(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.button("Import puppet...").on_hover_text("Add a .puppet or the puppets of a project").clicked() {
            view.prompt = Some((Prompt::ImportPuppet, super::default_dir()));
        }
        if ui.button("Save").on_hover_text("Save the project").clicked() {
            actions.push(Action::Workspace(WorkspaceAction::Save));
        }
    });
    ui.add(egui::TextEdit::singleline(&mut view.project.search).hint_text("Filter puppets").desired_width(f32::INFINITY));
}

fn puppets(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let visible: Vec<&PuppetRow> = ctx.rows.iter().filter(|r| matches(r, &view.project.search)).collect();
    let title =
        if visible.len() == ctx.rows.len() { format!("Puppets ({})", ctx.rows.len()) } else { format!("Puppets ({} of {})", visible.len(), ctx.rows.len()) };
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, |ui| {
        if ctx.rows.is_empty() {
            ui.weak("No puppets yet. Import a .puppet, or open artwork with File > Open.");
        }
        let mut folder: Vec<String> = Vec::new();
        for row in visible {
            if row.folders != folder {
                folder.clone_from(&row.folders);
                if !folder.is_empty() {
                    ui.label(RichText::new(format!("\u{25BE} {}", visual(&folder.join(" / ")))).weak());
                }
            }
            puppet_row(ui, view, ctx, row, actions);
        }
    });
}

fn puppet_row(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, row: &PuppetRow, actions: &mut Vec<Action>) {
    let selected = view.project.selected.as_deref() == Some(row.key.as_str());
    if view.project.renaming.as_ref().is_some_and(|(k, _)| *k == row.key) {
        let mut done = None;
        if let Some((_, text)) = view.project.renaming.as_mut() {
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
            edit.request_focus();
            if edit.lost_focus() {
                done = Some(ui.input(|i| i.key_pressed(egui::Key::Enter)).then(|| text.clone()));
            }
        }
        if let Some(result) = done {
            if let Some(name) = result.filter(|n| !n.trim().is_empty() && *n != row.name) {
                actions.push(Action::Run("puppet.rename", json!({ "puppet": row.key, "name": name })));
            }
            view.project.renaming = None;
        }
        return;
    }
    let indent = if row.folders.is_empty() { 0.0 } else { 12.0 };
    let response = ui
        .horizontal(|ui| {
            ui.add_space(indent);
            thumb(ui, ctx, &format!("puppet:{}", row.key), vec2(36.0, 28.0));
            let mut text = RichText::new(visual(&row.name).into_owned());
            if !row.tracks.is_empty() {
                text = text.strong();
            }
            let response = ui.selectable_label(selected, text);
            if !row.tracks.is_empty() {
                ui.label(RichText::new(format!("\u{25B6} {}", row.tracks.len())).size(10.0).color(ACCENT)).on_hover_text("Tracks that play this puppet");
            }
            if row.imported_from.is_some() {
                ui.label(RichText::new("imported").size(10.0).weak());
            }
            response
        })
        .inner;
    if response.clicked() {
        view.project.selected = Some(row.key.clone());
        if let Some(track) = row.tracks.first() {
            actions.push(Action::SelectTrack(*track));
        }
    }
    if response.double_clicked() {
        view.project.renaming = Some((row.key.clone(), row.name.clone()));
    }
    response.context_menu(|ui| {
        view.project.selected = Some(row.key.clone());
        row_menu(ui, view, row, actions);
    });
}

/// The commands for one puppet, in the row's menu and the properties section.
fn row_menu(ui: &mut Ui, view: &mut View, row: &PuppetRow, actions: &mut Vec<Action>) {
    if ui.button("Rename").clicked() {
        view.project.renaming = Some((row.key.clone(), row.name.clone()));
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        actions.push(Action::Run("puppet.duplicate", json!({ "puppet": row.key })));
        ui.close();
    }
    if ui.button("Add to scene").on_hover_text("A new track that plays it").clicked() {
        actions.push(Action::Run("track.add", json!({ "puppet": row.key, "name": row.name })));
        ui.close();
    }
    if ui.button("Save as .puppet...").clicked() {
        view.save_kind = SaveKind::Puppet;
        view.project.selected = Some(row.key.clone());
        view.prompt = Some((Prompt::Save, format!("{}{}.puppet", super::default_dir(), crate::workspace::safe_name(&row.name))));
        ui.close();
    }
    ui.separator();
    let in_use = !row.tracks.is_empty();
    if ui.add_enabled(!in_use, egui::Button::new("Remove from project")).on_disabled_hover_text("A track plays it; remove the track first").clicked() {
        actions.push(Action::Run("puppet.remove", json!({ "puppet": row.key })));
        view.project.selected = None;
        ui.close();
    }
}

fn scenes(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    egui::CollapsingHeader::new(RichText::new(format!("Scenes ({})", ctx.doc.rec.scene_count())).strong()).default_open(true).show(ui, |ui| {
        let active = ctx.doc.rec.active_scene();
        for (i, s) in ctx.doc.rec.scenes().enumerate() {
            ui.horizontal(|ui| {
                thumb(ui, ctx, &format!("scene:{i}"), vec2(48.0, 28.0));
                let label = format!("{}  {:.1} s", visual(&s.name), s.duration.seconds());
                if ui.selectable_label(i == active, label).clicked() && i != active {
                    actions.push(Action::Run("scene.select", json!({ "index": i })));
                }
            });
        }
        ui.separator();
        ui.label(RichText::new(visual(&scene.name).into_owned()));
        ui.weak(format!("{} x {} px, {:.0} fps, {:.1} s", scene.size.width, scene.size.height, scene.fps_f64(), scene.duration.seconds()));
        let mut name = scene.name.clone();
        ui.horizontal(|ui| {
            ui.weak("Name");
            let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
            if edit.lost_focus() && name != scene.name && !name.trim().is_empty() {
                actions.push(Action::Run("scene.set", json!({ "name": name })));
            }
        });
        for (i, track) in scene.tracks.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let label = format!(
                    "{}  ({})",
                    visual(&track.name),
                    ctx.rows.iter().find(|r| r.key == track.puppet).map_or_else(|| track.puppet.clone(), |r| r.name.clone())
                );
                if ui.selectable_label(view.track == i, label).clicked() {
                    actions.push(Action::SelectTrack(i));
                }
                ui.weak(format!("{} takes", track.takes.len()));
            });
        }
        if scene.tracks.is_empty() {
            ui.weak("No tracks. Right-click a puppet > Add to scene.");
        }
    });
}

fn audio(ui: &mut Ui, ctx: &Ctx<'_>) {
    let scene = &ctx.doc.rec.scene;
    let sounds: Vec<&str> = ctx.world.media.names().collect();
    egui::CollapsingHeader::new(RichText::new(format!("Audio ({})", scene.audio.len().max(sounds.len()))).strong()).default_open(false).show(ui, |ui| {
        for track in &scene.audio {
            ui.label(RichText::new(visual(&track.name).into_owned()));
            for clip in &track.clips {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.weak(format!("{} at {:.1} s", visual(&clip.name), clip.start.seconds()));
                });
            }
        }
        for name in sounds {
            if let Some(buffer) = ctx.world.media.get(name) {
                ui.weak(format!("{} ({:.1} s, {} Hz)", visual(name), buffer.seconds(), buffer.sample_rate));
            }
        }
        if scene.audio.is_empty() && ctx.world.media.names().next().is_none() {
            ui.weak("No sound yet. Lip sync a sound file to add one.");
        }
    });
}

fn properties(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let key = view.project.selected.clone().or_else(|| ctx.puppet.map(str::to_owned));
    let Some(row) = key.as_deref().and_then(|k| ctx.rows.iter().find(|r| r.key == k)) else { return };
    egui::CollapsingHeader::new(RichText::new("Puppet properties").strong()).default_open(true).show(ui, |ui| {
        let mut name = row.name.clone();
        ui.horizontal(|ui| {
            ui.weak("Name");
            let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
            if edit.lost_focus() && name != row.name && !name.trim().is_empty() {
                actions.push(Action::Run("puppet.rename", json!({ "puppet": row.key, "name": name })));
            }
        });
        egui::Grid::new("puppet_properties").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            let line = |ui: &mut Ui, label: &str, value: String| {
                ui.weak(label);
                ui.label(visual(&value).into_owned());
                ui.end_row();
            };
            line(ui, "Key", row.key.clone());
            line(ui, "Canvas", format!("{} x {} px", row.size[0], row.size[1]));
            line(ui, "Layers", row.layers.to_string());
            line(ui, "Independent groups", row.groups.to_string());
            line(ui, "Mouth shapes", row.visemes.to_string());
            line(ui, "Swap sets", row.swap_sets.to_string());
            line(ui, "Behaviors", row.behaviors.to_string());
            line(ui, "Tracks", row.tracks.len().to_string());
            if !row.folders.is_empty() {
                line(ui, "Folder", row.folders.join(" / "));
            }
            line(
                ui,
                "From",
                row.imported_from.clone().unwrap_or_else(|| ctx.world.source.as_ref().map_or_else(|| "built in".to_owned(), |p| p.display().to_string())),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Duplicate").clicked() {
                actions.push(Action::Run("puppet.duplicate", json!({ "puppet": row.key })));
            }
            if ui.button("Add to scene").clicked() {
                actions.push(Action::Run("track.add", json!({ "puppet": row.key, "name": row.name })));
            }
            if ui.button("Save as .puppet...").clicked() {
                view.save_kind = SaveKind::Puppet;
                view.prompt = Some((Prompt::Save, format!("{}{}.puppet", super::default_dir(), crate::workspace::safe_name(&row.name))));
            }
            if ui.add_enabled(row.tracks.is_empty(), egui::Button::new("Remove")).on_disabled_hover_text("A track plays it").clicked() {
                actions.push(Action::Run("puppet.remove", json!({ "puppet": row.key })));
                view.project.selected = None;
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, folders: &[&str]) -> PuppetRow {
        PuppetRow {
            key: format!("ProjectItems/{name}"),
            name: name.to_owned(),
            folders: folders.iter().map(|s| (*s).to_owned()).collect(),
            size: [1.0, 1.0],
            layers: 1,
            groups: 0,
            visemes: 0,
            swap_sets: 0,
            behaviors: 0,
            tracks: Vec::new(),
            imported_from: None,
        }
    }

    #[test]
    fn the_filter_looks_at_name_key_and_folder() {
        let r = row("Boy", &["Cast"]);
        assert!(matches(&r, ""));
        assert!(matches(&r, "bo"));
        assert!(matches(&r, "cast"));
        assert!(matches(&r, "ProjectItems"));
        assert!(!matches(&r, "girl"));
    }
}
