//! Stream mode: the performing puppet fills the window.
//!
//! The backdrop is dark, a green screen or fully transparent. A small overlay shows the live input and, until it is
//! hidden with H, the Controls, Triggers and Scenes panels and the output section that sends the puppet out of the app
//! as NDI, a virtual camera or a local web source.

use std::collections::BTreeSet;

use anim_core::num::f64_to_f32;
use charcraft_stream::{Background, KEY_GREEN, Kind, OutputInfo};
use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::json;

use super::panels::{camera_picker, chip, meter, mic_picker, mic_sensitivity, rest_pose, video_preview};
use crate::live::MicAction;
use crate::view::{Action, Ctx, View};

const GREEN_SCREEN: Color32 = Color32::from_rgb(0x00, 0xb1, 0x40);
const BACKDROP: Color32 = Color32::from_rgb(0x15, 0x16, 0x1b);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x3c);
const GOOD: Color32 = Color32::from_rgb(0x4c, 0xc2, 0x7a);

/// Canvas sizes the output offers.
const SIZES: [(u32, u32); 5] = [(854, 480), (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)];
/// Frame rates the output offers.
const RATES: [u32; 4] = [24, 30, 50, 60];

/// Text of the document (names of layers, scenes, puppets) in its display order, for right-to-left names.
fn shown(text: &str) -> String {
    anim_ui::theme::visual(text).into_owned()
}

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
                } else if cfg!(feature = "live") {
                    if ui.button("Camera on").clicked() {
                        actions.push(Action::Tracking("tracking_start", json!({ "index": view.live.camera_index })));
                    }
                    if view.live.problem.is_some() && ui.button("Demo face").clicked() {
                        actions.push(Action::Tracking("tracking_start", json!({ "demo": true })));
                    }
                } else if ui.button("Demo face on").clicked() {
                    actions.push(Action::Tracking("tracking_start", json!({ "demo": true })));
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
                ui.toggle_value(&mut view.transparent, "Transparent");
                ui.toggle_value(&mut view.stream.panels, "Panels");
                if ui.button("Hide (H)").clicked() {
                    view.hud = false;
                }
            });
            ui.horizontal(|ui| {
                chip(
                    ui,
                    if view.live.tracking {
                        "face tracked"
                    } else if view.live.problem.is_some() {
                        "camera unavailable"
                    } else {
                        "camera off"
                    },
                    if view.live.tracking {
                        Color32::from_rgb(0x2e, 0x9e, 0x5b)
                    } else if view.live.problem.is_some() {
                        Color32::from_rgb(0xc2, 0x7a, 0x1c)
                    } else {
                        Color32::from_gray(0x55)
                    },
                );
                chip(
                    ui,
                    if view.live.mic || view.live.demo_voice { "voice" } else { "silent" },
                    if view.live.mic || view.live.demo_voice { Color32::from_rgb(0x2e, 0x9e, 0x5b) } else { Color32::from_gray(0x55) },
                );
                if let Some(v) = view.live.viseme {
                    chip(ui, &format!("{v:?}"), Color32::from_rgb(0x3a, 0x6f, 0xc4));
                }
                if view.stream.status.active {
                    chip(ui, &format!("streaming {}", view.stream.status.kind.map_or("", Kind::label)), Color32::from_rgb(0xc4, 0x3a, 0x3a));
                }
            });
            if let Some(problem) = view.live.problem.as_ref().filter(|_| !view.live.tracking) {
                ui.set_max_width(460.0);
                ui.add(egui::Label::new(egui::RichText::new(anim_ui::theme::visual(problem).into_owned()).color(Color32::from_rgb(0xe8, 0xb0, 0x3c))).wrap());
            }
        });
    });
}

/// The camera, microphone and view controls of the performer.
fn controls(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Camera").strong());
    camera_picker(ui, view, actions);
    video_preview(ui, &view.live, view.mirror, ui.available_width().min(260.0));
    ui.horizontal_wrapped(|ui| {
        if view.live.tracking {
            if ui.button("Stop").clicked() {
                actions.push(Action::Tracking("tracking_stop", json!({})));
            }
        } else if ui.button("Start").clicked() {
            actions.push(Action::Tracking("tracking_start", json!({ "index": view.live.camera_index })));
        }
        let mut mirror = view.mirror;
        if ui.checkbox(&mut mirror, "Mirror").changed() {
            actions.push(Action::Mirror(mirror));
        }
    });
    rest_pose(ui, view, actions);
    ui.separator();
    ui.label(RichText::new("Microphone").strong());
    mic_picker(ui, view, actions);
    mic_sensitivity(ui, view, actions);
    meter(ui, "level", view.live.level, 0.0, 1.0);
    ui.horizontal_wrapped(|ui| {
        if view.live.mic {
            if ui.button("Stop").clicked() {
                actions.push(Action::Mic(MicAction::Stop));
            }
        } else if ui.button("Start").clicked() {
            actions.push(Action::Mic(MicAction::Start));
        }
        let mut demo = view.live.demo_voice;
        if ui.checkbox(&mut demo, "Demo voice").changed() {
            actions.push(Action::Mic(MicAction::DemoVoice(demo)));
        }
    });
    ui.separator();
    ui.label(RichText::new("Backdrop").strong());
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(!view.chroma && !view.transparent, "Dark").clicked() {
            view.chroma = false;
            view.transparent = false;
        }
        if ui.selectable_label(view.chroma && !view.transparent, "Green screen").clicked() {
            view.chroma = true;
            view.transparent = false;
        }
        if ui
            .selectable_label(view.transparent, "Transparent")
            .on_hover_text("No backdrop: the window and outputs that carry alpha show only the puppet.")
            .clicked()
        {
            view.transparent = true;
        }
    });
}

