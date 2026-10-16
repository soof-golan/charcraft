//! Windows for bringing puppets in and keeping them in step with their artwork: the import warnings list and the
//! Puppet Maker.

use anim_core::Rgba8;
use anim_render::Renderer;
use anim_ui::theme::visual;
use charcraft_maker::{BodyShape, EyeStyle, HairStyle, HeadShape, Spec};
use charcraft_puppet::PuppetPose;
use charcraft_record::{Cast, CastMember};
use egui::{Color32, RichText, TextureOptions, Ui};
use serde_json::json;

use crate::compose::Stage;
use crate::state::Mode;
use crate::view::{Action, Ctx, Dialog, MakerState, View};

const WARN: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x3c);
/// Pixels per artwork pixel in the Puppet Maker's picture.
const PREVIEW_SCALE: f64 = 0.45;

/// Draws whichever floating window is open.
pub fn show(ui: &Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    match view.dialog {
        Some(Dialog::Warnings) => warnings(ui, view, ctx, actions),
        Some(Dialog::Maker) => maker(ui, view, actions),
        None => {}
    }
}

fn warnings(ui: &Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let mut open = true;
    let rows = ctx.world.warning_rows();
    egui::Window::new("Import warnings").open(&mut open).default_width(680.0).show(ui.ctx(), |ui| {
        if ctx.world.stale {
            ui.horizontal(|ui| {
                ui.label(RichText::new("The artwork file changed on disk.").color(WARN));
                if ui.button("Reimport artwork").clicked() {
                    actions.push(Action::Run("puppet.reimport", json!({})));
                }
            });
            ui.separator();
        }
        if rows.is_empty() {
            ui.label("No warnings: the layer names and structure of every puppet look right.");
            return;
        }
        ui.weak(format!("{} warning{}. Click a layer to select it in the Rig view.", rows.len(), if rows.len() == 1 { "" } else { "s" }));
        egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("import_warnings").striped(true).num_columns(4).spacing([14.0, 5.0]).show(ui, |ui| {
                for row in &rows {
                    ui.label(RichText::new(visual(&row.kind).into_owned()).strong().color(WARN));
                    ui.weak(visual(row.puppet.as_deref().unwrap_or("Document")).into_owned());
                    match &row.layer {
                        Some(layer) => {
                            if ui.link(visual(layer).into_owned()).clicked() {
                                actions.push(Action::Mode(Mode::Rig));
                                actions.push(Action::SelectLayer(Some(layer.clone())));
                            }
                        }
                        None => {
                            ui.weak("-");
                        }
                    }
                    ui.label(visual(&row.message).into_owned());
                    ui.end_row();
                }
            });
        });
    });
    if !open {
        view.dialog = None;
    }
}

fn render_preview(spec: &Spec) -> Option<egui::ColorImage> {
    let puppet = charcraft_maker::puppet(spec).ok()?;
    let mut cast = Cast::new();
    cast.insert("preview", CastMember::new(puppet));
    let stage = Stage::new(cast);
    let mut renderer = Renderer::new();
    let backdrop = Some(Rgba8 { r: 236, g: 238, b: 244, a: 255 });
    let picture = stage.render_puppet(&mut renderer, "preview", &PuppetPose::neutral(), PREVIEW_SCALE, backdrop).ok()??;
    Some(egui::ColorImage::from_rgba_unmultiplied([usize::try_from(picture.width).ok()?, usize::try_from(picture.height).ok()?], &picture.rgba))
}

fn refresh_preview(ctx: &egui::Context, maker: &mut MakerState) {
    if maker.preview.as_ref().is_some_and(|(drawn, _)| *drawn == maker.spec) {
        return;
    }
    if let Some(image) = render_preview(&maker.spec) {
        let texture = ctx.load_texture("maker-preview", image, TextureOptions::LINEAR);
        maker.preview = Some((maker.spec.clone(), texture));
    }
}

