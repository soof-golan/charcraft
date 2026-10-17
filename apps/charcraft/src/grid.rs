//! Grid, guides and snapping for the Rig and Scene views.
//!
//! [`GridState`] lives in the hooks, like the MIDI state: the View menu and the rulers ask for changes with a
//! [`GridOp`] over [`crate::session::Links`] (or with the `grid.set` and `grid.guide` commands), the window copies the
//! state into the stage widgets each frame, and the rig tools snap handles and origins with [`GridState::snap`].
//! The settings are kept in the preferences; guides belong to the session.

use anim_session::{CommandError, CommandSpec, param, param_opt};
use anim_ui::stage::{Axis, Guide, StageViewState};
use egui::{Pos2, pos2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::session::AppDoc;

/// Most guides one view keeps.
const MAX_GUIDES: usize = 256;
/// Smallest grid spacing, in stage pixels.
const MIN_SIZE: f32 = 2.0;
/// Largest grid spacing, in stage pixels.
const MAX_SIZE: f32 = 2000.0;

/// Which view's guides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridView {
    /// The artwork view of Rig mode.
    Rig,
    /// The scene view of Record and Stream.
    Scene,
}

/// What shows and what snaps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "independent switches of the View menu")]
pub struct GridSettings {
    /// Draw the grid.
    pub show_grid: bool,
    /// Draw the guides.
    pub show_guides: bool,
    /// Draw the rulers (guides are dragged out of them).
    pub show_rulers: bool,
    /// Guides cannot be moved or removed.
    pub lock_guides: bool,
    /// Horizontal grid spacing in stage pixels.
    pub size: f32,
    /// Vertical grid spacing; 0 follows [`Self::size`].
    pub size_y: f32,
    /// Snap handles and origins while they are dragged or placed.
    pub snap: bool,
    /// Snap to the grid lines.
    pub snap_to_grid: bool,
    /// Snap to the guides.
    pub snap_to_guides: bool,
    /// How near, in screen pixels, a line must be to catch the point.
    pub snap_pixels: f32,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            show_grid: false,
            show_guides: true,
            show_rulers: true,
            lock_guides: false,
            size: 20.0,
            size_y: 0.0,
            snap: false,
            snap_to_grid: true,
            snap_to_guides: true,
            snap_pixels: 8.0,
        }
    }
}

impl GridSettings {
    /// The same settings with every number inside its range.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.size = if self.size.is_finite() { self.size.clamp(MIN_SIZE, MAX_SIZE) } else { 20.0 };
        self.size_y = if self.size_y.is_finite() && self.size_y > 0.0 { self.size_y.clamp(MIN_SIZE, MAX_SIZE) } else { 0.0 };
        self.snap_pixels = if self.snap_pixels.is_finite() { self.snap_pixels.clamp(1.0, 40.0) } else { 8.0 };
        self
    }

    fn step_y(&self) -> f32 {
        if self.size_y > 0.0 { self.size_y } else { self.size }
    }
}

/// A change of the settings; absent fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GridPatch {
    /// See [`GridSettings::show_grid`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_grid: Option<bool>,
    /// See [`GridSettings::show_guides`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_guides: Option<bool>,
    /// See [`GridSettings::show_rulers`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_rulers: Option<bool>,
    /// See [`GridSettings::lock_guides`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_guides: Option<bool>,
    /// See [`GridSettings::size`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// See [`GridSettings::size_y`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_y: Option<f32>,
    /// See [`GridSettings::snap`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap: Option<bool>,
    /// See [`GridSettings::snap_to_grid`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_to_grid: Option<bool>,
    /// See [`GridSettings::snap_to_guides`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_to_guides: Option<bool>,
    /// See [`GridSettings::snap_pixels`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_pixels: Option<f32>,
}

/// A request to the grid state.
#[derive(Debug, Clone, PartialEq)]
pub enum GridOp {
    /// Change settings.
    Set(GridPatch),
    /// Add a guide.
    AddGuide(GridView, Guide),
    /// Move a guide.
    MoveGuide(GridView, usize, f32),
    /// Remove a guide.
    RemoveGuide(GridView, usize),
    /// Remove every guide of a view.
    ClearGuides(GridView),
}

/// The settings and the guides of both views.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridState {
    /// What shows and what snaps.
    pub settings: GridSettings,
    /// Guides of the Rig view.
    pub rig: Vec<Guide>,
    /// Guides of the Scene view.
    pub scene: Vec<Guide>,
    dirty: bool,
}

fn finite(v: f32) -> bool {
    v.is_finite() && v.abs() < 1.0e7
}

impl GridState {
    /// State from stored settings.
    #[must_use]
    pub fn with_settings(settings: GridSettings) -> Self {
        Self { settings: settings.sanitized(), ..Self::default() }
    }

