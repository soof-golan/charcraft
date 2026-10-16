//! The docked arrangement of each workspace.
//!
//! Default layouts, which panels a workspace offers, and the layouts the user changed (kept per workspace in the
//! config folder, with "Save Changes to this Workspace" and "Reset to Saved Layout" as in Character Animator).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anim_ui::dock::PanelId;
use anim_ui::dock::docking::{Action, DockStyle, Layout, Node};
use craft_ui::layout::{SplitAxis, SplitSize};
use egui::{FontId, Stroke, Ui};

use super::theme::{ACCENT, DIVIDER, HEADER, PANEL, TEXT, TEXT_WEAK};
use crate::state::Mode;

/// The Project panel (puppets, scenes and sounds).
pub const PROJECT: PanelId = PanelId::Library;
/// The Scene panel (Record, Stream) or the puppet canvas (Rig).
pub const SCENE: PanelId = PanelId::Stage;
/// Camera & Microphone.
pub const CAMERA: PanelId = PanelId::Custom(1);
/// Controls: the trigger buttons.
pub const CONTROLS: PanelId = PanelId::Custom(2);
/// Triggers: where keys and swap sets are assigned.
pub const TRIGGERS: PanelId = PanelId::Custom(3);
/// Scene, track placement, takes, timing, keyframes and audio.
pub const EDIT: PanelId = PanelId::Custom(4);
/// The Puppet panel: the layer tree of the puppet being rigged.
pub const PUPPET: PanelId = PanelId::Custom(5);
/// Stream Output: NDI, virtual camera and web source.
pub const OUTPUT: PanelId = PanelId::Output;
/// Scenes of the stream view.
pub const SCENES: PanelId = PanelId::Custom(6);
/// Properties.
pub const PROPERTIES: PanelId = PanelId::Properties;
/// Timeline.
pub const TIMELINE: PanelId = PanelId::Timeline;

/// The panels a workspace can show, in the order of the Window menu.
#[must_use]
pub const fn panels(mode: Mode) -> &'static [PanelId] {
    match mode {
        Mode::Start => &[],
        Mode::Rig => &[PUPPET, SCENE, PROPERTIES, TRIGGERS, PROJECT, CAMERA],
        Mode::Record => &[PROJECT, SCENE, CAMERA, PROPERTIES, EDIT, TIMELINE, CONTROLS, TRIGGERS],
        Mode::Stream => &[SCENE, CAMERA, CONTROLS, TRIGGERS, OUTPUT, SCENES, TIMELINE, PROJECT],
    }
}

/// The tab title of a panel in a workspace.
#[must_use]
pub const fn title(mode: Mode, panel: PanelId) -> &'static str {
    match panel {
        PanelId::Library => "Project",
        PanelId::Stage => match mode {
            Mode::Rig => "Puppet Canvas",
            _ => "Scene",
        },
        PanelId::Properties => "Properties",
        PanelId::Timeline => "Timeline",
        PanelId::Custom(1) => "Camera & Microphone",
        PanelId::Custom(2) => "Controls",
        PanelId::Custom(3) => "Triggers",
        PanelId::Custom(4) => "Scene & Takes",
        PanelId::Custom(5) => "Puppet",
        PanelId::Tools => "Tools",
        PanelId::Output => "Stream Output",
        PanelId::Custom(6) => "Scenes",
        PanelId::Custom(_) => "Panel",
    }
}

