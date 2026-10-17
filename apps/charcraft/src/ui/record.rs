//! The panels of the Record workspace.
//!
//! The Scene with its transport and the big Record button, Camera & Microphone, Properties with the behaviors of the
//! selected puppet track (each with its arm dot), the editing panel and the Timeline.

use anim_ui::icons::Icon;
use anim_ui::timeline::LayerEvent;
use anim_ui::widgets::icon_button;
use anim_ui::{TimelineEvent, TransportEvent, TransportState};
use charcraft_behaviors::api::StackOp;
use charcraft_record::ParamGroup;
use egui::{Align, Color32, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use serde_json::json;

use super::panels::{camera_picker, chip, meter, mic_picker, mic_sensitivity, rest_pose, video_preview};
use super::theme::{HEADER, RED, TEXT, TEXT_WEAK, semibold};
use super::timeline::{is_behavior_row, model, row_target, track_row};
use super::widgets::{arm_dot, scrub_style};
use crate::live::MicAction;
use crate::view::{Action, Ctx, View};

/// The groups of the performance a behavior listens to (what its arm dot arms).
#[must_use]
pub fn arm_groups(behavior: &str) -> &'static [ParamGroup] {
    match behavior {
        "Adobe.FaceTracker2.js" => &ParamGroup::FACE,
        "Adobe.LipSync2.js" => &[ParamGroup::Voice],
        "Adobe.EyeGaze.js" => &[ParamGroup::Eyes, ParamGroup::Keys],
        "Adobe.Triggers.js" | "Charcraft.Walk.js" => &[ParamGroup::Keys],
        "Adobe.Dragger3.js" => &[ParamGroup::Pointer],
        _ => &[],
    }
}

fn timecode(seconds: f64, fps: f64) -> String {
    let whole = anim_core::num::round_f64_to_i64((seconds.max(0.0) * fps).floor());
    let fps_n = anim_core::num::round_f64_to_i64(fps.max(1.0)).max(1);
    let frames = whole.checked_rem(fps_n).unwrap_or(0);
    let total = whole.checked_div(fps_n).unwrap_or(0);
    format!("{}:{:02}:{:02}:{:02}", total / 3600, (total / 60) % 60, total % 60, frames)
}

// --- the Scene panel ------------------------------------------------------------------------------------------------

/// The big red Record button.
fn record_button(ui: &mut Ui, recording: bool, time: f64) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
    let centre = rect.center();
    let painter = ui.painter();
    let pulse = if recording { 0.65 + 0.35 * (time * 5.0).sin().abs() } else { 1.0 };
    let radius = if response.hovered() { 14.5 } else { 14.0 };
    painter.circle_stroke(centre, radius + 1.5, Stroke::new(1.5, Color32::from_gray(0xb8)));
    painter.circle_filled(centre, radius - 1.0, RED.gamma_multiply(anim_core::num::f64_to_f32(pulse)));
    if recording {
        painter.rect_filled(Rect::from_center_size(centre, vec2(10.0, 10.0)), 1.0, Color32::WHITE);
    }
    response.on_hover_text(if recording { "Stop recording (R)" } else { "Record a take on the selected puppet (R)" })
}

