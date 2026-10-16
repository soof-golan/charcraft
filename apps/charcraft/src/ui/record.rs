//! Record mode: the scene with its transport, the camera and microphone panels, arming, takes and the timeline.

use anim_ui::timeline::LayerEvent;
use anim_ui::{TimelineEvent, TransportEvent, TransportState};
use charcraft_record::ParamGroup;
use egui::{Color32, RichText, Ui};
use serde_json::json;

use super::panels::{camera, chip, microphone, record_button};
use super::timeline::{model, row_target, track_row};
use crate::view::{Action, Ctx, View};

const GROUPS: [(ParamGroup, &str); 7] = [
    (ParamGroup::Head, "Head"),
    (ParamGroup::Brows, "Brows"),
    (ParamGroup::Eyes, "Eyes"),
    (ParamGroup::Mouth, "Mouth"),
    (ParamGroup::Voice, "Voice"),
    (ParamGroup::Pointer, "Pointer"),
    (ParamGroup::Keys, "Keys"),
];

fn transport(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let frames = scene.frame_count().max(1);
    let state = TransportState {
        playing: view.playing,
        looping: view.looping,
        fps: anim_core::num::f64_to_f32(scene.fps_f64()),
        frame: ctx.frame.min(frames.saturating_sub(1)),
        frame_count: frames,
    };
    for event in anim_ui::transport_bar(ui, &state) {
        let fps = scene.fps_f64().max(1.0);
        let last = f64::from(frames.saturating_sub(1)) / fps;
        match event {
            TransportEvent::Play => actions.push(Action::Play(true)),
            TransportEvent::Pause => actions.push(Action::Play(false)),
            TransportEvent::GoToStart => actions.push(Action::Seek(0.0)),
            TransportEvent::GoToEnd => actions.push(Action::Seek(last)),
            TransportEvent::StepBack => actions.push(Action::Seek((view.playhead - 1.0 / fps).max(0.0))),
            TransportEvent::StepForward => actions.push(Action::Seek((view.playhead + 1.0 / fps).min(last))),
            TransportEvent::SetFrame(f) => actions.push(Action::Seek(f64::from(f) / fps)),
            TransportEvent::SetLoop(_) | TransportEvent::SetFps(_) => {}
        }
    }
}

fn arming(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let arm = &ctx.doc.rec.arming;
    ui.horizontal_wrapped(|ui| {
        let recording = view.recording.is_some();
        if record_button(ui, recording).on_hover_text("Record a take on the selected track (R)").clicked() {
            actions.push(if recording { Action::StopRecord } else { Action::StartRecord });
        }
        if let Some(active) = &view.recording {
            chip(ui, &format!("{:.1} s", (view.playhead - active.start).max(0.0)), Color32::from_rgb(0xe5, 0x39, 0x35));
        }
        ui.separator();
        ui.label(RichText::new("Arm").weak());
        for (group, label) in GROUPS {
            let mut on = arm.groups.contains(&group);
            if ui.toggle_value(&mut on, label).changed() {
                actions.push(Action::Run("record.arm", json!({ "groups": [group], "armed": on })));
            }
        }
        let mut audio = arm.audio;
        if ui.toggle_value(&mut audio, "Mic").on_hover_text("Keep the microphone's sound as an audio clip").changed() {
            actions.push(Action::Run("record.arm", json!({ "audio": audio })));
        }
        ui.separator();
        ui.checkbox(&mut view.monitor, "Monitor").on_hover_text("Show the live performance on the selected track");
        if ui.button("+ Track").on_hover_text("Put another puppet on the stage").clicked() {
            let puppet = ctx.puppet.unwrap_or("demo");
            actions.push(Action::Run("track.add", json!({ "puppet": puppet, "name": format!("Track {}", ctx.doc.rec.scene.tracks.len().saturating_add(1)) })));
        }
        if ui.add_enabled(!ctx.doc.rec.scene.tracks.is_empty(), egui::Button::new("Remove track")).clicked() {
            actions.push(Action::Run("track.remove", json!({ "track": view.track })));
        }
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
            TimelineEvent::Layer(LayerEvent::ToggleVisible(id)) => match row_target(id) {
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
            TimelineEvent::Layer(LayerEvent::Delete(id)) => match row_target(id) {
                Some((track, None)) => actions.push(Action::Run("track.remove", json!({ "track": track }))),
                Some((track, Some(take))) => actions.push(Action::Run("take.remove", json!({ "track": track, "take": take }))),
                None => {}
            },
            _ => {}
        }
    }
}

/// Record mode.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::Panel::right("record_inputs").default_size(300.0).min_size(240.0).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            camera(ui, view, ctx, actions);
            ui.separator();
            super::body::body(ui, view, actions);
            ui.separator();
            microphone(ui, view, actions);
            ui.separator();
            super::edit::panels(ui, view, ctx, actions);
        });
    });
    egui::Panel::bottom("record_timeline").resizable(true).default_size(250.0).min_size(150.0).show(ui, |ui| {
        super::edit::scene_bar(ui, ctx, actions);
        arming(ui, view, ctx, actions);
        ui.separator();
        timeline(ui, view, ctx, actions);
    });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical(|ui| {
            let rest = (ui.available_height() - 36.0).max(60.0);
            ui.allocate_ui(egui::vec2(ui.available_width(), rest), |ui| {
                let scene = &ctx.doc.rec.scene;
                view.scene_stage.doc_size = egui::vec2(anim_core::num::f64_to_f32(scene.size.width), anim_core::num::f64_to_f32(scene.size.height));
                view.scene_stage.stage_color = scene.background.map_or(Color32::TRANSPARENT, |c| Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a));
                for event in anim_ui::stage_view(ui, &view.scene_stage, ctx.texture.map(|(t, _)| t)) {
                    if let anim_ui::StageEvent::ViewChanged(v) = event {
                        view.scene_stage.view = v;
                        view.scene_stage.fit_request = false;
                    }
                }
            });
            transport(ui, view, ctx, actions);
        });
    });
}
