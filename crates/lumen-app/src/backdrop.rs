//! A photo of the rig behind the stage: load it, straighten it by its four
//! corners, and show it under the fixtures so they can be placed against the
//! real thing. The photo is only a guide; nothing is recognised from it.

use crate::{theme, View};
use eframe::egui::{self, Color32, RichText, Vec2};
use image::RgbaImage;
use lumen_fixtures::{full_photo, Backdrop};
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub struct State {
    /// The photo as loaded (and shrunk), with the path it came from.
    src: Option<(String, RgbaImage)>,
    /// The straightened picture on the GPU, with what it was made from.
    tex: Option<(u64, egui::TextureHandle)>,
    /// Dragging the corners over the photo.
    pub editing: bool,
    drag: Option<usize>,
    /// The photo as taken, for the corner editor.
    plain: Option<(String, egui::TextureHandle)>,
    pub error: Option<String>,
}

const MAX_SIDE: u32 = 1600;
const OUT_W: u32 = 1024;

/// Read a photo and shrink it so it stays light.
pub fn load(path: &str) -> Result<RgbaImage, String> {
    let img = image::open(path).map_err(|e| format!("Could not open that photo: {e}"))?;
    Ok(img.thumbnail(MAX_SIDE, MAX_SIDE).to_rgba8())
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Where in the photo stage point (s, t) (0..1 each way) lies: bilinear
/// between the four corners.
pub fn photo_point(corners: &[[f32; 2]; 4], s: f32, t: f32) -> [f32; 2] {
    let top = lerp(corners[0], corners[1], s);
    let bottom = lerp(corners[3], corners[2], s);
    lerp(top, bottom, t)
}

/// The stage as seen in the photo, redrawn as a straight `w` x `h` picture.
pub fn warp(src: &RgbaImage, corners: &[[f32; 2]; 4], w: u32, h: u32) -> RgbaImage {
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let [u, v] = photo_point(corners, (x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let px = (u * sw - 0.5).clamp(0.0, sw - 1.0);
            let py = (v * sh - 0.5).clamp(0.0, sh - 1.0);
            let (x0, y0) = (px.floor() as u32, py.floor() as u32);
            let (x1, y1) = ((x0 + 1).min(src.width() - 1), (y0 + 1).min(src.height() - 1));
            let (fx, fy) = (px - x0 as f32, py - y0 as f32);
            let mut c = [0u8; 4];
            for (k, slot) in c.iter_mut().enumerate() {
                let a = src.get_pixel(x0, y0)[k] as f32 * (1.0 - fx) + src.get_pixel(x1, y0)[k] as f32 * fx;
                let b = src.get_pixel(x0, y1)[k] as f32 * (1.0 - fx) + src.get_pixel(x1, y1)[k] as f32 * fx;
                *slot = (a * (1.0 - fy) + b * fy).round() as u8;
            }
            out.put_pixel(x, y, image::Rgba(c));
        }
    }
    out
}

fn key(b: &Backdrop, aspect: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.path.hash(&mut h);
    for c in b.corners {
        for v in c {
            v.to_bits().hash(&mut h);
        }
    }
    aspect.to_bits().hash(&mut h);
    h.finish()
}

/// Keep the picture in step with the show: load it when the path changes,
/// redraw it when the corners move, drop it when the show has none.
pub fn sync(v: &mut View, ctx: &egui::Context) {
    let Some(b) = v.e.output.backdrop.clone() else {
        v.s.backdrop.src = None;
        v.s.backdrop.tex = None;
        v.s.backdrop.plain = None;
        v.s.backdrop.editing = false;
        return;
    };
    let st = &mut v.s.backdrop;
    if st.src.as_ref().is_none_or(|(p, _)| *p != b.path) {
        match load(&b.path) {
            Ok(img) => {
                st.src = Some((b.path.clone(), img));
                st.error = None;
            }
            Err(e) => {
                st.src = None;
                st.tex = None;
                st.error = Some(e);
                return;
            }
        }
    }
    let aspect = v.e.output.aspect.max(0.1);
    let k = key(&b, aspect);
    if st.tex.as_ref().is_some_and(|(old, _)| *old == k) {
        return;
    }
    if let Some((_, img)) = &st.src {
        let h = ((OUT_W as f32 / aspect).round() as u32).max(1);
        let out = warp(img, &b.corners, OUT_W, h);
        let ci = egui::ColorImage::from_rgba_unmultiplied([out.width() as usize, out.height() as usize], out.as_raw());
        st.tex = Some((k, ctx.load_texture("backdrop", ci, egui::TextureOptions::LINEAR)));
    }
}

/// Paint the photo into `rect`, under whatever is drawn next.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, v: &View) {
    let (Some(b), Some((_, tex))) = (&v.e.output.backdrop, &v.s.backdrop.tex) else { return };
    let a = (b.opacity.clamp(0.0, 1.0) * 255.0) as u8;
    painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(a));
}

