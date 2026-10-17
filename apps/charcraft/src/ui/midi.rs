//! MIDI in the window: the device picker and monitor, the Controls sliders with their controller learn buttons, and the
//! learn button of a trigger.

use anim_core::num::f64_to_f32;
use charcraft_midi::note_name;
use charcraft_puppet::MidiTrigger;
use egui::{Color32, RichText, Ui};
use serde_json::json;

use super::panels::chip;
use crate::midi::{CONTROLS, ControlDef, Learn};
use crate::view::{Action, View};

const GOOD: Color32 = Color32::from_rgb(0x2e, 0x9e, 0x5b);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xb0, 0x3c);
const LEARN: Color32 = Color32::from_rgb(0xc4, 0x6a, 0x3a);

/// The text of a trigger's note: `C4`, or `C4 ch 2` when it listens to one channel.
#[must_use]
pub fn trigger_text(trigger: &MidiTrigger) -> String {
    trigger.channel.map_or_else(|| note_name(trigger.note), |c| format!("{} ch {c}", note_name(trigger.note)))
}

/// The device picker with its refresh button, and what the open device last said.
pub fn device(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    let midi = &view.midi;
    ui.horizontal(|ui| {
        let current = midi.connected.clone().unwrap_or_else(|| if midi.ports.is_empty() { "No MIDI device".to_owned() } else { "Choose a device".to_owned() });
        egui::ComboBox::from_id_salt("midi_device")
            .width((ui.available_width() - 34.0).max(80.0))
            .selected_text(anim_ui::theme::visual(&current).into_owned())
            .show_ui(ui, |ui| {
                if ui.selectable_label(midi.connected.is_none(), "None").clicked() {
                    actions.push(Action::Run("midi.connect", json!({ "device": null })));
                }
                for port in &midi.ports {
                    if ui.selectable_label(midi.connected.as_ref() == Some(port), anim_ui::theme::visual(port).into_owned()).clicked() {
                        actions.push(Action::Run("midi.connect", json!({ "device": port })));
                    }
                }
            });
        if ui.small_button("\u{21BB}").on_hover_text("Look for MIDI devices again").clicked() {
            actions.push(Action::Run("midi.refresh", json!({})));
        }
    });
    if let Some(error) = &midi.error {
        ui.add(egui::Label::new(RichText::new(error).size(11.0).color(WARN)).wrap());
    }
    ui.horizontal_wrapped(|ui| {
        if midi.connected.is_some() {
            chip(ui, "connected", GOOD);
        }
        match midi.history.first() {
            Some(last) => chip(ui, last, ui.visuals().selection.bg_fill),
            None => chip(ui, "no messages yet", ui.visuals().widgets.inactive.weak_bg_fill),
        };
    });
}

/// A button that starts learn mode for `learn` (or ends it when it is already on).
fn learn_button(ui: &mut Ui, view: &View, learn: &Learn, idle_text: &str, actions: &mut Vec<Action>) {
    let on = view.midi.learn.as_ref() == Some(learn);
    let (text, hint) = match learn {
        Learn::Trigger { .. } => ("Play a note\u{2026}", "Press a key on the MIDI keyboard: the layer shows while that note is held. Click again to cancel."),
        Learn::Control { .. } => ("Move a knob\u{2026}", "Turn a knob or fader on the MIDI controller: it moves this slider. Click again to cancel."),
    };
    let label = if on { RichText::new(text).color(Color32::WHITE) } else { RichText::new(idle_text) };
    let button = egui::Button::new(label).small().fill(if on { LEARN } else { ui.visuals().widgets.inactive.weak_bg_fill });
    if ui.add(button).on_hover_text(hint).clicked() {
        let params = match (on, learn) {
            (true, _) => json!({}),
            (false, Learn::Trigger { puppet, layer }) => json!({ "puppet": puppet, "layer": layer }),
            (false, Learn::Control { control }) => json!({ "control": control }),
        };
        actions.push(Action::Run("midi.learn", params));
    }
}

/// The learn button of a layer's MIDI trigger, with the note it has.
pub fn trigger_learn(ui: &mut Ui, view: &View, puppet: &str, layer: &str, current: Option<&MidiTrigger>, actions: &mut Vec<Action>) {
    if let Some(trigger) = current {
        chip(ui, &format!("MIDI {}", trigger_text(trigger)), ui.visuals().selection.bg_fill);
    }
    let learn = Learn::Trigger { puppet: puppet.to_owned(), layer: layer.to_owned() };
    learn_button(ui, view, &learn, if current.is_some() { "Learn again" } else { "MIDI learn" }, actions);
    if current.is_some() && ui.small_button("\u{00D7}").on_hover_text("Remove the MIDI note").clicked() {
        actions.push(Action::Run("rig.edit", json!({ "puppet": puppet, "op": "set_midi_trigger", "layer": layer })));
    }
}

/// One Controls slider with the controller that drives it.
fn control_row(ui: &mut Ui, view: &View, def: &ControlDef, actions: &mut Vec<Action>) {
    let mut value = if def.id == "mic_gain" { f64::from(view.live.mic_gain) } else { view.midi.controls.get(def.id).copied().unwrap_or(def.default) };
    value = value.clamp(def.min, def.max);
    ui.horizontal(|ui| {
        ui.add_sized([86.0, 14.0], egui::Label::new(RichText::new(def.label).size(11.0).weak()));
        let mut v = value;
        let slider = egui::Slider::new(&mut v, def.min..=def.max).logarithmic(def.log).show_value(true).fixed_decimals(2);
        if ui.add(slider).on_hover_text(def.hint).changed() {
            actions.push(Action::Run("controls.set", json!({ "control": def.id, "value": f64_to_f32(v) })));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(90.0);
        if let Some(text) = view.midi.bindings.get(def.id) {
            chip(ui, text, ui.visuals().selection.bg_fill);
        }
        let learn = Learn::Control { control: def.id.to_owned() };
        learn_button(ui, view, &learn, if view.midi.bindings.contains_key(def.id) { "Learn again" } else { "MIDI learn" }, actions);
        if view.midi.bindings.contains_key(def.id) && ui.small_button("\u{00D7}").on_hover_text("Forget the controller").clicked() {
            actions.push(Action::Run("midi.bind_cc", json!({ "control": def.id })));
        }
    });
}

/// The Controls sliders.
pub fn controls(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    for def in &CONTROLS {
        control_row(ui, view, def, actions);
    }
}
