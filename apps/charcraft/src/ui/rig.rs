//! Rig mode: the puppet tree with tags, the artwork with its handles and mesh regions, and the inspector with the
//! selected layer's tags and handles and the behaviors with their parameters.

use anim_core::num::f64_to_f32;
use anim_ui::StageEvent;
use anim_ui::stage::{MarkerShape, Overlay, OverlayStyle, PointerPhase};
use anim_ui::theme::visual;
use charcraft_behaviors::api::StackOp;
use charcraft_puppet::{HandleKind, Layer, LayerId, Puppet, Viseme};
use egui::{Color32, Pos2, Rect, RichText, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::view::{Action, Ctx, View};

const ACCENT: Color32 = Color32::from_rgb(0x4f, 0x9d, 0xff);
const HANDLE: Color32 = Color32::from_rgb(0xff, 0x9f, 0x1c);
const ORIGIN: Color32 = Color32::from_rgb(0xe5, 0x39, 0x35);

/// The last segment of a tag id (`Adobe.LipSynch.Ah` is shown as `Ah`).
fn short(tag: &str) -> &str {
    tag.rsplit('.').next().unwrap_or(tag)
}

const fn point(p: anim_core::Point) -> Pos2 {
    pos2(f64_to_f32(p.x), f64_to_f32(p.y))
}

const fn rect_of(r: anim_core::Rect) -> Rect {
    Rect::from_min_max(pos2(f64_to_f32(r.x0), f64_to_f32(r.y0)), pos2(f64_to_f32(r.x1), f64_to_f32(r.y1)))
}

/// The layers in display order (top of the puppet first), skipping what a collapsed parent hides.
fn rows<'a>(puppet: &'a Puppet, collapsed: &std::collections::BTreeSet<String>) -> Vec<(&'a Layer, usize)> {
    let mut out = Vec::new();
    let Ok(root) = puppet.root() else { return out };
    let mut stack: Vec<(LayerId, usize)> = root.children.iter().rev().map(|&c| (c, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(layer) = puppet.layer(id) else { continue };
        out.push((layer, depth));
        if !collapsed.contains(&layer.path) {
            stack.extend(layer.children.iter().map(|&c| (c, depth.saturating_add(1))));
        }
    }
    out
}

/// The smallest layer whose bounds contain `at` (artwork pixels).
fn pick(puppet: &Puppet, at: Pos2) -> Option<String> {
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
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        for (layer, depth) in rows(puppet, &view.collapsed) {
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
                let mut text = RichText::new(visual(&layer.name).into_owned());
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
            });
        }
    });
}

