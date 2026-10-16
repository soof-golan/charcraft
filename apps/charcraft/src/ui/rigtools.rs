//! The rig tools of Rig mode.
//!
//! The toolbar (Select, Handle, Stick, Pin, Origin, Mesh), the pointer work on the artwork (add, drag and delete
//! handles, drag the origin), the tag picker, the group, trigger and cycle sections of the Properties tab, the
//! Triggers tab (key triggers and swap sets) and the per-layer Behaviors tab.
//!
//! Everything here reads the state and pushes [`Action`]s: each one is a `rig.edit` or `track.behavior` command, so
//! undo and the MCP interface come with it.

use std::collections::BTreeSet;

use anim_core::Point;
use anim_ui::stage::{MarkerShape, Overlay, OverlayStyle, PointerPhase, StagePointer};
use anim_ui::theme::visual;
use charcraft_behaviors::api::{StackOp, catalog};
use charcraft_puppet::tags::{AUTO_TAGS, TagKind};
use charcraft_puppet::{AttachStyle, CycleDef, CycleMode, HandleDef, HandleKind, Layer, Puppet};
use egui::{Color32, Pos2, RichText, Ui};
use serde_json::{Value, json};

use super::rig::{ACCENT, HANDLE, ORIGIN, point, rect_of, rig_edit, short};
use crate::view::{Action, Ctx, View};

/// One of the tools of the rig toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Picks layers; drags handles and origins that are already there.
    #[default]
    Select,
    /// Clicks add a handle of the chosen kind.
    Handle,
    /// Drags add a stick: a rigid link between two points.
    Stick,
    /// Clicks add a pin: a fixed handle that holds the mesh in place.
    Pin,
    /// Drags move the origin of an independent group.
    Origin,
    /// Shows the mesh of the selected group and its density.
    Mesh,
}

impl Tool {
    /// Every tool, in toolbar order.
    pub const ALL: [Self; 6] = [Self::Select, Self::Handle, Self::Stick, Self::Pin, Self::Origin, Self::Mesh];

    /// The name on the button.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Handle => "Handle",
            Self::Stick => "Stick",
            Self::Pin => "Pin",
            Self::Origin => "Origin",
            Self::Mesh => "Mesh",
        }
    }

    /// The key that picks it.
    #[must_use]
    pub const fn key(self) -> egui::Key {
        match self {
            Self::Select => egui::Key::V,
            Self::Handle => egui::Key::H,
            Self::Stick => egui::Key::S,
            Self::Pin => egui::Key::P,
            Self::Origin => egui::Key::O,
            Self::Mesh => egui::Key::M,
        }
    }

    /// What the tool does, for the tooltip.
    #[must_use]
    pub const fn help(self) -> &'static str {
        match self {
            Self::Select => "Pick a layer; drag a handle or an origin to move it",
            Self::Handle => "Click an independent group to add a handle there; Alt+click a handle deletes it",
            Self::Stick => "Drag across a group to add a stick between two points",
            Self::Pin => "Click to pin the mesh in place with a fixed handle; Alt+click a pin deletes it",
            Self::Origin => "Drag to move the origin of the group",
            Self::Mesh => "Shows the mesh of the selected group; set how finely it is cut",
        }
    }
}

/// The tabs of the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// Tags, group settings and handles of the selected layer.
    #[default]
    Properties,
    /// Key triggers and swap sets.
    Triggers,
    /// The behaviors of the selected track.
    Behaviors,
}

/// What a drag on the artwork is moving.
#[derive(Debug, Clone, PartialEq)]
#[expect(clippy::derive_partial_eq_without_eq, reason = "holds floating point positions")]
pub enum Drag {
    /// A handle of a group (`end`: the far end of a stick).
    Handle {
        /// The group's path.
        group: String,
        /// The handle's key.
        key: String,
        /// The far end of a stick.
        end: bool,
    },
    /// The origin of a group.
    Origin {
        /// The group's path.
        group: String,
    },
    /// A stick being drawn.
    Stick {
        /// The group's path.
        group: String,
        /// Where it started.
        start: Pos2,
        /// Where the pointer is.
        now: Pos2,
    },
}

/// A swap set being defined or edited.
#[derive(Debug, Clone, Default)]
pub struct SwapDraft {
    /// The name.
    pub name: String,
    /// The paths of the members.
    pub members: Vec<String>,
    /// The set being edited, when it exists already (renamed on save when the name changed).
    pub editing: Option<String>,
}

/// The mesh wire of a group, kept until an edit changes it.
#[derive(Debug, Clone)]
pub struct WireCache {
    /// Puppet, group and number of edits it was made for.
    pub key: (String, String, usize),
    /// The edges.
    pub wire: Vec<[Point; 2]>,
}

/// The state of the rig tools.
#[derive(Debug)]
#[expect(clippy::struct_excessive_bools, reason = "independent view toggles")]
pub struct RigUi {
    /// The active tool.
    pub tool: Tool,
    /// The tab of the right panel.
    pub tab: Tab,
    /// The kind of handle the Handle tool adds.
    pub kind: HandleKind,
    /// The drag in progress.
    pub drag: Option<Drag>,
    /// The handle Delete removes: group path and key.
    pub selected_handle: Option<(String, String)>,
    /// Draw the handles and origins.
    pub show_handles: bool,
    /// Draw the mesh of the selected group.
    pub show_mesh: bool,
    /// Narrows the tag buttons.
    pub tag_filter: String,
    /// Narrows the tree to layers with this in the name or a tag.
    pub layer_filter: String,
    /// The swap set being edited.
    pub swap: SwapDraft,
    /// The key of the next trigger.
    pub new_key: String,
    /// The next key press names the key of the layer.
    pub capture: bool,
    /// Behaviors in the catalog: id and name.
    pub catalog: Vec<(String, String)>,
    /// The catalog entry the Add button adds.
    pub add_behavior: usize,
    /// List only the behaviors that use the selected layer's tags.
    pub only_layer: bool,
    /// The mesh wire of the selected group.
    pub wire: Option<WireCache>,
}

