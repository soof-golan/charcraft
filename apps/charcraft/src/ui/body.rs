//! The body panel: a skeleton preview of the tracked body against the rest pose, start and stop, and Set Rest Pose.

use anim_ui::theme::visual;
use charcraft_tracking::body::{BodyInput, Joint};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;

use super::panels::{chip, meter};
use crate::view::{Action, View};

const GOOD: Color32 = Color32::from_rgb(0x4c, 0xc2, 0x7a);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x3c);
const LIMB: Color32 = Color32::from_rgb(0x3c, 0x9a, 0xe8);
const TORSO: Color32 = Color32::from_rgb(0x9a, 0x6c, 0xe0);
const JOINT: Color32 = Color32::from_rgb(0xe8, 0x5a, 0x4c);

/// The bones of the skeleton, in the order they are drawn.
const BONES: [(Joint, Joint, bool); 14] = [
    (Joint::LeftShoulder, Joint::RightShoulder, true),
    (Joint::LeftHip, Joint::RightHip, true),
    (Joint::LeftShoulder, Joint::LeftHip, true),
    (Joint::RightShoulder, Joint::RightHip, true),
    (Joint::LeftShoulder, Joint::LeftElbow, false),
    (Joint::LeftElbow, Joint::LeftWrist, false),
    (Joint::RightShoulder, Joint::RightElbow, false),
    (Joint::RightElbow, Joint::RightWrist, false),
    (Joint::LeftHip, Joint::LeftKnee, false),
    (Joint::LeftKnee, Joint::LeftAnkle, false),
    (Joint::RightHip, Joint::RightKnee, false),
    (Joint::RightKnee, Joint::RightAnkle, false),
    (Joint::Nose, Joint::LeftEye, true),
    (Joint::Nose, Joint::RightEye, true),
];

/// Where body units land in a rectangle: x from -1.9 to 1.9 and y from -1.8 to 2.0 fit.
fn mapper(rect: Rect) -> impl Fn([f32; 2]) -> Pos2 {
    let scale = (rect.width() / 3.8).min(rect.height() / 3.8);
    let centre = rect.center();
    move |p: [f32; 2]| Pos2::new(centre.x + p[0] * scale, centre.y + (p[1] - 0.1) * scale)
}

/// Draws the skeleton of `body` over a ghost of its rest pose.
pub fn skeleton_preview(ui: &mut Ui, body: Option<&BodyInput>, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), StrokeKind::Inside);
    let Some(body) = body.filter(|b| b.tracked) else {
        painter.text(rect.center(), Align2::CENTER_CENTER, "no body", FontId::proportional(13.0), ui.visuals().weak_text_color());
        return;
    };
    let at = mapper(rect);
    let ghost = ui.visuals().weak_text_color().gamma_multiply(0.35);
    for (a, b, _) in BONES {
        if let (Some(p), Some(q)) = (body.rest.get(a.index()), body.rest.get(b.index())) {
            painter.line_segment([at(*p), at(*q)], Stroke::new(1.5, ghost));
        }
    }
    for (a, b, torso) in BONES {
        if let (Some(p), Some(q)) = (body.joint(a), body.joint(b)) {
            painter.line_segment([at(p), at(q)], Stroke::new(3.0, if torso { TORSO } else { LIMB }));
        }
    }
    for joint in Joint::ALL {
        if let Some(p) = body.joint(joint) {
            painter.circle_filled(at(p), 3.5, JOINT);
        }
    }
}

/// The body panel.
pub fn body(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.strong("Body");
        if view.live.body_tracking {
            chip(ui, "tracking", GOOD);
        } else {
            chip(ui, "off", ui.visuals().widgets.inactive.weak_bg_fill);
        }
        if view.live.rest_pose_set {
            chip(ui, "rest pose set", ui.visuals().selection.bg_fill);
        }
        if cfg!(not(feature = "live")) {
            ui.weak("demo body");
        }
    });
    skeleton_preview(ui, view.live.body.as_ref(), vec2(ui.available_width().min(260.0), 190.0));
    ui.horizontal_wrapped(|ui| {
        if view.live.body_tracking {
            if ui.button("Stop").clicked() {
                actions.push(Action::Tracking("tracking_body_stop", json!({})));
            }
        } else if ui.button("Start").clicked() {
            actions.push(Action::Tracking("tracking_body_start", json!({})));
        }
        let setting = view.live.setting_rest_pose.is_some();
        if ui
            .add_enabled(view.live.body_tracking && !setting, egui::Button::new(if setting { "Setting..." } else { "Set Rest Pose" }))
            .on_hover_text("Sit or stand the way your puppet does, arms hanging, shoulders in view, then press.")
            .clicked()
        {
            actions.push(Action::Tracking("tracking_body_set_rest_pose", json!({ "frames": 30 })));
        }
        if ui.add_enabled(view.live.rest_pose_set, egui::Button::new("Reset")).on_hover_text("Go back to the typical body.").clicked() {
            actions.push(Action::Tracking("tracking_reset_rest_pose", json!({})));
        }
    });
    if let Some(body) = view.live.body.as_ref().filter(|b| b.tracked && view.live.body_tracking) {
        meter(ui, "seen", body.confidence, 0.0, 1.0);
        let reach = |j: Joint| body.joint(j).zip(body.rest.get(j.index())).map_or(0.0, |(p, r)| (p[1] - r[1]).abs().min(2.0) / 2.0);
        meter(ui, "left arm", reach(Joint::LeftWrist), 0.0, 1.0);
        meter(ui, "right arm", reach(Joint::RightWrist), 0.0, 1.0);
    }
    if let Some((done, wanted)) = view.live.setting_rest_pose {
        ui.add(egui::ProgressBar::new(f32::from(u16::try_from(done).unwrap_or(0)) / f32::from(u16::try_from(wanted.max(1)).unwrap_or(1))).desired_height(6.0));
    }
    if let Some(problem) = &view.live.body_problem {
        ui.colored_label(WARN, visual(problem).into_owned());
    }
}
