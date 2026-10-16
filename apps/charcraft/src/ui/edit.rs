//! Record mode editing panels: scenes, track placement, the scene camera, parameter keyframes with easing, takes
//! (trim, split, blend, punch), sound, and the timing of mouth shapes and key presses.
//!
//! Every control sends a command of the registry; nothing here changes the document.

use anim_ui::theme::visual;
use charcraft_record::timing::{key_runs, voice_runs};
use charcraft_record::{Ease, Placement};
use egui::{CollapsingHeader, RichText, Ui};
use serde_json::{Map, Value, json};

use crate::view::{Action, Ctx, Prompt, View};

const EASES: [(&str, &str); 5] = [("linear", "Linear"), ("hold", "Hold"), ("ease_in", "Ease in"), ("ease_out", "Ease out"), ("ease_in_out", "Ease in-out")];
const VISEMES: [&str; 14] = ["Neutral", "Ah", "D", "Ee", "F", "L", "M", "Oh", "R", "S", "Uh", "W-Oo", "Smile", "Surprised"];
const FRAME_RATES: [i64; 7] = [12, 15, 24, 25, 30, 50, 60];

/// A labelled number. Returns the new value when it was edited.
fn drag(ui: &mut Ui, label: &str, value: f64, speed: f64) -> Option<f64> {
    let mut v = value;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_sized([92.0, 16.0], egui::Label::new(RichText::new(label).size(11.0)).truncate());
        if ui.add(egui::DragValue::new(&mut v).speed(speed)).changed() {
            out = Some(v);
        }
    });
    out
}

/// A number with a button that keys it at the playhead.
fn keyed(ui: &mut Ui, label: &str, value: f64, speed: f64, key: Option<(Option<usize>, &str, f64)>, actions: &mut Vec<Action>) -> Option<f64> {
    let mut v = value;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_sized([92.0, 16.0], egui::Label::new(RichText::new(label).size(11.0)).truncate());
        if ui.add(egui::DragValue::new(&mut v).speed(speed)).changed() {
            out = Some(v);
        }
        if let Some((track, target, time)) = key
            && ui.small_button("+key").on_hover_text("Set a keyframe here at the playhead").clicked()
        {
            actions.push(Action::Run("keyframe.set", key_params(track, target, json!({ "time_seconds": time, "value": v }))));
        }
    });
    out
}

/// Parameters of a keyframe command: the track (the camera when `None`), the lane, and the rest.
fn key_params(track: Option<usize>, target: &str, rest: Value) -> Value {
    let mut map = Map::new();
    if let Some(track) = track {
        map.insert("track".to_owned(), json!(track));
    }
    map.insert("target".to_owned(), json!(target));
    if let Value::Object(more) = rest {
        map.extend(more);
    }
    Value::Object(map)
}

/// The scene tabs: switch, add, duplicate, delete and reorder scenes.
pub fn scene_bar(ui: &mut Ui, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let rec = &ctx.doc.rec;
    let active = rec.active_scene();
    let count = rec.scene_count();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Scenes").weak());
        for (i, scene) in rec.scenes().enumerate() {
            if ui.selectable_label(i == active, visual(&scene.name).into_owned()).clicked() && i != active {
                actions.push(Action::Run("scene.select", json!({ "index": i })));
            }
        }
        if ui.small_button("+").on_hover_text("New scene").clicked() {
            actions.push(Action::Run("scene.add", json!({ "puppet": ctx.puppet })));
        }
        if ui.small_button("Duplicate").on_hover_text("Copy this scene").clicked() {
            actions.push(Action::Run("scene.duplicate", json!({ "open": true })));
        }
        if ui.add_enabled(active > 0, egui::Button::new("\u{2190}").small()).on_hover_text("Move scene earlier").clicked() {
            actions.push(Action::Run("scene.move", json!({ "from": active, "to": active.saturating_sub(1) })));
        }
        if ui.add_enabled(active.saturating_add(1) < count, egui::Button::new("\u{2192}").small()).on_hover_text("Move scene later").clicked() {
            actions.push(Action::Run("scene.move", json!({ "from": active, "to": active.saturating_add(1) })));
        }
        if ui.add_enabled(count > 1, egui::Button::new("Delete").small()).on_hover_text("Delete this scene").clicked() {
            actions.push(Action::Run("scene.remove", json!({ "index": active })));
        }
    });
}

