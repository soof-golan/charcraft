//! Small shared widgets and the camera and microphone panels.

use anim_ui::theme::visual;
use charcraft_tracking::input::FaceInput as TrackedFace;
use egui::epaint::EllipseShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;

use crate::live::MicAction;
use crate::view::{Action, Ctx, View};

const GOOD: Color32 = Color32::from_rgb(0x4c, 0xc2, 0x7a);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x3c);
const REC: Color32 = Color32::from_rgb(0xe5, 0x39, 0x35);
const SKIN: Color32 = Color32::from_rgb(0xf0, 0xc4, 0xa0);
const INK: Color32 = Color32::from_rgb(0x2a, 0x22, 0x2c);

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

/// The camera panel: preview, start and stop, calibration, mirror and the tracked values.
pub fn camera(ui: &mut Ui, view: &mut View, _ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.strong("Camera");
        if view.live.tracking {
            chip(ui, "tracking", GOOD);
        } else {
            chip(ui, "off", ui.visuals().widgets.inactive.weak_bg_fill);
        }
        if cfg!(not(feature = "live")) {
            ui.weak("demo face");
        }
    });
    face_preview(ui, view.live.face.as_ref(), vec2(ui.available_width().min(260.0), 130.0));
    ui.horizontal_wrapped(|ui| {
        if view.live.tracking {
            if ui.button("Stop").clicked() {
                actions.push(Action::Tracking("tracking_stop", json!({})));
            }
        } else if ui.button("Start").clicked() {
            actions.push(Action::Tracking("tracking_start", json!({})));
        }
        let calibrating = view.live.calibrating.is_some();
        if ui
            .add_enabled(view.live.tracking && !calibrating, egui::Button::new(if calibrating { "Calibrating..." } else { "Calibrate" }))
            .on_hover_text("Relax your face, look at the camera, then press.")
            .clicked()
        {
            actions.push(Action::Tracking("tracking_calibrate", json!({ "frames": 30 })));
        }
        let mut mirror = view.mirror;
        if ui.checkbox(&mut mirror, "Mirror").changed() {
            actions.push(Action::Mirror(mirror));
        }
    });
    if let Some(face) = view.live.face.as_ref().filter(|f| f.tracked) {
        meter(ui, "yaw", face.head.yaw, -0.8, 0.8);
        meter(ui, "pitch", face.head.pitch, -0.8, 0.8);
        meter(ui, "roll", face.head.roll, -0.8, 0.8);
        meter(ui, "brows", f32::midpoint(face.brow_l, face.brow_r), -1.0, 1.0);
        meter(ui, "eyes", f32::midpoint(face.eye_open_l, face.eye_open_r), 0.0, 1.0);
        meter(ui, "mouth", face.mouth_open, 0.0, 1.0);
        meter(ui, "smile", face.smile, -1.0, 1.0);
    }
    if let Some((done, wanted)) = view.live.calibrating {
        ui.add(egui::ProgressBar::new(f32::from(u16::try_from(done).unwrap_or(0)) / f32::from(u16::try_from(wanted.max(1)).unwrap_or(1))).desired_height(6.0));
    }
    if let Some(problem) = &view.live.problem {
        ui.colored_label(WARN, visual(problem).into_owned());
    }
}

/// The microphone panel: level, the mouth shape the voice makes, start and stop.
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