impl Default for RigUi {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            tab: Tab::Properties,
            kind: HandleKind::Draggable,
            drag: None,
            selected_handle: None,
            show_handles: true,
            show_mesh: false,
            tag_filter: String::new(),
            layer_filter: String::new(),
            swap: SwapDraft::default(),
            new_key: String::new(),
            capture: false,
            catalog: catalog().into_iter().map(|c| (c.id, c.name)).collect(),
            add_behavior: 0,
            only_layer: false,
            wire: None,
        }
    }
}

/// The name of a handle kind.
#[must_use]
pub const fn kind_label(kind: HandleKind) -> &'static str {
    match kind {
        HandleKind::Attach => "Attach",
        HandleKind::Fixed => "Fixed",
        HandleKind::Draggable => "Draggable",
        HandleKind::Dangle => "Dangle",
        HandleKind::Stick => "Stick",
        HandleKind::Walk => "Walk",
    }
}

/// The name of an attach style.
#[must_use]
pub const fn attach_label(style: AttachStyle) -> &'static str {
    match style {
        AttachStyle::Weld => "Weld",
        AttachStyle::Hinge => "Hinge",
        AttachStyle::Free => "Free",
    }
}

const fn cycle_label(mode: CycleMode) -> &'static str {
    match mode {
        CycleMode::Loop => "Loop",
        CycleMode::PingPong => "Ping-pong",
        CycleMode::Once => "Once",
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn find_layer<'a>(puppet: &'a Puppet, path: Option<&str>) -> Option<&'a Layer> {
    path.and_then(|p| puppet.find_path(p)).and_then(|id| puppet.layer(id))
}

/// The independent group (not the puppet itself) a layer belongs to or is.
fn group_of<'a>(puppet: &'a Puppet, layer: &'a Layer) -> Option<&'a Layer> {
    if layer.independent && !layer.path.is_empty() {
        return Some(layer);
    }
    let mut best: Option<&Layer> = None;
    for g in puppet.independent_groups().filter(|g| !g.path.is_empty()) {
        let inside = !layer.path.is_empty() && layer.path.starts_with(&g.path) && layer.path.len() > g.path.len();
        if inside && best.is_none_or(|b| g.path.len() > b.path.len()) {
            best = Some(g);
        }
    }
    best
}

/// The group a click at `at` belongs to: the selected group when the click is inside it, else the smallest group
/// under the click.
fn group_at(puppet: &Puppet, at: Pos2, selected: Option<&str>) -> Option<String> {
    let area = |l: &Layer| l.bounds.map(rect_of).map(|r| r.width() * r.height());
    if let Some(sel) = find_layer(puppet, selected).and_then(|l| group_of(puppet, l))
        && sel.bounds.map(rect_of).is_some_and(|r| r.contains(at))
    {
        return Some(sel.path.clone());
    }
    puppet
        .independent_groups()
        .filter(|g| !g.path.is_empty() && g.bounds.map(rect_of).is_some_and(|r| r.contains(at)))
        .min_by(|a, b| area(a).unwrap_or(f32::MAX).total_cmp(&area(b).unwrap_or(f32::MAX)))
        .map(|g| g.path.clone())
}

/// What is under the pointer.
enum Hit {
    Handle { group: String, key: String, end: bool },
    Origin { group: String },
}

fn hit(puppet: &Puppet, at: Pos2, tolerance: f32, origins: bool, handles: bool) -> Option<Hit> {
    let mut best: Option<(f32, Hit)> = None;
    let mut offer = |d: f32, h: Hit| {
        if d <= tolerance && best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, h));
        }
    };
    for g in puppet.independent_groups().filter(|g| !g.path.is_empty()) {
        if origins {
            offer(point(g.origin).distance(at), Hit::Origin { group: g.path.clone() });
        }
        if handles {
            for h in &g.handles {
                offer(point(h.position).distance(at), Hit::Handle { group: g.path.clone(), key: h.key.clone(), end: false });
                if let Some(e) = h.end {
                    offer(point(e).distance(at), Hit::Handle { group: g.path.clone(), key: h.key.clone(), end: true });
                }
            }
        }
    }
    best.map(|(_, h)| h)
}

/// A handle name the group does not use yet.
fn fresh_name(group: &Layer, base: &str) -> String {
    (1_u32..=u32::MAX).map(|n| format!("{base} {n}")).find(|name| !group.handles.iter().any(|h| h.name == *name)).unwrap_or_else(|| base.to_owned())
}

fn pt(p: Pos2) -> [f64; 2] {
    [f64::from(p.x), f64::from(p.y)]
}