fn tabs(panels: &[PanelId]) -> Node<PanelId> {
    Node::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(axis: SplitAxis, size: SplitSize, first: Node<PanelId>, second: Node<PanelId>) -> Node<PanelId> {
    Node::Split { axis, size, first: Box::new(first), second: Box::new(second) }
}

/// The arrangement a workspace starts with.
#[must_use]
pub fn default_layout(mode: Mode) -> Layout<PanelId> {
    use SplitAxis::{Horizontal as Side, Vertical as Stack};
    let root = match mode {
        // Project left, Scene centre, Camera & Microphone over Properties on the right, Timeline across the bottom.
        Mode::Record | Mode::Start => {
            let right = split(Stack, SplitSize::Ratio(0.46), tabs(&[CAMERA]), tabs(&[PROPERTIES, EDIT]));
            let upper = split(Side, SplitSize::FixedSecond(300.0), tabs(&[SCENE]), right);
            let upper = split(Side, SplitSize::FixedFirst(240.0), tabs(&[PROJECT]), upper);
            split(Stack, SplitSize::FixedSecond(260.0), upper, tabs(&[TIMELINE]))
        }
        // Puppet tree left, the canvas centre, Properties over Triggers on the right.
        Mode::Rig => {
            let right = split(Stack, SplitSize::Ratio(0.64), tabs(&[PROPERTIES]), tabs(&[TRIGGERS]));
            let centre = split(Side, SplitSize::FixedSecond(320.0), tabs(&[SCENE]), right);
            split(Side, SplitSize::FixedFirst(250.0), tabs(&[PUPPET]), centre)
        }
        // The Scene as large as it goes, the live inputs and the Controls beside it, a thin Timeline underneath.
        Mode::Stream => {
            let side = split(Stack, SplitSize::Ratio(0.55), tabs(&[CAMERA]), tabs(&[CONTROLS, TRIGGERS, OUTPUT, SCENES]));
            let main = split(Stack, SplitSize::FixedSecond(150.0), tabs(&[SCENE]), tabs(&[TIMELINE]));
            split(Side, SplitSize::FixedSecond(280.0), main, side)
        }
    };
    Layout { root: Some(root), floating: Vec::new() }
}

/// The dock chrome in the Character Animator palette.
#[must_use]
pub fn style(ui: &Ui) -> DockStyle {
    if !ui.visuals().dark_mode {
        return anim_ui::dock::dock_style(ui);
    }
    let mut style = DockStyle::from_ui(ui);
    style.tab_height = 22.0;
    style.gap = 1.0;
    style.min_pane = 56.0;
    style.background = PANEL;
    style.tab_background = HEADER;
    style.active_background = PANEL;
    style.text = TEXT;
    style.inactive_text = TEXT_WEAK;
    style.border = Stroke::new(1.0, DIVIDER);
    style.accent = ACCENT;
    style.font = FontId::proportional(12.0);
    style
}

/// The layouts of the workspaces, loaded on first use.
#[derive(Debug, Default)]
pub struct Docks {
    layouts: BTreeMap<Mode, Layout<PanelId>>,
    /// Read and write layout files (off for screenshots and tests so they never see or change the user's).
    persist: bool,
    /// The message of the last file problem.
    pub problem: Option<String>,
}

fn folder() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("charcraft").join("layouts"))
}

fn file(mode: Mode, saved: bool) -> Option<PathBuf> {
    folder().map(|d| d.join(format!("{}{}.json", if saved { "saved-" } else { "" }, mode.label().to_lowercase())))
}

impl Docks {
    /// Layouts that are read from and written to the config folder when `persist` is set.
    #[must_use]
    pub const fn new(persist: bool) -> Self {
        Self { layouts: BTreeMap::new(), persist, problem: None }
    }

    fn read(mode: Mode, saved: bool) -> Option<Layout<PanelId>> {
        let text = std::fs::read_to_string(file(mode, saved)?).ok()?;
        anim_ui::dock::layout_from_json(&text).ok()
    }

