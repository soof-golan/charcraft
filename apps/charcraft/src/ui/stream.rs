//! Stream mode: the performing puppet fills the window. A green screen option and a small overlay with the live
//! input; everything else stays out of the way so the window can be captured as it is.

use anim_core::num::f64_to_f32;
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};
use serde_json::json;

use super::panels::chip;
use crate::live::MicAction;
use crate::view::{Action, Ctx, View};

const GREEN_SCREEN: Color32 = Color32::from_rgb(0x00, 0xb1, 0x40);
const BACKDROP: Color32 = Color32::from_rgb(0x15, 0x16, 0x1b);

/// The largest rectangle with the aspect ratio of `size` that fits inside `area`, centred.
fn fit(area: Rect, size: [usize; 2]) -> Rect {
    let (w, h) = (f64_to_f32(f64::from(u32::try_from(size[0]).unwrap_or(1).max(1))), f64_to_f32(f64::from(u32::try_from(size[1]).unwrap_or(1).max(1))));
    let scale = (area.width() / w).min(area.height() / h);
    Rect::from_center_size(area.center(), vec2(w * scale, h * scale))
}

fn hud(ui: &Ui, view: &mut View, actions: &mut Vec<Action>) {
    egui::Area::new(egui::Id::new("stream_hud")).anchor(Align2::LEFT_TOP, vec2(12.0, 12.0)).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                if view.live.tracking {
                    if ui.button("Camera off").clicked() {
                        actions.push(Action::Tracking("tracking_stop", json!({})));
                    }
                } else if ui.button("Camera on").clicked() {
                    actions.push(Action::Tracking("tracking_start", json!({})));
                }
                if view.live.mic {
                    if ui.button("Mic off").clicked() {
                        actions.push(Action::Mic(MicAction::Stop));
                    }
                } else if ui.button("Mic on").clicked() {
                    actions.push(Action::Mic(MicAction::Start));
                }
                let mut demo = view.live.demo_voice;
                if ui.checkbox(&mut demo, "Demo voice").changed() {
                    actions.push(Action::Mic(MicAction::DemoVoice(demo)));
                }
                let mut mirror = view.mirror;
                if ui.checkbox(&mut mirror, "Mirror").changed() {
                    actions.push(Action::Mirror(mirror));
                }
                ui.toggle_value(&mut view.chroma, "Green screen");
                if ui.button("Hide (H)").clicked() {
                    view.hud = false;
                }
            });
            ui.horizontal(|ui| {
                chip(
                    ui,
                    if view.live.tracking { "face tracked" } else { "no camera" },
                    if view.live.tracking { Color32::from_rgb(0x2e, 0x9e, 0x5b) } else { Color32::from_gray(0x55) },
                );
                chip(
                    ui,
                    if view.live.mic || view.live.demo_voice { "voice" } else { "silent" },
                    if view.live.mic || view.live.demo_voice { Color32::from_rgb(0x2e, 0x9e, 0x5b) } else { Color32::from_gray(0x55) },
                );
                if let Some(v) = view.live.viseme {
                    chip(ui, &format!("{v:?}"), Color32::from_rgb(0x3a, 0x6f, 0xc4));
                }
            });
        });
    });
}

/// Stream mode.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let background = if view.chroma { GREEN_SCREEN } else { BACKDROP };
    egui::CentralPanel::default().frame(egui::Frame::new().fill(background)).show(ui, |ui| {
        let (area, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if let Some((texture, size)) = ctx.texture {
            let target = fit(area.shrink(8.0), size);
            ui.painter().image(texture.id(), target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let size_of = ctx.puppet.and_then(|k| ctx.world.cast.get(k)).map(|m| m.puppet.size);
            let down = response.is_pointer_button_down_on();
            view.pointer = response.hover_pos().zip(size_of).map(|(p, s)| {
                let u = (p.x - target.left()) / target.width().max(1.0);
                let v = (p.y - target.top()) / target.height().max(1.0);
                (f64::from(u) * s.width, f64::from(v) * s.height, down)
            });
        } else {
            ui.painter().text(area.center(), Align2::CENTER_CENTER, "waiting for the first frame", FontId::proportional(16.0), Color32::from_gray(0x99));
        }
    });
    if view.hud {
        hud(ui, view, actions);
    }
}