    /// The settings to save, when they changed since the last call.
    pub fn take_prefs(&mut self) -> Option<GridSettings> {
        std::mem::take(&mut self.dirty).then_some(self.settings)
    }

    /// The guides of a view.
    #[must_use]
    pub fn guides(&self, view: GridView) -> &[Guide] {
        match view {
            GridView::Rig => &self.rig,
            GridView::Scene => &self.scene,
        }
    }

    const fn guides_mut(&mut self, view: GridView) -> &mut Vec<Guide> {
        match view {
            GridView::Rig => &mut self.rig,
            GridView::Scene => &mut self.scene,
        }
    }

    /// Applies a request. Moving or removing a locked guide, and anything unusable, is ignored.
    pub fn apply(&mut self, op: GridOp) {
        match op {
            GridOp::Set(p) => {
                let s = &mut self.settings;
                let before = *s;
                macro_rules! take {
                    ($($field:ident),*) => { $(if let Some(v) = p.$field { s.$field = v; })* };
                }
                take!(show_grid, show_guides, show_rulers, lock_guides, size, size_y, snap, snap_to_grid, snap_to_guides, snap_pixels);
                *s = s.sanitized();
                self.dirty |= *s != before;
            }
            GridOp::AddGuide(view, guide) => {
                let list = self.guides_mut(view);
                if finite(guide.pos) && list.len() < MAX_GUIDES {
                    list.push(guide);
                }
            }
            GridOp::MoveGuide(view, index, pos) => {
                let locked = self.settings.lock_guides;
                if let (false, true, Some(g)) = (locked, finite(pos), self.guides_mut(view).get_mut(index)) {
                    g.pos = pos;
                }
            }
            GridOp::RemoveGuide(view, index) => {
                let locked = self.settings.lock_guides;
                let list = self.guides_mut(view);
                if !locked && index < list.len() {
                    list.remove(index);
                }
            }
            GridOp::ClearGuides(view) => self.guides_mut(view).clear(),
        }
    }

    /// Copies what the stage widget draws into it.
    pub fn apply_to(&self, view: GridView, stage: &mut StageViewState) {
        let s = &self.settings;
        stage.show_grid = s.show_grid;
        stage.show_guides = s.show_guides;
        stage.show_rulers = s.show_rulers;
        stage.lock_guides = s.lock_guides;
        stage.grid_size = s.size;
        stage.grid_size_y = s.size_y;
        stage.guides = self.guides(view).to_vec();
    }

    /// The point a drag lands on: the nearest grid line and guide on each axis within the snap distance, when
    /// snapping is on. `zoom` is the view's zoom, so the distance is the same on screen at every zoom.
    #[must_use]
    pub fn snap(&self, view: GridView, p: Pos2, zoom: f32) -> Pos2 {
        let s = &self.settings;
        if !s.snap || !finite(p.x) || !finite(p.y) {
            return p;
        }
        let reach = s.snap_pixels / zoom.max(0.01);
        let guides = self.guides(view);
        let line = |v: f32, step: f32, axis: Axis| -> f32 {
            let mut best: Option<(f32, f32)> = None;
            let mut offer = |target: f32| {
                let d = (target - v).abs();
                if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, target));
                }
            };
            if s.snap_to_grid && step > 0.0 {
                offer((v / step).round() * step);
            }
            if s.snap_to_guides {
                for g in guides.iter().filter(|g| g.axis == axis) {
                    offer(g.pos);
                }
            }
            best.map_or(v, |(_, t)| t)
        };
        // A vertical guide is an x coordinate, a horizontal one a y coordinate.
        pos2(line(p.x, s.size, Axis::Vertical), line(p.y, s.step_y(), Axis::Horizontal))
    }
}

// --- commands ----------------------------------------------------------------------------------------------------------

fn send(doc: &AppDoc, op: GridOp) -> Result<(), CommandError> {
    doc.links.grid.send(op).map_err(|_| CommandError::failed("the grid is not running"))
}

fn view_of(params: &Value) -> Result<GridView, CommandError> {
    let name: Option<String> = param_opt(params, "view")?;
    match name.as_deref().unwrap_or("rig") {
        "rig" => Ok(GridView::Rig),
        "scene" => Ok(GridView::Scene),
        other => Err(CommandError::invalid_params(format!("view is rig or scene, not {other:?}"))),
    }
}

fn grid_set(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let patch: GridPatch = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    for (name, v) in [("size", patch.size), ("size_y", patch.size_y), ("snap_pixels", patch.snap_pixels)] {
        if v.is_some_and(|v| !v.is_finite()) {
            return Err(CommandError::invalid_params(format!("{name} must be a finite number")));
        }
    }
    if patch.size.is_some_and(|v| !(MIN_SIZE..=MAX_SIZE).contains(&v)) {
        return Err(CommandError::invalid_params(format!("size is {MIN_SIZE} to {MAX_SIZE} stage pixels")));
    }
    send(doc, GridOp::Set(patch.clone()))?;
    Ok(json!({ "requested": patch }))
}

