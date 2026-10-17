//! The View menu's grid, guides and snapping items, and what dragging a guide out of a ruler asks for.

use anim_ui::stage::{Axis, StageEvent};
use egui::Ui;
use serde_json::json;

use crate::grid::GridView;
use crate::view::{Action, View};

/// The command a guide event of the stage widget stands for.
#[must_use]
pub fn guide_action(view: GridView, event: &StageEvent) -> Option<Action> {
    let name = match view {
        GridView::Rig => "rig",
        GridView::Scene => "scene",
    };
    Some(match event {
        StageEvent::GuideAdded(g) => {
            let axis = if g.axis == Axis::Horizontal { "horizontal" } else { "vertical" };
            Action::Run("grid.guide", json!({ "view": name, "action": "add", "axis": axis, "pos": g.pos }))
        }
        StageEvent::GuideMoved { index, pos } => Action::Run("grid.guide", json!({ "view": name, "action": "move", "index": index, "pos": pos })),
        StageEvent::GuideRemoved { index } => Action::Run("grid.guide", json!({ "view": name, "action": "remove", "index": index })),
        _ => return None,
    })
}

fn toggle(ui: &mut Ui, label: &str, key: &str, now: bool, actions: &mut Vec<Action>) {
    let mut on = now;
    if ui.checkbox(&mut on, label).changed() {
        actions.push(Action::Run("grid.set", json!({ key: on })));
    }
}

/// Grid, guides and snapping, as menu items.
pub fn items(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    let s = &view.grid.settings;
    toggle(ui, "Show Grid", "show_grid", s.show_grid, actions);
    toggle(ui, "Show Rulers", "show_rulers", s.show_rulers, actions);
    toggle(ui, "Show Guides", "show_guides", s.show_guides, actions);
    toggle(ui, "Lock Guides", "lock_guides", s.lock_guides, actions);
    if ui.button("Clear Guides").clicked() {
        let which = if view.mode == crate::state::Mode::Rig { "rig" } else { "scene" };
        actions.push(Action::Run("grid.guide", json!({ "view": which, "action": "clear" })));
        ui.close();
    }
    ui.separator();
    toggle(ui, "Snap Handles and Origins", "snap", s.snap, actions);
    ui.add_enabled_ui(s.snap, |ui| {
        toggle(ui, "Snap to Grid", "snap_to_grid", s.snap_to_grid, actions);
        toggle(ui, "Snap to Guides", "snap_to_guides", s.snap_to_guides, actions);
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Grid size");
        let mut size = s.size;
        if ui.add(egui::DragValue::new(&mut size).range(2.0..=2000.0).speed(1.0).suffix(" px")).changed() {
            actions.push(Action::Run("grid.set", json!({ "size": size })));
        }
    });
}
