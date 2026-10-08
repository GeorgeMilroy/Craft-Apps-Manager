//! The look of getartcraft.com: warm paper and ink colours, square corners,
//! hairline rules, Archivo headings, Inter text and uppercase Geist Mono
//! labels. Colour values are the site's own CSS variables.

use crate::settings::ThemeChoice;
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Theme,
    ThemePreference, Vec2,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub raised: Color32,
    pub sunken: Color32,
    /// Row hover tint.
    pub hover: Color32,
    pub ink: Color32,
    pub strong: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub line: Color32,
    pub line_strong: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    pub invert_bg: Color32,
    pub invert_fg: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub update: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `c` at opacity `a`, premultiplied in sRGB space so it blends like CSS
/// colours do. (egui's `from_rgba_unmultiplied` premultiplies in linear
/// space, which makes faint tints far too bright.)
fn alpha(c: Color32, a: u8) -> Color32 {
    let k = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(k(c.r()), k(c.g()), k(c.b()), a)
}

impl Palette {
    pub fn light() -> Self {
        let ink = rgb(0x101014);
        Palette {
            dark: false,
            bg: rgb(0xf2f1ee),
            raised: rgb(0xfaf9f7),
            sunken: rgb(0xe9e8e4),
            hover: alpha(ink, 0x0a),
            ink,
            strong: Color32::BLACK,
            muted: alpha(ink, 0x9e),
            faint: alpha(ink, 0x6b),
            line: alpha(ink, 0x29),
            line_strong: alpha(ink, 0x66),
            accent: rgb(0x2d81ff),
            accent_ink: rgb(0x1659c4),
            invert_bg: ink,
            invert_fg: rgb(0xf2f1ee),
            danger: rgb(0xb3261e),
            success: rgb(0x007a54),
            update: rgb(0xb35f00),
        }
    }

    pub fn dark() -> Self {
        let ink = rgb(0xf2f1ee);
        Palette {
            dark: true,
            bg: rgb(0x121316),
            raised: rgb(0x101014),
            sunken: rgb(0x060607),
            hover: alpha(ink, 0x0c),
            ink,
            strong: Color32::WHITE,
            muted: alpha(ink, 0x99),
            faint: alpha(ink, 0x61),
            line: alpha(ink, 0x26),
            line_strong: alpha(ink, 0x66),
            accent: rgb(0x2d81ff),
            accent_ink: rgb(0x74aaff),
            invert_bg: Color32::WHITE,
            invert_fg: rgb(0x0b0b0c),
            danger: rgb(0xff6259),
            success: rgb(0x2fd39c),
            update: rgb(0xffb347),
        }
    }

    pub fn of(ctx: &egui::Context) -> Self {
        if ctx.style().visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

/// Headings and app names (Archivo).
pub fn display() -> FontFamily {
    FontFamily::Name("display".into())
}

/// Buttons and tabs (Geist Mono Bold).
pub fn mono_bold() -> FontFamily {
    FontFamily::Name("mono-bold".into())
}

/// The small uppercase labels the site calls "HUD labels" (Geist Mono).
pub fn hud() -> TextStyle {
    TextStyle::Name("hud".into())
}

pub fn preference(choice: ThemeChoice) -> ThemePreference {
    match choice {
        ThemeChoice::System => ThemePreference::System,
        ThemeChoice::Light => ThemePreference::Light,
        ThemeChoice::Dark => ThemePreference::Dark,
    }
}

pub fn install(ctx: &egui::Context, choice: ThemeChoice) {
    install_fonts(ctx);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
    ctx.set_theme(preference(choice));
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("inter", &include_bytes!("../assets/fonts/Inter-Regular.ttf")[..]),
        ("archivo", &include_bytes!("../assets/fonts/Archivo-SemiBold.ttf")[..]),
        ("geist-mono", &include_bytes!("../assets/fonts/GeistMono-Medium.ttf")[..]),
        ("geist-mono-bold", &include_bytes!("../assets/fonts/GeistMono-Bold.ttf")[..]),
    ] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    // egui's own fonts stay as fallbacks for symbols the site fonts lack.
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    let with_fallbacks = |first: &str| [vec![first.to_owned()], fallbacks.clone()].concat();
    fonts.families.insert(FontFamily::Proportional, with_fallbacks("inter"));
    fonts.families.insert(FontFamily::Monospace, with_fallbacks("geist-mono"));
    fonts.families.insert(display(), with_fallbacks("archivo"));
    fonts.families.insert(mono_bold(), with_fallbacks("geist-mono-bold"));
    ctx.set_fonts(fonts);
}

fn style(theme: Theme) -> egui::Style {
    let p = if theme == Theme::Dark { Palette::dark() } else { Palette::light() };
    let mut style = egui::Style::default();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, display())),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(10.5, mono_bold())),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(11.5, FontFamily::Monospace)),
        (hud(), FontId::new(10.0, FontFamily::Monospace)),
    ]
    .into();

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, 6.0);
    s.button_padding = Vec2::new(12.0, 5.0);
    s.interact_size.y = 26.0;
    s.menu_margin = Margin::same(4);
    s.window_margin = Margin::same(16);

    let mut v = if theme == Theme::Dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = if p.dark { rgb(0x18191d) } else { p.raised };
    v.extreme_bg_color = p.sunken;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.sunken;
    v.window_stroke = Stroke::new(1.0_f32, p.line_strong);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = p.accent_ink;
    v.error_fg_color = p.danger;
    v.warn_fg_color = p.update;
    v.indent_has_left_vline = false;
    v.text_cursor.stroke = Stroke::new(2.0_f32, p.accent);

    let w = &mut v.widgets;
    // Default buttons are the site's outlined secondary buttons.
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.inactive.bg_fill = p.sunken;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.line_strong);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.ink);
    w.hovered.bg_fill = p.sunken;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.ink);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, p.strong);
    w.active.bg_fill = p.line;
    w.active.weak_bg_fill = p.line;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.strong);
    w.active.fg_stroke = Stroke::new(2.0_f32, p.strong);
    w.open = w.hovered;
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = CornerRadius::ZERO;
        state.expansion = 0.0;
    }
    style.visuals = v;
    style
}
