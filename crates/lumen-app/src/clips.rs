use crate::util::band_combo;
use crate::View;
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
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        ui.checkbox(&mut layer.bypass, "Bypass");
        ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0).text("Opacity"));
        egui::ComboBox::from_id_salt(("blend", li))
            .width(80.0)
            .selected_text(layer.blend.name())
            .show_ui(ui, |ui| {
                for m in BlendMode::ALL {
                    ui.selectable_value(&mut layer.blend, m, m.name());
                }
            });
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        ui.label("🔊 Opacity follows");
        let mut on = layer.audio_opacity.is_some();
        let mut link = layer.audio_opacity.unwrap_or(AudioLink {
            band: AudioBand::Bass,
            amount: 1.0,
        });
        ui.checkbox(&mut on, "");
        ui.add_enabled_ui(on, |ui| {
            band_combo(ui, ("oplink", li), &mut link.band, false);
            ui.add(egui::Slider::new(&mut link.amount, 0.0..=1.0).text("amount"));
        });
        layer.audio_opacity = on.then_some(link);
        ui.separator();
        ui.label("Next clip on");
        let label = layer.advance_on.map(|b| b.name()).unwrap_or("Off");
        egui::ComboBox::from_id_salt(("adv", li))
            .width(70.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut layer.advance_on, None, "Off");
                for b in AudioBand::HITS {
                    ui.selectable_value(&mut layer.advance_on, Some(b), b.name());
                }
            });
    });
}

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Clips");
        ui.weak("right-click a clip to change it");
    });
    let mut trigger: Option<(usize, usize)> = None;
    let mut column: Option<usize> = None;
    let mut stop: Option<usize> = None;
    let mut remove_layer: Option<usize> = None;
    let cols = app.e.comp.columns;
    egui::ScrollArea::both().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(118.0);
            for c in 0..cols {
                if ui
                    .add_sized([96.0, 22.0], egui::Button::new(format!("▶ Col {}", c + 1)))
                    .clicked()
                {
                    column = Some(c);
                }
            }
        });
        for li in 0..app.e.comp.layers.len() {
            let layer = &mut app.e.comp.layers[li];
            ui.horizontal(|ui| {
                ui.add_sized(
                    [110.0, 30.0],
                    egui::TextEdit::singleline(&mut layer.name).frame(egui::Frame::NONE),
                );
                for ci in 0..layer.clips.len() {
                    let active = layer.active == Some(ci);
                    let text = layer.clips[ci]
                        .as_ref()
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "—".into());
                    let resp = ui.add_sized([96.0, 30.0], egui::Button::new(text).selected(active));
                    if resp.clicked() {
                        trigger = Some((li, ci));
                    }
                    resp.context_menu(|ui| clip_menu(ui, (li, ci), &mut layer.clips[ci]));
                }
                if ui.button("■").on_hover_text("Stop layer").clicked() {
                    stop = Some(li);
                }
                if ui.small_button("✕").on_hover_text("Remove layer").clicked() {
                    remove_layer = Some(li);
                }
            });
            layer_controls(ui, li, layer);
            ui.collapsing(
                egui::RichText::new(format!("{} effects ({})", layer.name, layer.effects.len()))
                    .small(),
                |ui| {
                    let mut remove = None;
                    for (ei, e) in layer.effects.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            effect_row(ui, (li, ei), e);
                            if ui.small_button("✕").clicked() {
                                remove = Some(ei);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        layer.effects.remove(i);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button("+ Brightness").clicked() {
                            layer.effects.push(Effect::Brightness(1.0));
                        }
                        if ui.small_button("+ Hue").clicked() {
                            layer.effects.push(Effect::HueShift(0.0));
                        }
                        if ui.small_button("+ Invert").clicked() {
                            layer.effects.push(Effect::Invert);
                        }
                        if ui.small_button("+ Pulse").clicked() {
                            layer.effects.push(Effect::Pulse {
                                depth: 0.8,
                                per_beat: 1.0,
                            });
                        }
                        if ui.small_button("+ Audio brightness").clicked() {
                            layer.effects.push(Effect::AudioBrightness {
                                band: AudioBand::Bass,
                                amount: 1.0,
                            });
                        }
                        if ui.small_button("+ Audio hue").clicked() {
                            layer.effects.push(Effect::AudioHue {
                                band: AudioBand::HighMid,
                                amount: 0.5,
                            });
                        }
                    });
                },
            );
            ui.separator();
        }
        if ui.button("+ Layer").clicked() {
            let n = app.e.comp.layers.len() + 1;
            app.e
                .comp
                .layers
                .insert(0, Layer::new(format!("Layer {n}"), cols));
        }
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
