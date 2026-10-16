//! The start workspace: new project, open, recent files, autosaved work to recover, and the starter gallery.

use std::path::Path;

use anim_ui::theme::visual;
use egui::{Color32, RichText, Ui, vec2};

use crate::view::{Action, Ctx, Prompt, View, WorkspaceAction};
use crate::workspace::{StarterSource, ago};

const ACCENT: Color32 = Color32::from_rgb(0x4f, 0x9d, 0xff);

fn file_label(path: &Path) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    visual(&name).into_owned()
}

/// The start workspace in place of the mode view.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(18.0);
                ui.label(RichText::new("CharCraft").size(30.0).strong());
                ui.weak("Rig, record and stream puppets that open in Character Animator");
                ui.add_space(14.0);
            });
            let width = ui.available_width().min(980.0);
            ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                    ui.columns(2, |cols| {
                        if let [left, right] = cols {
                            begin(left, view, actions);
                            open_now(left, ctx, actions);
                            recent(left, view, ctx, actions);
                            recovery(left, view, actions);
                            gallery(right, view, ctx, actions);
                        }
                    });
                });
            });
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                let mut show_again = view.prefs.start_screen;
                if ui.checkbox(&mut show_again, "Show this when CharCraft starts").changed() {
                    let mut prefs = view.prefs.clone();
                    prefs.start_screen = show_again;
                    actions.push(Action::Workspace(WorkspaceAction::SetPrefs(Box::new(prefs))));
                }
                if ui.button(format!("Continue with {}", visual(&ctx.world.title))).clicked() {
                    actions.push(Action::Workspace(WorkspaceAction::Start(false)));
                }
            });
        });
    });
}

fn begin(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Start").strong().size(16.0));
    ui.add_space(4.0);
    if ui.button(RichText::new("New project").size(14.0)).on_hover_text("A stage with no puppets; import puppets into it").clicked() {
        actions.push(Action::Run("project.new", serde_json::json!({})));
        actions.push(Action::Workspace(WorkspaceAction::Start(false)));
    }
    if ui.button(RichText::new("Open or import...").size(14.0)).on_hover_text(".puppet, .chproj, .psd, .ai, .svg, .charcraft").clicked() {
        view.prompt = Some((Prompt::Open, super::default_dir()));
    }
    if ui.button(RichText::new("Import puppet into this project...").size(14.0)).clicked() {
        view.prompt = Some((Prompt::ImportPuppet, super::default_dir()));
    }
    ui.add_space(12.0);
}

type Picture<'a> = Option<(&'a egui::TextureHandle, [usize; 2])>;

/// A picture fitted into `rect` (the art fills the rect's height or width, whichever is the limit) on a dark well.
fn picture(ui: &Ui, rect: egui::Rect, tex: Picture<'_>) {
    ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(0x1b, 0x1b, 0x1b));
    let Some((texture, size)) = tex else { return };
    let w = anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(size[0].max(1)));
    let h = anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(size[1].max(1)));
    let scale = (rect.width() / w).min(rect.height() / h);
    let fitted = egui::Rect::from_center_size(rect.center(), vec2(w * scale, h * scale));
    ui.painter().image(texture.id(), fitted, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
}

/// The document that is open now, as a card whose picture fills it.
fn open_now(ui: &mut Ui, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Open now").strong().size(16.0));
    let (rect, response) = ui.allocate_exact_size(vec2(220.0, 200.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect_filled(rect, 8.0, visuals.bg_fill);
    ui.painter().rect_stroke(rect, 8.0, visuals.bg_stroke, egui::StrokeKind::Inside);
    picture(ui, egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(rect.width() - 12.0, 160.0)), ctx.texture);
    ui.painter().text(
        rect.left_bottom() + vec2(10.0, -26.0),
        egui::Align2::LEFT_TOP,
        visual(&ctx.world.title).into_owned(),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    if response.on_hover_text("Continue with this document").clicked() {
        actions.push(Action::Workspace(WorkspaceAction::Start(false)));
    }
    ui.add_space(12.0);
}

fn recent(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Recent").strong().size(16.0));
        if !view.prefs.common.recent.is_empty() && ui.small_button("Clear").clicked() {
            actions.push(Action::Workspace(WorkspaceAction::ClearRecent));
        }
    });
    if view.prefs.common.recent.is_empty() {
        ui.weak("Nothing opened yet.");
    }
    for path in view.prefs.common.recent.iter().take(8) {
        let missing = !path.exists();
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(52.0, 40.0), egui::Sense::hover());
            picture(ui, rect, ctx.thumbs.get(&format!("recent:{}", path.display())).map(|(t, s)| (t, *s)));
            ui.vertical(|ui| {
                let text = RichText::new(file_label(path)).color(if missing { Color32::GRAY } else { ACCENT });
                let response = ui.add_enabled(!missing, egui::Button::new(text).frame(false)).on_hover_text(path.display().to_string());
                if response.clicked() {
                    actions.push(Action::Open(path.to_path_buf()));
                    actions.push(Action::Workspace(WorkspaceAction::Start(false)));
                }
                if let Some(dir) = path.parent() {
                    ui.label(RichText::new(visual(&dir.display().to_string()).into_owned()).size(10.0).weak());
                }
            });
        });
    }
    ui.add_space(12.0);
}

fn recovery(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    if view.start_info.recoverable.is_empty() {
        return;
    }
    ui.label(RichText::new("Recover unsaved work").strong().size(16.0));
    for (path, age) in &view.start_info.recoverable {
        ui.horizontal(|ui| {
            ui.label(file_label(path).replace(".autosave.charcraft", ""));
            ui.weak(ago(*age));
            if ui.small_button("Recover").clicked() {
                actions.push(Action::Workspace(WorkspaceAction::Recover(path.clone())));
            }
            if ui.small_button("Discard").clicked() {
                actions.push(Action::Workspace(WorkspaceAction::Discard(path.clone())));
            }
        });
    }
}

fn gallery(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Starter puppets").strong().size(16.0));
    ui.weak("Put .puppet files in the starters folder to find them here.");
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (index, starter) in view.start_info.starters.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(190.0, 244.0), egui::Sense::click());
            let visuals = ui.style().interact(&response);
            ui.painter().rect_filled(rect, 8.0, visuals.bg_fill);
            ui.painter().rect_stroke(rect, 8.0, visuals.bg_stroke, egui::StrokeKind::Inside);
            // The neutral pose of the puppet, drawn by the Start thumbnail thread, fills the top of the card.
            let art = egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(rect.width() - 12.0, 140.0));
            picture(ui, art, ctx.thumbs.get(&format!("starter:{index}")).map(|(t, s)| (t, *s)));
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(rect.min + vec2(10.0, 152.0), rect.max - vec2(10.0, 6.0))));
            inner.label(RichText::new(visual(&starter.name).into_owned()).strong());
            inner.label(RichText::new(visual(&starter.about).into_owned()).size(11.0).weak());
            if let StarterSource::File(path) = &starter.source {
                inner.label(RichText::new(file_label(path)).size(10.0).weak());
            }
            if response.clicked() {
                actions.push(Action::Workspace(WorkspaceAction::Starter(index)));
            }
        }
    });
}
