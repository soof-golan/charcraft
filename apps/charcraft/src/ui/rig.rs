//! Rig mode.
//!
//! The puppet tree with tags, the rig toolbar, the artwork with its handles and mesh, and the right panel with the
//! Properties, Triggers and Behaviors tabs (the tools themselves live in [`super::rigtools`]).

use anim_core::num::f64_to_f32;
use anim_ui::StageEvent;
use anim_ui::stage::{Overlay, OverlayStyle};
use anim_ui::theme::visual;
use charcraft_behaviors::api::StackOp;
use charcraft_puppet::{Layer, LayerId, Puppet, Viseme};
use egui::{Color32, Pos2, Rect, RichText, Ui, pos2, vec2};
use serde_json::{Value, json};

use super::rigtools::{self, Tab};
use crate::view::{Action, Ctx, View};

pub(super) const ACCENT: Color32 = Color32::from_rgb(0x4f, 0x9d, 0xff);
pub(super) const HANDLE: Color32 = Color32::from_rgb(0xff, 0x9f, 0x1c);
pub(super) const ORIGIN: Color32 = Color32::from_rgb(0xe5, 0x39, 0x35);

/// The last segment of a tag id (`Adobe.LipSynch.Ah` is shown as `Ah`).
pub(super) fn short(tag: &str) -> &str {
    tag.rsplit('.').next().unwrap_or(tag)
}

pub(super) const fn point(p: anim_core::Point) -> Pos2 {
    pos2(f64_to_f32(p.x), f64_to_f32(p.y))
}

pub(super) const fn rect_of(r: anim_core::Rect) -> Rect {
    Rect::from_min_max(pos2(f64_to_f32(r.x0), f64_to_f32(r.y0)), pos2(f64_to_f32(r.x1), f64_to_f32(r.y1)))
}

/// An unnamed layer with no children and no tags: a bare artwork path. Listing it would draw an empty row, which
/// shows up as a blank gap in the tree, and it can neither be told apart nor targeted by a behavior.
fn is_anonymous_leaf(layer: &Layer) -> bool {
    layer.name.trim().is_empty() && layer.children.is_empty() && layer.tags.is_empty()
}

/// The layers in display order (top of the puppet first), skipping what a collapsed parent hides and unnamed leaves.
fn rows<'a>(puppet: &'a Puppet, collapsed: &std::collections::BTreeSet<String>) -> Vec<(&'a Layer, usize)> {
    let mut out = Vec::new();
    let Ok(root) = puppet.root() else { return out };
    let mut stack: Vec<(LayerId, usize)> = root.children.iter().rev().map(|&c| (c, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(layer) = puppet.layer(id) else { continue };
        if is_anonymous_leaf(layer) {
            continue;
        }
        out.push((layer, depth));
        if !collapsed.contains(&layer.path) {
            stack.extend(layer.children.iter().map(|&c| (c, depth.saturating_add(1))));
        }
    }
    out
}

/// The smallest layer whose bounds contain `at` (artwork pixels).
pub(super) fn pick(puppet: &Puppet, at: Pos2) -> Option<String> {
    puppet
        .layers()
        .iter()
        .skip(1)
        .filter_map(|l| l.bounds.map(|b| (l, rect_of(b))))
        .filter(|(_, r)| r.contains(at))
        .min_by(|a, b| (a.1.width() * a.1.height()).total_cmp(&(b.1.width() * b.1.height())))
        .map(|(l, _)| l.path.clone())
}

fn tree(ui: &mut Ui, puppet: &Puppet, view: &mut View, actions: &mut Vec<Action>) {
    ui.add(egui::TextEdit::singleline(&mut view.rig.layer_filter).hint_text("Filter by name or tag").desired_width(f32::INFINITY));
    let keep = rigtools::visible_paths(puppet, &view.rig.layer_filter);
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        for (layer, depth) in rows(puppet, &view.collapsed) {
            if keep.as_ref().is_some_and(|k| !k.contains(&layer.path)) {
                continue;
            }
            ui.horizontal(|ui| {
                ui.add_space(f32::from(u16::try_from(depth.saturating_mul(14)).unwrap_or(0)));
                if layer.children.is_empty() {
                    ui.add_space(16.0);
                } else {
                    let open = !view.collapsed.contains(&layer.path);
                    if ui.small_button(if open { "\u{25BE}" } else { "\u{25B8}" }).clicked() {
                        if open {
                            view.collapsed.insert(layer.path.clone());
                        } else {
                            view.collapsed.remove(&layer.path);
                        }
                    }
                }
                let selected = view.layer.as_deref() == Some(layer.path.as_str());
                let mut text =
                    if layer.name.trim().is_empty() { RichText::new("(unnamed)").weak().italics() } else { RichText::new(visual(&layer.name).into_owned()) };
                if layer.independent {
                    text = text.strong();
                }
                if !layer.visible {
                    text = text.weak();
                }
                if ui.selectable_label(selected, text).clicked() {
                    actions.push(Action::SelectLayer(Some(layer.path.clone())));
                }
                for tag in layer.tags.iter().take(3) {
                    ui.label(RichText::new(short(tag)).size(10.0).color(ACCENT));
                }
                if layer.tags.len() > 3 {
                    ui.label(RichText::new(format!("+{}", layer.tags.len().saturating_sub(3))).size(10.0).weak());
                }
                if let Some(key) = &layer.key_trigger {
                    ui.label(RichText::new(format!("[{key}]")).size(10.0).color(HANDLE));
                }
            });
        }
    });
}