fn scene_settings(ui: &mut Ui, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let mut name = scene.name.clone();
    if ui.text_edit_singleline(&mut name).changed() {
        actions.push(Action::RunMerged("scene.set", json!({ "name": name }), "scene.name"));
    }
    if let Some(w) = drag(ui, "Width", scene.size.width, 1.0) {
        actions.push(Action::RunMerged("scene.set", json!({ "width": w.max(16.0) }), "scene.width"));
    }
    if let Some(h) = drag(ui, "Height", scene.size.height, 1.0) {
        actions.push(Action::RunMerged("scene.set", json!({ "height": h.max(16.0) }), "scene.height"));
    }
    if let Some(d) = drag(ui, "Duration (s)", scene.duration.seconds(), 0.1) {
        actions.push(Action::RunMerged("scene.set", json!({ "duration_seconds": d.max(0.1) }), "scene.duration"));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Frame rate").size(11.0));
        let now = scene.fps.num;
        egui::ComboBox::from_id_salt("scene_fps").selected_text(format!("{now} fps")).show_ui(ui, |ui| {
            for rate in FRAME_RATES {
                if ui.selectable_label(now == rate && scene.fps.den == 1, format!("{rate} fps")).clicked() {
                    actions.push(Action::Run("scene.set", json!({ "fps_num": rate, "fps_den": 1 })));
                }
            }
        });
    });
    ui.horizontal(|ui| {
        let mut transparent = scene.background.is_none();
        if ui.checkbox(&mut transparent, "Transparent").changed() {
            actions.push(Action::Run("scene.set", if transparent { json!({ "transparent": true }) } else { json!({ "background": [32, 32, 40, 255] }) }));
        }
        if let Some(bg) = scene.background {
            let mut color = [bg.r, bg.g, bg.b, bg.a];
            if ui.color_edit_button_srgba_unmultiplied(&mut color).changed() {
                actions.push(Action::RunMerged("scene.set", json!({ "background": color }), "scene.background"));
            }
        }
    });
}

fn track_section(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let Some(track) = scene.tracks.get(view.track) else {
        ui.weak("Add a puppet track first.");
        return;
    };
    let index = view.track;
    ui.horizontal(|ui| {
        let mut name = track.name.clone();
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0)).changed() {
            actions.push(Action::RunMerged("track.set", json!({ "track": index, "name": name }), "track.name"));
        }
        let mut locked = track.locked;
        if ui.checkbox(&mut locked, "Lock").on_hover_text("A locked track refuses edits").changed() {
            actions.push(Action::Run("track.set", json!({ "track": index, "locked": locked })));
        }
        if ui.add_enabled(index > 0, egui::Button::new("\u{2191}").small()).on_hover_text("Move track up (drawn behind)").clicked() {
            actions.push(Action::Run("track.move", json!({ "from": index, "to": index.saturating_sub(1) })));
            actions.push(Action::SelectTrack(index.saturating_sub(1)));
        }
        if ui.add_enabled(index.saturating_add(1) < scene.tracks.len(), egui::Button::new("\u{2193}").small()).on_hover_text("Move track down (drawn in front)").clicked() {
            actions.push(Action::Run("track.move", json!({ "from": index, "to": index.saturating_add(1) })));
            actions.push(Action::SelectTrack(index.saturating_add(1)));
        }
    });
    let mut p: Placement = track.placement;
    let time = view.playhead;
    let mut changed = false;
    let tr = Some(index);
    changed |= keyed(ui, "Position X", p.position.x, 1.0, Some((tr, "position_x", time)), actions).map(|v| p.position.x = v).is_some();
    changed |= keyed(ui, "Position Y", p.position.y, 1.0, Some((tr, "position_y", time)), actions).map(|v| p.position.y = v).is_some();
    changed |= keyed(ui, "Scale X", p.scale.x, 0.01, Some((tr, "scale_x", time)), actions).map(|v| p.scale.x = v).is_some();
    changed |= keyed(ui, "Scale Y", p.scale.y, 0.01, Some((tr, "scale_y", time)), actions).map(|v| p.scale.y = v).is_some();
    changed |= keyed(ui, "Rotation", p.rotation, 0.5, Some((tr, "rotation", time)), actions).map(|v| p.rotation = v).is_some();
    changed |= keyed(ui, "Opacity", p.opacity, 0.01, Some((tr, "opacity", time)), actions).map(|v| p.opacity = v.clamp(0.0, 1.0)).is_some();
    if changed {
        actions.push(Action::RunMerged("track.set", json!({ "track": index, "placement": p }), "track.placement"));
    }
    if ui.small_button("Reset placement").clicked() {
        actions.push(Action::Run("track.set", json!({ "track": index, "placement": Placement::default() })));
    }
}

