use crate::util::band_combo;
use crate::{theme, View};
use eframe::egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use lumen_core::{AudioBands, Frame};
use eframe::egui;
use lumen_core::palette::Rgb;
use lumen_core::{AudioBand, AudioLink, BlendMode, Clip, Effect, Generator, Layer, LayerLook, Paint, Palette, Section, StripClip, StripEffect, StripPixel};

/// Right-click menu on a clip slot: choose the source and edit its settings.
fn c32(c: Rgb) -> Color32 {
    Color32::from_rgb((c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8)
}

/// Colour button that treats the values as the 0..1 levels sent to the LEDs
/// (egui's own rgb button assumes linear light and shows them too bright).
fn rgb_button(ui: &mut egui::Ui, c: &mut Rgb) -> egui::Response {
    let mut b = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let r = ui.color_edit_button_srgb(&mut b);
    if r.changed() {
        *c = b.map(|v| v as f32 / 255.0);
    }
    r
}

fn swatch(ui: &mut egui::Ui, c: Rgb, size: Vec2, selected: bool) -> egui::Response {
    let t = theme::current();
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(r, 3.0, c32(c));
    let stroke = if selected { Stroke::new(2.5, t.text) } else if resp.hovered() { Stroke::new(1.5, t.weak) } else { Stroke::new(1.0, t.line) };
    ui.painter().rect_stroke(r, 3.0, stroke, StrokeKind::Outside);
    resp
}

/// Colour picker for a clip: one of the four palette colours, or a fixed colour.
fn paint_edit(ui: &mut egui::Ui, paint: &mut Paint, palette: &[Rgb; 4]) {
    ui.horizontal(|ui| {
        ui.label("Colour");
        for i in 0..4u8 {
            let sel = *paint == Paint::Slot(i);
            if swatch(ui, palette[i as usize], Vec2::new(22.0, 22.0), sel).on_hover_text(format!("Palette colour {} (follows palette changes)", i + 1)).clicked() {
                *paint = Paint::Slot(i);
            }
        }
        ui.separator();
        let mut fixed = paint.resolve(palette);
        let was_fixed = matches!(paint, Paint::Rgb(_));
        let r = rgb_button(ui, &mut fixed).on_hover_text("A fixed colour that ignores the palette");
        if r.changed() || (r.clicked() && !was_fixed) {
            *paint = Paint::Rgb(fixed);
        }
        ui.label(RichText::new(if was_fixed { "fixed" } else { "palette" }).color(theme::current().weak));
    });
}

fn effect_row(ui: &mut egui::Ui, id: (usize, usize), e: &mut Effect) {
    ui.label(e.name());
    match e {
        Effect::Brightness(k) => {
            ui.add(egui::Slider::new(k, 0.0..=2.0));
        }
        Effect::HueShift(h) => {
            ui.add(egui::Slider::new(h, 0.0..=1.0));
        }
        Effect::Invert => {}
        Effect::Pulse { depth, per_beat } => {
            ui.add(egui::Slider::new(depth, 0.0..=1.0).text("depth"));
            ui.add(egui::Slider::new(per_beat, 0.25..=8.0).text("per beat"));
        }
        Effect::AudioBrightness { band, amount } | Effect::AudioHue { band, amount } => {
            band_combo(ui, ("fxband", id), band, false);
            ui.add(egui::Slider::new(amount, 0.0..=1.0).text("amount"));
        }
    }
}

const CELL: Vec2 = Vec2::new(96.0, 54.0);
const NAME_W: f32 = 168.0;

/// One clip slot: a live mini-preview of the clip with its name on top.
fn clip_cell(ui: &mut egui::Ui, size: Vec2, clip: Option<&Clip>, active: bool, color: Color32, beat: f64, audio: &AudioBands, palette: &[Rgb; 4]) -> egui::Response {
    let t = theme::current();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter_at(rect);
    let radius = CornerRadius::same(t.radius);
    match clip {
        None => {
            let fill = if resp.hovered() { t.raised } else { t.panel };
            p.rect_filled(rect, radius, fill);
            p.rect_stroke(rect.shrink(0.5), radius, Stroke::new(1.0, t.line), StrokeKind::Inside);
            p.text(rect.center(), Align2::CENTER_CENTER, "+", FontId::proportional(t.text_size + 2.0), theme::dim(t.weak, if resp.hovered() { 1.0 } else { 0.5 }));
        }
        Some(c) => {
            // Render the clip itself, small, as its thumbnail.
            let mut f = Frame::transparent(32, 18);
            if let Generator::Strip(sc) = c.generator {
                // Four little strips, so you can see it run along each one.
                for (k, row) in [2usize, 6, 10, 14].into_iter().enumerate() {
                    for x in 0..32 {
                        let px = StripPixel { fixture: k, fixtures: 4, index: x, count: 32, x: x as f32 / 31.0, y: 0.0 };
                        let col = sc.shade(px, beat, audio, palette);
                        f.pixels[row * 32 + x] = col;
                        f.pixels[(row + 1) * 32 + x] = col;
                    }
                }
            } else {
                c.generator.render(&mut f, beat, audio, palette);
            }
            let inner = rect;
            p.rect_filled(inner, radius, Color32::BLACK);
            let (cw, ch) = (inner.width() / f.width as f32, inner.height() / f.height as f32);
            let clip_rect = inner.shrink(1.0);
            let painter = p.with_clip_rect(clip_rect);
            for y in 0..f.height {
                for x in 0..f.width {
                    let px = f.get(x, y);
                    if px.a <= 0.0 {
                        continue;
                    }
                    let k = px.a.clamp(0.0, 1.0);
                    let col = Color32::from_rgb(
                        (px.r.clamp(0.0, 1.0) * k * 255.0) as u8,
                        (px.g.clamp(0.0, 1.0) * k * 255.0) as u8,
                        (px.b.clamp(0.0, 1.0) * k * 255.0) as u8,
                    );
                    let r = Rect::from_min_size(inner.min + Vec2::new(x as f32 * cw, y as f32 * ch), Vec2::new(cw + 0.5, ch + 0.5));
                    painter.rect_filled(r, 0.0, col);
                }
            }
            // Name on a dark band at the bottom so it's readable over any colour.
            let band = Rect::from_min_max(egui::pos2(inner.min.x, inner.max.y - (size.y * 0.36).clamp(14.0, 22.0)), inner.max);
            p.rect_filled(band, CornerRadius { nw: 0, ne: 0, sw: t.radius, se: t.radius }, Color32::from_black_alpha(190));
            p.text(
                band.left_center() + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                &c.name,
                FontId::proportional(if size.y < 50.0 { t.text_size - 4.0 } else { t.text_size - 2.0 }),
                if active { Color32::WHITE } else { t.text },
            );
            if active {
                p.rect_stroke(rect, radius, Stroke::new(3.0, color), StrokeKind::Inside);
            } else if resp.hovered() {
                p.rect_stroke(rect, radius, Stroke::new(1.5, t.weak), StrokeKind::Inside);
            }
        }
    }
    resp
}

const CHANGE_ON: [Option<AudioBand>; 6] = [None, Some(AudioBand::Bar), Some(AudioBand::Beat), Some(AudioBand::Kick), Some(AudioBand::Snare), Some(AudioBand::HiHat)];

fn change_on_name(b: Option<AudioBand>) -> &'static str {
    match b {
        None => "Only when I pick",
        Some(AudioBand::Bar) => "Bar",
        Some(AudioBand::Beat) => "Beat",
        Some(AudioBand::Kick) => "Kick",
        Some(AudioBand::Snare) => "Snare",
        Some(AudioBand::HiHat) => "Hi-hat",
        Some(_) => "?",
    }
}

