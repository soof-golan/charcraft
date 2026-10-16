//! Small widgets in Character Animator's manner: the red arm dot, number scrubbies, panel section headers.

use egui::{Color32, Response, Sense, Stroke, Ui, vec2};

use super::theme::{ACCENT, RED, TEXT_WEAK, WELL};

/// The light blue of a number you can drag.
pub const SCRUB: Color32 = Color32::from_rgb(0x6f, 0xb1, 0xf5);

/// The record-enable dot: filled red when on, a ring when off.
pub fn arm_dot(ui: &mut Ui, on: bool, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    let centre = rect.center();
    let hot = response.hovered();
    if on {
        ui.painter().circle_filled(centre, 5.0, RED);
        if hot {
            ui.painter().circle_stroke(centre, 6.5, Stroke::new(1.0, RED.gamma_multiply(0.6)));
        }
    } else {
        ui.painter().circle_stroke(centre, 4.5, Stroke::new(1.3, if hot { RED } else { RED.gamma_multiply(0.55) }));
    }
    response.on_hover_text(tooltip)
}

/// Runs `add` with number fields styled as scrubbies: a dark well, light blue digits, a horizontal drag cursor.
pub fn scrub_style<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
            state.fg_stroke.color = SCRUB;
        }
        widgets.inactive.weak_bg_fill = WELL;
        widgets.inactive.bg_fill = WELL;
        widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2a, 0x2a, 0x2a);
        widgets.active.weak_bg_fill = Color32::from_rgb(0x1e, 0x3a, 0x5c);
        widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
        add(ui)
    })
    .inner
}

/// A labelled number scrubby: drag the number, or click it to type. Returns the new value when it changed.
pub fn scrubby(ui: &mut Ui, label: &str, value: f64, speed: f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> Option<f64> {
    let mut edited = value;
    let changed = ui
        .horizontal(|ui| {
            ui.add_sized([96.0, 16.0], egui::Label::new(egui::RichText::new(label).color(TEXT_WEAK)).truncate());
            scrub_style(ui, |ui| ui.add(egui::DragValue::new(&mut edited).speed(speed).range(range).suffix(suffix)).changed())
        })
        .inner;
    changed.then_some(edited)
}

/// A flat section header strip as in Properties panels: a title with a thin line under it.
pub fn section_title(ui: &mut Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(title).family(super::theme::semibold()).color(TEXT_WEAK));
    ui.separator();
}