fn choice<T: Copy + PartialEq>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, &str)]) {
    let current = options.iter().find(|(v, _)| v == value).map_or("", |(_, label)| *label);
    egui::ComboBox::from_id_salt(id).selected_text(current).show_ui(ui, |ui| {
        for (option, label) in options {
            ui.selectable_value(value, *option, *label);
        }
    });
}

fn maker(ui: &Ui, view: &mut View, actions: &mut Vec<Action>) {
    let mut open = true;
    let egui_ctx = ui.ctx().clone();
    refresh_preview(&egui_ctx, &mut view.maker);
    let state = &mut view.maker;
    egui::Window::new("Puppet Maker").open(&mut open).resizable(false).show(&egui_ctx, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(300.0);
                ui.label(RichText::new("Start from").strong());
                ui.horizontal_wrapped(|ui| {
                    for template in charcraft_maker::templates() {
                        if ui.button(template.title).on_hover_text(template.about).clicked() {
                            state.spec = template.spec;
                        }
                    }
                });
                ui.separator();
                let spec = &mut state.spec;
                egui::Grid::new("maker_spec").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut spec.name).desired_width(170.0));
                    ui.end_row();
                    ui.label("Head");
                    choice(ui, "maker_head", &mut spec.head, &[(HeadShape::Round, "Round"), (HeadShape::Oval, "Oval"), (HeadShape::Square, "Square")]);
                    ui.end_row();
                    ui.label("Skin");
                    egui::color_picker::color_edit_button_srgb(ui, &mut spec.skin);
                    ui.end_row();
                    ui.label("Hair");
                    choice(
                        ui,
                        "maker_hair",
                        &mut spec.hair_style,
                        &[
                            (HairStyle::Bald, "Bald"),
                            (HairStyle::Short, "Short"),
                            (HairStyle::Fringe, "Fringe"),
                            (HairStyle::Long, "Long"),
                            (HairStyle::Bun, "Bun"),
                        ],
                    );
                    ui.end_row();
                    ui.label("Hair colour");
                    egui::color_picker::color_edit_button_srgb(ui, &mut spec.hair);
                    ui.end_row();
                    ui.label("Eyes");
                    choice(ui, "maker_eyes", &mut spec.eyes, &[(EyeStyle::Round, "Round"), (EyeStyle::Big, "Big"), (EyeStyle::Small, "Small")]);
                    ui.end_row();
                    ui.label("Eye colour");
                    egui::color_picker::color_edit_button_srgb(ui, &mut spec.iris);
                    ui.end_row();
                    ui.label("Lips");
                    egui::color_picker::color_edit_button_srgb(ui, &mut spec.lips);
                    ui.end_row();
                    ui.label("Body");
                    choice(ui, "maker_body", &mut spec.body, &[(BodyShape::Slim, "Slim"), (BodyShape::Regular, "Regular"), (BodyShape::Wide, "Wide")]);
                    ui.end_row();
                    ui.label("Shirt");
                    egui::color_picker::color_edit_button_srgb(ui, &mut spec.shirt);
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut spec.lashes, "Lashes");
                    ui.checkbox(&mut spec.ears, "Ears");
                    ui.checkbox(&mut spec.cheeks, "Cheeks");
                });
            });
            ui.separator();
            ui.vertical(|ui| match &state.preview {
                Some((_, texture)) => {
                    ui.image((texture.id(), texture.size_vec2()));
                }
                None => {
                    ui.label("No preview");
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            let named = !state.spec.name.trim().is_empty();
            if ui.add_enabled(named, egui::Button::new("Add to scene")).on_hover_text("Rigged, with every mouth shape, blink and eye, on a new track").clicked()
            {
                actions.push(Action::Run("puppet.make", json!({ "spec": state.spec })));
            }
            if ui.add_enabled(named, egui::Button::new("New document")).on_hover_text("Start over with only this puppet").clicked() {
                actions.push(Action::NewMade(Box::new(state.spec.clone())));
            }
        });
    });
    if !open {
        view.dialog = None;
    }
}