fn transport_bar(ui: &mut Ui, bar: Rect, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>, stream: bool) {
    let scene = &ctx.doc.rec.scene;
    let frames = scene.frame_count().max(1);
    let fps = scene.fps_f64().max(1.0);
    let last = f64::from(frames.saturating_sub(1)) / fps;
    let state = TransportState {
        playing: view.playing,
        looping: view.looping,
        fps: anim_core::num::f64_to_f32(fps),
        frame: ctx.frame.min(frames.saturating_sub(1)),
        frame_count: frames,
    };
    ui.painter().rect_filled(bar, 0.0, HEADER);
    let recording = view.recording.is_some();

    // Left: the readouts.
    let left = Rect::from_min_size(bar.min + vec2(8.0, 0.0), vec2((bar.width() / 2.0 - 40.0).max(60.0), bar.height()));
    let mut readouts = ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center)));
    readouts.label(RichText::new(timecode(view.playhead, fps)).monospace().color(TEXT));
    let mut frame = f64::from(state.frame.saturating_add(1));
    let shown =
        scrub_style(&mut readouts, |ui| ui.add(egui::DragValue::new(&mut frame).speed(0.25).range(1.0..=f64::from(frames)).fixed_decimals(0).prefix("Frame ")));
    if shown.changed() {
        actions.push(Action::Seek((frame - 1.0).max(0.0) / fps));
    }
    readouts.label(RichText::new(format!("/ {frames}")).color(TEXT_WEAK));
    if let Some(active) = &view.recording {
        chip(&mut readouts, &format!("REC {:.1} s", (view.playhead - active.start).max(0.0)), RED);
    }

    // Centre: Record, then the transport.
    let centre = Rect::from_center_size(bar.center(), vec2(40.0, bar.height()));
    if !stream {
        let mut record = ui.new_child(UiBuilder::new().max_rect(centre).layout(Layout::left_to_right(Align::Center)));
        if record_button(&mut record, recording, ctx.time).clicked() {
            actions.push(if recording { Action::StopRecord } else { Action::StartRecord });
        }
    }
    let size = vec2(26.0, 24.0);
    let buttons = Rect::from_min_size(pos2(if stream { bar.center().x - 80.0 } else { centre.right() + 8.0 }, bar.top()), vec2(190.0, bar.height()));
    let mut transport = ui.new_child(UiBuilder::new().max_rect(buttons).layout(Layout::left_to_right(Align::Center)));
    transport.spacing_mut().item_spacing.x = 2.0;
    let mut events = Vec::new();
    if icon_button(&mut transport, size, Icon::ToStart, false, true, "Go to start (Home)").clicked() {
        events.push(TransportEvent::GoToStart);
    }
    if icon_button(&mut transport, size, Icon::StepBack, false, true, "Step back (Left)").clicked() {
        events.push(TransportEvent::StepBack);
    }
    let play = if state.playing { Icon::Pause } else { Icon::Play };
    if icon_button(&mut transport, size, play, state.playing, true, "Play or pause (Space)").clicked() {
        events.push(if state.playing { TransportEvent::Pause } else { TransportEvent::Play });
    }
    if icon_button(&mut transport, size, Icon::StepForward, false, true, "Step forward (Right)").clicked() {
        events.push(TransportEvent::StepForward);
    }
    if icon_button(&mut transport, size, Icon::ToEnd, false, true, "Go to end (End)").clicked() {
        events.push(TransportEvent::GoToEnd);
    }
    if icon_button(&mut transport, size, Icon::Loop, view.looping, true, "Loop playback").clicked() {
        view.looping = !view.looping;
    }
    for event in events {
        match event {
            TransportEvent::Play => actions.push(Action::Play(true)),
            TransportEvent::Pause => actions.push(Action::Play(false)),
            TransportEvent::GoToStart => actions.push(Action::Seek(0.0)),
            TransportEvent::GoToEnd => actions.push(Action::Seek(last)),
            TransportEvent::StepBack => actions.push(Action::Seek((view.playhead - 1.0 / fps).max(0.0))),
            TransportEvent::StepForward => actions.push(Action::Seek((view.playhead + 1.0 / fps).min(last))),
            TransportEvent::SetFrame(_) | TransportEvent::SetLoop(_) | TransportEvent::SetFps(_) => {}
        }
    }

    // Right: zoom.
    let zoom_width = 150.0_f32.min(bar.width() / 4.0);
    let right = Rect::from_min_size(pos2(bar.right() - zoom_width - 8.0, bar.top()), vec2(zoom_width, bar.height()));
    let mut zoom = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
    if zoom.small_button("Fit").on_hover_text("Fit the scene in the panel").clicked() {
        view.scene_stage.fit_request = true;
    }
    if zoom_width > 90.0 {
        zoom.spacing_mut().slider_width = zoom_width - 52.0;
        let mut level = view.scene_stage.view.zoom;
        if zoom.add(egui::Slider::new(&mut level, 0.1..=4.0).logarithmic(true).show_value(false)).on_hover_text("Zoom").changed() {
            view.scene_stage.view.zoom = level;
            view.scene_stage.fit_request = false;
        }
    }
}

