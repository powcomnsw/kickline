use crate::util::band_combo;
use crate::{theme, View};
use eframe::egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use lumen_core::{AudioBands, Frame};
use eframe::egui;
use lumen_core::palette::Rgb;
use lumen_core::{AudioBand, AudioLink, BlendMode, Clip, Effect, Generator, Layer, Paint, Palette, StripClip, StripEffect, StripPixel};

fn generator_presets() -> Vec<Generator> {
    vec![
        Generator::Solid { color: Paint::Slot(0) },
        Generator::PaletteWash { beats_per_cycle: 16.0 },
        Generator::Rainbow {
            scale: 1.0,
            beats_per_cycle: 8.0,
        },
        Generator::Plasma {
            scale: 1.0,
            speed: 1.0,
        },
        Generator::Chase {
            color: Paint::Slot(1),
            width: 0.25,
            beats_per_cycle: 2.0,
        },
        Generator::Strobe {
            color: Paint::WHITE,
            per_beat: 2.0,
            duty: 0.3,
        },
        Generator::AudioFlash {
            band: AudioBand::Kick,
            color: Paint::Slot(0),
        },
        Generator::Bands,
    ]
}

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

fn clip_menu(ui: &mut egui::Ui, id: (usize, usize), slot: &mut Option<Clip>, palette: &[Rgb; 4]) {
    ui.set_min_width(240.0);
    ui.label("Strip effects").on_hover_text("These run along every strip (or fixture) on its own, whatever angle it's at. They're drawn on top of the picture layers.");
    ui.horizontal_wrapped(|ui| {
        for (name, sc) in StripClip::presets() {
            let current = slot.as_ref().is_some_and(|c| c.name == name && c.generator.is_strip());
            if ui.selectable_label(current, name).clicked() {
                *slot = Some(Clip::new(name, Generator::Strip(sc)));
            }
        }
    });
    ui.add_space(4.0);
    ui.label("Picture");
    ui.horizontal_wrapped(|ui| {
        for g in generator_presets() {
            let current = slot.as_ref().map(|c| c.generator.kind_name());
            if ui
                .selectable_label(current == Some(g.kind_name()), g.kind_name())
                .clicked()
            {
                let name = g.kind_name().to_string();
                *slot = Some(Clip::new(name, g));
            }
        }
    });
    let Some(clip) = slot else { return };
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut clip.name);
    });
    match &mut clip.generator {
        Generator::Strip(sc) => strip_edit(ui, id, sc, palette),
        Generator::Solid { color } => paint_edit(ui, color, palette),
        Generator::PaletteWash { beats_per_cycle } => {
            ui.label("The four palette colours, blending into each other.");
            ui.add(egui::Slider::new(beats_per_cycle, 1.0..=64.0).text("beats / drift"));
        }
        Generator::Rainbow {
            scale,
            beats_per_cycle,
        } => {
            ui.add(egui::Slider::new(scale, 0.1..=4.0).text("scale"));
            ui.add(egui::Slider::new(beats_per_cycle, 0.5..=32.0).text("beats / cycle"));
        }
        Generator::Plasma { scale, speed } => {
            ui.add(egui::Slider::new(scale, 0.1..=4.0).text("scale"));
            ui.add(egui::Slider::new(speed, 0.0..=4.0).text("speed"));
        }
        Generator::Chase {
            color,
            width,
            beats_per_cycle,
        } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(width, 0.02..=1.0).text("width"));
            ui.add(egui::Slider::new(beats_per_cycle, 0.25..=16.0).text("beats / cycle"));
        }
        Generator::Strobe {
            color,
            per_beat,
            duty,
        } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(per_beat, 0.25..=8.0).text("flashes / beat"));
            ui.add(egui::Slider::new(duty, 0.05..=0.95).text("on time"));
        }
        Generator::AudioFlash { band, color } => {
            ui.horizontal(|ui| {
                ui.label("Follows");
                band_combo(ui, ("flashband", id), band, false);
            });
            paint_edit(ui, color, palette);
        }
        Generator::Bands => {
            ui.label("Sub, bass, low mid, high mid and highs as coloured bars.");
        }
    }
    ui.separator();
    if ui.button("Clear slot").clicked() {
        *slot = None;
        ui.close();
    }
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