/// Buttons above the stage: choose, remove, fade, straighten.
pub fn controls(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    ui.horizontal(|ui| {
        let has = v.e.output.backdrop.is_some();
        if ui.button(if has { "Change photo…" } else { "Add photo of the rig…" }).on_hover_text("Shows a photo behind the stage so you can place fixtures against the real rig. It is only a guide.").clicked() {
            if let Some(p) = rfd::FileDialog::new().set_title("Photo of the rig").add_filter("Photo", &["jpg", "jpeg", "png"]).pick_file() {
                let path = p.to_string_lossy().to_string();
                match load(&path) {
                    Ok(_) => {
                        v.e.output.backdrop = Some(Backdrop::new(path));
                        v.s.backdrop.error = None;
                    }
                    Err(e) => v.s.backdrop.error = Some(e),
                }
            }
        }
        if let Some(b) = &mut v.e.output.backdrop {
            ui.spacing_mut().slider_width = 90.0;
            ui.label(RichText::new("Photo").color(t.weak));
            ui.add(egui::Slider::new(&mut b.opacity, 0.0..=1.0).show_value(false)).on_hover_text("How visible the photo is");
            let edit = v.s.backdrop.editing;
            if ui.selectable_label(edit, "Straighten").on_hover_text("Drag the four corners onto the corners of your stage in the photo").clicked() {
                v.s.backdrop.editing = !edit;
            }
            if ui.small_button("Reset corners").clicked() {
                b.corners = full_photo();
            }
            if ui.small_button("Remove").clicked() {
                v.e.output.backdrop = None;
                v.s.backdrop.editing = false;
            }
        }
    });
    if let Some(e) = &v.s.backdrop.error {
        ui.colored_label(t.err, e);
    }
}

/// The photo as taken, with four handles to drag onto the stage's corners.
pub fn edit(v: &mut View, ui: &mut egui::Ui, size: Vec2) {
    let t = theme::current();
    let Some((path, img)) = &v.s.backdrop.src else { return };
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    if v.s.backdrop.plain.as_ref().is_none_or(|(p, _)| p != path) {
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let tex = ui.ctx().load_texture("backdrop_plain", ci, egui::TextureOptions::LINEAR);
        v.s.backdrop.plain = Some((path.clone(), tex));
    }
    let Some((_, tex)) = &v.s.backdrop.plain else { return };
    let k = (size.x / iw).min(size.y / ih).min(1.0e3);
    let (resp, painter) = ui.allocate_painter(Vec2::new(iw * k, ih * k), egui::Sense::click_and_drag());
    let r = resp.rect;
    painter.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    let Some(b) = &mut v.e.output.backdrop else { return };
    let to_screen = |c: [f32; 2]| egui::pos2(r.left() + c[0] * r.width(), r.top() + c[1] * r.height());
    let pts: Vec<egui::Pos2> = b.corners.iter().map(|c| to_screen(*c)).collect();
    for i in 0..4 {
        painter.line_segment([pts[i], pts[(i + 1) % 4]], egui::Stroke::new(2.0, t.accent));
    }
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.drag_started() {
            v.s.backdrop.drag = pts.iter().enumerate().map(|(i, p)| (i, p.distance(pos))).filter(|(_, d)| *d < 30.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
        }
        if let Some(i) = v.s.backdrop.drag {
            b.corners[i] = [((pos.x - r.left()) / r.width()).clamp(0.0, 1.0), ((pos.y - r.top()) / r.height()).clamp(0.0, 1.0)];
        }
    }
    if resp.drag_stopped() {
        v.s.backdrop.drag = None;
    }
    for (i, p) in pts.iter().enumerate() {
        painter.circle_filled(*p, 7.0, t.accent);
        painter.text(*p + Vec2::new(10.0, -10.0), egui::Align2::LEFT_BOTTOM, ["top left", "top right", "bottom right", "bottom left"][i], egui::FontId::proportional(t.text_size - 2.0), Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> RgbaImage {
        RgbaImage::from_fn(8, 6, |x, y| image::Rgba([(x * 30) as u8, (y * 40) as u8, 7, 255]))
    }

    #[test]
    fn full_corners_give_back_the_same_picture() {
        let src = grid();
        let out = warp(&src, &full_photo(), 8, 6);
        for (a, b) in src.pixels().zip(out.pixels()) {
            for k in 0..4 {
                assert!((a[k] as i32 - b[k] as i32).abs() <= 1, "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn corners_pick_which_part_of_the_photo_becomes_the_stage() {
        let src = grid();
        // The right half of the photo only.
        let half = [[0.5, 0.0], [1.0, 0.0], [1.0, 1.0], [0.5, 1.0]];
        let out = warp(&src, &half, 4, 6);
        assert!(out.get_pixel(0, 0)[0] > 90, "the left edge of the stage is mid-photo");
        assert_eq!(photo_point(&half, 0.0, 0.0), [0.5, 0.0]);
        assert_eq!(photo_point(&half, 1.0, 1.0), [1.0, 1.0]);
        assert_eq!(photo_point(&half, 0.5, 0.5), [0.75, 0.5]);
    }

    #[test]
    fn a_missing_photo_is_an_error_not_a_crash() {
        assert!(load("/no/such/photo.jpg").unwrap_err().starts_with("Could not open"));
    }

    #[test]
    fn a_photo_survives_the_show_file_and_old_shows_open() {
        let mut o = lumen_fixtures::AdvancedOutput::default();
        o.backdrop = Some(Backdrop::new("/x/rig.jpg"));
        let json = serde_json::to_string(&o).unwrap();
        let back: lumen_fixtures::AdvancedOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.backdrop, o.backdrop);
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("backdrop");
        let old: lumen_fixtures::AdvancedOutput = serde_json::from_value(v).unwrap();
        assert!(old.backdrop.is_none());
    }
}