fn grid_guide(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let view = view_of(params)?;
    let action: String = param(params, "action")?;
    let op = match action.as_str() {
        "add" => {
            let axis: String = param(params, "axis")?;
            let pos: f32 = param(params, "pos")?;
            let axis = match axis.as_str() {
                "horizontal" => Axis::Horizontal,
                "vertical" => Axis::Vertical,
                other => return Err(CommandError::invalid_params(format!("axis is horizontal or vertical, not {other:?}"))),
            };
            if !finite(pos) {
                return Err(CommandError::invalid_params("pos must be a finite number"));
            }
            GridOp::AddGuide(view, Guide { axis, pos })
        }
        "move" => {
            let (index, pos): (usize, f32) = (param(params, "index")?, param(params, "pos")?);
            if !finite(pos) {
                return Err(CommandError::invalid_params("pos must be a finite number"));
            }
            GridOp::MoveGuide(view, index, pos)
        }
        "remove" => GridOp::RemoveGuide(view, param(params, "index")?),
        "clear" => GridOp::ClearGuides(view),
        other => return Err(CommandError::invalid_params(format!("action is add, move, remove or clear, not {other:?}"))),
    };
    send(doc, op)?;
    Ok(json!({ "view": view, "action": action }))
}

/// The grid commands.
#[must_use]
pub fn commands() -> Vec<CommandSpec<AppDoc>> {
    vec![
        CommandSpec::new("grid.set", "Grid and snapping", grid_set)
            .transient()
            .describe("Change what the Rig and Scene views show and snap to: show_grid, show_guides, show_rulers, lock_guides, size (grid spacing in stage pixels, 2 to 2000) and size_y (vertical spacing; 0 follows size), snap (handles and origins snap while dragged or placed), snap_to_grid, snap_to_guides, snap_pixels (catch distance on screen). Only the given fields change. Read the result with the grid_state query.")
            .with_schema(r#"{"type":"object","properties":{"show_grid":{"type":"boolean"},"show_guides":{"type":"boolean"},"show_rulers":{"type":"boolean"},"lock_guides":{"type":"boolean"},"size":{"type":"number"},"size_y":{"type":"number"},"snap":{"type":"boolean"},"snap_to_grid":{"type":"boolean"},"snap_to_guides":{"type":"boolean"},"snap_pixels":{"type":"number"}},"additionalProperties":false}"#),
        CommandSpec::new("grid.guide", "Guides", grid_guide)
            .transient()
            .describe("Edit the guides of a view (rig or scene, default rig): action add (axis horizontal|vertical, pos in stage pixels), move (index, pos), remove (index) or clear. Locked guides cannot be moved or removed. The same thing dragging out of a ruler does.")
            .with_schema(r#"{"type":"object","required":["action"],"properties":{"view":{"enum":["rig","scene"]},"action":{"enum":["add","move","remove","clear"]},"axis":{"enum":["horizontal","vertical"]},"pos":{"type":"number"},"index":{"type":"integer","minimum":0}},"additionalProperties":false}"#),
    ]
}

/// The `grid_state` query: settings and guides.
#[must_use]
pub fn state_json(state: &GridState) -> Value {
    let guides = |list: &[Guide]| -> Vec<Value> {
        list.iter().map(|g| json!({ "axis": if g.axis == Axis::Horizontal { "horizontal" } else { "vertical" }, "pos": g.pos })).collect()
    };
    json!({ "settings": state.settings, "guides": { "rig": guides(&state.rig), "scene": guides(&state.scene) } })
}

/// The `grid_snap` query: where a point lands.
///
/// # Errors
/// A message when the parameters are unusable.
pub fn snap_json(state: &GridState, params: &Value) -> Result<Value, CommandError> {
    let view = view_of(params)?;
    let (x, y): (f32, f32) = (param(params, "x")?, param(params, "y")?);
    let zoom: Option<f32> = param_opt(params, "zoom")?;
    let p = state.snap(view, pos2(x, y), zoom.unwrap_or(1.0));
    Ok(json!({ "x": p.x, "y": p.y, "snapped": (p.x - x).abs() > f32::EPSILON || (p.y - y).abs() > f32::EPSILON }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> GridState {
        let mut s = GridState::default();
        s.apply(GridOp::Set(GridPatch { snap: Some(true), size: Some(20.0), ..GridPatch::default() }));
        s
    }

    #[test]
    fn snapping_is_off_until_asked_for() {
        let s = GridState::default();
        assert_eq!(s.snap(GridView::Rig, pos2(19.0, 41.0), 1.0), pos2(19.0, 41.0));
    }

    #[test]
    fn points_near_a_grid_line_land_on_it_per_axis() {
        let s = on();
        assert_eq!(s.snap(GridView::Rig, pos2(19.0, 100.0), 1.0), pos2(20.0, 100.0));
        assert_eq!(s.snap(GridView::Rig, pos2(31.0, 43.0), 1.0), pos2(31.0, 40.0));
        // Zoomed in, the same screen distance is a smaller stage distance.
        assert_eq!(s.snap(GridView::Rig, pos2(18.5, 100.0), 8.0), pos2(18.5, 100.0));
        assert_eq!(s.snap(GridView::Rig, pos2(19.5, 100.0), 8.0), pos2(20.0, 100.0));
        // Zoomed out, a far point is caught.
        assert_eq!(s.snap(GridView::Rig, pos2(25.0, 100.0), 0.5), pos2(20.0, 100.0));
    }

    #[test]
    fn the_nearer_of_a_guide_and_a_grid_line_wins() {
        let mut s = on();
        s.apply(GridOp::AddGuide(GridView::Rig, Guide { axis: Axis::Vertical, pos: 33.0 }));
        assert_eq!(s.snap(GridView::Rig, pos2(32.0, 0.0), 1.0), pos2(33.0, 0.0));
        assert_eq!(s.snap(GridView::Rig, pos2(23.0, 0.0), 1.0), pos2(20.0, 0.0));
        // Guides are per view and per axis: a horizontal guide does not catch x.
        s.apply(GridOp::AddGuide(GridView::Rig, Guide { axis: Axis::Horizontal, pos: 7.0 }));
        assert_eq!(s.snap(GridView::Rig, pos2(11.0, 7.5), 1.0), pos2(11.0, 7.0));
        assert_eq!(s.snap(GridView::Rig, pos2(31.0, 0.0), 1.0), pos2(33.0, 0.0));
        assert_eq!(s.snap(GridView::Scene, pos2(31.0, 0.0), 1.0), pos2(31.0, 0.0), "the scene has no such guide and 31 is between grid lines");
    }

    #[test]
    fn grid_and_guide_snapping_switch_independently_and_y_has_its_own_spacing() {
        let mut s = on();
        s.apply(GridOp::Set(GridPatch { snap_to_grid: Some(false), ..GridPatch::default() }));
        assert_eq!(s.snap(GridView::Rig, pos2(19.0, 19.0), 1.0), pos2(19.0, 19.0));
        s.apply(GridOp::Set(GridPatch { snap_to_grid: Some(true), size_y: Some(50.0), ..GridPatch::default() }));
        assert_eq!(s.snap(GridView::Rig, pos2(19.0, 47.0), 1.0), pos2(20.0, 50.0));
    }

    #[test]
    fn locked_guides_stay_and_hostile_numbers_are_ignored() {
        let mut s = GridState::default();
        s.apply(GridOp::AddGuide(GridView::Scene, Guide { axis: Axis::Vertical, pos: 10.0 }));
        s.apply(GridOp::AddGuide(GridView::Scene, Guide { axis: Axis::Vertical, pos: f32::NAN }));
        assert_eq!(s.scene.len(), 1);
        s.apply(GridOp::Set(GridPatch { lock_guides: Some(true), ..GridPatch::default() }));
        s.apply(GridOp::MoveGuide(GridView::Scene, 0, 50.0));
        s.apply(GridOp::RemoveGuide(GridView::Scene, 0));
        assert_eq!(s.scene[0].pos, 10.0);
        s.apply(GridOp::Set(GridPatch { lock_guides: Some(false), size: Some(f32::NAN), ..GridPatch::default() }));
        assert!((s.settings.size - 20.0).abs() < f32::EPSILON, "NaN spacing falls back");
        s.apply(GridOp::Set(GridPatch { size: Some(1.0e9), ..GridPatch::default() }));
        assert!(s.settings.size <= MAX_SIZE);
        s.apply(GridOp::MoveGuide(GridView::Scene, 0, 50.0));
        assert_eq!(s.scene[0].pos, 50.0);
        s.apply(GridOp::ClearGuides(GridView::Scene));
        assert!(s.scene.is_empty());
        assert_eq!(s.snap(GridView::Rig, pos2(f32::NAN, 1.0), 1.0).y, 1.0);
    }

    #[test]
    fn settings_changes_are_reported_once_for_saving() {
        let mut s = GridState::default();
        assert!(s.take_prefs().is_none());
        s.apply(GridOp::Set(GridPatch { show_grid: Some(true), ..GridPatch::default() }));
        assert!(s.take_prefs().is_some_and(|p| p.show_grid));
        assert!(s.take_prefs().is_none());
        s.apply(GridOp::Set(GridPatch { show_grid: Some(true), ..GridPatch::default() }));
        assert!(s.take_prefs().is_none(), "nothing changed");
    }
}
