//! Small shared widgets and the camera and microphone panels.

use std::collections::BTreeSet;

use anim_ui::theme::visual;
use charcraft_tracking::input::FaceInput as TrackedFace;
use egui::epaint::EllipseShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use crate::live::{GAIN_RANGE, MicAction};
use crate::view::{Action, Ctx, LiveSnapshot, View};

const GOOD: Color32 = Color32::from_rgb(0x4c, 0xc2, 0x7a);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x3c);
const REC: Color32 = Color32::from_rgb(0xe5, 0x39, 0x35);
const SKIN: Color32 = Color32::from_rgb(0xf0, 0xc4, 0xa0);
const INK: Color32 = Color32::from_rgb(0x2a, 0x22, 0x2c);
const POINT: Color32 = Color32::from_rgb(0x5d, 0xf2, 0x9a);

/// A small coloured pill with text.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(visual(text).into_owned(), FontId::proportional(12.0), Color32::WHITE);
    let size = galley.size() + vec2(14.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 9.0, color);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    response
}

/// A labelled horizontal meter. `lo..=hi` is the range; a range below zero draws from the middle.
pub fn meter(ui: &mut Ui, label: &str, value: f32, lo: f32, hi: f32) {
    ui.horizontal(|ui| {
        ui.add_sized([62.0, 14.0], egui::Label::new(egui::RichText::new(label).size(11.0).weak()));
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(40.0) - 4.0, 10.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
        let span = (hi - lo).max(f32::EPSILON);
        let at = |v: f32| rect.left() + rect.width() * ((v.clamp(lo, hi) - lo) / span);
        let zero = at(0.0f32.clamp(lo, hi));
        let x = at(value);
        let bar = Rect::from_x_y_ranges(zero.min(x)..=zero.max(x), rect.y_range());
        painter.rect_filled(bar, 3.0, ui.visuals().selection.bg_fill);
        if lo < 0.0 {
            painter.line_segment([Pos2::new(zero, rect.top()), Pos2::new(zero, rect.bottom())], Stroke::new(1.0, ui.visuals().weak_text_color()));
        }
    });
}

fn rotate(p: egui::Vec2, angle: f32) -> egui::Vec2 {
    let (s, c) = angle.sin_cos();
    vec2(p.x * c - p.y * s, p.x * s + p.y * c)
}

/// A schematic face that follows the tracked values: head turn and tilt, brows, eyes, gaze, mouth.
pub fn face_preview(ui: &mut Ui, face: Option<&TrackedFace>, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), StrokeKind::Inside);
    let Some(face) = face.filter(|f| f.tracked) else {
        painter.text(rect.center(), Align2::CENTER_CENTER, "no face", FontId::proportional(13.0), ui.visuals().weak_text_color());
        return;
    };
    let unit = rect.height() / 2.6;
    let centre = rect.center() + vec2(face.head.x * unit * 0.6, -face.head.y * unit * 0.6);
    let roll = face.head.roll;
    let yaw = face.head.yaw.clamp(-0.8, 0.8);
    let place = |p: egui::Vec2| centre + rotate(vec2(p.x * (1.0 - yaw.abs() * 0.25) + yaw * unit * 0.45, p.y - face.head.pitch * unit * 0.3), roll);
    painter.add(EllipseShape::filled(centre, vec2(unit * 0.92, unit * 1.12), SKIN));
    for (side, brow, open) in [(-1.0_f32, face.brow_l, face.eye_open_l), (1.0, face.brow_r, face.eye_open_r)] {
        let eye = place(vec2(side * unit * 0.42, -unit * 0.1));
        let height = unit * 0.2 * open.clamp(0.08, 1.0);
        painter.add(EllipseShape::filled(eye, vec2(unit * 0.22, height), Color32::WHITE));
        let pupil = eye + vec2(face.gaze[0] * unit * 0.08, -face.gaze[1] * unit * 0.05);
        painter.add(EllipseShape::filled(pupil, vec2(unit * 0.09, height.min(unit * 0.09)), INK));
        let a = place(vec2(side * unit * 0.42 - unit * 0.2, -unit * 0.42 - brow * unit * 0.14));
        let b = place(vec2(side * unit * 0.42 + unit * 0.2, -unit * 0.42 - brow * unit * 0.14));
        painter.line_segment([a, b], Stroke::new(2.5, INK));
    }
    let mouth = place(vec2(0.0, unit * 0.52));
    let width = unit * (0.2 + 0.18 * face.mouth_wide.clamp(0.0, 1.0));
    let open = unit * (0.03 + 0.34 * face.mouth_open.clamp(0.0, 1.0));
    let smile = face.smile.clamp(-1.0, 1.0);
    let points: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = f32::from(u8::try_from(i).unwrap_or(0)) / 12.0 * 2.0 - 1.0;
            mouth + rotate(vec2(t * width, smile * unit * 0.12 * (1.0 - t * t) + open * (1.0 - t * t)), roll)
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(2.5, INK)));
}