const HIT_BANDS: [AudioBand; 5] = [AudioBand::Kick, AudioBand::Snare, AudioBand::HiHat, AudioBand::Beat, AudioBand::Bar];

fn hit_band(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, band: &mut AudioBand, label: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(id).selected_text(band.name()).show_ui(ui, |ui| {
            for b in HIT_BANDS {
                ui.selectable_value(band, b, b.name());
            }
        });
    });
}

/// Settings for a strip clip.
fn strip_edit(ui: &mut egui::Ui, id: (usize, usize), sc: &mut StripClip) {
    match &mut sc.effect {
        StripEffect::Meteor { beats, tail, bounce, .. } => {
            ui.add(egui::Slider::new(beats, 0.25..=16.0).logarithmic(true).text("beats per run"));
            ui.add(egui::Slider::new(tail, 0.05..=1.0).text("tail length"));
            ui.checkbox(bounce, "Bounce back");
        }
        StripEffect::Fill { beats, .. } => {
            ui.add(egui::Slider::new(beats, 0.25..=32.0).logarithmic(true).text("beats to fill"));
        }
        StripEffect::Burst { band, .. } => {
            hit_band(ui, ("sb", id), band, "Burst on");
        }
        StripEffect::Segments { count, band, .. } => {
            ui.add(egui::Slider::new(count, 2..=16).text("segments"));
            hit_band(ui, ("ss", id), band, "New segment on");
        }
        StripEffect::Sparkle { density, band, .. } => {
            ui.add(egui::Slider::new(density, 0.05..=1.0).text("density"));
            crate::util::band_combo(ui, ("sp", id), band, false);
        }
        StripEffect::Meter { band, .. } => {
            ui.horizontal(|ui| {
                ui.label("Follows");
                crate::util::band_combo(ui, ("sm", id), band, false);
            });
        }
        StripEffect::Dashes { beats, count, .. } => {
            ui.add(egui::Slider::new(count, 1..=32).text("dashes"));
            ui.add(egui::Slider::new(beats, 0.25..=16.0).logarithmic(true).text("beats per step"));
        }
        StripEffect::Alternate { band, .. } => {
            hit_band(ui, ("sa", id), band, "Swap on");
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.checkbox(&mut sc.reverse, "Reverse").on_hover_text("Run from the other end");
        ui.checkbox(&mut sc.mirror, "From the middle").on_hover_text("Mirror: run from the middle out to both ends");
    });
    ui.add(egui::Slider::new(&mut sc.stagger, 0.0..=4.0).text("ripple across strips (beats)"))
        .on_hover_text("Start each strip a little later than the one before, so the effect ripples across the rig. 0 = all together.");
}

