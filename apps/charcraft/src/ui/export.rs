//! The export dialog (presets, format, range, size, frame rate, alpha, destination) and the export queue window.

use std::path::PathBuf;

use anim_ui::theme::visual;
use egui::{RichText, Ui};

use crate::export::{self, Dialog, Format, ItemState};
use crate::view::{Action, Ctx, View};

/// Opens the dialog with the first preset, saving next to `dir`.
pub fn open_dialog(view: &mut View, ctx: &Ctx<'_>, dir: &str) {
    view.export.dialog = Some(Dialog::new(&ctx.doc.rec.scene.name, dir));
}

/// Draws the dialog and the queue window when they are open.
pub fn show(ui: &Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    dialog(ui, view, ctx, actions);
    queue(ui, view, actions);
}

#[expect(clippy::too_many_lines, reason = "one flat form")]
fn dialog(ui: &Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some(mut state) = view.export.dialog.take() else { return };
    let scene = &ctx.doc.rec.scene;
    let total = scene.frame_count();
    let mut keep = true;
    let mut close = false;
    egui::Window::new("Export").collapsible(false).resizable(true).default_width(520.0).open(&mut keep).show(ui.ctx(), |ui| {
        egui::Grid::new("export_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Preset");
            let presets = export::presets();
            let shown = state.preset.as_ref().and_then(|id| presets.iter().find(|p| &p.id == id)).map_or("Custom", |p| p.label.as_str());
            egui::ComboBox::from_id_salt("export_preset").selected_text(shown).width(300.0).show_ui(ui, |ui| {
                for preset in &presets {
                    if ui.selectable_label(state.preset.as_deref() == Some(&preset.id), &preset.label).on_hover_text(&preset.description).clicked() {
                        state.preset = Some(preset.id.clone());
                        state.format = preset.format;
                        state.options = preset.options.clone();
                        state.path = export::with_extension(&state.path, preset.format);
                    }
                }
            });
            ui.end_row();

            ui.label("Format");
            let before = state.format;
            egui::ComboBox::from_id_salt("export_format").selected_text(state.format.label()).width(300.0).show_ui(ui, |ui| {
                for format in Format::ALL {
                    ui.selectable_value(&mut state.format, format, format.label());
                }
            });
            if state.format != before {
                state.preset = None;
                state.path = export::with_extension(&state.path, state.format);
            }
            ui.end_row();

            if state.format.has_picture() {
                ui.label("Range");
                ui.horizontal(|ui| {
                    let last = total.saturating_sub(1);
                    ui.add(egui::DragValue::new(&mut state.options.start_frame).range(0..=last).prefix("from frame "));
                    if state.format == Format::Png {
                        ui.label("(a single frame)");
                    } else {
                        let mut to_end = state.options.frames.is_none();
                        if ui.checkbox(&mut to_end, "to the end").changed() {
                            state.options.frames = if to_end { None } else { Some(total.saturating_sub(state.options.start_frame).max(1)) };
                        }
                        if let Some(frames) = &mut state.options.frames {
                            ui.add(egui::DragValue::new(frames).range(1..=total.max(1)).suffix(" frames"));
                        }
                    }
                });
                ui.end_row();

                ui.label("Size");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut state.options.scale).range(0.05..=8.0).speed(0.01).fixed_decimals(2).suffix("x"));
                    let (w, h) = state.options.pixels(scene.size.width, scene.size.height);
                    ui.label(RichText::new(format!("{w} x {h} px")).weak());
                });
                ui.end_row();

                ui.label("Frame rate");
                ui.horizontal(|ui| {
                    let scene_fps = anim_core::FrameRate { num: scene.fps.num.max(1), den: scene.fps.den.max(1) }.as_f64();
                    let mut same = state.options.fps.is_none();
                    if ui.checkbox(&mut same, format!("scene rate ({scene_fps:.3} fps)")).changed() {
                        state.options.fps = if same { None } else { Some(scene_fps.round().clamp(1.0, 240.0)) };
                    }
                    if let Some(fps) = &mut state.options.fps {
                        ui.add(egui::DragValue::new(fps).range(1.0..=240.0).suffix(" fps"));
                    }
                });
                ui.end_row();

                ui.label("Background");
                let default = state.format.transparent_by_default();
                let mut transparent = state.options.transparent.unwrap_or(default);
                if ui.checkbox(&mut transparent, "transparent (alpha)").changed() {
                    state.options.transparent = Some(transparent);
                }
                ui.end_row();
            }

            ui.label("Save to");
            ui.add(egui::TextEdit::singleline(&mut state.path).desired_width(300.0));
            ui.end_row();
        });
        if state.format == Format::AfterEffects {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Adobe Dynamic Link cannot be cloned. This writes a folder After Effects and Premiere import directly: PNG sequence, WAV, ProRes 4444 MOV, and the command to export it again.",
                )
                .weak()
                .size(12.0),
            );
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let path = PathBuf::from(state.path.trim());
            let ready = !state.path.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Export")).clicked() {
                actions.push(Action::Export(state.format, path.clone(), state.options.clone()));
                close = true;
            }
            if ui.add_enabled(ready, egui::Button::new("Add to queue")).clicked() {
                actions.push(Action::Enqueue(state.format, path, state.options.clone()));
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if keep && !close {
        view.export.dialog = Some(state);
    }
}

fn queue(ui: &Ui, view: &mut View, actions: &mut Vec<Action>) {
    if !view.export.queue_open {
        return;
    }
    let mut open = true;
    let running = view.export.queue.running;
    let progress = view.exporting.clone();
    egui::Window::new("Export queue").collapsible(false).resizable(true).default_width(520.0).open(&mut open).show(ui.ctx(), |ui| {
        if view.export.queue.items.is_empty() {
            ui.label(RichText::new("Nothing queued. Use File > Export... and choose Add to queue.").weak());
        }
        for item in &view.export.queue.items {
            ui.horizontal(|ui| {
                let (mark, color) = match &item.state {
                    ItemState::Waiting => ("waiting", ui.visuals().weak_text_color()),
                    ItemState::Running => ("running", ui.visuals().hyperlink_color),
                    ItemState::Done => ("done", egui::Color32::from_rgb(90, 180, 110)),
                    ItemState::Failed(_) => ("failed", ui.visuals().error_fg_color),
                };
                ui.label(RichText::new(mark).color(color).strong());
                ui.label(item.format.label());
                ui.label(RichText::new(visual(&item.path.display().to_string()).into_owned()).weak());
                if let ItemState::Failed(message) = &item.state {
                    ui.label(RichText::new(visual(message).into_owned()).color(color));
                }
                if item.state == ItemState::Running
                    && let Some((fraction, message)) = &progress
                {
                    ui.add(egui::ProgressBar::new(*fraction).desired_width(120.0).text(message.clone()));
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let waiting = view.export.queue.waiting();
            if ui.add_enabled(waiting > 0 && !running, egui::Button::new(format!("Start queue ({waiting})"))).clicked() {
                actions.push(Action::RunQueue);
            }
            if ui.button("Clear finished").clicked() {
                actions.push(Action::ClearQueue);
            }
        });
    });
    view.export.queue_open = open;
}
