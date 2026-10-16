//! The start workspace: new project, open, recent files with pictures, autosaved work to recover, and the starter
//! gallery (the Puppet Maker's templates and the `.puppet` files in the starters folder).

use std::path::Path;

use anim_ui::icons::Icon;
use anim_ui::theme::visual;
use egui::text::LayoutJob;
use egui::{Color32, FontId, Rect, RichText, Sense, TextFormat, Ui, vec2};

use super::theme::{ACCENT, SELECTED, TEXT, TEXT_WEAK, WELL, semibold};
use crate::view::{Action, Ctx, Prompt, View, WorkspaceAction};
use crate::workspace::{StarterSource, ago};

/// The widest the content gets, so it stays one readable column on a big window.
const COLUMN: f32 = 920.0;
const CARD: egui::Vec2 = vec2(200.0, 214.0);

fn file_label(path: &Path) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    visual(&name).into_owned()
}

type Picture<'a> = Option<(&'a egui::TextureHandle, [usize; 2])>;

/// A picture fitted into `rect` on a dark well; without one, a neutral placeholder with an icon.
fn picture(ui: &Ui, rect: Rect, tex: Picture<'_>) {
    ui.painter().rect_filled(rect, 4.0, WELL);
    let Some((texture, size)) = tex else {
        anim_ui::icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(34.0, 34.0)), Icon::Graphic, Color32::from_gray(0x55));
        return;
    };
    let w = anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(size[0].max(1)));
    let h = anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(size[1].max(1)));
    let scale = (rect.width() / w).min(rect.height() / h);
    let fitted = Rect::from_center_size(rect.center(), vec2(w * scale, h * scale));
    ui.painter().image(texture.id(), fitted, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
}

/// Text in at most `rows` lines, with an ellipsis where it is cut.
fn paint_clamped(ui: &Ui, at: egui::Pos2, text: &str, font: FontId, color: Color32, width: f32, rows: usize) {
    let mut job = LayoutJob::single_section(visual(text).into_owned(), TextFormat::simple(font, color));
    job.wrap.max_width = width;
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('\u{2026}');
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(at, galley, color);
}

/// A card: a picture on top, a name, and a short description clamped to two lines (the whole text is the tooltip).
fn card(ui: &mut Ui, name: &str, about: &str, tex: Picture<'_>, tooltip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(CARD, Sense::click());
    let hot = response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        6.0,
        if hot { super::theme::HEADER } else { super::theme::PANEL },
        egui::Stroke::new(1.0, if hot { ACCENT } else { Color32::from_rgb(0x38, 0x38, 0x38) }),
        egui::StrokeKind::Inside,
    );
    picture(ui, Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(rect.width() - 12.0, 140.0)), tex);
    let text_width = rect.width() - 20.0;
    paint_clamped(ui, rect.min + vec2(10.0, 152.0), name, FontId::new(12.0, semibold()), TEXT, text_width, 1);
    if !about.is_empty() {
        paint_clamped(ui, rect.min + vec2(10.0, 170.0), about, FontId::proportional(10.5), TEXT_WEAK, text_width, 2);
    }
    let tip = if tooltip.is_empty() { about.to_owned() } else { format!("{about}\n{tooltip}") };
    response.on_hover_text(visual(tip.trim()).into_owned()).clicked()
}

fn big_button(ui: &mut Ui, label: &str, primary: bool, tip: &str) -> bool {
    let mut button = egui::Button::new(RichText::new(label).family(semibold()).size(13.0)).min_size(vec2(190.0, 34.0));
    if primary {
        button = button.fill(SELECTED);
    }
    ui.add(button).on_hover_text(tip).clicked()
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(text).family(semibold()).size(15.0).color(TEXT));
    ui.add_space(4.0);
}

/// The start workspace in place of the mode view.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style())).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width().min(COLUMN);
            ui.horizontal_top(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(22.0);
                    ui.label(RichText::new("CharCraft").family(semibold()).size(28.0).color(Color32::WHITE));
                    ui.label(RichText::new("Rig, record and stream puppets that open in Character Animator").color(TEXT_WEAK));
                    ui.add_space(14.0);
                    begin(ui, view, actions);
                    heading(ui, "Recent");
                    recent(ui, view, ctx, actions);
                    recovery(ui, view, actions);
                    heading(ui, "Starter puppets");
                    gallery(ui, view, ctx, actions);
                    ui.add_space(16.0);
                    let mut show_again = view.prefs.start_screen;
                    if ui.checkbox(&mut show_again, "Show this when CharCraft starts").changed() {
                        let mut prefs = view.prefs.clone();
                        prefs.start_screen = show_again;
                        actions.push(Action::Workspace(WorkspaceAction::SetPrefs(Box::new(prefs))));
                    }
                    ui.add_space(24.0);
                });
            });
        });
    });
}

fn begin(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if big_button(ui, "New Project", true, "A stage with no puppets; import puppets into it") {
            actions.push(Action::Run("project.new", serde_json::json!({})));
            actions.push(Action::Workspace(WorkspaceAction::Start(false)));
        }
        if big_button(ui, "Open Project...", false, ".puppet, .chproj, .psd, .ai, .svg, .charcraft") {
            view.prompt = Some((Prompt::Open, super::default_dir()));
        }
        if big_button(ui, "Import Puppet...", false, "Add a puppet to this project") {
            view.prompt = Some((Prompt::ImportPuppet, super::default_dir()));
        }
    });
}

/// The document that is open now, then the recent files, as cards with their pictures.
fn recent(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        let source = ctx.world.source.as_deref().map(|p| p.display().to_string()).unwrap_or_default();
        if card(ui, &ctx.world.title, "Open now", ctx.texture, &source) {
            actions.push(Action::Workspace(WorkspaceAction::Start(false)));
        }
        for path in view.prefs.common.recent.iter().take(8) {
            let missing = !path.exists();
            let about = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
            let thumb = ctx.thumbs.get(&format!("recent:{}", path.display())).map(|(t, s)| (t, *s));
            if card(ui, &file_label(path), &if missing { "Missing".to_owned() } else { about }, thumb, &path.display().to_string()) && !missing {
                actions.push(Action::Open(path.to_path_buf()));
                actions.push(Action::Workspace(WorkspaceAction::Start(false)));
            }
        }
    });
    if !view.prefs.common.recent.is_empty() && ui.small_button("Clear recent").clicked() {
        actions.push(Action::Workspace(WorkspaceAction::ClearRecent));
    }
}

fn recovery(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    if view.start_info.recoverable.is_empty() {
        return;
    }
    heading(ui, "Recover unsaved work");
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
    ui.weak("Puppet Maker templates, and the .puppet files in your starters folder.");
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        for (index, starter) in view.start_info.starters.iter().enumerate() {
            let thumb = ctx.thumbs.get(&format!("starter:{index}")).map(|(t, s)| (t, *s));
            let tip = match &starter.source {
                StarterSource::File(path) => path.display().to_string(),
                _ => String::new(),
            };
            if card(ui, &starter.name, &starter.about, thumb, &tip) {
                actions.push(Action::Workspace(WorkspaceAction::Starter(index)));
            }
        }
    });
}