/// What the settings panel on the right is showing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inspect {
    Layer(usize),
    Clip(usize, usize),
    Palette,
}

pub struct State {
    pub inspect: Option<Inspect>,
    /// Show the effect browser for the clip being inspected.
    pub browsing: bool,
    /// Colour the browser gives new effects (kept separate from the effect).
    pub paint: Paint,
    /// Effect browser category.
    pub category: usize,
}

impl Default for State {
    fn default() -> Self {
        State { inspect: None, browsing: false, paint: Paint::Slot(0), category: 0 }
    }
}

/// Effects by kind, for the browser. Colours are set separately.
pub fn catalog() -> Vec<(&'static str, &'static str, Vec<Generator>)> {
    lumen_core::scenes::catalog()
}

fn generator_name(g: &Generator) -> String {
    lumen_core::scenes::name(g)
}

/// The Play screen's centre: palette row, then the layers and their clips.
pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let mut trigger: Option<(usize, usize)> = None;
    let mut column: Option<usize> = None;
    let mut inspect: Option<Inspect> = None;
    let mut browse_slot: Option<(usize, usize)> = None;
    let cols = app.e.comp.columns;
    let beat = app.e.comp.beat;
    let audio = app.e.comp.audio;
    let palette = app.e.comp.palettes.current(beat);
    let selected = app.s.play.inspect;
    palette_bar(app, ui);
    ui.add_space(8.0);
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(NAME_W + 12.0);
            for c in 0..cols {
                let b = egui::Button::new(RichText::new(format!("▶ {}", c + 1)).color(t.weak)).min_size(Vec2::new(CELL.x, 22.0)).frame(false);
                if ui.add(b).on_hover_text("Play this column on every layer").clicked() {
                    column = Some(c);
                }
            }
        });
        for li in 0..app.e.comp.layers.len() {
            let color = t.layers[li % t.layers.len()];
            let layer_selected = selected == Some(Inspect::Layer(li));
            let layer = &mut app.e.comp.layers[li];
            let local = (beat - layer.trigger_beat).max(0.0);
            let fill = if layer_selected { theme::mix(t.panel, color, 0.12) } else { t.panel };
            egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(t.radius + 2)).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(NAME_W, CELL.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_min_width(NAME_W);
                        ui.set_max_width(NAME_W);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 20.0), Sense::hover());
                            ui.painter().rect_filled(r, 2.0, if layer.bypass { t.line } else { color });
                            ui.allocate_ui_with_layout(Vec2::new(NAME_W - 34.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.set_max_width(NAME_W - 34.0);
                                let name = egui::Label::new(RichText::new(&layer.name).font(theme::bold(t.text_size)).color(if layer.bypass { t.weak } else { t.text })).truncate().sense(Sense::click());
                                if ui.add(name).on_hover_text("Layer settings").clicked() {
                                    inspect = Some(Inspect::Layer(li));
                                }
                            });
                            if ui.add(egui::Button::new(RichText::new("⋯").font(theme::bold(t.text_size))).frame(false)).on_hover_text("Layer settings").clicked() {
                                inspect = Some(Inspect::Layer(li));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = 58.0;
                            ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).show_value(false)).on_hover_text("Opacity");
                            let m = egui::Button::new(RichText::new("M").font(theme::bold(t.text_size - 2.0)).color(if layer.bypass { t.on_accent } else { t.weak }))
                                .fill(if layer.bypass { t.warn } else { t.raised })
                                .min_size(Vec2::new(22.0, 20.0));
                            if ui.add(m).on_hover_text("Mute this layer").clicked() {
                                layer.bypass = !layer.bypass;
                            }
                            if layer.audio_opacity.is_some() || layer.advance_on.is_some() {
                                ui.label(RichText::new("♪").color(t.accent)).on_hover_text("Reacts to the music");
                            }
                            if let Some(g) = &layer.target {
                                let short: String = if g.chars().count() > 8 { g.chars().take(7).chain(['…']).collect() } else { g.clone() };
                                ui.label(RichText::new(short).color(t.weak).size(t.text_size - 3.0)).on_hover_text(format!("Only lights the group \"{g}\""));
                            }
                        });
                    });
                    for ci in 0..layer.clips.len() {
                        let active = layer.active == Some(ci);
                        let b = if active { local } else { beat };
                        let resp = clip_cell(ui, CELL, layer.clips[ci].as_ref(), active, color, b, &audio, &palette);
                        let is_sel = selected == Some(Inspect::Clip(li, ci));
                        if is_sel {
                            ui.painter().rect_stroke(resp.rect.expand(2.0), t.radius + 1, Stroke::new(1.5, t.accent), StrokeKind::Outside);
                        }
                        if layer.clips[ci].is_none() {
                            if resp.on_hover_text("Add an effect").clicked() {
                                browse_slot = Some((li, ci));
                            }
                        } else {
                            if resp.clicked() {
                                trigger = Some((li, ci));
                            }
                            if resp.secondary_clicked() || resp.double_clicked() {
                                inspect = Some(Inspect::Clip(li, ci));
                            }
                        }
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            if ui.button("+ Layer").clicked() {
                let n = app.e.comp.layers.len() + 1;
                app.e.comp.layers.insert(0, Layer::new(format!("Layer {n}"), cols));
            }
            ui.weak("Click a clip to play it · right-click to change it · + to add an effect");
        });
    });
    if let Some((l, c)) = trigger {
        app.e.comp.trigger(l, c);
    }
    if let Some(c) = column {
        app.e.comp.trigger_column(c);
    }
    if let Some(i) = inspect {
        app.s.play.inspect = Some(i);
        app.s.play.browsing = false;
    }
    if let Some((l, c)) = browse_slot {
        app.s.play.inspect = Some(Inspect::Clip(l, c));
        app.s.play.browsing = true;
    }
}