/// The selected layer's box and a grid over the regions that warp, then what the rig tools draw.
fn overlays(puppet: &Puppet, key: &str, view: &mut View, edits: usize) -> Vec<Overlay> {
    let mut out = Vec::new();
    let selected = view.layer.as_deref();
    if let Some(layer) = selected.and_then(|p| puppet.find_path(p)).and_then(|id| puppet.layer(id))
        && let Some(bounds) = layer.bounds
    {
        out.push(Overlay::Rect { rect: rect_of(bounds), style: OverlayStyle::dashed(ACCENT) });
    }
    let wire = if view.rig.show_mesh || view.rig.tool == rigtools::Tool::Mesh { rigtools::mesh_wire(puppet, key, edits, view) } else { Vec::new() };
    if wire.is_empty() {
        for id in puppet.warped_groups() {
            let Some(bounds) = puppet.layer(id).and_then(|l| l.bounds).map(rect_of) else { continue };
            let line = OverlayStyle::line(ACCENT.gamma_multiply(0.35));
            for i in 0..=8_u8 {
                let t = f32::from(i) / 8.0;
                let x = bounds.left() + bounds.width() * t;
                let y = bounds.top() + bounds.height() * t;
                out.push(Overlay::Line { a: pos2(x, bounds.top()), b: pos2(x, bounds.bottom()), style: line });
                out.push(Overlay::Line { a: pos2(bounds.left(), y), b: pos2(bounds.right(), y), style: line });
            }
        }
    }
    out.extend(rigtools::overlays(puppet, view, &wire));
    out
}

fn pose_bar(ui: &mut Ui, view: &mut View) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Preview").weak());
        if ui.selectable_label(view.preview_viseme.is_none(), "Live").clicked() {
            view.preview_viseme = None;
        }
        for v in Viseme::ALL {
            if ui.selectable_label(view.preview_viseme == Some(v), v.label()).clicked() {
                view.preview_viseme = Some(v);
            }
        }
        ui.separator();
        ui.toggle_value(&mut view.preview_blink, "Blink");
    });
}

fn stage(ui: &mut Ui, puppet: &Puppet, key: &str, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    view.rig_stage.overlays = overlays(puppet, key, view, ctx.doc.rig.edits.len());
    view.rig_stage.doc_size = vec2(f64_to_f32(puppet.size.width), f64_to_f32(puppet.size.height));
    view.rig_stage.stage_color = if view.dark { Color32::from_rgb(0xec, 0xee, 0xf4) } else { Color32::WHITE };
    let texture = ctx.texture.map(|(t, _)| t);
    for event in anim_ui::stage_view(ui, &view.rig_stage, texture) {
        match event {
            StageEvent::ViewChanged(v) => {
                view.rig_stage.view = v;
                view.rig_stage.fit_request = false;
            }
            StageEvent::Pointer(p) => rigtools::pointer(puppet, key, view, &p, actions),
            _ => {}
        }
    }
}

pub(super) fn rig_edit(key: &str, op: Value) -> Action {
    let mut params = op;
    if let Some(map) = params.as_object_mut() {
        map.insert("puppet".to_owned(), json!(key));
    }
    Action::Run("rig.edit", params)
}