/// The Scene panel: the stage with the transport under it.
pub fn scene_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>, stream: bool) {
    let full = ui.available_rect_before_wrap();
    let bar_height = 40.0;
    let stage = Rect::from_min_max(full.min, pos2(full.max.x, (full.max.y - bar_height).max(full.min.y)));
    let bar = Rect::from_min_max(pos2(full.min.x, stage.max.y), full.max);
    let mut canvas = ui.new_child(UiBuilder::new().max_rect(stage));
    let scene = &ctx.doc.rec.scene;
    view.scene_stage.doc_size = vec2(anim_core::num::f64_to_f32(scene.size.width), anim_core::num::f64_to_f32(scene.size.height));
    view.scene_stage.stage_color = scene.background.map_or(Color32::TRANSPARENT, |c| Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a));
    view.grid.apply_to(crate::grid::GridView::Scene, &mut view.scene_stage);
    if stream {
        // The performance view stays clean: no rulers, grid or guides over the picture.
        view.scene_stage.show_rulers = false;
        view.scene_stage.show_grid = false;
        view.scene_stage.show_guides = false;
    }
    for event in anim_ui::stage_view(&mut canvas, &view.scene_stage, ctx.texture.map(|(t, _)| t)) {
        match event {
            anim_ui::StageEvent::ViewChanged(v) => {
                view.scene_stage.view = v;
                view.scene_stage.fit_request = false;
            }
            guide => actions.extend(super::grid::guide_action(crate::grid::GridView::Scene, &guide)),
        }
    }
    let at = canvas.min_rect();
    let response = ui.interact(stage, ui.id().with("scene_menu"), Sense::click());
    response.context_menu(|ui| {
        if ui.button("Fit Scene in Panel").clicked() {
            view.scene_stage.fit_request = true;
            ui.close();
        }
        if ui.button("Zoom to 100%").clicked() {
            view.scene_stage.view.zoom = 1.0;
            ui.close();
        }
        ui.separator();
        super::grid::items(ui, view, actions);
        ui.separator();
        ui.checkbox(&mut view.monitor, "Monitor Live Performance");
    });
    let _ = at;
    let mut bar_ui = ui.new_child(UiBuilder::new().max_rect(bar));
    transport_bar(&mut bar_ui, bar, view, ctx, actions, stream);
    ui.allocate_rect(full, Sense::hover());
}

// --- Camera & Microphone --------------------------------------------------------------------------------------------

fn camera_toggle(view: &View, actions: &mut Vec<Action>) {
    if view.live.tracking {
        actions.push(Action::Tracking("tracking_stop", json!({})));
    } else if cfg!(feature = "live") {
        actions.push(Action::Tracking("tracking_start", json!({})));
    } else {
        actions.push(Action::Tracking("tracking_start", json!({ "demo": true })));
    }
}

fn mic_toggle(view: &View, actions: &mut Vec<Action>) {
    if view.live.mic {
        actions.push(Action::Mic(MicAction::Stop));
    } else if view.live.demo_voice {
        actions.push(Action::Mic(MicAction::DemoVoice(false)));
    } else if cfg!(feature = "live") {
        actions.push(Action::Mic(MicAction::Start));
    } else {
        actions.push(Action::Mic(MicAction::DemoVoice(true)));
    }
}

/// The camera preview with the tracked features marked as dots.
fn preview(ui: &mut Ui, view: &View) {
    // The live picture with the tracked points drawn on it, about as wide as the panel (but not taller than it is
    // worth in a short panel).
    video_preview(ui, &view.live, view.mirror, ui.available_width().clamp(60.0, 420.0));
}