const CHANGE_LABEL: &str = "Change";

/// One row: the palettes, Next, when it changes, and Edit.
fn palette_bar(app: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let beat = app.e.comp.beat;
    let pals = &mut app.e.comp.palettes;
    let mut pick = None;
    let mut edit = false;
    egui::Frame::new().fill(t.panel).corner_radius(CornerRadius::same(t.radius + 2)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Colours").font(theme::bold(t.text_size)));
            ui.add_space(4.0);
            egui::ScrollArea::horizontal().max_width((ui.available_width() - 380.0).max(120.0)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, p) in pals.list.iter().enumerate() {
                        if palette_chip(ui, p, i == pals.active).on_hover_text(&p.name).clicked() {
                            pick = Some(i);
                        }
                    }
                });
            });
            if ui.button("Next ⏵").on_hover_text("Move to the next palette").clicked() {
                pick = Some(pals.active + 1);
            }
            ui.label(RichText::new(CHANGE_LABEL).color(t.weak));
            egui::ComboBox::from_id_salt("pal_change_on").width(110.0).selected_text(change_on_name(pals.advance_on)).show_ui(ui, |ui| {
                for b in CHANGE_ON {
                    ui.selectable_value(&mut pals.advance_on, b, change_on_name(b));
                }
            });
            if ui.button("Edit").on_hover_text("Edit the colours").clicked() {
                edit = true;
            }
        });
    });
    if let Some(i) = pick {
        pals.select(i, beat);
    }
    if edit {
        app.s.play.inspect = Some(Inspect::Palette);
    }
}

