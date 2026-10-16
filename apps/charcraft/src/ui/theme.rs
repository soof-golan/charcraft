//! The look of the window: a dark grey palette with blue selection and red arming, compact 12 pt text in Inter and
//! tight spacing, laid over the shared `anim-ui` theme.
//!
//! Inter (SIL OFL 1.1, see `ATTRIBUTION.md`) has no Hebrew or Arabic glyphs; system fonts that do are appended as
//! fallbacks, so right-to-left names still render (the text is reordered with `anim_ui::theme::visual`).

use egui::{Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals, vec2};

/// The window and panel background.
pub const PANEL: Color32 = Color32::from_rgb(0x23, 0x23, 0x23);
/// Panel headers, tab strips and rulers: slightly lighter than the panels.
pub const HEADER: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
/// Wells, fields and the pasteboard.
pub const WELL: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1b);
/// Dividers.
pub const DIVIDER: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xd2, 0xd2, 0xd2);
/// Secondary text.
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x8c, 0x8c, 0x8c);
/// The selection and focus blue.
pub const ACCENT: Color32 = Color32::from_rgb(0x3b, 0x8e, 0xea);
/// Selected rows.
pub const SELECTED: Color32 = Color32::from_rgb(0x2a, 0x5a, 0x96);
/// Recording and arming.
pub const RED: Color32 = Color32::from_rgb(0xe6, 0x3a, 0x35);

/// The font family of semi-bold text (panel titles, mode tabs).
#[must_use]
pub fn semibold() -> FontFamily {
    FontFamily::Name("Inter-SemiBold".into())
}

/// Candidate system fonts that cover Hebrew and Arabic, per platform. Every readable file is appended.
const FALLBACKS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/System/Library/Fonts/ArialHB.ttc",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/SFHebrew.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
];

/// Installs Inter first and the system fallbacks after it. Returns how many fallbacks were found (zero: Hebrew and
/// Arabic cannot render).
#[must_use = "zero means no system font covers Hebrew or Arabic"]
pub fn install_fonts(ctx: &Context) -> usize {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("Inter".to_owned(), FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf")).into());
    fonts.font_data.insert("Inter-SemiBold".to_owned(), FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")).into());
    let mut fallbacks = Vec::new();
    for path in FALLBACKS {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = format!("charcraft-fallback-{path}");
        fonts.font_data.insert(name.clone(), FontData::from_owned(bytes).into());
        fallbacks.push(name);
    }
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter".to_owned());
    proportional.extend(fallbacks.iter().cloned());
    fonts.families.entry(FontFamily::Monospace).or_default().extend(fallbacks.iter().cloned());
    let mut bold = vec!["Inter-SemiBold".to_owned(), "Inter".to_owned()];
    bold.extend(fallbacks.iter().cloned());
    fonts.families.insert(semibold(), bold);
    ctx.set_fonts(fonts);
    fallbacks.len()
}

fn dark_visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = HEADER;
    v.window_stroke = Stroke::new(1.0, DIVIDER);
    v.extreme_bg_color = WELL;
    v.faint_bg_color = Color32::from_rgb(0x28, 0x28, 0x28);
    v.code_bg_color = WELL;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = SELECTED;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.warn_fg_color = Color32::from_rgb(0xe8, 0xb0, 0x3c);
    v.error_fg_color = RED;
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(3);
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(120) };
    let radius = CornerRadius::same(2);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, DIVIDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.bg_fill = Color32::from_rgb(0x3a, 0x3a, 0x3a);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x38, 0x38, 0x38);
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2a, 0x2a));
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_fill = Color32::from_rgb(0x4a, 0x4a, 0x4a);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x46, 0x46, 0x46);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x5a, 0x5a, 0x5a));
    w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.active.bg_fill = SELECTED;
    w.active.weak_bg_fill = SELECTED;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.open.bg_fill = Color32::from_rgb(0x44, 0x44, 0x44);
    w.open.weak_bg_fill = Color32::from_rgb(0x44, 0x44, 0x44);
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = radius;
    }
    v
}

/// Installs the theme: the CA-style dark palette, or the shared light one, with compact text and spacing.
pub fn apply(ctx: &Context, dark: bool) {
    if dark {
        ctx.set_visuals(dark_visuals());
    } else {
        anim_ui::Theme::Light.apply(ctx);
    }
    ctx.global_style_mut(|style| {
        let text = [
            (TextStyle::Heading, FontId::new(14.0, semibold())),
            (TextStyle::Body, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(10.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(11.0, FontFamily::Monospace)),
        ];
        for (id, font) in text {
            style.text_styles.insert(id, font);
        }
        let s = &mut style.spacing;
        s.item_spacing = vec2(6.0, 3.0);
        s.button_padding = vec2(7.0, 2.0);
        s.interact_size = vec2(34.0, 19.0);
        s.window_margin = egui::Margin::same(8);
        s.menu_margin = egui::Margin::same(4);
        s.indent = 12.0;
        s.slider_width = 110.0;
        s.combo_width = 90.0;
        s.scroll.bar_width = 8.0;
        s.scroll.floating = false;
    });
}