fn add_handle(puppet: &Puppet, key: &str, group: &str, kind: HandleKind, at: Pos2, end: Option<Pos2>, actions: &mut Vec<Action>) {
    let Some(layer) = find_layer(puppet, Some(group)) else { return };
    let base = match kind {
        HandleKind::Fixed => "Pin",
        HandleKind::Stick => "Stick",
        other => kind_label(other),
    };
    let mut op = json!({ "op": "add_handle", "group": group, "name": fresh_name(layer, base), "kind": json_of(&kind), "position": pt(at) });
    if let (Some(e), Some(map)) = (end, op.as_object_mut()) {
        map.insert("end".to_owned(), json!(pt(e)));
    }
    actions.push(rig_edit(key, op));
}

/// Pointer work on the artwork. Returns nothing; the actions carry the edits.
pub fn pointer(puppet: &Puppet, key: &str, view: &mut View, p: &StagePointer, actions: &mut Vec<Action>) {
    let tolerance = 10.0 / p.zoom.max(0.05);
    let tool = view.rig.tool;
    let (origins, handles) = match tool {
        Tool::Select => (true, true),
        Tool::Handle | Tool::Pin | Tool::Stick => (false, true),
        Tool::Origin => (true, false),
        Tool::Mesh => (false, false),
    };
    let selected = view.layer.clone();
    match p.phase {
        PointerPhase::Down => {
            let at = p.origin;
            match hit(puppet, at, tolerance, origins, handles) {
                Some(Hit::Handle { group, key: k, end }) => {
                    if p.modifiers.alt && matches!(tool, Tool::Handle | Tool::Pin | Tool::Stick) {
                        actions.push(rig_edit(key, json!({ "op": "remove_handle", "group": group, "key": k })));
                        view.rig.selected_handle = None;
                    } else {
                        view.rig.selected_handle = Some((group.clone(), k.clone()));
                        view.rig.drag = Some(Drag::Handle { group, key: k, end });
                    }
                }
                Some(Hit::Origin { group }) => view.rig.drag = Some(Drag::Origin { group }),
                None if tool == Tool::Stick => {
                    if let Some(group) = group_at(puppet, at, selected.as_deref()) {
                        view.rig.drag = Some(Drag::Stick { group, start: at, now: p.pos });
                    }
                }
                None if tool == Tool::Origin => {
                    if let Some(group) = group_at(puppet, at, selected.as_deref()) {
                        actions.push(rig_edit(key, json!({ "op": "set_origin", "group": group, "origin": pt(p.pos) })));
                        view.rig.drag = Some(Drag::Origin { group });
                    }
                }
                None => view.pointer = Some((f64::from(p.pos.x), f64::from(p.pos.y), true)),
            }
        }
        PointerPhase::Drag => match view.rig.drag.clone() {
            Some(Drag::Handle { group, key: k, end }) => {
                let current = find_layer(puppet, Some(&group)).and_then(|g| g.handles.iter().find(|h| h.key == k));
                let mut op = json!({ "op": "move_handle", "group": group, "key": k, "position": pt(p.pos) });
                if let (true, Some(h), Some(map)) = (end, current, op.as_object_mut()) {
                    map.insert("position".to_owned(), json!([h.position.x, h.position.y]));
                    map.insert("end".to_owned(), json!(pt(p.pos)));
                }
                if let Value::Object(map) = &mut op {
                    map.insert("puppet".to_owned(), json!(key));
                }
                actions.push(Action::RunMerged("rig.edit", op, "rig-handle"));
            }
            Some(Drag::Origin { group }) => {
                let op = json!({ "op": "set_origin", "group": group, "origin": pt(p.pos), "puppet": key });
                actions.push(Action::RunMerged("rig.edit", op, "rig-origin"));
            }
            Some(Drag::Stick { group, start, .. }) => view.rig.drag = Some(Drag::Stick { group, start, now: p.pos }),
            None => view.pointer = Some((f64::from(p.pos.x), f64::from(p.pos.y), true)),
        },
        PointerPhase::Up => {
            if let Some(Drag::Stick { group, start, now }) = view.rig.drag.take()
                && start.distance(now) > tolerance
            {
                add_handle(puppet, key, &group, HandleKind::Stick, start, Some(now), actions);
            }
            view.rig.drag = None;
            view.pointer = view.pointer.map(|(x, y, _)| (x, y, false));
        }
        PointerPhase::Click => {
            let existing = hit(puppet, p.pos, tolerance, false, true);
            match (tool, existing) {
                (Tool::Handle | Tool::Pin | Tool::Stick, Some(Hit::Handle { group, key: k, .. })) => {
                    if p.modifiers.alt {
                        actions.push(rig_edit(key, json!({ "op": "remove_handle", "group": group, "key": k })));
                        view.rig.selected_handle = None;
                    } else {
                        view.rig.selected_handle = Some((group, k));
                    }
                }
                (Tool::Handle | Tool::Pin, None) => {
                    if let Some(group) = group_at(puppet, p.pos, selected.as_deref()) {
                        let kind = if tool == Tool::Pin { HandleKind::Fixed } else { view.rig.kind };
                        add_handle(puppet, key, &group, kind, p.pos, None, actions);
                        view.layer = Some(group);
                    } else {
                        actions.push(Action::SelectLayer(super::rig::pick(puppet, p.pos)));
                    }
                }
                (_, Some(Hit::Handle { group, key: k, .. })) => view.rig.selected_handle = Some((group, k)),
                _ => {
                    view.rig.selected_handle = None;
                    actions.push(Action::SelectLayer(super::rig::pick(puppet, p.pos)));
                }
            }
        }
        PointerPhase::Move => view.pointer = Some((f64::from(p.pos.x), f64::from(p.pos.y), false)),
        PointerPhase::DoubleClick => view.pointer = view.pointer.map(|(x, y, _)| (x, y, false)),
    }
}