fn palette_chip(ui: &mut egui::Ui, p: &Palette, active: bool) -> egui::Response {
    let t = theme::current();
    let (r, resp) = ui.allocate_exact_size(Vec2::new(56.0, 22.0), Sense::click());
    let w = r.width() / 4.0;
    for (k, c) in p.colors.iter().enumerate() {
        let cr = Rect::from_min_size(r.min + Vec2::new(k as f32 * w, 0.0), Vec2::new(w, r.height()));
        let rad = CornerRadius { nw: if k == 0 { 3 } else { 0 }, sw: if k == 0 { 3 } else { 0 }, ne: if k == 3 { 3 } else { 0 }, se: if k == 3 { 3 } else { 0 } };
        ui.painter().rect_filled(cr, rad, c32(*c));
    }
    let stroke = if active { Stroke::new(2.5, t.text) } else if resp.hovered() { Stroke::new(1.5, t.weak) } else { Stroke::new(1.0, t.line) };
    ui.painter().rect_stroke(r, 3.0, stroke, StrokeKind::Outside);
    resp
}

/// The right-hand panel on the Play screen when something is selected.
/// Returns false when it should close.
pub fn inspector(app: &mut View, ui: &mut egui::Ui, groups: &[String]) -> bool {
    let t = theme::current();
    let Some(what) = app.s.play.inspect else { return false };
    let mut open = true;
    let title = match what {
        Inspect::Layer(l) => app.e.comp.layers.get(l).map(|l| l.name.clone()).unwrap_or_default(),
        Inspect::Clip(l, c) => match app.e.comp.layers.get(l).and_then(|x| x.clips.get(c)) {
            Some(Some(clip)) if !app.s.play.browsing => clip.name.clone(),
            Some(_) => "Add an effect".into(),
            None => String::new(),
        },
        Inspect::Palette => "Colours".into(),
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(theme::bold(t.text_size + 2.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕").on_hover_text("Close").clicked() {
                open = false;
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match what {
        Inspect::Layer(l) if l < app.e.comp.layers.len() => layer_inspector(app, ui, l, groups),
        Inspect::Clip(l, c) if l < app.e.comp.layers.len() && c < app.e.comp.columns => clip_inspector(app, ui, l, c),
        Inspect::Palette => palette_editor(app, ui),
        _ => open = false,
    });
    open
}

fn section_label(ui: &mut egui::Ui, s: &str) {
    let t = theme::current();
    ui.add_space(8.0);
    ui.label(RichText::new(s).font(theme::bold(t.text_size - 1.0)).color(t.weak));
}

fn layer_inspector(app: &mut View, ui: &mut egui::Ui, li: usize, groups: &[String]) {
    let t = theme::current();
    let mut remove = false;
    let mut stop = false;
    let cols = app.e.comp.columns;
    let layer = &mut app.e.comp.layers[li];
    egui::Grid::new(("layer_grid", li)).num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label(RichText::new("Name").color(t.weak));
        ui.text_edit_singleline(&mut layer.name);
        ui.end_row();
        ui.label(RichText::new("Opacity").color(t.weak));
        ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).show_value(false));
        ui.end_row();
        ui.label(RichText::new("Mix").color(t.weak));
        egui::ComboBox::from_id_salt(("blend", li)).selected_text(layer.blend.name()).show_ui(ui, |ui| {
            for m in BlendMode::ALL {
                ui.selectable_value(&mut layer.blend, m, m.name());
            }
        });
        ui.end_row();
        ui.label(RichText::new("Lights").color(t.weak)).on_hover_text("Which fixtures this layer lights. Make groups in Set up → Stage.");
        let label = layer.target.clone().unwrap_or_else(|| "All fixtures".into());
        egui::ComboBox::from_id_salt(("target", li)).selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut layer.target, None, "All fixtures");
            for g in groups {
                ui.selectable_value(&mut layer.target, Some(g.clone()), g);
            }
            if groups.is_empty() {
                ui.weak("No groups yet: make them in Set up → Stage");
            }
        });
        ui.end_row();
        ui.label(RichText::new("Reacts to").color(t.weak));
        ui.horizontal(|ui| {
            let mut link = layer.audio_opacity.unwrap_or(AudioLink { band: AudioBand::Bass, amount: 1.0 });
            let label = layer.audio_opacity.map(|l| l.band.name()).unwrap_or("Nothing");
            let mut on = layer.audio_opacity.is_some();
            egui::ComboBox::from_id_salt(("oplink", li)).width(90.0).selected_text(label).show_ui(ui, |ui| {
                if ui.selectable_label(!on, "Nothing").clicked() {
                    on = false;
                }
                for b in AudioBand::ALL {
                    if ui.selectable_label(on && link.band == b, b.name()).clicked() {
                        on = true;
                        link.band = b;
                    }
                }
            });
            if on {
                ui.spacing_mut().slider_width = 70.0;
                ui.add(egui::Slider::new(&mut link.amount, 0.0..=1.0).show_value(false)).on_hover_text("How much the music drives opacity");
            }
            layer.audio_opacity = on.then_some(link);
        });
        ui.end_row();
        ui.label(RichText::new("Next clip on").color(t.weak));
        let label = layer.advance_on.map(|b| b.name()).unwrap_or("Off");
        egui::ComboBox::from_id_salt(("adv", li)).selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut layer.advance_on, None, "Off");
            for b in AudioBand::HITS {
                ui.selectable_value(&mut layer.advance_on, Some(b), b.name());
            }
        });
        ui.end_row();
    });

    section_label(ui, "In each part of the song");
    ui.weak("Used when Auto looks is on (Music card).");
    egui::Grid::new(("looks", li)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        for s in Section::ALL {
            let [r, g, b] = s.color();
            ui.label(RichText::new(s.name()).color(c32([r, g, b])));
            let cur = &mut layer.looks[s.index()];
            let name = |l: &LayerLook, layer_clips: &Vec<Option<Clip>>| match l {
                LayerLook::Keep => "Keep playing".to_string(),
                LayerLook::Stop => "Off".to_string(),
                LayerLook::Play(c) => layer_clips.get(*c).and_then(|x| x.as_ref()).map(|x| format!("{} · {}", c + 1, x.name)).unwrap_or_else(|| format!("Column {}", c + 1)),
            };
            let text = name(cur, &layer.clips);
            egui::ComboBox::from_id_salt(("look", li, s.index())).width(150.0).selected_text(text).show_ui(ui, |ui| {
                ui.selectable_value(cur, LayerLook::Keep, "Keep playing");
                ui.selectable_value(cur, LayerLook::Stop, "Off");
                for c in 0..cols {
                    if let Some(Some(clip)) = layer.clips.get(c) {
                        ui.selectable_value(cur, LayerLook::Play(c), format!("{} · {}", c + 1, clip.name));
                    }
                }
            });
            ui.end_row();
        }
    });

    section_label(ui, &format!("Effects ({})", layer.effects.len()));
    let mut rm = None;
    for (ei, e) in layer.effects.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = 90.0;
            effect_row(ui, (li, ei), e);
            if ui.small_button("✕").clicked() {
                rm = Some(ei);
            }
        });
    }
    if let Some(i) = rm {
        layer.effects.remove(i);
    }
    ui.menu_button("+ Add effect", |ui| {
        let list = [
            Effect::Brightness(1.0),
            Effect::HueShift(0.0),
            Effect::Invert,
            Effect::Pulse { depth: 0.8, per_beat: 1.0 },
            Effect::AudioBrightness { band: AudioBand::Bass, amount: 1.0 },
            Effect::AudioHue { band: AudioBand::HighMid, amount: 0.5 },
        ];
        for e in list {
            if ui.button(e.name()).clicked() {
                layer.effects.push(e);
                ui.close();
            }
        }
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("■ Stop layer").clicked() {
            stop = true;
        }
        if ui.button(RichText::new("Remove layer").color(t.err)).clicked() {
            remove = true;
        }
    });
    if stop {
        app.e.comp.stop(li);
    }
    if remove {
        app.e.comp.layers.remove(li);
        app.s.play.inspect = None;
    }
}