/// What an output kind needs, as one short line under its button.
fn output_note(ui: &mut Ui, info: &OutputInfo) {
    ui.add(egui::Label::new(RichText::new(&info.note).size(11.0).color(if info.available { ui.visuals().weak_text_color() } else { WARN })).wrap());
}

/// The background an output gets from the view's backdrop choice.
const fn background_of(view: &View) -> Background {
    if view.transparent {
        Background::Transparent
    } else if view.chroma {
        Background::Colour(KEY_GREEN)
    } else {
        Background::Colour([0x15, 0x16, 0x1b])
    }
}

/// The output section: pick where the puppet goes, start and stop it, and say what the machine still needs.
fn output(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    let selected = view.stream.spec.kind;
    ui.horizontal_wrapped(|ui| {
        for kind in Kind::ALL {
            let available = view.stream.outputs.iter().find(|o| o.kind == kind).is_some_and(|o| o.available);
            let text = if available { RichText::new(kind.label()) } else { RichText::new(kind.label()).weak() };
            if ui.selectable_label(selected == kind, text).clicked() {
                view.stream.spec.kind = kind;
            }
        }
    });
    let kind = view.stream.spec.kind;
    if let Some(info) = view.stream.outputs.iter().find(|o| o.kind == kind).cloned() {
        output_note(ui, &info);
        if kind == Kind::VirtualCamera && !info.devices.is_empty() {
            let current = view.stream.spec.device.clone().unwrap_or_else(|| info.devices.first().cloned().unwrap_or_default());
            egui::ComboBox::from_id_salt("vcam_device").selected_text(current).show_ui(ui, |ui| {
                for device in &info.devices {
                    ui.selectable_value(&mut view.stream.spec.device, Some(device.clone()), device);
                }
            });
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Name").size(11.0).weak());
        ui.add(egui::TextEdit::singleline(&mut view.stream.spec.name).desired_width(120.0));
    });
    ui.horizontal(|ui| {
        let current = (view.stream.spec.width, view.stream.spec.height);
        egui::ComboBox::from_id_salt("stream_size").selected_text(format!("{}x{}", current.0, current.1)).show_ui(ui, |ui| {
            for (w, h) in SIZES {
                ui.selectable_value(&mut (view.stream.spec.width, view.stream.spec.height), (w, h), format!("{w}x{h}"));
            }
        });
        egui::ComboBox::from_id_salt("stream_fps").selected_text(format!("{} fps", view.stream.spec.fps)).show_ui(ui, |ui| {
            for fps in RATES {
                ui.selectable_value(&mut view.stream.spec.fps, fps, format!("{fps} fps"));
            }
        });
    });
    let alpha = kind.carries_alpha();
    ui.label(
        RichText::new(match (view.transparent, alpha) {
            (true, true) => "Sends the puppet with transparency.",
            (true, false) => "This output has no alpha: the puppet is shown over a chroma key green. The backdrop is read when the output starts.",
            (false, _) => "Sends the puppet over the backdrop chosen above, as it was when the output started.",
        })
        .size(11.0)
        .weak(),
    );
    let status = view.stream.status.clone();
    ui.horizontal(|ui| {
        if status.active {
            if ui.button("Stop output").clicked() {
                actions.push(Action::StreamStop);
            }
            chip(ui, &format!("{} frames", status.frames_sent), GOOD);
        } else if ui.button("Start output").clicked() {
            let mut spec = view.stream.spec.clone();
            spec.background = background_of(view);
            actions.push(Action::StreamStart(Box::new(spec)));
        }
        if ui.small_button("\u{21BB}").on_hover_text("Check what each output needs again").clicked() {
            actions.push(Action::StreamRefresh);
        }
    });
    if status.active {
        ui.add(egui::Label::new(RichText::new(&status.detail).size(11.0)).wrap());
    }
    if let Some(error) = status.last_error.as_ref().filter(|_| !status.active) {
        ui.add(egui::Label::new(RichText::new(error).size(11.0).color(WARN)).wrap());
    }
}