fn camera_section(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let cam = &scene.camera;
    let time = view.playhead;
    let (mut x, mut y, mut zoom, mut rot) = (cam.position.x, cam.position.y, cam.zoom, cam.rotation);
    let mut changed = false;
    changed |= keyed(ui, "Pan X", x, 1.0, Some((None, "position_x", time)), actions).map(|v| x = v).is_some();
    changed |= keyed(ui, "Pan Y", y, 1.0, Some((None, "position_y", time)), actions).map(|v| y = v).is_some();
    changed |= keyed(ui, "Zoom", zoom, 0.01, Some((None, "zoom", time)), actions).map(|v| zoom = v.clamp(0.05, 40.0)).is_some();
    changed |= keyed(ui, "Rotation", rot, 0.5, Some((None, "rotation", time)), actions).map(|v| rot = v).is_some();
    if changed {
        actions.push(Action::RunMerged("camera.set", json!({ "position_x": x, "position_y": y, "zoom": zoom, "rotation": rot }), "camera.set"));
    }
    let face = cam.zoom_on_face;
    let mut follow = face.is_some();
    if ui.checkbox(&mut follow, "Zoom on face").on_hover_text("The camera moves towards a puppet's face").changed() {
        actions.push(Action::Run("camera.set", if follow { json!({ "zoom_on_face": { "track": view.track } }) } else { json!({ "zoom_on_face": null }) }));
    }
    if let Some(f) = face {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Face of").size(11.0));
            let name = scene.tracks.get(f.track).map_or("(none)", |t| t.name.as_str());
            egui::ComboBox::from_id_salt("zoom_face_track").selected_text(visual(name).into_owned()).show_ui(ui, |ui| {
                for (i, t) in scene.tracks.iter().enumerate() {
                    if ui.selectable_label(i == f.track, visual(&t.name).into_owned()).clicked() {
                        actions.push(Action::Run("camera.set", json!({ "zoom_on_face": { "track": i } })));
                    }
                }
            });
        });
        if let Some(a) = drag(ui, "Amount", f.amount, 0.01) {
            actions.push(Action::RunMerged("camera.set", json!({ "zoom_on_face": { "track": f.track, "amount": a.clamp(0.0, 1.0) } }), "camera.face"));
        }
        if let Some(z) = drag(ui, "Face zoom", f.zoom, 0.01) {
            actions.push(Action::RunMerged("camera.set", json!({ "zoom_on_face": { "track": f.track, "zoom": z.clamp(0.05, 40.0) } }), "camera.facezoom"));
        }
    }
    if ui.small_button("Reset camera").clicked() {
        actions.push(Action::Run("camera.set", json!({ "reset": true })));
    }
}

const fn ease_name(ease: &Ease) -> &'static str {
    match ease {
        Ease::Linear => "Linear",
        Ease::Hold => "Hold",
        Ease::EaseIn => "Ease in",
        Ease::EaseOut => "Ease out",
        Ease::EaseInOut => "Ease in-out",
        Ease::Bezier { .. } => "Bezier",
    }
}

