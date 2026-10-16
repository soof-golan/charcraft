//! The top of the window as in Character Animator.
//!
//! The menus sit on the left (File, Edit, Scene, Timeline, Window, Help, plus the app menu on macOS), the workspace
//! tabs are centred in large text, and the document title is on the right.

use anim_ui::theme::visual;
use egui::{Align, Align2, Button, Color32, FontId, Id, Layout, Rect, RichText, Sense, Ui, UiBuilder, vec2};
use serde_json::json;

use super::dock;
use super::theme::{ACCENT, HEADER, TEXT, TEXT_WEAK, semibold};
use crate::state::Mode;
use crate::view::{Action, Ctx, Prompt, View};

/// The platform's command key as written on menus.
const COMMAND: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };

fn item(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    let mut button = Button::new(label);
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut.replace("Mod+", &format!("{COMMAND}+")));
    }
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn enabled_item(ui: &mut Ui, enabled: bool, label: &str, shortcut: &str) -> bool {
    ui.add_enabled_ui(enabled, |ui| item(ui, label, shortcut)).inner
}

fn app_menu(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.menu_button("CharCraft", |ui| {
        if item(ui, "About CharCraft", "") {
            view.show_about = true;
        }
        ui.separator();
        let mut dark = view.dark;
        if ui.checkbox(&mut dark, "Dark theme").changed() {
            actions.push(Action::Dark(dark));
        }
        ui.separator();
        if item(ui, "Quit CharCraft", "Mod+Q") {
            actions.push(Action::Quit);
        }
    });
}

fn file_menu(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.menu_button("File", |ui| {
        super::file_menu(ui, view, ctx, actions);
        ui.separator();
        if item(ui, "Import Audio...", "") {
            view.prompt = Some((Prompt::Audio, super::default_dir()));
        }
    });
}

fn edit_menu(ui: &mut Ui, view: &View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.menu_button("Edit", |ui| {
        if enabled_item(ui, ctx.can_undo, "Undo", "Mod+Z") {
            actions.push(Action::Undo);
        }
        if enabled_item(ui, ctx.can_redo, "Redo", "Mod+Shift+Z") {
            actions.push(Action::Redo);
        }
        if !cfg!(target_os = "macos") {
            ui.separator();
            let mut dark = view.dark;
            if ui.checkbox(&mut dark, "Dark theme").changed() {
                actions.push(Action::Dark(dark));
            }
        }
    });
}

fn scene_menu(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.menu_button("Scene", |ui| {
        if item(ui, "Add Puppet to Scene", "") {
            let puppet = ctx.puppet.unwrap_or("demo");
            actions.push(Action::Run("track.add", json!({ "puppet": puppet, "name": format!("Track {}", ctx.doc.rec.scene.tracks.len().saturating_add(1)) })));
        }
        if enabled_item(ui, !ctx.doc.rec.scene.tracks.is_empty(), "Remove Selected Puppet", "") {
            actions.push(Action::Run("track.remove", json!({ "track": view.track })));
        }
        if item(ui, "Add Scene", "") {
            actions.push(Action::Run("scene.add", json!({ "puppet": ctx.puppet })));
        }
        if item(ui, "Duplicate Scene", "") {
            actions.push(Action::Run("scene.duplicate", json!({ "open": true })));
        }
        if item(ui, "Scene Settings...", "") {
            actions.push(Action::Mode(Mode::Record));
            view.docks.reveal(Mode::Record, dock::EDIT);
        }
        ui.separator();
        let calibrating = view.live.calibrating.is_some();
        if enabled_item(ui, view.live.tracking && !calibrating, "Set Rest Pose", "") {
            actions.push(Action::Tracking("tracking_calibrate", json!({ "frames": 30 })));
        }
        let mut mirror = view.mirror;
        if ui.checkbox(&mut mirror, "Mirror Camera").changed() {
            actions.push(Action::Mirror(mirror));
        }
        let mut stream_overlay = view.hud;
        if ui.checkbox(&mut stream_overlay, "Stream Overlay").changed() {
            view.hud = stream_overlay;
        }
    });
}