pub(super) fn param_editor(ui: &mut Ui, track: usize, index: usize, name: &str, value: &Value, choices: &[&str], actions: &mut Vec<Action>) {
    let set = |value: Value| Action::Run("track.behavior", json!({ "track": track, "op": StackOp::SetParam { index, name: name.to_owned(), value } }));
    match value {
        Value::Bool(b) => {
            let mut on = *b;
            if ui.checkbox(&mut on, "").changed() {
                actions.push(set(json!(on)));
            }
        }
        Value::Number(n) => {
            let current = n.as_f64().unwrap_or(0.0);
            let id = ui.id().with(("param", track, index, name));
            let mut edited: f64 = ui.data(|d| d.get_temp(id)).unwrap_or(current);
            let speed = (current.abs() * 0.01).max(0.01);
            let response = ui.add(egui::DragValue::new(&mut edited).speed(speed));
            if response.changed() {
                ui.data_mut(|d| d.insert_temp(id, edited));
            }
            let done = response.drag_stopped() || response.lost_focus();
            if done && ui.data(|d| d.get_temp::<f64>(id)).is_some() {
                ui.data_mut(|d| d.remove_temp::<f64>(id));
                let as_int = n.is_i64() || n.is_u64();
                actions.push(set(if as_int { json!(anim_core::num::round_f64_to_i64(edited)) } else { json!(edited) }));
            }
        }
        Value::String(s) if !choices.is_empty() => {
            egui::ComboBox::from_id_salt(("param_choice", track, index, name)).selected_text(s.as_str()).width(110.0).show_ui(ui, |ui| {
                for &choice in choices {
                    if ui.selectable_label(s == choice, choice).clicked() && s != choice {
                        actions.push(set(json!(choice)));
                    }
                }
            });
        }
        Value::String(s) => {
            let id = ui.id().with(("param", track, index, name));
            let mut edited: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| s.clone());
            let response = ui.add(egui::TextEdit::singleline(&mut edited).desired_width(110.0));
            if response.changed() {
                ui.data_mut(|d| d.insert_temp(id, edited.clone()));
            }
            if response.lost_focus() && ui.data(|d| d.get_temp::<String>(id)).is_some() {
                ui.data_mut(|d| d.remove_temp::<String>(id));
                if edited != *s {
                    actions.push(set(json!(edited)));
                }
            }
        }
        other => {
            ui.label(RichText::new(other.to_string()).weak().size(10.0));
        }
    }
}

/// Rig mode.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some((key, member)) = ctx.puppet.and_then(|k| ctx.world.cast.get(k).map(|m| (k, m))) else {
        ui.centered_and_justified(|ui| ui.label("No puppet loaded. Use File > Open."));
        return;
    };
    let puppet = &member.puppet;
    rigtools::keys(ui, puppet, key, view, actions);
    egui::Panel::left("rig_tree").default_size(290.0).min_size(200.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong(visual(key).into_owned());
            ui.weak(format!("{} layers", puppet.layers().len().saturating_sub(1)));
        });
        ui.separator();
        tree(ui, puppet, view, actions);
    });
    egui::Panel::right("rig_inspector").default_size(330.0).min_size(240.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (tab, label) in [(Tab::Properties, "Properties"), (Tab::Triggers, "Triggers"), (Tab::Behaviors, "Behaviors")] {
                if ui.selectable_label(view.rig.tab == tab, label).clicked() {
                    view.rig.tab = tab;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            match view.rig.tab {
                Tab::Properties => rigtools::properties(ui, puppet, key, view, actions),
                Tab::Triggers => rigtools::triggers(ui, puppet, key, view, actions),
                Tab::Behaviors => {
                    ui.weak("Parameters of the selected track, as the scene plays them.");
                    rigtools::behaviors(ui, puppet, view, ctx, actions);
                }
            }
            if !ctx.world.warnings.is_empty() {
                ui.separator();
                ui.label(RichText::new("Warnings").strong());
                for w in ctx.world.warnings.iter().take(8) {
                    ui.label(RichText::new(visual(w).into_owned()).weak().size(11.0));
                }
            }
        });
    });
    rigtools::toolbar(ui, puppet, key, view, actions);
    egui::Panel::bottom("rig_pose").show(ui, |ui| pose_bar(ui, view));
    egui::CentralPanel::default().show(ui, |ui| stage(ui, puppet, key, view, ctx, actions));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::world::World;

    /// No row of the tree may be a blank line: it would show as a gap between its neighbours.
    fn assert_no_blank_rows(puppet: &Puppet) {
        let rows = rows(puppet, &BTreeSet::new());
        assert!(!rows.is_empty());
        for (layer, _) in rows {
            assert!(!is_anonymous_leaf(layer), "blank row for {:?}", layer.path);
        }
    }

    #[test]
    fn the_demo_tree_has_no_blank_rows() {
        let opened = World::demo().unwrap();
        let member = opened.world.cast.keys().next().and_then(|k| opened.world.cast.get(k)).unwrap();
        assert_no_blank_rows(&member.puppet);
    }

    #[test]
    fn the_boy_tree_has_no_blank_rows() {
        let Ok(samples) = std::env::var("ANICRAFT_SAMPLES") else { return };
        let path = std::path::Path::new(&samples).join("Boy.puppet");
        if !path.exists() {
            return;
        }
        let opened = World::open(&path).unwrap();
        let anonymous = |p: &Puppet| p.layers().iter().filter(|l| is_anonymous_leaf(l)).count();
        let mut seen_blank = false;
        for key in opened.world.cast.keys() {
            let Some(member) = opened.world.cast.get(key) else { continue };
            seen_blank |= anonymous(&member.puppet) > 0;
            assert_no_blank_rows(&member.puppet);
        }
        assert!(seen_blank, "the Boy artwork is expected to hold unnamed leaves");
    }
}