/// Camera & Microphone: the live preview, Set Rest Pose, the camera and microphone switches and the level meter.
pub fn camera_panel(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        preview(ui, view);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let mic_on = view.live.mic || view.live.demo_voice;
            let cam = if view.live.tracking { "Camera on: click to turn off" } else { "Camera off: click to turn on" };
            if icon_button(ui, vec2(26.0, 24.0), Icon::Camera, view.live.tracking, true, cam).clicked() {
                camera_toggle(view, actions);
            }
            if icon_button(ui, vec2(26.0, 24.0), Icon::Mic, mic_on, true, if mic_on { "Microphone on" } else { "Microphone off" }).clicked() {
                mic_toggle(view, actions);
            }
        });
        ui.horizontal_wrapped(|ui| rest_pose(ui, view, actions));
        if let Some((done, wanted)) = view.live.calibrating {
            ui.add(
                egui::ProgressBar::new(f32::from(u16::try_from(done).unwrap_or(0)) / f32::from(u16::try_from(wanted.max(1)).unwrap_or(1))).desired_height(5.0),
            );
        }
        ui.horizontal(|ui| {
            let mut mirror = view.mirror;
            if ui.checkbox(&mut mirror, "Mirror").changed() {
                actions.push(Action::Mirror(mirror));
            }
            let mut demo = view.live.demo_voice;
            if ui.checkbox(&mut demo, "Demo voice").on_hover_text("Plays a talking mouth when there is no microphone.").changed() {
                actions.push(Action::Mic(MicAction::DemoVoice(demo)));
            }
        });
        ui.horizontal_wrapped(|ui| {
            if view.live.tracking {
                chip(ui, view.live.source.as_deref().unwrap_or("tracking"), Color32::from_rgb(0x2e, 0x9e, 0x5b));
            } else {
                chip(ui, "camera off", Color32::from_gray(0x55));
            }
            if view.live.mic {
                chip(ui, "listening", Color32::from_rgb(0x2e, 0x9e, 0x5b));
            } else if view.live.demo_voice {
                chip(ui, "demo voice", Color32::from_rgb(0xc2, 0x7a, 0x1c));
            }
        });
        meter(ui, "Mic level", view.live.level, 0.0, 1.0);
        if let Some(v) = view.live.viseme {
            ui.horizontal(|ui| {
                ui.add_sized([62.0, 14.0], egui::Label::new(RichText::new("Mouth").size(11.0).color(TEXT_WEAK)));
                chip(ui, &format!("{v:?}"), ui.visuals().selection.bg_fill);
            });
        }
        if let Some(face) = view.live.face.as_ref().filter(|f| f.tracked) {
            egui::CollapsingHeader::new("Tracked values").default_open(false).show(ui, |ui| {
                meter(ui, "yaw", face.head.yaw, -0.8, 0.8);
                meter(ui, "pitch", face.head.pitch, -0.8, 0.8);
                meter(ui, "roll", face.head.roll, -0.8, 0.8);
                meter(ui, "brows", f32::midpoint(face.brow_l, face.brow_r), -1.0, 1.0);
                meter(ui, "eyes", f32::midpoint(face.eye_open_l, face.eye_open_r), 0.0, 1.0);
                meter(ui, "mouth", face.mouth_open, 0.0, 1.0);
                meter(ui, "smile", face.smile, -1.0, 1.0);
            });
        }
        egui::CollapsingHeader::new("Body").default_open(false).show(ui, |ui| super::body::body(ui, view, actions));
        egui::CollapsingHeader::new("Devices").default_open(false).show(ui, |ui| {
            camera_picker(ui, view, actions);
            mic_picker(ui, view, actions);
            mic_sensitivity(ui, view, actions);
        });
        if let Some(problem) = &view.live.problem {
            ui.colored_label(Color32::from_rgb(0xe8, 0xb0, 0x3c), anim_ui::theme::visual(problem).into_owned());
        }
    });
}

// --- Properties -----------------------------------------------------------------------------------------------------

/// Properties of the selected puppet track: its behaviors as collapsible groups, each with the arm dot.
pub fn properties_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let Some(track) = scene.tracks.get(view.track) else {
        ui.weak("Select a puppet track in the Timeline to see its behaviors.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(anim_ui::theme::visual(&track.name).into_owned()).family(semibold()));
        ui.label(RichText::new(anim_ui::theme::visual(track.puppet.rsplit('/').next().unwrap_or(&track.puppet)).into_owned()).color(TEXT_WEAK));
    });
    ui.horizontal(|ui| {
        let audio = ctx.doc.rec.arming.audio;
        if arm_dot(ui, audio, "Arm: keep the microphone's sound as an audio clip in the next take").clicked() {
            actions.push(Action::Run("record.arm", json!({ "audio": !audio })));
        }
        ui.label(RichText::new("Microphone audio").color(if audio { TEXT } else { TEXT_WEAK }));
    });
    let Some(stack) = ctx.stack else {
        ui.weak("No behaviors.");
        return;
    };
    let armed = &ctx.doc.rec.arming.groups;
    let track_index = view.track;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(2.0);
        let count = stack.behaviors.len();
        for info in &stack.behaviors {
            super::params::behavior_block(ui, track_index, info, count, &super::params::BlockEnv { armed, presets: &view.presets }, info.index < 3, actions);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let names = view.rig.catalog.clone();
            let current = names.get(view.rig.add_behavior).map_or("Add Behavior", |(_, n)| n.as_str());
            egui::ComboBox::from_id_salt("record_add_behavior").selected_text(current).width(150.0).show_ui(ui, |ui| {
                for (i, (_, name)) in names.iter().enumerate() {
                    ui.selectable_value(&mut view.rig.add_behavior, i, name);
                }
            });
            if ui.button("Add").clicked()
                && let Some((id, _)) = names.get(view.rig.add_behavior)
            {
                actions
                    .push(Action::Run("track.behavior", json!({ "track": track_index, "op": StackOp::Add { behavior: id.clone(), params: None, at: None } })));
            }
        });
    });
}