fn timeline_menu(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    ui.menu_button("Timeline", |ui| {
        let fps = ctx.doc.rec.scene.fps_f64().max(1.0);
        let last = f64::from(ctx.doc.rec.scene.frame_count().max(1).saturating_sub(1)) / fps;
        if item(ui, if view.playing { "Pause" } else { "Play" }, "Space") {
            actions.push(Action::Play(!view.playing));
        }
        let recording = view.recording.is_some();
        if item(ui, if recording { "Stop Recording" } else { "Record" }, "R") {
            actions.push(if recording { Action::StopRecord } else { Action::StartRecord });
        }
        ui.separator();
        if item(ui, "Go to Start", "Home") {
            actions.push(Action::Seek(0.0));
        }
        if item(ui, "Go to End", "End") {
            actions.push(Action::Seek(last));
        }
        if item(ui, "Step Back", "Left") {
            actions.push(Action::Seek((view.playhead - 1.0 / fps).max(0.0)));
        }
        if item(ui, "Step Forward", "Right") {
            actions.push(Action::Seek((view.playhead + 1.0 / fps).min(last)));
        }
        ui.separator();
        if item(ui, "Add Marker at Playhead", "") {
            actions.push(Action::Run("marker.add", json!({ "seconds": view.playhead.clamp(0.0, last) })));
        }
        let markers = &ctx.doc.rec.scene.markers;
        ui.add_enabled_ui(!markers.is_empty(), |ui| {
            ui.menu_button("Markers", |ui| {
                for (index, marker) in markers.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(format!("{}  {:.2}s", marker.name, marker.at.seconds())).clicked() {
                            actions.push(Action::Seek(marker.at.seconds()));
                            ui.close();
                        }
                        if ui.small_button("Remove").clicked() {
                            actions.push(Action::Run("marker.remove", json!({ "index": index })));
                            ui.close();
                        }
                    });
                }
                ui.separator();
                if ui.button("Remove All Markers").clicked() {
                    actions.push(Action::Run("marker.remove", json!({})));
                    ui.close();
                }
            });
        });
        ui.separator();
        ui.checkbox(&mut view.looping, "Loop Playback");
        ui.checkbox(&mut view.monitor, "Monitor Live Performance");
        ui.separator();
        if item(ui, "Import Audio...", "") {
            view.prompt = Some((Prompt::Audio, super::default_dir()));
        }
    });
}

fn window_menu(ui: &mut Ui, view: &mut View, actions: &mut Vec<Action>) {
    ui.menu_button("Window", |ui| {
        ui.menu_button("Workspace", |ui| {
            for mode in Mode::ALL {
                let label = mode.label();
                let chord = crate::workspace::chord_text(&view.prefs, &format!("mode.{}", label.to_lowercase()));
                if ui.add(Button::new(label).selected(view.mode == mode).shortcut_text(chord)).clicked() {
                    actions.push(Action::Mode(mode));
                    ui.close();
                }
            }
        });
        ui.separator();
        let mode = view.mode;
        if dock::panels(mode).is_empty() {
            ui.weak("The Start workspace has no panels");
        }
        for &panel in dock::panels(mode) {
            let mut shown = view.docks.contains(mode, panel);
            let protected = panel == dock::SCENE;
            if ui.add_enabled(!protected, egui::Checkbox::new(&mut shown, dock::title(mode, panel))).changed() {
                view.docks.toggle(mode, panel);
            }
        }
        ui.separator();
        let arranged = !dock::panels(mode).is_empty();
        if ui.add_enabled(arranged, Button::new("Save Changes to this Workspace")).clicked() {
            view.docks.save_workspace(mode);
            ui.close();
        }
        if ui.add_enabled(arranged, Button::new("Reset to Saved Layout")).clicked() {
            view.docks.reset(mode);
            ui.close();
        }
        if ui.add_enabled(arranged, Button::new("Reset to Default Layout")).clicked() {
            view.docks.reset_to_default(mode);
            ui.close();
        }
        ui.separator();
        super::workspace_presets(ui, view, actions);
    });
}

fn help_menu(ui: &mut Ui, view: &mut View) {
    ui.menu_button("Help", |ui| {
        if item(ui, "Keyboard Shortcuts", "") {
            view.show_shortcuts = true;
        }
        if !cfg!(target_os = "macos") && item(ui, "About CharCraft", "") {
            view.show_about = true;
        }
    });
}