/// The keys of one lane, each with time, value, easing and delete.
fn lane_rows(ui: &mut Ui, track: Option<usize>, lane: &charcraft_record::Lane, actions: &mut Vec<Action>) {
    for (k, key) in lane.keys.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut t = key.t.seconds();
            if ui.add(egui::DragValue::new(&mut t).speed(0.02).suffix(" s")).changed() {
                actions.push(Action::RunMerged("keyframe.set", key_params(track, &lane.target, json!({ "index": k, "time_seconds": t.max(0.0) })), "key.time"));
            }
            let mut v = key.value;
            if ui.add(egui::DragValue::new(&mut v).speed(0.05)).changed() {
                actions.push(Action::RunMerged("keyframe.set", key_params(track, &lane.target, json!({ "index": k, "value": v })), "key.value"));
            }
            egui::ComboBox::from_id_salt(("ease", track, &lane.target, k)).width(84.0).selected_text(ease_name(&key.ease)).show_ui(ui, |ui| {
                for (id, label) in EASES {
                    if ui.selectable_label(ease_name(&key.ease) == label, label).clicked() {
                        actions.push(Action::Run("keyframe.set", key_params(track, &lane.target, json!({ "index": k, "ease": id }))));
                    }
                }
                if ui.selectable_label(matches!(key.ease, Ease::Bezier { .. }), "Bezier").clicked() {
                    let bezier = json!({ "kind": "bezier", "x1": 0.42, "y1": 0.0, "x2": 0.58, "y2": 1.0 });
                    actions.push(Action::Run("keyframe.set", key_params(track, &lane.target, json!({ "index": k, "ease": bezier }))));
                }
            });
            if ui.small_button("\u{00D7}").on_hover_text("Delete keyframe").clicked() {
                actions.push(Action::Run("keyframe.remove", key_params(track, &lane.target, json!({ "index": k }))));
            }
        });
        if let Ease::Bezier { x1, y1, x2, y2 } = key.ease {
            ui.horizontal(|ui| {
                let (mut a, mut b, mut c, mut d) = (x1, y1, x2, y2);
                let mut changed = false;
                for value in [&mut a, &mut b, &mut c, &mut d] {
                    changed |= ui.add(egui::DragValue::new(value).speed(0.01)).changed();
                }
                if changed {
                    let bezier = json!({ "kind": "bezier", "x1": a.clamp(0.0, 1.0), "y1": b, "x2": c.clamp(0.0, 1.0), "y2": d });
                    actions.push(Action::RunMerged("keyframe.set", key_params(track, &lane.target, json!({ "index": k, "ease": bezier })), "key.bezier"));
                }
            });
        }
    }
}

fn keyframes_section(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    let time = view.playhead;
    if let Some(track) = scene.tracks.get(view.track) {
        ui.strong(visual(&track.name).into_owned());
        if track.lanes.is_empty() {
            ui.weak("No keyframes. Use +key beside a value above, or pick a behavior parameter below.");
        }
        for lane in &track.lanes {
            CollapsingHeader::new(format!("{} ({})", lane.target, lane.keys.len())).id_salt(("lane", view.track, &lane.target)).default_open(true).show(ui, |ui| {
                lane_rows(ui, Some(view.track), lane, actions);
                if ui.small_button("Clear lane").clicked() {
                    actions.push(Action::Run("keyframe.clear", key_params(Some(view.track), &lane.target, json!({}))));
                }
            });
        }
        if let Some(stack) = ctx.stack {
            ui.menu_button("Key a behavior parameter...", |ui| {
                for info in &stack.behaviors {
                    let Some(map) = info.params.as_object() else { continue };
                    ui.menu_button(visual(&info.name).into_owned(), |ui| {
                        for (name, value) in map {
                            let Some(now) = value.as_f64() else { continue };
                            if ui.button(name).clicked() {
                                let target = format!("behavior.{}.{name}", info.index);
                                actions.push(Action::Run("keyframe.set", key_params(Some(view.track), &target, json!({ "time_seconds": time, "value": now }))));
                                ui.close();
                            }
                        }
                    });
                }
            });
        }
    }
    ui.separator();
    ui.strong("Camera");
    if scene.camera.lanes.is_empty() {
        ui.weak("No camera keyframes.");
    }
    for lane in &scene.camera.lanes {
        CollapsingHeader::new(format!("{} ({})", lane.target, lane.keys.len())).id_salt(("camlane", &lane.target)).default_open(true).show(ui, |ui| {
            lane_rows(ui, None, lane, actions);
            if ui.small_button("Clear lane").clicked() {
                actions.push(Action::Run("keyframe.clear", key_params(None, &lane.target, json!({}))));
            }
        });
    }
}