/// Tool shortcuts and Delete, when no text field has the keyboard.
pub fn keys(ui: &Ui, puppet: &Puppet, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    ui.input(|i| {
        if i.modifiers.any() {
            return;
        }
        for tool in Tool::ALL {
            if i.key_pressed(tool.key()) {
                view.rig.tool = tool;
            }
        }
        if (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && let Some((group, k)) = view.rig.selected_handle.take()
            && find_layer(puppet, Some(&group)).is_some_and(|g| g.handles.iter().any(|h| h.key == k))
        {
            actions.push(rig_edit(key, json!({ "op": "remove_handle", "group": group, "key": k })));
        }
    });
}

/// The toolbar: one button per tool and the options of the active one.
pub fn toolbar(ui: &mut Ui, puppet: &Puppet, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    // The tools sit along the bottom edge of the canvas, as in Character Animator.
    egui::Panel::bottom("rig_tools").resizable(false).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for tool in Tool::ALL {
                // Compact icon buttons (painted, so crisp at any scale); the name and the key are in the tooltip.
                let icon = match tool {
                    Tool::Select => anim_ui::icons::Icon::Select,
                    Tool::Handle => anim_ui::icons::Icon::Pen,
                    Tool::Stick => anim_ui::icons::Icon::Bone,
                    Tool::Pin => anim_ui::icons::Icon::Pin,
                    Tool::Origin => anim_ui::icons::Icon::FreeTransform,
                    Tool::Mesh => anim_ui::icons::Icon::Outline,
                };
                let tip = format!("{} ({}): {}", tool.label(), tool.key().name(), tool.help());
                if anim_ui::widgets::icon_button(ui, egui::vec2(28.0, 26.0), icon, view.rig.tool == tool, true, &tip).clicked() {
                    view.rig.tool = tool;
                }
            }
            ui.separator();
            ui.toggle_value(&mut view.rig.show_handles, "Handles").on_hover_text("Show the handles and origins");
            ui.toggle_value(&mut view.rig.show_mesh, "Mesh").on_hover_text("Show the mesh of the selected group");
            ui.separator();
            match view.rig.tool {
                Tool::Handle => {
                    egui::ComboBox::from_id_salt("handle_kind").selected_text(kind_label(view.rig.kind)).width(84.0).show_ui(ui, |ui| {
                        for kind in [HandleKind::Draggable, HandleKind::Fixed, HandleKind::Dangle, HandleKind::Attach, HandleKind::Walk] {
                            ui.selectable_value(&mut view.rig.kind, kind, kind_label(kind));
                        }
                    });
                }
                Tool::Mesh => {
                    let group = find_layer(puppet, view.layer.as_deref()).and_then(|l| group_of(puppet, l));
                    if let Some(g) = group {
                        let mut density = g.mesh_density;
                        ui.label(RichText::new("Density").size(11.0));
                        if ui.add(egui::Slider::new(&mut density, 0.5..=4.0).fixed_decimals(1)).changed() {
                            actions.push(Action::RunMerged(
                                "rig.edit",
                                json!({ "op": "set_mesh_density", "group": g.path, "density": density, "puppet": key }),
                                "rig-density",
                            ));
                        }
                    } else {
                        ui.weak("Select a group");
                    }
                }
                Tool::Stick | Tool::Pin | Tool::Origin | Tool::Select => {
                    ui.weak(match view.rig.tool {
                        Tool::Stick => "Drag a link",
                        Tool::Pin => "Click to pin",
                        Tool::Origin => "Drag the origin",
                        _ => "Click a layer",
                    });
                }
            }
            if let Some((group, k)) = view.rig.selected_handle.clone() {
                ui.separator();
                ui.label(RichText::new(visual(k.rsplit('/').next().unwrap_or(&k)).into_owned()).size(11.0).color(HANDLE));
                if ui.button("Delete").on_hover_text("Delete the selected handle (Delete)").clicked() {
                    actions.push(rig_edit(key, json!({ "op": "remove_handle", "group": group, "key": k })));
                    view.rig.selected_handle = None;
                }
            }
        });
    });
}

/// The edges of the selected group's mesh, rebuilt after each edit.
pub fn mesh_wire(puppet: &Puppet, key: &str, edits: usize, view: &mut View) -> Vec<[Point; 2]> {
    let Some(group) = find_layer(puppet, view.layer.as_deref()).and_then(|l| group_of(puppet, l)) else { return Vec::new() };
    let wanted = (key.to_owned(), group.path.clone(), edits);
    if let Some(cache) = view.rig.wire.as_ref().filter(|c| c.key == wanted) {
        return cache.wire.clone();
    }
    let Some(id) = puppet.find_path(&group.path) else { return Vec::new() };
    let wire = puppet.mesh_wire(id);
    view.rig.wire = Some(WireCache { key: wanted, wire: wire.clone() });
    wire
}