fn clip_inspector(app: &mut View, ui: &mut egui::Ui, li: usize, ci: usize) {
    let t = theme::current();
    let beat = app.e.comp.beat;
    let audio = app.e.comp.audio;
    let palette = app.e.comp.palettes.current(beat);
    let empty = app.e.comp.layers[li].clips[ci].is_none();
    if empty || app.s.play.browsing {
        let picked = browser(ui, &mut app.s.play, beat, &audio, &palette);
        if let Some((name, g)) = picked {
            app.e.comp.layers[li].clips[ci] = Some(Clip::new(name, g));
            app.e.comp.trigger(li, ci);
            app.s.play.browsing = false;
        }
        if !empty && ui.button("Back to settings").clicked() {
            app.s.play.browsing = false;
        }
        return;
    }
    let mut clear = false;
    let mut play = false;
    let slot = &mut app.e.comp.layers[li].clips[ci];
    let Some(clip) = slot else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new("Name").color(t.weak));
        ui.text_edit_singleline(&mut clip.name);
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Effect: {}", generator_name(&clip.generator))).color(t.weak));
        if ui.button("Change…").clicked() {
            app.s.play.browsing = true;
        }
    });
    if let Some(paint) = clip.generator.paint_mut() {
        section_label(ui, "Colour");
        paint_edit(ui, paint, &palette);
    }
    section_label(ui, "Settings");
    gen_edit(ui, (li, ci), &mut clip.generator);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("▶ Play").clicked() {
            play = true;
        }
        if ui.button(RichText::new("Clear slot").color(t.err)).clicked() {
            clear = true;
        }
    });
    if clear {
        *slot = None;
        if app.e.comp.layers[li].active == Some(ci) {
            app.e.comp.stop(li);
        }
        app.s.play.inspect = None;
    }
    if play {
        app.e.comp.trigger(li, ci);
    }
}