fn punch_section(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let punch = ctx.doc.rec.arming.punch.map(|r| (r.start.seconds(), r.end().seconds()));
    let mut on = punch.is_some();
    if ui.checkbox(&mut on, "Punch in and out").on_hover_text("Record only between these times").changed() {
        actions.push(Action::Run("record.punch", if on { json!({ "start_seconds": view.playhead, "end_seconds": view.playhead + 2.0 }) } else { json!({}) }));
    }
    if let Some((start, end)) = punch {
        ui.horizontal(|ui| {
            ui.label(RichText::new("In").size(11.0));
            let mut s = start;
            if ui.add(egui::DragValue::new(&mut s).speed(0.05).suffix(" s")).changed() && s < end {
                actions.push(Action::RunMerged("record.punch", json!({ "start_seconds": s.max(0.0), "end_seconds": end }), "punch"));
            }
            if ui.small_button("= playhead").clicked() && view.playhead < end {
                actions.push(Action::Run("record.punch", json!({ "start_seconds": view.playhead, "end_seconds": end })));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Out").size(11.0));
            let mut e = end;
            if ui.add(egui::DragValue::new(&mut e).speed(0.05).suffix(" s")).changed() && e > start {
                actions.push(Action::RunMerged("record.punch", json!({ "start_seconds": start, "end_seconds": e }), "punch"));
            }
            if ui.small_button("= playhead").clicked() && view.playhead > start {
                actions.push(Action::Run("record.punch", json!({ "start_seconds": start, "end_seconds": view.playhead })));
            }
        });
    }
}

/// The takes of the selected track: enable, select, move, blend, trim, split and delete.
pub fn takes_section(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    punch_section(ui, view, ctx, actions);
    ui.separator();
    let Some(track) = ctx.doc.rec.scene.tracks.get(view.track) else {
        ui.weak("Add a puppet track first.");
        return;
    };
    if track.takes.is_empty() {
        ui.weak("Nothing recorded yet. Arm the parts you want, then press Record.");
    }
    let playhead = view.playhead;
    let count = track.takes.len();
    for (k, take) in track.takes.iter().enumerate() {
        let selected = view.take == k;
        ui.horizontal(|ui| {
            let mut on = take.enabled;
            if ui.checkbox(&mut on, "").changed() {
                actions.push(Action::Run("take.set", json!({ "track": view.track, "take": k, "enabled": on })));
            }
            if ui.selectable_label(selected, RichText::new(visual(&take.name).into_owned()).strong()).clicked() {
                view.take = k;
            }
            ui.weak(format!("{:.1}s at {:.1}s", take.duration.seconds(), take.start.seconds()));
            if ui.add_enabled(k > 0, egui::Button::new("\u{2191}").small()).on_hover_text("Stack lower (earlier takes are overridden by later ones)").clicked() {
                actions.push(Action::Run("take.move", json!({ "track": view.track, "from": k, "to": k.saturating_sub(1) })));
            }
            if ui.add_enabled(k.saturating_add(1) < count, egui::Button::new("\u{2193}").small()).on_hover_text("Stack higher").clicked() {
                actions.push(Action::Run("take.move", json!({ "track": view.track, "from": k, "to": k.saturating_add(1) })));
            }
            if ui.small_button("\u{00D7}").on_hover_text("Delete take").clicked() {
                actions.push(Action::Run("take.remove", json!({ "track": view.track, "take": k })));
            }
        });
        if selected {
            let (start, end) = (take.start.seconds(), take.start.seconds() + take.duration.seconds());
            let inside = playhead > start && playhead < end;
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(inside, egui::Button::new("Split").small()).on_hover_text("Cut the take in two at the playhead").clicked() {
                    actions.push(Action::Run("take.split", json!({ "track": view.track, "take": k, "time_seconds": playhead })));
                }
                if ui.add_enabled(inside, egui::Button::new("Trim start").small()).on_hover_text("Cut the take's start to the playhead").clicked() {
                    actions.push(Action::Run("take.trim", json!({ "track": view.track, "take": k, "start_seconds": playhead })));
                }
                if ui.add_enabled(inside, egui::Button::new("Trim end").small()).on_hover_text("Cut the take's end to the playhead").clicked() {
                    actions.push(Action::Run("take.trim", json!({ "track": view.track, "take": k, "end_seconds": playhead })));
                }
            });
            if let Some(s) = drag(ui, "Starts at (s)", start, 0.05) {
                actions.push(Action::RunMerged("take.set", json!({ "track": view.track, "take": k, "start_seconds": s.max(0.0) }), "take.start"));
            }
            if let Some(b) = drag(ui, "Blend in (s)", take.blend_in.seconds(), 0.02) {
                actions.push(Action::RunMerged("take.set", json!({ "track": view.track, "take": k, "blend_in_seconds": b.max(0.0) }), "take.blend_in"));
            }
            if let Some(b) = drag(ui, "Blend out (s)", take.blend_out.seconds(), 0.02) {
                actions.push(Action::RunMerged("take.set", json!({ "track": view.track, "take": k, "blend_out_seconds": b.max(0.0) }), "take.blend_out"));
            }
        }
    }
}