/// What the tools draw over the artwork: the mesh, handles, sticks, origins and the stick being drawn.
#[must_use]
pub fn overlays(puppet: &Puppet, view: &View, wire: &[[Point; 2]]) -> Vec<Overlay> {
    let mut out = Vec::new();
    let line = OverlayStyle::line(ACCENT.gamma_multiply(0.45));
    if view.rig.show_mesh || view.rig.tool == Tool::Mesh {
        for [a, b] in wire {
            out.push(Overlay::Line { a: point(*a), b: point(*b), style: line });
        }
    }
    if view.rig.show_handles {
        let chosen = view.rig.selected_handle.as_ref();
        for layer in puppet.independent_groups().filter(|l| !l.path.is_empty()) {
            out.push(Overlay::Marker { pos: point(layer.origin), shape: MarkerShape::Diamond, color: ORIGIN, filled: true });
            for h in &layer.handles {
                let on = chosen.is_some_and(|(g, k)| *g == layer.path && *k == h.key);
                let color = if on { Color32::WHITE } else { HANDLE };
                if let Some(end) = h.end {
                    out.push(Overlay::Line { a: point(h.position), b: point(end), style: OverlayStyle { width: 2.0, ..OverlayStyle::line(color) } });
                    out.push(Overlay::Marker { pos: point(end), shape: MarkerShape::Square, color, filled: true });
                }
                let shape = match h.kind {
                    HandleKind::Fixed => MarkerShape::Square,
                    HandleKind::Attach | HandleKind::Walk | HandleKind::Stick => MarkerShape::Diamond,
                    HandleKind::Draggable | HandleKind::Dangle => MarkerShape::Circle,
                };
                out.push(Overlay::Marker { pos: point(h.position), shape, color, filled: h.kind.pins_mesh() || h.kind == HandleKind::Stick });
                out.push(Overlay::Text { pos: point(h.position) + egui::vec2(8.0, -8.0), text: h.name.clone(), color });
            }
        }
    }
    if let Some(Drag::Stick { start, now, .. }) = &view.rig.drag {
        out.push(Overlay::Line { a: *start, b: *now, style: OverlayStyle { width: 2.0, ..OverlayStyle::dashed(HANDLE) } });
    }
    out
}

/// The names and tags the filter box matches.
#[must_use]
pub fn matches_filter(layer: &Layer, filter: &str) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || layer.name.to_lowercase().contains(&f) || layer.tags.iter().any(|t| t.to_lowercase().contains(&f))
}

/// The tag buttons grouped by what they are for.
fn tag_groups(independent: bool) -> Vec<(&'static str, Vec<&'static str>)> {
    let mut groups: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for t in AUTO_TAGS {
        if t.kind == TagKind::Handle && !independent {
            continue;
        }
        let name = match t.id.split('.').nth(1).unwrap_or("") {
            "Face" => "Face",
            "LipSynch" => "Mouths",
            "Dragger" | "HandleFixer" | "RigDynamics" | "Transform" | "FlappyChin" => "Motion",
            "Walk" => "Walk",
            _ => "Other",
        };
        match groups.iter_mut().find(|(g, _)| *g == name) {
            Some((_, ids)) => {
                if !ids.contains(&t.id) {
                    ids.push(t.id);
                }
            }
            None => groups.push((name, vec![t.id])),
        }
    }
    groups
}

/// The tag picker: buttons for the standard tags (on when the layer has it), a filter and a field for any other.
fn tag_picker(ui: &mut Ui, layer: &Layer, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Tags").strong());
    ui.horizontal_wrapped(|ui| {
        for tag in layer.tags.iter().filter(|t| !AUTO_TAGS.iter().any(|a| a.id == t.as_str())) {
            ui.label(RichText::new(short(tag)).color(ACCENT)).on_hover_text(tag);
            if ui.small_button("\u{00D7}").on_hover_text("Remove tag").clicked() {
                actions.push(rig_edit(key, json!({ "op": "remove_tag", "layer": layer.path, "tag": tag })));
            }
        }
    });
    ui.add(egui::TextEdit::singleline(&mut view.rig.tag_filter).hint_text("Find a tag").desired_width(f32::INFINITY));
    let filter = view.rig.tag_filter.trim().to_lowercase();
    egui::ScrollArea::vertical().id_salt("tag_buttons").max_height(190.0).auto_shrink([false, true]).show(ui, |ui| {
        for (name, ids) in tag_groups(layer.independent) {
            let shown: Vec<&str> = ids.into_iter().filter(|id| filter.is_empty() || id.to_lowercase().contains(&filter)).collect();
            if shown.is_empty() {
                continue;
            }
            ui.label(RichText::new(name).weak().size(11.0));
            ui.horizontal_wrapped(|ui| {
                for id in shown {
                    let on = layer.has_tag(id);
                    if ui.selectable_label(on, short(id)).on_hover_text(id).clicked() {
                        let op = if on { "remove_tag" } else { "add_tag" };
                        actions.push(rig_edit(key, json!({ "op": op, "layer": layer.path, "tag": id })));
                    }
                }
            });
        }
    });
    ui.horizontal(|ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut view.new_tag).hint_text("Custom tag id").desired_width(140.0));
        let submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add tag").clicked() || submit) && !view.new_tag.trim().is_empty() {
            actions.push(rig_edit(key, json!({ "op": "add_tag", "layer": layer.path, "tag": view.new_tag.trim() })));
            view.new_tag.clear();
        }
    });
}

/// A text field with a draft that survives frames; returns the text when it was committed (Enter or focus lost)
/// and differs from `current`.
fn text_field(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, current: &str, width: f32, hint: &str) -> Option<String> {
    let id = ui.id().with(salt);
    let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| current.to_owned());
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width).hint_text(hint));
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(id));
        return (text != current).then_some(text);
    }
    None
}