/// Handles, origins, the selected layer's box and a grid over the regions that warp.
fn overlays(puppet: &Puppet, selected: Option<&str>) -> Vec<Overlay> {
    let mut out = Vec::new();
    if let Some(layer) = selected.and_then(|p| puppet.find_path(p)).and_then(|id| puppet.layer(id))
        && let Some(bounds) = layer.bounds
    {
        out.push(Overlay::Rect { rect: rect_of(bounds), style: OverlayStyle::dashed(ACCENT) });
    }
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
    for layer in puppet.independent_groups().filter(|l| !l.path.is_empty()) {
        out.push(Overlay::Marker { pos: point(layer.origin), shape: MarkerShape::Diamond, color: ORIGIN, filled: true });
        for handle in &layer.handles {
            out.push(Overlay::Marker { pos: point(handle.position), shape: MarkerShape::Circle, color: HANDLE, filled: handle.kind.pins_mesh() });
            out.push(Overlay::Text { pos: point(handle.position) + vec2(8.0, -8.0), text: handle.name.clone(), color: HANDLE });
        }
    }
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

fn stage(ui: &mut Ui, puppet: &Puppet, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    view.rig_stage.overlays = overlays(puppet, view.layer.as_deref());
    view.rig_stage.doc_size = vec2(f64_to_f32(puppet.size.width), f64_to_f32(puppet.size.height));
    view.rig_stage.stage_color = if view.dark { Color32::from_rgb(0xec, 0xee, 0xf4) } else { Color32::WHITE };
    let texture = ctx.texture.map(|(t, _)| t);
    for event in anim_ui::stage_view(ui, &view.rig_stage, texture) {
        match event {
            StageEvent::ViewChanged(v) => {
                view.rig_stage.view = v;
                view.rig_stage.fit_request = false;
            }
            StageEvent::Pointer(p) => match p.phase {
                PointerPhase::Click => actions.push(Action::SelectLayer(pick(puppet, p.pos))),
                PointerPhase::Down | PointerPhase::Drag => view.pointer = Some((f64::from(p.pos.x), f64::from(p.pos.y), true)),
                PointerPhase::Move => view.pointer = Some((f64::from(p.pos.x), f64::from(p.pos.y), false)),
                PointerPhase::Up | PointerPhase::DoubleClick => view.pointer = view.pointer.map(|(x, y, _)| (x, y, false)),
            },
            _ => {}
        }
    }
}

fn rig_edit(key: &str, op: Value) -> Action {
    let mut params = op;
    if let Some(map) = params.as_object_mut() {
        map.insert("puppet".to_owned(), json!(key));
    }
    Action::Run("rig.edit", params)
}

fn inspector_layer(ui: &mut Ui, puppet: &Puppet, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    let Some(layer) = view.layer.as_deref().and_then(|p| puppet.find_path(p)).and_then(|id| puppet.layer(id)) else {
        ui.weak("Select a layer in the tree or click the artwork.");
        return;
    };
    ui.label(RichText::new(visual(&layer.path).into_owned()).strong());
    ui.weak(format!("{:?}{}", layer.kind, if layer.independent { ", independent group" } else { "" }));
    let mut visible = layer.visible;
    if ui.checkbox(&mut visible, "Visible by default").changed() {
        actions.push(rig_edit(key, json!({ "op": "set_visible", "layer": layer.path, "visible": visible })));
    }
    ui.separator();
    ui.label(RichText::new("Tags").strong());
    ui.horizontal_wrapped(|ui| {
        for tag in &layer.tags {
            ui.label(RichText::new(short(tag)).color(ACCENT)).on_hover_text(tag);
            if ui.small_button("\u{00D7}").on_hover_text("Remove tag").clicked() {
                actions.push(rig_edit(key, json!({ "op": "remove_tag", "layer": layer.path, "tag": tag })));
            }
        }
    });
    ui.horizontal(|ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut view.new_tag).hint_text("Adobe.Draggable").desired_width(150.0));
        let submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add tag").clicked() || submit) && !view.new_tag.trim().is_empty() {
            actions.push(rig_edit(key, json!({ "op": "add_tag", "layer": layer.path, "tag": view.new_tag.trim() })));
            view.new_tag.clear();
        }
    });
    if layer.independent {
        ui.separator();
        ui.label(RichText::new("Handles").strong());
        ui.weak(format!("origin {:.0}, {:.0}", layer.origin.x, layer.origin.y));
        for handle in &layer.handles {
            ui.horizontal(|ui| {
                ui.label(format!("{} ({:?}) {:.0}, {:.0}", visual(&handle.name), handle.kind, handle.position.x, handle.position.y));
                if ui.small_button("\u{00D7}").on_hover_text("Remove handle").clicked() {
                    actions.push(rig_edit(key, json!({ "op": "remove_handle", "group": layer.path, "key": handle.key })));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut view.new_handle).desired_width(90.0));
            egui::ComboBox::from_id_salt("handle_kind").selected_text(view.handle_kind).width(90.0).show_ui(ui, |ui| {
                for kind in ["fixed", "draggable", "dangle", "attach", "stick", "walk"] {
                    ui.selectable_value(&mut view.handle_kind, kind, kind);
                }
            });
        });
        if ui.button("Add handle at the origin").clicked() && !view.new_handle.trim().is_empty() {
            let kind = match view.handle_kind {
                "fixed" => HandleKind::Fixed,
                "dangle" => HandleKind::Dangle,
                "attach" => HandleKind::Attach,
                "stick" => HandleKind::Stick,
                "walk" => HandleKind::Walk,
                _ => HandleKind::Draggable,
            };
            actions.push(rig_edit(
                key,
                json!({ "op": "add_handle", "group": layer.path, "name": view.new_handle.trim(), "kind": serde_json::to_value(kind).unwrap_or(Value::Null), "position": [layer.origin.x, layer.origin.y] }),
            ));
        }
    }
}

fn behaviors(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some(stack) = ctx.stack else {
        ui.weak("No track selected.");
        return;
    };
    for info in &stack.behaviors {
        let id = ui.make_persistent_id(("behavior", info.index));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                let mut enabled = info.enabled;
                if ui.checkbox(&mut enabled, "").changed() {
                    actions.push(Action::Run("track.behavior", json!({ "track": view.track, "op": StackOp::SetEnabled { index: info.index, enabled } })));
                }
                ui.label(RichText::new(&info.name).strong());
            })
            .body(|ui| {
                let Some(map) = info.params.as_object() else { return };
                for (name, value) in map {
                    ui.horizontal(|ui| {
                        ui.add_sized([130.0, 16.0], egui::Label::new(RichText::new(name).size(11.0)).truncate());
                        param_editor(ui, view.track, info.index, name, value, actions);
                    });
                }
            });
    }
}

fn param_editor(ui: &mut Ui, track: usize, index: usize, name: &str, value: &Value, actions: &mut Vec<Action>) {
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
        Value::String(s) => {
            ui.label(RichText::new(visual(s).into_owned()).weak());
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
    egui::Panel::left("rig_tree").default_size(290.0).min_size(200.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong(visual(key).into_owned());
            ui.weak(format!("{} layers", puppet.layers().len().saturating_sub(1)));
        });
        ui.separator();
        tree(ui, puppet, view, actions);
    });
    egui::Panel::right("rig_inspector").default_size(330.0).min_size(240.0).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            inspector_layer(ui, puppet, key, view, actions);
            ui.separator();
            ui.label(RichText::new("Behaviors").strong());
            ui.weak("Parameters of the selected track, as the scene plays them.");
            behaviors(ui, view, ctx, actions);
            if !ctx.world.warnings.is_empty() {
                ui.separator();
                ui.label(RichText::new("Warnings").strong());
                for w in ctx.world.warnings.iter().take(8) {
                    ui.label(RichText::new(visual(w).into_owned()).weak().size(11.0));
                }
            }
        });
    });
    egui::Panel::bottom("rig_pose").show(ui, |ui| pose_bar(ui, view));
    egui::CentralPanel::default().show(ui, |ui| stage(ui, puppet, view, ctx, actions));
}