/// The editing panel: scenes, track placement, camera, takes, timing, keyframes and audio.
pub fn edit_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        super::edit::scene_bar(ui, ctx, actions);
        ui.separator();
        super::edit::panels(ui, view, ctx, actions);
    });
}

// --- Timeline -------------------------------------------------------------------------------------------------------

/// The thin strip above the tracks: only the zoom slider (arming lives in Properties, tracks are managed from the
/// track context menu and the Scene menu).
fn zoom_row(ui: &mut Ui, view: &mut View) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut width = view.timeline.frame_width;
            ui.spacing_mut().slider_width = 120.0;
            if ui.add(egui::Slider::new(&mut width, 2.0..=48.0).logarithmic(true).show_value(false)).on_hover_text("Timeline zoom").changed() {
                view.timeline.frame_width = width;
            }
            ui.label(RichText::new("Zoom").color(TEXT_WEAK));
        });
    });
}

fn timeline(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let tl = model(view, scene, &ctx.world.media, view.playing);
    view.timeline.selected_layers = vec![track_row(view.track)];
    let fps = scene.fps_f64().max(1.0);
    for event in anim_ui::timeline(ui, &tl, &view.timeline) {
        view.timeline.apply(&event);
        match event {
            TimelineEvent::SetFrame(f) => actions.push(Action::Seek(f64::from(f) / fps)),
            TimelineEvent::Layer(LayerEvent::Select { id, .. }) => {
                if let Some((track, take)) = row_target(id) {
                    actions.push(Action::SelectTrack(track));
                    if let Some(take) = take {
                        view.take = take;
                    }
                }
            }
            TimelineEvent::Layer(LayerEvent::ToggleVisible(id)) if !is_behavior_row(id) => match row_target(id) {
                Some((track, None)) => {
                    let enabled = scene.tracks.get(track).is_some_and(|t| t.enabled);
                    actions.push(Action::Run("track.set", json!({ "track": track, "enabled": !enabled })));
                }
                Some((track, Some(take))) => {
                    let enabled = scene.tracks.get(track).and_then(|t| t.takes.get(take)).is_some_and(|t| t.enabled);
                    actions.push(Action::Run("take.set", json!({ "track": track, "take": take, "enabled": !enabled })));
                }
                None => {}
            },
            TimelineEvent::Layer(LayerEvent::Delete(id)) if !is_behavior_row(id) => match row_target(id) {
                Some((track, None)) => actions.push(Action::Run("track.remove", json!({ "track": track }))),
                Some((track, Some(take))) => actions.push(Action::Run("take.remove", json!({ "track": track, "take": take }))),
                None => {}
            },
            TimelineEvent::Layer(LayerEvent::Add) => {
                let puppet = ctx.puppet.unwrap_or("demo");
                actions.push(Action::Run("track.add", json!({ "puppet": puppet, "name": format!("Track {}", scene.tracks.len().saturating_add(1)) })));
            }
            _ => {}
        }
    }
}

/// The Timeline: the tracks only (the Stream workspace's minimal one has no zoom strip either).
pub fn timeline_panel(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>, minimal: bool) {
    if !minimal {
        zoom_row(ui, view);
    }
    timeline(ui, view, ctx, actions);
}