/// Settings for an effect, without its colour (that's set separately).
fn gen_edit(ui: &mut egui::Ui, id: (usize, usize), g: &mut Generator) {
    let beats = |ui: &mut egui::Ui, v: &mut f32, label: &str| {
        ui.add(egui::Slider::new(v, 0.25..=32.0).logarithmic(true).text(label));
    };
    match g {
        Generator::Strip(sc) => strip_edit(ui, id, sc),
        Generator::Solid { .. } => {
            ui.weak("One colour everywhere.");
        }
        Generator::PaletteWash { beats_per_cycle } => {
            ui.weak("The four palette colours, blending into each other.");
            beats(ui, beats_per_cycle, "beats to drift");
        }
        Generator::Rainbow { scale, beats_per_cycle } => {
            ui.add(egui::Slider::new(scale, 0.1..=4.0).text("size"));
            beats(ui, beats_per_cycle, "beats per cycle");
        }
        Generator::Plasma { scale, speed } => {
            ui.add(egui::Slider::new(scale, 0.1..=4.0).text("size"));
            ui.add(egui::Slider::new(speed, 0.0..=4.0).text("speed"));
        }
        Generator::Chase { width, beats_per_cycle, .. } => {
            ui.add(egui::Slider::new(width, 0.02..=1.0).text("width"));
            beats(ui, beats_per_cycle, "beats per run");
        }
        Generator::Strobe { per_beat, duty, .. } => {
            ui.add(egui::Slider::new(per_beat, 0.25..=8.0).text("flashes per beat"));
            ui.add(egui::Slider::new(duty, 0.05..=0.95).text("on time"));
        }
        Generator::AudioFlash { band, .. } => {
            ui.horizontal(|ui| {
                ui.label("Follows");
                band_combo(ui, ("flashband", id), band, false);
            });
        }
        Generator::Bands => {
            ui.weak("Sub, bass, low mid, high mid and highs as coloured bars.");
        }
        Generator::Radar { beats_per_turn, tail, .. } => {
            beats(ui, beats_per_turn, "beats per turn");
            ui.add(egui::Slider::new(tail, 0.02..=1.0).text("tail"));
        }
        Generator::LeftRight { beats: b, width, .. } | Generator::Scanner { beats: b, width, .. } => {
            beats(ui, b, "beats there and back");
            ui.add(egui::Slider::new(width, 0.02..=1.0).text("width"));
        }
        Generator::Sweep { beats: b, width, inward, .. } => {
            beats(ui, b, "beats per sweep");
            ui.add(egui::Slider::new(width, 0.02..=1.0).text("width"));
            ui.horizontal(|ui| {
                ui.selectable_value(inward, true, "Inwards");
                ui.selectable_value(inward, false, "Outwards");
            });
        }
        Generator::Wipe { beats: b, .. } => beats(ui, b, "beats to fill and clear"),
        Generator::BeatPulse { per_beat, fade, .. } => {
            ui.add(egui::Slider::new(per_beat, 0.25..=4.0).text("pulses per beat"));
            ui.add(egui::Slider::new(fade, 0.05..=1.0).text("fade"));
        }
        Generator::Twinkle { density, per_beat, .. } => {
            ui.add(egui::Slider::new(density, 0.05..=1.0).text("how many"));
            ui.add(egui::Slider::new(per_beat, 0.25..=8.0).text("changes per beat"));
        }
        Generator::Breathe { beats: b, .. } => beats(ui, b, "beats per breath"),
        Generator::Checker { beats: b, .. } => beats(ui, b, "beats between swaps"),
        Generator::Burst { per_beat, .. } => {
            ui.add(egui::Slider::new(per_beat, 0.25..=4.0).text("rings per beat"));
        }
        Generator::Throb { .. } | Generator::Grow { .. } => {
            ui.label("Follows the bass.");
        }
    }
}