/// The workspace tabs, centred in `rect`.
fn mode_tabs(ui: &Ui, rect: Rect, view: &View, actions: &mut Vec<Action>) {
    let font = FontId::new(15.0, semibold());
    let padding = 18.0;
    let galleys: Vec<_> = Mode::ALL.into_iter().map(|m| ui.painter().layout_no_wrap(m.label().to_owned(), font.clone(), TEXT)).collect();
    let total: f32 = galleys.iter().map(|g| g.size().x + padding * 2.0).sum();
    let mut x = rect.center().x - total / 2.0;
    for (mode, galley) in Mode::ALL.into_iter().zip(galleys) {
        let width = galley.size().x + padding * 2.0;
        let tab = Rect::from_min_size(egui::pos2(x, rect.top()), vec2(width, rect.height()));
        x += width;
        let response = ui.interact(tab, Id::new(("mode_tab", mode)), Sense::click());
        let active = view.mode == mode;
        let color = if active {
            Color32::WHITE
        } else if response.hovered() {
            TEXT
        } else {
            TEXT_WEAK
        };
        let text_pos = egui::pos2(tab.center().x - galley.size().x / 2.0, tab.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley, color);
        if active {
            let bar =
                Rect::from_min_max(egui::pos2(tab.left() + padding * 0.5, tab.bottom() - 3.0), egui::pos2(tab.right() - padding * 0.5, tab.bottom() - 1.0));
            ui.painter().rect_filled(bar, 1.0, ACCENT);
        }
        if response.on_hover_text(format!("{} workspace", mode.label())).clicked() {
            actions.push(Action::Mode(mode));
        }
    }
}

/// The menu bar, workspace tabs and document title.
pub fn top_bar(ui: &mut Ui, view: &mut View, ctx: &Ctx<'_>, actions: &mut Vec<Action>) {
    egui::Panel::top("top_bar").frame(egui::Frame::new().fill(HEADER).inner_margin(egui::Margin::symmetric(8, 3))).show(ui, |ui| {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let left = Rect::from_min_size(bar.min, vec2((bar.width() / 2.0 - 190.0).max(260.0).min(bar.width()), bar.height()));
        let mut menus = ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center)));
        egui::MenuBar::new().ui(&mut menus, |ui| {
            if cfg!(target_os = "macos") {
                app_menu(ui, view, actions);
            }
            file_menu(ui, view, ctx, actions);
            edit_menu(ui, view, ctx, actions);
            ui.menu_button("Puppet", |ui| super::puppet_menu(ui, view, ctx, actions));
            scene_menu(ui, view, ctx, actions);
            timeline_menu(ui, view, ctx, actions);
            window_menu(ui, view, actions);
            help_menu(ui, view);
        });
        mode_tabs(ui, bar, view, actions);
        let title = format!("{}{}", ctx.world.title, if ctx.dirty { " *" } else { "" });
        ui.painter().text(
            egui::pos2(bar.right() - 4.0, bar.center().y),
            Align2::RIGHT_CENTER,
            visual(&title).into_owned(),
            FontId::proportional(12.0),
            TEXT_WEAK,
        );
    });
}

/// The Keyboard Shortcuts and About windows.
pub fn windows(ui: &Ui, view: &mut View) {
    let mut open = view.show_shortcuts;
    if open {
        egui::Window::new("Keyboard Shortcuts").open(&mut open).collapsible(false).resizable(false).show(ui.ctx(), |ui| {
            egui::Grid::new("shortcuts").num_columns(2).spacing([24.0, 3.0]).show(ui, |ui| {
                let rows = [
                    ("Space", "Play or pause"),
                    ("R", "Start or stop recording a take"),
                    ("Home / End", "Go to the start or end of the scene"),
                    ("Left / Right", "Step one frame (Shift: ten frames)"),
                    ("Cmd/Ctrl+Z, +Shift+Z", "Undo, redo"),
                    ("Cmd/Ctrl+1 to 4", "Start, Rig, Record, Stream workspace"),
                    ("V H S P O M", "Rig tools: Select, Handle, Stick, Pin, Origin, Mesh"),
                    ("Delete", "Delete the selected handle"),
                    ("H (Stream)", "Hide or show the Stream overlay"),
                ];
                for (keys, what) in rows {
                    ui.label(RichText::new(keys).strong());
                    ui.label(what);
                    ui.end_row();
                }
            });
        });
    }
    view.show_shortcuts = open;
    let mut about = view.show_about;
    if about {
        egui::Window::new("About CharCraft").open(&mut about).collapsible(false).resizable(false).show(ui.ctx(), |ui| {
            ui.label(RichText::new("CharCraft").family(semibold()).size(18.0));
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("Live character animation, file-compatible with Adobe Character Animator puppets.");
            ui.add_space(6.0);
            ui.weak("Interface typeface: Inter, SIL Open Font License 1.1.");
        });
    }
    view.show_about = about;
}