fn number(ui: &mut Ui, value: &mut f64) -> bool {
    ui.add(egui::DragValue::new(value).speed(0.5).max_decimals(1)).changed()
}

fn handle_row(ui: &mut Ui, group: &Layer, h: &HandleDef, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let on = view.rig.selected_handle.as_ref().is_some_and(|(g, k)| *g == group.path && *k == h.key);
        if ui.selectable_label(on, format!("{} ({})", visual(&h.name), kind_label(h.kind))).clicked() {
            view.rig.selected_handle = Some((group.path.clone(), h.key.clone()));
        }
        let (mut x, mut y) = (h.position.x, h.position.y);
        let moved = number(ui, &mut x) | number(ui, &mut y);
        if moved {
            actions.push(Action::RunMerged(
                "rig.edit",
                json!({ "op": "move_handle", "group": group.path, "key": h.key, "position": [x, y], "puppet": key }),
                "rig-handle-number",
            ));
        }
        if ui.small_button("\u{00D7}").on_hover_text("Remove handle").clicked() {
            actions.push(rig_edit(key, json!({ "op": "remove_handle", "group": group.path, "key": h.key })));
        }
    });
}

/// The group section: independent toggle, attach style, warp independence, mesh density, origin and handles.
fn group_section(ui: &mut Ui, puppet: &Puppet, layer: &Layer, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    ui.separator();
    ui.label(RichText::new("Group").strong());
    if !layer.path.is_empty() {
        let mut independent = layer.independent;
        if ui.checkbox(&mut independent, "Independent group").on_hover_text("Moves, rotates and scales on its own, with its own origin and handles").changed() {
            actions.push(rig_edit(key, json!({ "op": "set_independent", "layer": layer.path, "independent": independent })));
        }
    }
    if layer.independent && !layer.path.is_empty() {
        ui.horizontal(|ui| {
            ui.label("Attach");
            for style in AttachStyle::ALL {
                if ui.selectable_label(layer.attach == style, attach_label(style)).clicked() {
                    actions.push(rig_edit(key, json!({ "op": "set_attach_style", "group": layer.path, "style": json_of(&style) })));
                }
            }
        });
        let mut warp = layer.warp_independent;
        if ui.checkbox(&mut warp, "Independent warp").on_hover_text("Its mesh ignores the groups attached to it").changed() {
            actions.push(rig_edit(key, json!({ "op": "set_warp_independent", "group": layer.path, "on": warp })));
        }
        let mut density = layer.mesh_density;
        ui.horizontal(|ui| {
            ui.label("Mesh density");
            if ui.add(egui::Slider::new(&mut density, 0.5..=4.0).fixed_decimals(1)).changed() {
                actions.push(Action::RunMerged(
                    "rig.edit",
                    json!({ "op": "set_mesh_density", "group": layer.path, "density": density, "puppet": key }),
                    "rig-density",
                ));
            }
        });
        let (mut ox, mut oy) = (layer.origin.x, layer.origin.y);
        ui.horizontal(|ui| {
            ui.label("Origin");
            if number(ui, &mut ox) | number(ui, &mut oy) {
                actions.push(Action::RunMerged(
                    "rig.edit",
                    json!({ "op": "set_origin", "group": layer.path, "origin": [ox, oy], "puppet": key }),
                    "rig-origin-number",
                ));
            }
        });
        ui.label(RichText::new("Handles").strong());
        for h in &layer.handles {
            handle_row(ui, layer, h, key, view, actions);
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut view.new_handle).desired_width(90.0));
            egui::ComboBox::from_id_salt("handle_kind_inspector").selected_text(kind_label(view.rig.kind)).width(90.0).show_ui(ui, |ui| {
                for kind in [HandleKind::Draggable, HandleKind::Fixed, HandleKind::Dangle, HandleKind::Attach, HandleKind::Stick, HandleKind::Walk] {
                    ui.selectable_value(&mut view.rig.kind, kind, kind_label(kind));
                }
            });
            if ui.button("Add at origin").clicked() && !view.new_handle.trim().is_empty() {
                let mut op = json!({
                    "op": "add_handle", "group": layer.path, "name": view.new_handle.trim(), "kind": json_of(&view.rig.kind),
                    "position": [layer.origin.x, layer.origin.y],
                });
                if let (HandleKind::Stick, Some(map)) = (view.rig.kind, op.as_object_mut()) {
                    map.insert("end".to_owned(), json!([layer.origin.x + 60.0, layer.origin.y]));
                }
                actions.push(rig_edit(key, op));
            }
        });
        ui.weak("Or pick the Handle, Stick or Pin tool and click the artwork.");
    } else if let Some(g) = group_of(puppet, layer) {
        ui.weak(format!("Inside the group {}", visual(&g.name)));
    }
}