/// Sound tracks and their clips.
fn audio_section(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let scene = &ctx.doc.rec.scene;
    ui.horizontal(|ui| {
        if ui.button("Import audio...").on_hover_text("Put a sound file on an audio track at the playhead").clicked() {
            view.prompt = Some((Prompt::Audio, std::env::current_dir().map(|d| format!("{}/", d.display())).unwrap_or_default()));
        }
        if ui.button("+ Audio track").clicked() {
            actions.push(Action::Run("audio.add_track", json!({})));
        }
    });
    let playhead = view.playhead;
    for (a, audio) in scene.audio.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut on = audio.enabled;
            if ui.checkbox(&mut on, "").changed() {
                actions.push(Action::Run("audio.set_track", json!({ "track": a, "enabled": on })));
            }
            let mut name = audio.name.clone();
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(110.0)).changed() {
                actions.push(Action::RunMerged("audio.set_track", json!({ "track": a, "name": name }), "audio.name"));
            }
            if ui.small_button("\u{00D7}").on_hover_text("Remove the audio track").clicked() {
                actions.push(Action::Run("audio.remove_track", json!({ "track": a })));
            }
        });
        for (c, clip) in audio.clips.iter().enumerate() {
            let start = clip.start.seconds();
            let length = clip.duration.map_or(0.0, anim_core::Tick::seconds);
            let inside = playhead > start && (length <= 0.0 || playhead < start + length);
            ui.indent(("clip", a, c), |ui| {
                ui.label(RichText::new(visual(&clip.name).into_owned()).strong());
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Lip sync").on_hover_text("Compute a lip sync take on the selected puppet track from this sound").clicked() {
                        actions.push(Action::LipsyncClip { track: view.track, audio_track: a, clip: c });
                    }
                    if ui.add_enabled(inside, egui::Button::new("Split").small()).on_hover_text("Cut the clip at the playhead").clicked() {
                        actions.push(Action::Run("audio.split_clip", json!({ "track": a, "clip": c, "time_seconds": playhead })));
                    }
                    if ui.small_button("\u{00D7}").on_hover_text("Delete clip").clicked() {
                        actions.push(Action::Run("audio.remove_clip", json!({ "track": a, "clip": c })));
                    }
                });
                if let Some(s) = drag(ui, "Starts at (s)", start, 0.05) {
                    actions.push(Action::RunMerged("audio.move_clip", json!({ "track": a, "clip": c, "start_seconds": s.max(0.0) }), "clip.start"));
                }
                if let Some(t) = drag(ui, "Trim in (s)", clip.trim_in.seconds(), 0.05) {
                    actions.push(Action::RunMerged("audio.set_clip", json!({ "track": a, "clip": c, "trim_in_seconds": t.max(0.0) }), "clip.trim"));
                }
                if let Some(g) = drag(ui, "Gain", f64::from(clip.gain), 0.01) {
                    actions.push(Action::RunMerged("audio.set_clip", json!({ "track": a, "clip": c, "gain": g.clamp(0.0, 4.0) }), "clip.gain"));
                }
                if scene.audio.len() > 1 {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Move to").size(11.0));
                        for (b, other) in scene.audio.iter().enumerate().filter(|(b, _)| *b != a) {
                            if ui.small_button(visual(&other.name).into_owned()).clicked() {
                                actions.push(Action::Run("audio.move_clip", json!({ "track": a, "clip": c, "to_track": b })));
                            }
                        }
                    });
                }
            });
        }
    }
}