fn layer_controls(ui: &mut egui::Ui, li: usize, layer: &mut Layer) {
    let t = theme::current();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().slider_width = 110.0;
        ui.label(RichText::new("Opacity").color(t.weak));
        ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).show_value(false));
        egui::ComboBox::from_id_salt(("blend", li)).width(84.0).selected_text(layer.blend.name()).show_ui(ui, |ui| {
            for m in BlendMode::ALL {
                ui.selectable_value(&mut layer.blend, m, m.name());
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Reacts to").color(t.weak));
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
            ui.add(egui::Slider::new(&mut link.amount, 0.0..=1.0).show_value(false)).on_hover_text("How much the band drives opacity");
        }
        layer.audio_opacity = on.then_some(link);
        ui.add_space(10.0);
        ui.label(RichText::new("Next clip on").color(t.weak));
        let label = layer.advance_on.map(|b| b.name()).unwrap_or("Off");
        egui::ComboBox::from_id_salt(("adv", li)).width(70.0).selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut layer.advance_on, None, "Off");
            for b in AudioBand::HITS {
                ui.selectable_value(&mut layer.advance_on, Some(b), b.name());
            }
        });
    });
}

const CELL: Vec2 = Vec2::new(104.0, 60.0);
const NAME_W: f32 = 140.0;