/// The points to draw over the video: the landmarks, once each (a scripted face leaves most of the mesh in one place).
fn distinct_points(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut seen = BTreeSet::new();
    points
        .iter()
        .copied()
        .filter(|p| {
            // 1/256 of the image is finer than the eye can tell apart at the preview's size.
            let key = (anim_core::num::round_f64_to_i32(f64::from(p[0]) * 256.0), anim_core::num::round_f64_to_i32(f64::from(p[1]) * 256.0));
            seen.insert(key)
        })
        .collect()
}

/// The live camera image with the tracked landmark points over it. `mirror` shows it as a mirror would.
///
/// Without an image (a scripted demo source has none) the points are drawn over a plain backdrop and say so.
pub fn video_preview(ui: &mut Ui, live: &LiveSnapshot, mirror: bool, width: f32) {
    let aspect = live.preview.as_ref().map_or(4.0 / 3.0, |t| {
        let [w, h] = t.size();
        anim_core::num::f64_to_f32(anim_core::num::usize_to_f64(w).max(1.0) / anim_core::num::usize_to_f64(h).max(1.0))
    });
    let (rect, _) = ui.allocate_exact_size(vec2(width, width / aspect.max(0.2)), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_rgb(0x10, 0x12, 0x16));
    let uv = if mirror { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)) };
    if let Some(texture) = &live.preview {
        painter.image(texture.id(), rect, uv, Color32::WHITE);
    }
    let at = |p: &[f32; 2]| pos2(rect.left() + rect.width() * if mirror { 1.0 - p[0] } else { p[0] }, rect.top() + rect.height() * p[1]);
    if live.tracking {
        for p in distinct_points(&live.preview_points) {
            painter.circle_filled(at(&p), 1.6, POINT);
        }
    }
    let note = if !live.tracking {
        "camera off"
    } else if live.preview.is_none() && live.preview_points.is_empty() {
        "waiting for the camera"
    } else if live.preview.is_none() {
        "demo source: tracked points only"
    } else {
        ""
    };
    if !note.is_empty() {
        let pos = if live.tracking { rect.center_bottom() - vec2(0.0, 10.0) } else { rect.center() };
        painter.text(pos, Align2::CENTER_CENTER, note, FontId::proportional(12.0), Color32::from_gray(0xa0));
    }
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), StrokeKind::Inside);
}

/// The camera device picker: a list of the system's cameras and a refresh button.
pub fn camera_picker(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let current = if view.live.cameras_searching {
            "Looking for cameras\u{2026}".to_owned()
        } else {
            view.live.cameras.iter().find(|(index, _)| *index == view.live.camera_index).map_or_else(|| "No camera found".to_owned(), |(_, name)| name.clone())
        };
        let before = view.live.camera_index;
        egui::ComboBox::from_id_salt("camera_picker").width((ui.available_width() - 34.0).max(80.0)).selected_text(visual(&current).into_owned()).show_ui(
            ui,
            |ui| {
                for (index, name) in &view.live.cameras {
                    ui.selectable_value(&mut view.live.camera_index, *index, visual(name).into_owned());
                }
            },
        );
        if view.live.camera_index != before && view.live.tracking {
            actions.push(Action::Tracking("tracking_start", json!({ "index": view.live.camera_index })));
        }
        let refresh = ui.add_enabled(!view.live.cameras_searching, egui::Button::new("\u{21BB}").small());
        if refresh.on_hover_text("Look for cameras and microphones again").clicked() {
            actions.push(Action::InputsRefresh);
        }
    });
    if let Some(error) = view.live.cameras_error.as_ref().filter(|_| !view.live.cameras_searching) {
        ui.colored_label(ui.visuals().error_fg_color, visual(error).into_owned());
    }
}