fn viseme_combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, mut pick: impl FnMut(&str)) {
    egui::ComboBox::from_id_salt(id).width(86.0).selected_text(current).show_ui(ui, |ui| {
        for name in VISEMES {
            if ui.selectable_label(name == current, name).clicked() {
                pick(name);
            }
        }
    });
}

/// Mouth shapes and key presses of the selected take, as editable runs.
fn timing_section(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some(take) = ctx.doc.rec.scene.tracks.get(view.track).and_then(|t| t.takes.get(view.take)) else {
        ui.weak("Select a take in the Takes list.");
        return;
    };
    ui.weak(format!("Take: {}", visual(&take.name)));
    let voice = voice_runs(take);
    let keys = key_runs(take);
    if voice.is_empty() && keys.is_empty() {
        ui.weak("This take has no mouth shapes or key presses.");
    }
    let (track, t) = (view.track, view.take);
    for (i, run) in voice.iter().enumerate() {
        ui.horizontal(|ui| {
            let current = run.viseme.map_or("Neutral", |v| v.ca_layer_name());
            viseme_combo(ui, ("viseme", track, t, i), current, |name| {
                actions.push(Action::Run("timing.set_viseme", json!({ "track": track, "take": t, "index": i, "viseme": name })));
            });
            let mut start = run.start.seconds();
            let enabled = i > 0;
            if ui.add_enabled(enabled, egui::DragValue::new(&mut start).speed(0.01).suffix(" s")).changed() {
                actions.push(Action::RunMerged("timing.set_viseme", json!({ "track": track, "take": t, "index": i, "start_seconds": start }), "timing.start"));
            }
            ui.weak(format!("to {:.2}", run.end.seconds()));
            if ui.small_button("\u{00D7}").on_hover_text("Remove this run").clicked() {
                actions.push(Action::Run("timing.remove_viseme", json!({ "track": track, "take": t, "index": i })));
            }
        });
    }
    if ui.small_button("Insert mouth shape at playhead").clicked() {
        actions.push(Action::Run("timing.insert_viseme", json!({ "track": track, "take": t, "start_seconds": view.playhead, "end_seconds": view.playhead + 0.15, "viseme": "Ah" })));
    }
    for (i, run) in keys.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(visual(&run.key).into_owned()).strong());
            let (mut down, mut up) = (run.start.seconds(), run.end.seconds());
            let moved_down = ui.add(egui::DragValue::new(&mut down).speed(0.01).suffix(" s")).changed();
            let moved_up = ui.add(egui::DragValue::new(&mut up).speed(0.01).suffix(" s")).changed();
            if (moved_down || moved_up) && up > down {
                actions.push(Action::RunMerged("timing.set_key", json!({ "track": track, "take": t, "index": i, "start_seconds": down, "end_seconds": up }), "timing.key"));
            }
            if ui.small_button("\u{00D7}").on_hover_text("Remove this key press").clicked() {
                actions.push(Action::Run("timing.remove_key", json!({ "track": track, "take": t, "index": i })));
            }
        });
    }
}

/// The right-hand editing panels of Record mode.
pub fn panels(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    CollapsingHeader::new("Scene").default_open(true).show(ui, |ui| scene_settings(ui, ctx, actions));
    CollapsingHeader::new("Track placement").default_open(true).show(ui, |ui| track_section(ui, view, ctx, actions));
    CollapsingHeader::new("Camera").default_open(true).show(ui, |ui| camera_section(ui, view, ctx, actions));
    CollapsingHeader::new("Takes").default_open(true).show(ui, |ui| takes_section(ui, view, ctx, actions));
    CollapsingHeader::new("Timing").default_open(false).show(ui, |ui| timing_section(ui, view, ctx, actions));
    CollapsingHeader::new("Keyframes").default_open(false).show(ui, |ui| keyframes_section(ui, view, ctx, actions));
    CollapsingHeader::new("Audio").default_open(true).show(ui, |ui| audio_section(ui, view, ctx, actions));
}