/// One clip slot: a live mini-preview of the clip with its name on top.
fn clip_cell(ui: &mut egui::Ui, clip: Option<&Clip>, active: bool, color: Color32, beat: f64, audio: &AudioBands, palette: &[Rgb; 4]) -> egui::Response {
    let t = theme::current();
    let (rect, resp) = ui.allocate_exact_size(CELL, Sense::click());
    let p = ui.painter_at(rect);
    let radius = CornerRadius::same(t.radius);
    match clip {
        None => {
            let fill = if resp.hovered() { t.raised } else { t.panel };
            p.rect_filled(rect, radius, fill);
            p.rect_stroke(rect.shrink(0.5), radius, Stroke::new(1.0, t.line), StrokeKind::Inside);
            p.text(rect.center(), Align2::CENTER_CENTER, "empty", FontId::proportional(t.text_size - 3.0), theme::dim(t.weak, 0.7));
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
            let band = Rect::from_min_max(egui::pos2(inner.min.x, inner.max.y - 22.0), inner.max);
            p.rect_filled(band, CornerRadius { nw: 0, ne: 0, sw: t.radius, se: t.radius }, Color32::from_black_alpha(190));
            p.text(
                band.left_center() + Vec2::new(8.0, 0.0),
                Align2::LEFT_CENTER,
                &c.name,
                FontId::proportional(t.text_size - 2.0),
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

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let mut trigger: Option<(usize, usize)> = None;
    let mut column: Option<usize> = None;
    let mut stop: Option<usize> = None;
    let mut remove_layer: Option<usize> = None;
    let cols = app.e.comp.columns;
    let beat = app.e.comp.beat;
    let audio = app.e.comp.audio;
    let palette = app.e.comp.palettes.current(beat);
    palette_bar(app, ui);
    ui.add_space(6.0);
    egui::ScrollArea::both().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(NAME_W + 8.0);
            for c in 0..cols {
                let b = egui::Button::new(RichText::new(format!("▶  {}", c + 1)).color(t.weak)).min_size(Vec2::new(CELL.x, 24.0));
                if ui.add(b).on_hover_text("Play this column on every layer").clicked() {
                    column = Some(c);
                }
            }
        });
        ui.add_space(4.0);
        for li in 0..app.e.comp.layers.len() {
            let color = t.layers[li % t.layers.len()];
            let layer = &mut app.e.comp.layers[li];
            let local = (beat - layer.trigger_beat).max(0.0);
            egui::Frame::new().fill(t.panel).corner_radius(CornerRadius::same(t.radius + 2)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Layer header: colour, name, bypass / stop / remove.
                    ui.vertical(|ui| {
                        ui.set_width(NAME_W);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::new(5.0, 22.0), Sense::hover());
                            ui.painter().rect_filled(r, 2.0, if layer.bypass { t.line } else { color });
                            ui.add(egui::TextEdit::singleline(&mut layer.name).font(theme::bold(t.text_size)).frame(egui::Frame::NONE).desired_width(NAME_W - 20.0));
                        });
                        ui.horizontal(|ui| {
                            ui.toggle_value(&mut layer.bypass, "Bypass").on_hover_text("Hide this layer");
                            if ui.button("■").on_hover_text("Stop layer").clicked() {
                                stop = Some(li);
                            }
                            if ui.button("×").on_hover_text("Remove layer").clicked() {
                                remove_layer = Some(li);
                            }
                        });
                    });
                    for ci in 0..layer.clips.len() {
                        let active = layer.active == Some(ci);
                        let b = if active { local } else { beat };
                        let resp = clip_cell(ui, layer.clips[ci].as_ref(), active, color, b, &audio, &palette);
                        if resp.clicked() {
                            trigger = Some((li, ci));
                        }
                        resp.context_menu(|ui| clip_menu(ui, (li, ci), &mut layer.clips[ci], &palette));
                    }
                });
                ui.add_space(2.0);
                layer_controls(ui, li, layer);
                ui.collapsing(RichText::new(format!("Effects ({})", layer.effects.len())).color(t.weak), |ui| {
                    let mut remove = None;
                    for (ei, e) in layer.effects.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            effect_row(ui, (li, ei), e);
                            if ui.small_button("×").clicked() {
                                remove = Some(ei);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        layer.effects.remove(i);
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.weak("Add:");
                        if ui.small_button("Brightness").clicked() {
                            layer.effects.push(Effect::Brightness(1.0));
                        }
                        if ui.small_button("Hue").clicked() {
                            layer.effects.push(Effect::HueShift(0.0));
                        }
                        if ui.small_button("Invert").clicked() {
                            layer.effects.push(Effect::Invert);
                        }
                        if ui.small_button("Pulse").clicked() {
                            layer.effects.push(Effect::Pulse { depth: 0.8, per_beat: 1.0 });
                        }
                        if ui.small_button("Audio brightness").clicked() {
                            layer.effects.push(Effect::AudioBrightness { band: AudioBand::Bass, amount: 1.0 });
                        }
                        if ui.small_button("Audio hue").clicked() {
                            layer.effects.push(Effect::AudioHue { band: AudioBand::HighMid, amount: 0.5 });
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if ui.button("+ Layer").clicked() {
                let n = app.e.comp.layers.len() + 1;
                app.e.comp.layers.insert(0, Layer::new(format!("Layer {n}"), cols));
            }
            ui.weak("Click a clip to play it · right-click to change it");
        });
    });
    if let Some((l, c)) = trigger {
        app.e.comp.trigger(l, c);
    }
    if let Some(c) = column {
        app.e.comp.trigger_column(c);
    }
    if let Some(l) = stop {
        app.e.comp.stop(l);
    }
    if let Some(l) = remove_layer {
        app.e.comp.layers.remove(l);
    }
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

