//! The Preferences window: general settings, appearance and the keyboard shortcut editor.

use egui::{RichText, Ui};

use crate::view::{Action, View, WorkspaceAction};
use crate::workspace::{SHORTCUTS, chord_text, clashes, from_press};

/// Which page of the window is showing (kept in egui's memory, not in the settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    General,
    Appearance,
    Shortcuts,
}

/// The window, when it is open.
pub fn show(ui: &Ui, view: &mut View, actions: &mut Vec<Action>) {
    if !view.prefs_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let mut open = true;
    let mut prefs = view.prefs.clone();
    let page_id = egui::Id::new("prefs_page");
    let mut page: Page = ctx.data(|d| d.get_temp(page_id)).unwrap_or_default();
    egui::Window::new("Preferences").collapsible(false).resizable(true).default_width(540.0).open(&mut open).show(&ctx, |ui| {
        ui.horizontal(|ui| {
            for (p, label) in [(Page::General, "General"), (Page::Appearance, "Appearance"), (Page::Shortcuts, "Shortcuts")] {
                if ui.selectable_label(page == p, label).clicked() {
                    page = p;
                }
            }
        });
        ui.separator();
        match page {
            Page::General => general(ui, &mut prefs),
            Page::Appearance => appearance(ui, &mut prefs),
            Page::Shortcuts => shortcuts(ui, view, &mut prefs),
        }
    });
    ctx.data_mut(|d| d.insert_temp(page_id, page));
    if !open {
        view.prefs_open = false;
        view.capturing = None;
    }
    if prefs != view.prefs {
        view.prefs = prefs.clone();
        if prefs.dark != view.dark {
            actions.push(Action::Dark(prefs.dark));
        }
        actions.push(Action::Workspace(WorkspaceAction::SetPrefs(Box::new(prefs))));
    }
}

fn general(ui: &mut Ui, prefs: &mut crate::workspace::Prefs) {
    egui::Grid::new("prefs_general").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
        ui.label("Autosave every");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut prefs.common.autosave_minutes).range(0..=240).suffix(" min"));
            ui.weak("0 turns autosave off");
        });
        ui.end_row();
        ui.label("Undo steps");
        ui.add(egui::DragValue::new(&mut prefs.common.history.max_steps).range(10..=2000));
        ui.end_row();
        ui.label("Undo memory");
        ui.add(egui::DragValue::new(&mut prefs.common.history.max_megabytes).range(16..=8192).suffix(" MB"));
        ui.end_row();
        ui.label("Recent files kept");
        ui.add(egui::DragValue::new(&mut prefs.common.recent.limit).range(0..=50));
        ui.end_row();
        ui.label("Start");
        ui.checkbox(&mut prefs.start_screen, "Show the start workspace when no file is given");
        ui.end_row();
        ui.label("Performance");
        ui.vertical(|ui| {
            ui.checkbox(&mut prefs.mirror, "Mirror the performer by default");
            ui.checkbox(&mut prefs.looping, "Loop playback by default");
        });
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.weak("Undo limits and the performance defaults apply the next time the app starts.");
}

fn appearance(ui: &mut Ui, prefs: &mut crate::workspace::Prefs) {
    ui.checkbox(&mut prefs.dark, "Dark theme");
    ui.horizontal(|ui| {
        ui.label("Interface size");
        ui.add(egui::Slider::new(&mut prefs.zoom, 0.8..=1.6).step_by(0.05).fixed_decimals(2));
        if ui.small_button("Reset").clicked() {
            prefs.zoom = 1.0;
        }
    });
}

fn shortcuts(ui: &mut Ui, view: &mut View, prefs: &mut crate::workspace::Prefs) {
    let ctx = ui.ctx().clone();
    if let Some(id) = view.capturing {
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, repeat: false, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, modifiers)) = pressed {
            if key != egui::Key::Escape {
                prefs.shortcuts.insert(id.to_owned(), from_press(key, modifiers).to_string());
            }
            view.capturing = None;
        }
    }
    let clashing = clashes(prefs);
    if !clashing.is_empty() {
        let names: Vec<String> = clashing
            .iter()
            .filter_map(|(a, b)| Some(format!("{} and {}", SHORTCUTS.iter().find(|d| d.id == *a)?.label, SHORTCUTS.iter().find(|d| d.id == *b)?.label)))
            .collect();
        ui.colored_label(egui::Color32::from_rgb(0xe8, 0xb0, 0x3c), format!("Same shortcut: {}", names.join("; ")));
    }
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        let mut group = "";
        for def in SHORTCUTS {
            if def.group != group {
                group = def.group;
                ui.add_space(4.0);
                ui.label(RichText::new(group).strong());
            }
            ui.horizontal(|ui| {
                ui.add_sized([170.0, 20.0], egui::Label::new(def.label));
                let waiting = view.capturing == Some(def.id);
                let text = if waiting {
                    "press a key...".to_owned()
                } else {
                    let text = chord_text(prefs, def.id);
                    if text.is_empty() { "none".to_owned() } else { text }
                };
                if ui
                    .add_sized([130.0, 20.0], egui::Button::new(text).selected(waiting))
                    .on_hover_text("Click, then press the new shortcut (Esc cancels)")
                    .clicked()
                {
                    view.capturing = if waiting { None } else { Some(def.id) };
                }
                if ui.small_button("Clear").clicked() {
                    prefs.shortcuts.insert(def.id.to_owned(), String::new());
                }
                if ui.small_button("Reset").clicked() {
                    prefs.shortcuts.remove(def.id);
                }
            });
        }
    });
    if ui.button("Reset all shortcuts").clicked() {
        prefs.shortcuts.clear();
    }
}