/// The trigger and cycle sections of a layer.
fn layer_sections(ui: &mut Ui, layer: &Layer, key: &str, actions: &mut Vec<Action>) {
    ui.separator();
    ui.label(RichText::new("Trigger").strong());
    ui.horizontal(|ui| {
        ui.label("Key");
        let current = layer.key_trigger.clone().unwrap_or_default();
        if let Some(new) = text_field(ui, ("layer_key", &layer.path), &current, 70.0, "none") {
            let key_value = if new.trim().is_empty() { Value::Null } else { json!(new.trim().to_lowercase()) };
            actions.push(rig_edit(key, json!({ "op": "set_key_trigger", "layer": layer.path, "key": key_value })));
        }
    });
    if layer.key_trigger.is_some() {
        let mut hide = layer.hide_others;
        if ui.checkbox(&mut hide, "Hide others when triggered").changed() {
            actions.push(rig_edit(key, json!({ "op": "set_hide_others", "layer": layer.path, "on": hide })));
        }
    }
    if layer.children.len() >= 2 {
        ui.separator();
        ui.label(RichText::new("Cycle layers").strong());
        let mut on = layer.cycle.is_some();
        if ui.checkbox(&mut on, "Show the layers inside one after the other").changed() {
            let cycle = on.then(|| CycleDef::looping(200));
            actions.push(rig_edit(key, json!({ "op": "set_cycle", "layer": layer.path, "cycle": cycle.as_ref().map(json_of) })));
        }
        if let Some(c) = layer.cycle {
            let mut step = c.step_millis;
            ui.horizontal(|ui| {
                ui.label("Step (ms)");
                if ui.add(egui::DragValue::new(&mut step).range(CycleDef::MIN_STEP..=CycleDef::MAX_STEP).speed(5)).changed() {
                    let next = CycleDef { step_millis: step, ..c };
                    actions.push(Action::RunMerged(
                        "rig.edit",
                        json!({ "op": "set_cycle", "layer": layer.path, "cycle": json_of(&next), "puppet": key }),
                        "rig-cycle",
                    ));
                }
                for mode in [CycleMode::Loop, CycleMode::PingPong, CycleMode::Once] {
                    if ui.selectable_label(c.mode == mode, cycle_label(mode)).clicked() {
                        let next = CycleDef { mode, ..c };
                        actions.push(rig_edit(key, json!({ "op": "set_cycle", "layer": layer.path, "cycle": json_of(&next) })));
                    }
                }
            });
        }
    }
}

/// The Properties tab: the selected layer.
pub fn properties(ui: &mut Ui, puppet: &Puppet, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    let Some(layer) = find_layer(puppet, view.layer.as_deref()) else {
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
    tag_picker(ui, layer, key, view, actions);
    group_section(ui, puppet, layer, key, view, actions);
    layer_sections(ui, layer, key, actions);
}

/// The key a pressed key is stored as.
fn pressed_key(ui: &Ui) -> Option<String> {
    ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !modifiers.any() && *key != egui::Key::Escape => {
                Some(key.name().to_lowercase())
            }
            _ => None,
        })
    })
}

fn path_name(puppet: &Puppet, path: &str) -> String {
    find_layer(puppet, Some(path)).map_or_else(|| path.to_owned(), |l| visual(&l.name).into_owned())
}

/// The Triggers tab: the layers with a key, and the swap sets.
pub fn triggers(ui: &mut Ui, puppet: &Puppet, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    ui.label(RichText::new("Triggers").strong());
    ui.weak("A layer with a key shows while the key is down. Pick a layer, then give it a key.");
    let triggered: Vec<&Layer> = puppet.layers().iter().filter(|l| l.key_trigger.is_some()).collect();
    if triggered.is_empty() {
        ui.weak("No layer has a key yet.");
    }
    for layer in triggered {
        ui.horizontal(|ui| {
            let selected = view.layer.as_deref() == Some(layer.path.as_str());
            if ui.selectable_label(selected, visual(&layer.name).into_owned()).clicked() {
                view.layer = Some(layer.path.clone());
            }
            let current = layer.key_trigger.clone().unwrap_or_default();
            if let Some(new) = text_field(ui, ("trigger_key", &layer.path), &current, 56.0, "key") {
                let value = if new.trim().is_empty() { Value::Null } else { json!(new.trim().to_lowercase()) };
                actions.push(rig_edit(key, json!({ "op": "set_key_trigger", "layer": layer.path, "key": value })));
            }
            let mut hide = layer.hide_others;
            if ui.checkbox(&mut hide, "Hide others").changed() {
                actions.push(rig_edit(key, json!({ "op": "set_hide_others", "layer": layer.path, "on": hide })));
            }
            if ui.small_button("\u{00D7}").on_hover_text("Remove the trigger").clicked() {
                actions.push(rig_edit(key, json!({ "op": "set_key_trigger", "layer": layer.path, "key": null })));
            }
        });
    }
    ui.add_space(4.0);
    if let Some(layer) = find_layer(puppet, view.layer.as_deref()) {
        ui.horizontal(|ui| {
            ui.label(format!("New trigger on {}", visual(&layer.name)));
            if view.rig.capture {
                ui.label(RichText::new("press a key...").color(ACCENT));
                if let Some(k) = pressed_key(ui) {
                    actions.push(rig_edit(key, json!({ "op": "set_key_trigger", "layer": layer.path, "key": k })));
                    view.rig.capture = false;
                }
                if ui.button("Cancel").clicked() {
                    view.rig.capture = false;
                }
            } else if ui.button("Press a key").clicked() {
                view.rig.capture = true;
            }
        });
    }
    ui.separator();
    ui.label(RichText::new("Swap sets").strong());
    ui.weak("Of the layers in a set, only the one the trigger picks is shown.");
    for set in &puppet.swap_sets {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(visual(&set.name).into_owned()).strong());
            for id in &set.members {
                if let Some(l) = puppet.layer(*id) {
                    ui.label(RichText::new(visual(&l.name).into_owned()).color(ACCENT));
                }
            }
            if ui.small_button("Edit").clicked() {
                view.rig.swap = SwapDraft {
                    name: set.name.clone(),
                    members: set.members.iter().filter_map(|id| puppet.layer(*id)).map(|l| l.path.clone()).collect(),
                    editing: Some(set.name.clone()),
                };
            }
            if ui.small_button("\u{00D7}").on_hover_text("Delete the swap set").clicked() {
                actions.push(rig_edit(key, json!({ "op": "remove_swap_set", "name": set.name })));
            }
        });
    }
    swap_editor(ui, puppet, key, view, actions);
}