/// The palette strip above the clips: pick a palette, edit its colours,
/// and choose when Kickline moves on to the next one.
fn palette_bar(app: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let beat = app.e.comp.beat;
    let pals = &mut app.e.comp.palettes;
    let mut pick = None;
    egui::Frame::new().fill(t.panel).corner_radius(CornerRadius::same(t.radius + 2)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Palette").font(theme::bold(t.text_size)));
            ui.add_space(4.0);
            for (i, p) in pals.list.iter().enumerate() {
                let active = i == pals.active;
                let size = Vec2::new(64.0, 26.0);
                let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                let w = r.width() / 4.0;
                for (k, c) in p.colors.iter().enumerate() {
                    let cr = Rect::from_min_size(r.min + Vec2::new(k as f32 * w, 0.0), Vec2::new(w, r.height()));
                    let rad = CornerRadius { nw: if k == 0 { 3 } else { 0 }, sw: if k == 0 { 3 } else { 0 }, ne: if k == 3 { 3 } else { 0 }, se: if k == 3 { 3 } else { 0 } };
                    ui.painter().rect_filled(cr, rad, c32(*c));
                }
                let stroke = if active { Stroke::new(2.5, t.text) } else if resp.hovered() { Stroke::new(1.5, t.weak) } else { Stroke::new(1.0, t.line) };
                ui.painter().rect_stroke(r, 3.0, stroke, StrokeKind::Outside);
                let resp = resp.on_hover_text(&p.name);
                if resp.clicked() {
                    pick = Some(i);
                }
            }
            if ui.button("Next ⏵").on_hover_text("Move to the next palette").clicked() {
                pick = Some(pals.active + 1);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(p) = pals.list.get_mut(pals.active) {
                ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(90.0));
                for c in p.colors.iter_mut() {
                    rgb_button(ui, c);
                }
            }
            if ui.small_button("Duplicate").on_hover_text("Copy this palette to make your own").clicked() {
                if let Some(p) = pals.list.get(pals.active).cloned() {
                    pals.list.insert(pals.active + 1, Palette { name: format!("{} copy", p.name), ..p });
                    pick = Some(pals.active + 1);
                }
            }
            if pals.list.len() > 1 && ui.small_button("Remove").clicked() {
                pals.list.remove(pals.active);
                pals.active = pals.active.min(pals.list.len() - 1);
            }
            ui.separator();
            ui.label(RichText::new("Change on").color(t.weak));
            egui::ComboBox::from_id_salt("pal_change_on").selected_text(change_on_name(pals.advance_on)).show_ui(ui, |ui| {
                for b in CHANGE_ON {
                    ui.selectable_value(&mut pals.advance_on, b, change_on_name(b));
                }
            });
            if pals.advance_on.is_some() {
                ui.label(RichText::new("every").color(t.weak));
                ui.add(egui::DragValue::new(&mut pals.every).range(1..=64));
            }
            ui.label(RichText::new("Fade").color(t.weak));
            ui.add(egui::DragValue::new(&mut pals.fade_beats).range(0.0..=16.0).speed(0.05).suffix(" beats"));
        });
    });
    if let Some(i) = pick {
        pals.select(i, beat);
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
fn strip_edit(ui: &mut egui::Ui, id: (usize, usize), sc: &mut StripClip, palette: &[Rgb; 4]) {
    match &mut sc.effect {
        StripEffect::Meteor { color, beats, tail, bounce } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(beats, 0.25..=16.0).logarithmic(true).text("beats per run"));
            ui.add(egui::Slider::new(tail, 0.05..=1.0).text("tail length"));
            ui.checkbox(bounce, "Bounce back");
        }
        StripEffect::Fill { color, beats } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(beats, 0.25..=32.0).logarithmic(true).text("beats to fill"));
        }
        StripEffect::Burst { color, band } => {
            paint_edit(ui, color, palette);
            hit_band(ui, ("sb", id), band, "Burst on");
        }
        StripEffect::Segments { color, count, band } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(count, 2..=16).text("segments"));
            hit_band(ui, ("ss", id), band, "New segment on");
        }
        StripEffect::Sparkle { color, density, band } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(density, 0.05..=1.0).text("density"));
            crate::util::band_combo(ui, ("sp", id), band, false);
        }
        StripEffect::Meter { color, band } => {
            paint_edit(ui, color, palette);
            ui.horizontal(|ui| {
                ui.label("Follows");
                crate::util::band_combo(ui, ("sm", id), band, false);
            });
        }
        StripEffect::Dashes { color, beats, count } => {
            paint_edit(ui, color, palette);
            ui.add(egui::Slider::new(count, 1..=32).text("dashes"));
            ui.add(egui::Slider::new(beats, 0.25..=16.0).logarithmic(true).text("beats per step"));
        }
        StripEffect::Alternate { color, band } => {
            paint_edit(ui, color, palette);
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