/// Pick an effect: categories of live tiles, colour chosen on its own.
fn browser(ui: &mut egui::Ui, st: &mut State, beat: f64, audio: &AudioBands, palette: &[Rgb; 4]) -> Option<(String, Generator)> {
    let t = theme::current();
    let cat = catalog();
    let mut picked = None;
    ui.add_space(4.0);
    paint_edit(ui, &mut st.paint, palette);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for (i, (name, _, _)) in cat.iter().enumerate() {
            ui.selectable_value(&mut st.category, i, *name);
        }
    });
    let (_, note, list) = &cat[st.category.min(cat.len() - 1)];
    ui.label(RichText::new(*note).color(t.weak).size(t.text_size - 2.0));
    ui.add_space(4.0);
    let w = ((ui.available_width() - 12.0) / 2.0).floor();
    let size = Vec2::new(w, (w * 0.55).round());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        for g in list {
            let name = generator_name(g);
            let mut g = g.clone();
            if let Some(p) = g.paint_mut() {
                *p = st.paint;
            }
            let clip = Clip::new(name.clone(), g.clone());
            let r = clip_cell(ui, size, Some(&clip), false, t.accent, beat, audio, palette);
            if r.on_hover_text("Use this effect").clicked() {
                picked = Some((name, g));
            }
        }
    });
    picked
}

fn palette_editor(app: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let beat = app.e.comp.beat;
    let pals = &mut app.e.comp.palettes;
    let mut pick = None;
    ui.horizontal_wrapped(|ui| {
        for (i, p) in pals.list.iter().enumerate() {
            if palette_chip(ui, p, i == pals.active).on_hover_text(&p.name).clicked() {
                pick = Some(i);
            }
        }
    });
    ui.add_space(6.0);
    if let Some(p) = pals.list.get_mut(pals.active) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Name").color(t.weak));
            ui.text_edit_singleline(&mut p.name);
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Colours").color(t.weak));
            for c in p.colors.iter_mut() {
                rgb_button(ui, c);
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.button("Duplicate").on_hover_text("Copy this palette to make your own").clicked() {
            if let Some(p) = pals.list.get(pals.active).cloned() {
                pals.list.insert(pals.active + 1, Palette { name: format!("{} copy", p.name), ..p });
                pick = Some(pals.active + 1);
            }
        }
        if pals.list.len() > 1 && ui.button("Remove").clicked() {
            pals.list.remove(pals.active);
            pals.active = pals.active.min(pals.list.len() - 1);
        }
    });
    section_label(ui, "Changing palettes");
    egui::Grid::new("pal_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label(RichText::new(CHANGE_LABEL).color(t.weak));
        egui::ComboBox::from_id_salt("pal_change_on2").selected_text(change_on_name(pals.advance_on)).show_ui(ui, |ui| {
            for b in CHANGE_ON {
                ui.selectable_value(&mut pals.advance_on, b, change_on_name(b));
            }
        });
        ui.end_row();
        if pals.advance_on.is_some() {
            ui.label(RichText::new("Every").color(t.weak));
            ui.add(egui::DragValue::new(&mut pals.every).range(1..=64));
            ui.end_row();
        }
        ui.label(RichText::new("Fade").color(t.weak));
        ui.add(egui::DragValue::new(&mut pals.fade_beats).range(0.0..=16.0).speed(0.05).suffix(" beats"));
        ui.end_row();
    });
    if let Some(i) = pick {
        pals.select(i, beat);
    }
}