/// The camera panel: picker, live video with the tracked points, start and stop, rest pose, mirror and the values.
pub fn camera(ui: &mut Ui, view: &mut View, _ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.strong("Camera");
        if view.live.tracking {
            chip(ui, "tracking", GOOD);
        } else {
            chip(ui, "off", ui.visuals().widgets.inactive.weak_bg_fill);
        }
        if let Some(source) = view.live.source.as_ref().filter(|_| view.live.tracking) {
            ui.weak(visual(source).into_owned());
        } else if cfg!(not(feature = "live")) {
            ui.weak("this build has no camera support: demo face only");
        }
    });
    camera_picker(ui, view, actions);
    video_preview(ui, &view.live, view.mirror, ui.available_width().min(300.0));
    ui.horizontal_wrapped(|ui| {
        if view.live.tracking {
            if ui.button("Stop").clicked() {
                actions.push(Action::Tracking("tracking_stop", json!({})));
            }
        } else if cfg!(feature = "live") {
            if ui.button("Start camera").clicked() {
                actions.push(Action::Tracking("tracking_start", json!({ "index": view.live.camera_index })));
            }
            if view.live.problem.is_some() && ui.button("Use demo face").on_hover_text("Plays a scripted face instead of the camera.").clicked() {
                actions.push(Action::Tracking("tracking_start", json!({ "demo": true })));
            }
        } else if ui.button("Start demo face").clicked() {
            actions.push(Action::Tracking("tracking_start", json!({ "demo": true })));
        }
        let mut mirror = view.mirror;
        if ui.checkbox(&mut mirror, "Mirror").changed() {
            actions.push(Action::Mirror(mirror));
        }
    });
    rest_pose(ui, view, actions);
    ui.add_space(2.0);
    face_preview(ui, view.live.face.as_ref(), vec2(ui.available_width().min(260.0), 96.0));
    if let Some(face) = view.live.face.as_ref().filter(|f| f.tracked) {
        meter(ui, "yaw", face.head.yaw, -0.8, 0.8);
        meter(ui, "pitch", face.head.pitch, -0.8, 0.8);
        meter(ui, "roll", face.head.roll, -0.8, 0.8);
        meter(ui, "brows", f32::midpoint(face.brow_l, face.brow_r), -1.0, 1.0);
        meter(ui, "eyes", f32::midpoint(face.eye_open_l, face.eye_open_r), 0.0, 1.0);
        meter(ui, "mouth", face.mouth_open, 0.0, 1.0);
        meter(ui, "smile", face.smile, -1.0, 1.0);
    }
    if let Some(problem) = &view.live.problem {
        ui.colored_label(WARN, visual(problem).into_owned());
    }
}

/// Set Rest Pose for the face (neutral expressions) and the body (neutral posture), with the capture's progress.
pub fn rest_pose(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    let calibrating = view.live.calibrating.is_some();
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Set Rest Pose").size(11.0).weak());
        for (target, label, hint) in [
            ("face", "Face", "Relax your face and look at the camera, then press: the neutral expressions."),
            ("body", "Body", "Sit the way you perform, then press: where the head sits, how far it is from the camera and how it is turned. A webcam sees the body through the head."),
            ("both", "Both", "Capture the neutral face and the neutral posture together."),
        ] {
            if ui.add_enabled(view.live.tracking && !calibrating, egui::Button::new(label)).on_hover_text(hint).clicked() {
                actions.push(Action::Tracking("tracking_set_rest_pose", json!({ "target": target, "frames": 30 })));
            }
        }
        if ui.add_enabled(view.live.tracking, egui::Button::new("Reset")).on_hover_text("Forget the captured rest pose and use the typical one.").clicked() {
            actions.push(Action::Tracking("tracking_reset_calibration", json!({})));
        }
    });
    if let Some((done, wanted)) = view.live.calibrating {
        ui.add(
            egui::ProgressBar::new(f32::from(u16::try_from(done).unwrap_or(0)) / f32::from(u16::try_from(wanted.max(1)).unwrap_or(1)))
                .desired_height(6.0)
                .text("hold still"),
        );
    }
}