    fn write(&mut self, mode: Mode, saved: bool, layout: &Layout<PanelId>) {
        if !self.persist {
            return;
        }
        let Some(path) = file(mode, saved) else { return };
        let result = anim_ui::dock::layout_to_json(layout).map_err(|e| e.to_string()).and_then(|json| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, json).map_err(|e| e.to_string())
        });
        self.problem = result.err().map(|e| format!("layout {}: {e}", path.display()));
    }

    /// The layout of a workspace, taken out for drawing (give it back with [`Self::put`]).
    pub fn take(&mut self, mode: Mode) -> Layout<PanelId> {
        self.layouts.remove(&mode).unwrap_or_else(|| {
            let loaded = if self.persist { Self::read(mode, false).or_else(|| Self::read(mode, true)) } else { None };
            loaded.unwrap_or_else(|| default_layout(mode))
        })
    }

    /// Gives a layout back; `changed` writes it to the config folder.
    pub fn put(&mut self, mode: Mode, layout: Layout<PanelId>, changed: bool) {
        if changed {
            self.write(mode, false, &layout);
        }
        self.layouts.insert(mode, layout);
    }

    /// Whether the workspace shows the panel now.
    pub fn contains(&mut self, mode: Mode, panel: PanelId) -> bool {
        let layout = self.take(mode);
        let has = layout.contains(&panel);
        self.layouts.insert(mode, layout);
        has
    }

    /// Shows a hidden panel beside a panel that is open, or hides a shown one.
    pub fn toggle(&mut self, mode: Mode, panel: PanelId) {
        let mut layout = self.take(mode);
        let action = if layout.contains(&panel) {
            Action::Close { panel }
        } else {
            let anchor = [PROPERTIES, TIMELINE, SCENE, CAMERA].into_iter().find(|a| *a != panel && layout.contains(a));
            Action::Open { panel, anchor }
        };
        let _ = layout.apply_with_permissions(action, |p| p.permissions());
        self.put(mode, layout, true);
    }

    /// Makes `panel` the visible tab of its group (opening it first when hidden).
    pub fn reveal(&mut self, mode: Mode, panel: PanelId) {
        if !self.contains(mode, panel) {
            self.toggle(mode, panel);
        }
        let mut layout = self.take(mode);
        let _ = layout.apply_with_permissions(Action::Activate { panel }, |p| p.permissions());
        self.put(mode, layout, false);
    }

    /// Keeps the current arrangement as the one "Reset to Saved Layout" returns to.
    pub fn save_workspace(&mut self, mode: Mode) {
        let layout = self.take(mode);
        self.write(mode, true, &layout);
        self.put(mode, layout, false);
    }

    /// Goes back to the saved arrangement (the built-in one when none was saved).
    pub fn reset(&mut self, mode: Mode) {
        let layout = if self.persist { Self::read(mode, true) } else { None }.unwrap_or_else(|| default_layout(mode));
        self.put(mode, layout, true);
    }

    /// Replaces the arrangement with the built-in one and forgets the saved one.
    pub fn reset_to_default(&mut self, mode: Mode) {
        if self.persist
            && let Some(path) = file(mode, true)
        {
            let _ = std::fs::remove_file(path);
        }
        self.put(mode, default_layout(mode), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_layout_is_valid_and_has_the_scene() {
        for mode in Mode::ALL {
            let layout = default_layout(mode);
            assert_eq!(layout.validate(), Ok(()), "{mode:?}");
            assert!(layout.contains(&SCENE), "{mode:?}");
            for panel in layout.panels() {
                assert!(panels(mode).contains(panel) || mode == Mode::Start, "{mode:?} {panel:?}");
            }
        }
    }

    #[test]
    fn hidden_panels_open_and_close_again() {
        let mut docks = Docks::new(false);
        assert!(!docks.contains(Mode::Record, CONTROLS));
        docks.toggle(Mode::Record, CONTROLS);
        assert!(docks.contains(Mode::Record, CONTROLS));
        docks.toggle(Mode::Record, CONTROLS);
        assert!(!docks.contains(Mode::Record, CONTROLS));
    }

    #[test]
    fn reset_returns_to_the_built_in_layout() {
        let mut docks = Docks::new(false);
        docks.toggle(Mode::Rig, CAMERA);
        docks.reset(Mode::Rig);
        assert!(!docks.contains(Mode::Rig, CAMERA));
    }
}
