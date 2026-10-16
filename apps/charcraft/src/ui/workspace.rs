//! A docked workspace: draws the dock of the current mode and fills each panel with its contents.

use anim_ui::dock::PanelId;
use anim_ui::dock::docking::{DockArea, DockContent, PanelLimits};
use anim_ui::theme::visual;
use egui::{Id, Ui};

use super::dock::{self, CAMERA, CONTROLS, EDIT, OUTPUT, PROJECT, PROPERTIES, PUPPET, SCENE, SCENES, TIMELINE, TRIGGERS};
use super::{project, record, rig, stream};
use crate::state::Mode;
use crate::view::{Action, Ctx, View};

struct Content<'a, 'c> {
    mode: Mode,
    view: &'a mut View,
    ctx: &'a Ctx<'c>,
    actions: &'a mut Vec<Action>,
}

impl DockContent<PanelId> for Content<'_, '_> {
    fn body(&mut self, ui: &mut Ui, panel: &PanelId) {
        let (view, ctx, actions) = (&mut *self.view, self.ctx, &mut *self.actions);
        let panel = *panel;
        // Panels that fill their area with a canvas or a timeline take no margin; the rest get a little air.
        let margin = if matches!((self.mode, panel), (_, SCENE | TIMELINE)) { 0 } else { 4 };
        egui::Frame::new().inner_margin(egui::Margin::same(margin)).show(ui, |ui| match (self.mode, panel) {
            (Mode::Rig, SCENE) => rig::canvas_panel(ui, view, ctx, actions),
            (Mode::Stream, SCENE) => stream::stage_panel(ui, view, ctx),
            (_, SCENE) => record::scene_panel(ui, view, ctx, actions, false),
            (_, PROJECT) => project::panel(ui, view, ctx, actions),
            (_, CAMERA) => record::camera_panel(ui, view, actions),
            (_, OUTPUT) => stream::output_panel(ui, view, actions),
            (_, SCENES) => stream::scenes_panel(ui, view, ctx, actions),
            (Mode::Rig, PROPERTIES) => rig::properties_panel(ui, view, ctx, actions),
            (_, PROPERTIES) => record::properties_panel(ui, view, ctx, actions),
            (_, EDIT) => record::edit_panel(ui, view, ctx, actions),
            (_, TIMELINE) => record::timeline_panel(ui, view, ctx, actions, self.mode == Mode::Stream),
            (_, CONTROLS) => stream::controls_panel(ui, view, ctx, actions),
            (_, TRIGGERS) => rig::triggers_panel(ui, view, ctx, actions),
            (_, PUPPET) => rig::puppet_panel(ui, view, ctx, actions),
            _ => {
                ui.weak("Empty panel");
            }
        });
    }

    fn group_header_width(&self, _panels: &[PanelId], _active: &PanelId, floating: bool) -> f32 {
        if floating { 0.0 } else { 26.0 }
    }

    /// The panel menu at the right end of a tab strip.
    fn group_header(&mut self, ui: &mut Ui, _panels: &[PanelId], active: &PanelId, floating: bool) {
        if floating {
            return;
        }
        let panel = *active;
        ui.menu_button("\u{2261}", |ui| {
            if ui.add_enabled(panel != SCENE, egui::Button::new("Close Panel")).clicked() {
                self.view.panel_close = Some(panel);
                ui.close();
            }
            ui.separator();
            if ui.button("Save Changes to this Workspace").clicked() {
                self.view.docks.save_workspace(self.mode);
                ui.close();
            }
            if ui.button("Reset to Saved Layout").clicked() {
                self.view.reset_layout = true;
                ui.close();
            }
        });
    }
}

/// Draws the dock of the current workspace into the free space of `ui`.
pub fn show(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    let mode = view.mode;
    let mut layout = view.docks.take(mode);
    let style = dock::style(ui);
    let output = {
        let mut content = Content { mode, view, ctx, actions };
        DockArea::new(Id::new(("workspace", mode))).show_customized(
            ui,
            &layout,
            &style,
            |panel| visual(dock::title(mode, *panel)).into_owned(),
            |panel| panel.permissions(),
            |_| PanelLimits::default(),
            &mut content,
        )
    };
    let changed = !output.actions.is_empty();
    let _ = anim_ui::dock::apply_actions(&mut layout, output.actions);
    view.docks.put(mode, layout, changed);
    if let Some(panel) = view.panel_close.take()
        && view.docks.contains(mode, panel)
    {
        view.docks.toggle(mode, panel);
    }
    if std::mem::take(&mut view.reset_layout) {
        view.docks.reset(mode);
    }
}
