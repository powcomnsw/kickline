use crate::util::band_combo;
use crate::{theme, View};
use eframe::egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use lumen_core::{AudioBands, Frame};
use eframe::egui;
use lumen_core::{AudioBand, AudioLink, BlendMode, Clip, Effect, Generator, Layer};

fn generator_presets() -> Vec<Generator> {
    vec![
        Generator::Solid {
            color: [1.0, 1.0, 1.0],
        },
        Generator::Rainbow {
            scale: 1.0,
            beats_per_cycle: 8.0,
        },
        Generator::Plasma {
            scale: 1.0,
            speed: 1.0,
        },
        Generator::Chase {
            color: [1.0, 1.0, 1.0],
            width: 0.25,
            beats_per_cycle: 2.0,
        },
        Generator::Strobe {
            color: [1.0, 1.0, 1.0],
            per_beat: 2.0,
            duty: 0.3,
        },
        Generator::AudioFlash {
            band: AudioBand::Kick,
            color: [1.0, 1.0, 1.0],
        },
        Generator::Bands,
    ]
}

/// Right-click menu on a clip slot: choose the source and edit its settings.
fn clip_menu(ui: &mut egui::Ui, id: (usize, usize), slot: &mut Option<Clip>) {
    ui.set_min_width(240.0);
    ui.label("Source");
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
        Generator::Solid { color } => {
            ui.color_edit_button_rgb(color);
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
            ui.color_edit_button_rgb(color);
            ui.add(egui::Slider::new(width, 0.02..=1.0).text("width"));
            ui.add(egui::Slider::new(beats_per_cycle, 0.25..=16.0).text("beats / cycle"));
        }
        Generator::Strobe {
            color,
            per_beat,
            duty,
        } => {
            ui.color_edit_button_rgb(color);
            ui.add(egui::Slider::new(per_beat, 0.25..=8.0).text("flashes / beat"));
            ui.add(egui::Slider::new(duty, 0.05..=0.95).text("on time"));
        }
        Generator::AudioFlash { band, color } => {
            ui.horizontal(|ui| {
                ui.label("Follows");
                band_combo(ui, ("flashband", id), band, false);
                ui.color_edit_button_rgb(color);
            });
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
fn clip_cell(ui: &mut egui::Ui, clip: Option<&Clip>, active: bool, color: Color32, beat: f64, audio: &AudioBands) -> egui::Response {
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
            c.generator.render(&mut f, beat, audio);
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
                        let resp = clip_cell(ui, layer.clips[ci].as_ref(), active, color, b, &audio);
                        if resp.clicked() {
                            trigger = Some((li, ci));
                        }
                        resp.context_menu(|ui| clip_menu(ui, (li, ci), &mut layer.clips[ci]));
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