/// The trigger keys of the performed puppet and its swap sets, as buttons.
fn triggers(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some(member) = ctx.puppet.and_then(|key| ctx.world.cast.get(key)) else {
        ui.weak("Add a puppet track first.");
        return;
    };
    let puppet = &member.puppet;
    let mut keys: Vec<(String, Vec<String>)> = Vec::new();
    for layer in puppet.layers() {
        let Some(key) = layer.key_trigger.as_ref() else { continue };
        let name = if layer.label.is_empty() { layer.name.clone() } else { layer.label.clone() };
        match keys.iter_mut().find(|(k, _)| k == key) {
            Some((_, names)) => names.push(name),
            None => keys.push((key.clone(), vec![name])),
        }
    }
    if keys.is_empty() && puppet.swap_sets.is_empty() {
        ui.weak("This puppet has no triggers. Give a layer a key trigger in Rig mode.");
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (key, names) in &keys {
            let lower = key.to_lowercase();
            let mut on = view.stream.held.contains(&lower);
            let label = format!("{}: {}", key.to_uppercase(), names.join(", "));
            if ui.toggle_value(&mut on, shown(&label)).on_hover_text("Click to turn the trigger on or off. The key on the keyboard works too.").changed() {
                actions.push(Action::Trigger(lower, on));
            }
        }
    });
    for set in &puppet.swap_sets {
        ui.label(RichText::new(shown(&set.name)).size(11.0).weak());
        let members: Vec<String> =
            set.members.iter().filter_map(|id| puppet.layer(*id)).map(|l| if l.label.is_empty() { l.name.clone() } else { l.label.clone() }).collect();
        ui.horizontal_wrapped(|ui| {
            for name in &members {
                let chosen = view.stream.swaps.get(&set.name) == Some(name);
                if ui.selectable_label(chosen, shown(name)).clicked() {
                    actions.push(Action::Swap(set.name.clone(), name.clone()));
                }
            }
        });
    }
}

/// The scene: its tracks (the puppets performing) to pick from, and the puppets that can still be put on stage.
fn scenes(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    ui.label(
        RichText::new(format!("{}  {:.0}x{:.0} at {:.0} fps", shown(&scene.name), scene.size.width, scene.size.height, scene.fps_f64())).size(11.0).weak(),
    );
    if scene.tracks.is_empty() {
        ui.weak("The scene has no puppet track yet.");
    }
    for (i, track) in scene.tracks.iter().enumerate() {
        if ui.selectable_label(view.track == i, shown(&track.name)).on_hover_text("Perform this puppet").clicked() {
            actions.push(Action::SelectTrack(i));
        }
    }
    let used: BTreeSet<&str> = scene.tracks.iter().map(|t| t.puppet.as_str()).collect();
    let spare: Vec<&str> = ctx.world.cast.keys().filter(|k| !used.contains(*k)).collect();
    if !spare.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("In the project").size(11.0).weak());
        for key in spare {
            if ui.button(format!("+ {}", shown(key))).on_hover_text("Put this puppet on stage").clicked() {
                actions.push(Action::Run("track.add", json!({ "puppet": key, "name": key })));
            }
        }
    }
}

/// The panel column: Controls, Triggers and Scenes.
fn panels(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::Panel::right("stream_panels").default_size(300.0).min_size(240.0).resizable(true).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new("Controls").strong()).default_open(true).show(ui, |ui| controls(ui, view, actions));
            egui::CollapsingHeader::new(RichText::new("Stream output").strong()).default_open(true).show(ui, |ui| output(ui, view, actions));
            egui::CollapsingHeader::new(RichText::new("Triggers").strong()).default_open(true).show(ui, |ui| triggers(ui, view, ctx, actions));
            egui::CollapsingHeader::new(RichText::new("Scenes").strong()).default_open(true).show(ui, |ui| scenes(ui, view, ctx, actions));
        });
    });
}

/// Stream mode.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    if view.hud && view.stream.panels {
        panels(ui, view, ctx, actions);
    }
    let background = backdrop(view);
    egui::CentralPanel::default().frame(egui::Frame::new().fill(background)).show(ui, |ui| stage(ui, view, ctx));
    if view.hud {
        hud(ui, view, actions);
    }
}

const fn backdrop(view: &View) -> Color32 {
    if view.transparent {
        Color32::TRANSPARENT
    } else if view.chroma {
        GREEN_SCREEN
    } else {
        BACKDROP
    }
}

/// The Camera & Microphone panel: the pickers, the live preview with its tracked points, Set Rest Pose, the level
/// meter and the backdrop.
pub fn camera_panel(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| controls(ui, view, actions));
}

/// The Controls panel: the trigger buttons and swap sets.
pub fn controls_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| triggers(ui, view, ctx, actions));
}

/// The Stream Output panel: NDI, virtual camera and web source.
pub fn output_panel(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| output(ui, view, actions));
}

/// The Scenes panel of the stream view.
pub fn scenes_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| scenes(ui, view, ctx, actions));
}

/// The performing puppet filling its panel, on the backdrop, the green screen or transparent.
pub fn stage_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>) {
    let background = backdrop(view);
    egui::Frame::new().fill(background).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        stage(ui, view, ctx);
    });
}

fn stage(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>) {
    {
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
    }
}