/// The microphone device picker.
pub fn mic_picker(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    let current = view.live.mic_device.clone().unwrap_or_else(|| "System default".to_owned());
    ui.horizontal(|ui| {
        let mut chosen = view.live.mic_device.clone();
        egui::ComboBox::from_id_salt("mic_picker").width((ui.available_width() - 34.0).max(80.0)).selected_text(visual(&current).into_owned()).show_ui(
            ui,
            |ui| {
                ui.selectable_value(&mut chosen, None, "System default");
                for name in &view.live.mic_devices {
                    ui.selectable_value(&mut chosen, Some(name.clone()), visual(name).into_owned());
                }
            },
        );
        if chosen != view.live.mic_device {
            view.live.mic_device.clone_from(&chosen);
            actions.push(Action::Mic(MicAction::SetDevice(chosen)));
        }
        if ui.small_button("\u{21BB}").on_hover_text("Look for microphones again").clicked() {
            actions.push(Action::InputsRefresh);
        }
    });
}

/// The input sensitivity slider (a gain applied before the level meter and the lip sync).
pub fn mic_sensitivity(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.add_sized([62.0, 14.0], egui::Label::new(egui::RichText::new("sensitivity").size(11.0).weak()));
        let mut gain = view.live.mic_gain;
        let slider = egui::Slider::new(&mut gain, GAIN_RANGE.0..=GAIN_RANGE.1).logarithmic(true).show_value(true).fixed_decimals(1);
        if ui.add(slider).on_hover_text("Raise it for a quiet voice. The mouth shapes and the level meter follow it.").changed() {
            view.live.mic_gain = gain;
            actions.push(Action::Mic(MicAction::SetGain(gain)));
        }
    });
}

/// The microphone panel: device, sensitivity, level, the mouth shape the voice makes, start and stop.
pub fn microphone(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.strong("Microphone");
        if view.live.mic {
            chip(ui, "listening", GOOD);
        } else if view.live.demo_voice {
            chip(ui, "demo voice", WARN);
        } else {
            chip(ui, "off", ui.visuals().widgets.inactive.weak_bg_fill);
        }
    });
    mic_picker(ui, view, actions);
    mic_sensitivity(ui, view, actions);
    meter(ui, "level", view.live.level, 0.0, 1.0);
    ui.horizontal(|ui| {
        ui.add_sized([62.0, 14.0], egui::Label::new(egui::RichText::new("mouth").size(11.0).weak()));
        match view.live.viseme {
            Some(v) => chip(ui, &format!("{v:?}"), ui.visuals().selection.bg_fill),
            None => chip(ui, "quiet", ui.visuals().widgets.inactive.weak_bg_fill),
        };
    });
    ui.horizontal_wrapped(|ui| {
        if view.live.mic {
            if ui.button("Stop").clicked() {
                actions.push(Action::Mic(MicAction::Stop));
            }
        } else if ui.button("Start").clicked() {
            actions.push(Action::Mic(MicAction::Start));
        }
        let mut demo = view.live.demo_voice;
        if ui.checkbox(&mut demo, "Demo voice").on_hover_text("Plays a talking mouth when there is no microphone.").changed() {
            actions.push(Action::Mic(MicAction::DemoVoice(demo)));
        }
    });
}

/// The record button style: a red dot while recording.
pub fn record_button(ui: &mut Ui, recording: bool) -> egui::Response {
    let text = if recording { "Stop" } else { "Record" };
    let label = egui::RichText::new(format!("\u{25CF} {text}")).color(if recording { Color32::WHITE } else { REC }).strong();
    let button = egui::Button::new(label).fill(if recording { REC } else { ui.visuals().widgets.inactive.weak_bg_fill });
    ui.add(button)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_are_drawn_once_each() {
        let points = [[0.5, 0.5], [0.5, 0.5], [0.5001, 0.5001], [0.25, 0.75]];
        assert_eq!(distinct_points(&points).len(), 2);
        assert!(distinct_points(&[]).is_empty());
    }
}