fn swap_editor(ui: &mut Ui, puppet: &Puppet, key: &str, view: &mut View, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    let title = if view.rig.swap.editing.is_some() { "Edit swap set" } else { "New swap set" };
    ui.label(RichText::new(title).strong().size(12.0));
    ui.add(egui::TextEdit::singleline(&mut view.rig.swap.name).hint_text("Name").desired_width(160.0));
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for (i, path) in view.rig.swap.members.iter().enumerate() {
            ui.label(RichText::new(path_name(puppet, path)).color(ACCENT));
            if ui.small_button("\u{00D7}").clicked() {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        view.rig.swap.members.remove(i);
    }
    ui.horizontal(|ui| {
        let selected = view.layer.clone();
        let can_add = selected.as_ref().is_some_and(|s| !view.rig.swap.members.contains(s));
        if ui.add_enabled(can_add, egui::Button::new("Add selected layer")).clicked()
            && let Some(s) = selected
        {
            view.rig.swap.members.push(s);
        }
        let ready = !view.rig.swap.name.trim().is_empty() && view.rig.swap.members.len() >= 2;
        let label = if view.rig.swap.editing.is_some() { "Save" } else { "Define" };
        if ui.add_enabled(ready, egui::Button::new(label)).on_hover_text("A swap set needs a name and two layers").clicked() {
            let draft = std::mem::take(&mut view.rig.swap);
            let name = draft.name.trim().to_owned();
            if let Some(old) = draft.editing.filter(|o| *o != name) {
                if puppet.swap_sets.iter().any(|s| s.name == name) {
                    actions.push(rig_edit(key, json!({ "op": "remove_swap_set", "name": name })));
                }
                actions.push(rig_edit(key, json!({ "op": "rename_swap_set", "name": old, "new_name": name })));
            }
            actions.push(rig_edit(key, json!({ "op": "add_swap_set", "name": name, "members": draft.members })));
        }
        if (view.rig.swap.editing.is_some() || !view.rig.swap.members.is_empty()) && ui.button("Clear").clicked() {
            view.rig.swap = SwapDraft::default();
        }
    });
}

/// Whether a behavior works on the layer: the layer carries a tag of the behavior's namespace.
fn uses_layer(behavior_id: &str, layer: &Layer) -> bool {
    let name = behavior_id.split('.').nth(1);
    name.is_some() && layer.tags.iter().any(|t| t.split('.').nth(1) == name)
}

/// The Behaviors tab: add, remove, reorder and tune the behaviors of the selected track, optionally only those that
/// use the selected layer.
pub fn behaviors(ui: &mut Ui, puppet: &Puppet, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let Some(stack) = ctx.stack else {
        ui.weak("No track selected.");
        return;
    };
    let track = view.track;
    let layer = find_layer(puppet, view.layer.as_deref());
    ui.horizontal(|ui| {
        ui.label(RichText::new("Behaviors").strong());
        if let Some(l) = layer {
            ui.checkbox(&mut view.rig.only_layer, format!("Only {}", visual(&l.name))).on_hover_text("List the behaviors that use this layer's tags");
        }
    });
    ui.horizontal(|ui| {
        let names = view.rig.catalog.clone();
        let current = names.get(view.rig.add_behavior).map_or("", |(_, n)| n.as_str());
        egui::ComboBox::from_id_salt("add_behavior").selected_text(current).width(170.0).show_ui(ui, |ui| {
            for (i, (_, name)) in names.iter().enumerate() {
                ui.selectable_value(&mut view.rig.add_behavior, i, name);
            }
        });
        if ui.button("Add").clicked()
            && let Some((id, _)) = names.get(view.rig.add_behavior)
        {
            actions.push(Action::Run("track.behavior", json!({ "track": track, "op": StackOp::Add { behavior: id.clone(), params: None, at: None } })));
        }
    });
    let count = stack.behaviors.len();
    for info in &stack.behaviors {
        if view.rig.only_layer && !layer.is_some_and(|l| uses_layer(&info.id, l)) {
            continue;
        }
        super::params::behavior_block(ui, track, info, count, &ctx.doc.rec.arming.groups, false, actions);
    }
}

/// The tree's rows that pass the filter, with the parents that lead to them.
#[must_use]
pub fn visible_paths(puppet: &Puppet, filter: &str) -> Option<BTreeSet<String>> {
    if filter.trim().is_empty() {
        return None;
    }
    let mut keep = BTreeSet::new();
    for l in puppet.layers().iter().filter(|l| matches_filter(l, filter)) {
        let mut path = l.path.as_str();
        loop {
            keep.insert(path.to_owned());
            match path.rfind('/') {
                Some(i) => path = path.get(..i).unwrap_or(""),
                None => break,
            }
        }
    }
    Some(keep)
}
