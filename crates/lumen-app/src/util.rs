use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use lumen_core::{AudioBand, AudioBands, Frame, Rgba};

pub fn to_color(p: Rgba) -> Color32 {
    let [r, g, b] = p.to_u8();
    Color32::from_rgb(r, g, b)
}

pub fn band_color(b: AudioBand, k: f32) -> Color32 {
    let [r, g, bl] = b.color();
    Color32::from_rgb(
        (r * 255.0 * k) as u8,
        (g * 255.0 * k) as u8,
        (bl * 255.0 * k) as u8,
    )
}

pub fn to_screen(rect: Rect, u: f32, v: f32) -> Pos2 {
    rect.min + Vec2::new(u * rect.width(), v * rect.height())
}

pub fn to_canvas(rect: Rect, p: Pos2) -> (f32, f32) {
    (
        (p.x - rect.min.x) / rect.width(),
        (p.y - rect.min.y) / rect.height(),
    )
}

pub fn paint_frame(painter: &egui::Painter, r: Rect, frame: &Frame) {
    let cw = r.width() / frame.width as f32;
    let ch = r.height() / frame.height as f32;
    for y in 0..frame.height {
        for x in 0..frame.width {
            let min = r.min + Vec2::new(x as f32 * cw, y as f32 * ch);
            painter.rect_filled(
                Rect::from_min_size(min, Vec2::new(cw + 0.6, ch + 0.6)),
                0.0,
                to_color(frame.get(x, y)),
            );
        }
    }
}

/// Three little lights that flash on kick / snare / hat.
pub fn hit_dots(ui: &mut egui::Ui, bands: &AudioBands) {
    for b in AudioBand::HITS {
        let v = bands.get(b);
        let (r, p) = ui.allocate_painter(Vec2::splat(14.0), egui::Sense::hover());
        p.circle_filled(r.rect.center(), 5.0, band_color(b, 0.2 + 0.8 * v));
        r.on_hover_text(b.name());
    }
}

pub fn band_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    band: &mut AudioBand,
    only_hits: bool,
) {
    let list: &[AudioBand] = if only_hits {
        &AudioBand::HITS
    } else {
        &AudioBand::ALL
    };
    egui::ComboBox::from_id_salt(id)
        .width(90.0)
        .selected_text(band.name())
        .show_ui(ui, |ui| {
            for &b in list {
                ui.selectable_value(band, b, b.name());
            }
        });
}

/// Four dots showing the position in the bar; the current beat is lit.
pub fn beat_dots(ui: &mut egui::Ui, beat: f64) {
    let t = crate::theme::current();
    let b = beat.max(0.0);
    let idx = (b.floor() as i64).rem_euclid(4) as usize;
    let fade = 1.0 - b.fract() as f32;
    for i in 0..4 {
        let (r, p) = ui.allocate_painter(Vec2::new(12.0, 16.0), egui::Sense::hover());
        let lit = if i == 0 { t.accent } else { t.text };
        let k = if i == idx { 0.35 + 0.65 * fade } else { 0.0 };
        p.circle_filled(r.rect.center(), if i == 0 { 4.5 } else { 3.5 }, crate::theme::mix(t.raised, lit, k));
    }
}
