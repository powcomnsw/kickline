//! Kickline's own look: always dark (it's used in dark rooms), a legible font
//! bundled with the app, and one accent per theme instead of egui's stock blue.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub note: &'static str,
    /// Window background.
    pub bg: Color32,
    /// Panels, cards.
    pub panel: Color32,
    /// Buttons and inputs at rest.
    pub raised: Color32,
    pub hover: Color32,
    pub line: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    /// Text on top of the accent colour.
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    /// Per-layer identity colours.
    pub layers: [Color32; 5],
    pub radius: u8,
    pub text_size: f32,
}

const fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const PRESETS: [Theme; 3] = [
    Theme {
        name: "Tungsten",
        note: "Warm black, amber like a lighting desk's work lights",
        bg: c(0x14110E),
        panel: c(0x1C1814),
        raised: c(0x2A241E),
        hover: c(0x3A3129),
        line: c(0x3A332B),
        text: c(0xEEE6D8),
        weak: c(0x9E9283),
        accent: c(0xF0A030),
        on_accent: c(0x1A1207),
        ok: c(0x7CC47A),
        warn: c(0xF0C040),
        err: c(0xF0604E),
        layers: [c(0xF0A030), c(0xE86A84), c(0x9D85F0), c(0x6CC08E), c(0x5E9EEA)],
        radius: 6,
        text_size: 14.5,
    },
    Theme {
        name: "Gel",
        note: "Neutral black with a magenta gel accent",
        bg: c(0x0F0F11),
        panel: c(0x17171A),
        raised: c(0x24242A),
        hover: c(0x33333B),
        line: c(0x2E2E35),
        text: c(0xF1F0F3),
        weak: c(0x8F8E98),
        accent: c(0xE23D8E),
        on_accent: c(0xFFFFFF),
        ok: c(0x6CC98A),
        warn: c(0xF2B84B),
        err: c(0xFF5A4F),
        layers: [c(0xE23D8E), c(0xF2994A), c(0x8F7BF5), c(0x45C3A0), c(0x6AA6FF)],
        radius: 10,
        text_size: 14.5,
    },
    Theme {
        name: "Stage",
        note: "Pure black, white and console yellow; biggest text, for the booth",
        bg: c(0x000000),
        panel: c(0x0C0C0C),
        raised: c(0x1C1C1C),
        hover: c(0x2C2C2C),
        line: c(0x333333),
        text: c(0xFFFFFF),
        weak: c(0xA8A8A8),
        accent: c(0xFFD21F),
        on_accent: c(0x000000),
        ok: c(0x4FE07A),
        warn: c(0xFFB020),
        err: c(0xFF4040),
        layers: [c(0xFFD21F), c(0xFF5C8A), c(0xA98CFF), c(0x4FE07A), c(0x5AB0FF)],
        radius: 3,
        text_size: 16.0,
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn current() -> Theme {
    PRESETS[CURRENT.load(Ordering::Relaxed).min(PRESETS.len() - 1)]
}

pub fn index() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

pub fn set(ctx: &egui::Context, i: usize) {
    CURRENT.store(i.min(PRESETS.len() - 1), Ordering::Relaxed);
    apply(ctx);
}

fn settings_file() -> std::path::PathBuf {
    crate::paths::data_dir().join("theme.txt")
}

/// The theme chosen last time (by name), if any.
pub fn saved() -> Option<usize> {
    let name = std::fs::read_to_string(settings_file()).ok()?;
    PRESETS.iter().position(|t| t.name == name.trim())
}

/// Switch theme and remember it for next launch.
pub fn choose(ctx: &egui::Context, i: usize) {
    set(ctx, i);
    let _ = std::fs::write(settings_file(), current().name);
}

/// Atkinson Hyperlegible (designed for low-vision legibility) for text,
/// IBM Plex Mono for numbers; egui's own fonts stay as fallbacks for symbols.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "atkinson".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/AtkinsonHyperlegible-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "atkinson-bold".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/AtkinsonHyperlegible-Bold.ttf"))),
    );
    fonts.font_data.insert("plex-mono".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"))));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "atkinson".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "plex-mono".into());
    let mut bold = vec!["atkinson-bold".to_string()];
    bold.extend(fonts.families[&FontFamily::Proportional].iter().skip(1).cloned());
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn apply(ctx: &egui::Context) {
    let t = current();
    ctx.set_theme(egui::ThemePreference::Dark);
    let r = CornerRadius::same(t.radius);
    let mut v = egui::Visuals::dark();
    v.override_text_color = None;
    v.panel_fill = t.bg;
    v.window_fill = t.panel;
    v.window_stroke = Stroke::new(1.0, t.line);
    v.window_corner_radius = CornerRadius::same(t.radius + 4);
    v.menu_corner_radius = r;
    v.extreme_bg_color = t.bg;
    v.faint_bg_color = t.panel;
    v.code_bg_color = t.raised;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.err;
    v.hyperlink_color = t.accent;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.0, t.on_accent);
    v.slider_trailing_fill = true;
    v.striped = true;
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.panel;
    w.noninteractive.weak_bg_fill = t.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.noninteractive.corner_radius = r;
    for (wv, fill, fg) in [
        (&mut w.inactive, t.raised, t.text),
        (&mut w.hovered, t.hover, t.text),
        // Pressed. egui also uses this text colour for bold/strong labels, so it must be readable on the panel.
        (&mut w.active, t.hover, t.text),
        (&mut w.open, t.hover, t.text),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::NONE;
        wv.fg_stroke = Stroke::new(1.5, fg);
        wv.corner_radius = r;
        wv.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, t.line);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        let size = t.text_size;
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(size - 3.0)),
            (TextStyle::Body, FontId::proportional(size)),
            (TextStyle::Button, FontId::proportional(size)),
            (TextStyle::Monospace, FontId::monospace(size - 1.5)),
            (TextStyle::Heading, bold(size + 5.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = size + 10.0;
        s.spacing.window_margin = egui::Margin::same(12);
    });
}

/// A colour at a fraction of its brightness, keeping hue (for dim/lit indicators).
pub fn dim(c: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    Color32::from_rgb((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8)
}

/// Blend toward the background (for tinted fills).
pub fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
