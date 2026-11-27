//! Auto mode controls: the card on the Play screen and the Set up → Auto page.
//!
//! The director itself lives in `lumen_core::director`; this is only the
//! buttons, the genre and mood settings, what Auto may use, and a readout of
//! what it is showing.

use crate::{stage, theme, View};
use eframe::egui::{self, Color32, RichText, Vec2};
use lumen_core::{playbook, scenes, Genre, Section};

/// Add `name` to `list` when it is switched off, take it out when it is on.
fn allow(list: &mut Vec<String>, name: &str, on: bool) {
    let present = list.iter().any(|n| n == name);
    if on && present {
        list.retain(|n| n != name);
    } else if !on && !present {
        list.push(name.to_string());
    }
}

fn genre_combo(v: &mut View, ui: &mut egui::Ui) {
    let genre = &mut v.e.comp.director.genre;
    egui::ComboBox::from_id_salt("auto_genre").selected_text(genre.name()).width(140.0).show_ui(ui, |ui| {
        for g in Genre::ALL {
            ui.selectable_value(genre, g, g.name()).on_hover_text(g.note());
        }
    });
}

pub fn toggle_auto(v: &mut View) {
    v.e.comp.director.on = !v.e.comp.director.on;
    v.e.comp.director_rt.hold = false;
}

/// The Auto card on the Play screen.
pub fn card(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let on = v.e.comp.director.on;
    ui.horizontal(|ui| {
        let b = egui::Button::new(RichText::new(if on { "AUTO ON" } else { "AUTO OFF" }).font(theme::bold(t.text_size + 1.0)).color(if on { t.on_accent } else { t.weak }))
            .fill(if on { t.accent } else { t.raised })
            .min_size(Vec2::new(100.0, 30.0));
        if ui.add(b).on_hover_text("Kickline picks effects, colours and changes by itself, following the music (A)").clicked() {
            toggle_auto(v);
        }
        let hold = v.e.comp.director_rt.hold;
        let b = egui::Button::new(RichText::new("HOLD").font(theme::bold(t.text_size)).color(if hold { Color32::BLACK } else { t.weak }))
            .fill(if hold { t.warn } else { t.raised })
            .min_size(Vec2::new(60.0, 30.0));
        if ui.add_enabled(on, b).on_hover_text("Freeze the look Auto is showing now (H)").clicked() {
            v.e.comp.director_rt.hold = !hold;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Auto settings…").clicked() {
                v.s.mode = crate::Mode::Setup;
                v.s.page = crate::Page::Auto;
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        genre_combo(v, ui);
        ui.checkbox(&mut v.e.comp.director.flash_safe, "Flash-safe").on_hover_text("Never pick strobing effects");
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Calm").color(t.weak).size(t.text_size - 2.0));
        ui.spacing_mut().slider_width = 150.0;
        ui.add(egui::Slider::new(&mut v.e.comp.director.energy, 0.0..=1.0).show_value(false)).on_hover_text("How energetic Auto is overall");
        ui.label(RichText::new("Wild").color(t.weak).size(t.text_size - 2.0));
    });
    if !on {
        ui.label(RichText::new("Auto runs its own layers under yours. Turn it on and Kickline follows the music.").color(t.weak).size(t.text_size - 2.0));
        return;
    }
    ui.add_space(4.0);
    let d = &v.e.comp.director_rt;
    let sec = v.e.comp.sections.current();
    let listening = v.e.comp.sections.listening();
    ui.horizontal(|ui| {
        let (txt, col) = if listening { (sec.name(), stage::section_color(sec)) } else { ("Waiting for music…", t.weak) };
        ui.label(RichText::new("●").color(col));
        ui.label(RichText::new(txt).color(col));
        ui.label(RichText::new(format!("energy {:.0}% · sure {:.0}%", d.energy * 100.0, v.e.comp.sections.reading.confidence * 100.0)).color(t.weak).size(t.text_size - 2.0));
    });
    if !d.look_name.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Look").color(t.weak).size(t.text_size - 2.0));
            ui.label(RichText::new(look_title(&d.look_name)).size(t.text_size - 1.0));
        });
    }
    for (what, name) in &d.now {
        ui.horizontal(|ui| {
            ui.label(RichText::new(what).color(t.weak).size(t.text_size - 2.0));
            ui.label(RichText::new(if name.is_empty() { "…" } else { name }).size(t.text_size - 1.0));
        });
    }
    if !d.palette_name.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Palette").color(t.weak).size(t.text_size - 2.0));
            ui.label(RichText::new(&d.palette_name).size(t.text_size - 1.0));
        });
    }
    if d.hold {
        ui.colored_label(t.warn, "Held: Auto keeps this look until you release it");
    }
}

/// Set up → Auto.
pub fn page(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        section_box(ui, "Auto mode", |ui| {
            ui.label(RichText::new("Kickline runs a wash, some motion and accents of its own, chosen from the effects and colours you allow below. It changes on the bar, crossfades, and follows the song: calm in a breakdown, building up, then full power on the drop. Your own layers always sit on top.").color(t.weak).size(t.text_size - 1.0));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut v.e.comp.director.on, "Auto on");
                ui.label(RichText::new("(A on the keyboard)").color(t.weak).size(t.text_size - 2.0));
            });
        });
        section_box(ui, "Style of music", |ui| {
            ui.label(RichText::new("This tells Kickline what a groove looks like (drum & bass and hip-hop have two kicks a bar, not four) and how fast to change.").color(t.weak).size(t.text_size - 2.0));
            ui.add_space(4.0);
            for g in Genre::ALL {
                let sel = v.e.comp.director.genre == g;
                ui.horizontal(|ui| {
                    if ui.selectable_label(sel, RichText::new(g.name()).font(theme::bold(t.text_size))).clicked() {
                        v.e.comp.director.genre = g;
                    }
                    ui.label(RichText::new(g.note()).color(t.weak).size(t.text_size - 2.0));
                });
            }
        });
        section_box(ui, "Mood and pace", |ui| {
            ui.horizontal(|ui| {
                ui.label("Energy");
                ui.spacing_mut().slider_width = 200.0;
                ui.add(egui::Slider::new(&mut v.e.comp.director.energy, 0.0..=1.0).custom_formatter(|x, _| if x < 0.34 { "calm".into() } else if x < 0.67 { "normal".into() } else { "wild".into() }));
            });
            ui.horizontal(|ui| {
                ui.label("Change every");
                ui.add(egui::DragValue::new(&mut v.e.comp.director.change_bars).range(1..=32).suffix(" bars"));
                ui.label(RichText::new("for motion; washes stay twice as long, accents half").color(t.weak).size(t.text_size - 2.0));
            });
            ui.horizontal(|ui| {
                ui.label("When you play a layer, Auto drops to");
                ui.spacing_mut().slider_width = 120.0;
                ui.add(egui::Slider::new(&mut v.e.comp.director.under_manual, 0.0..=1.0).custom_formatter(|x, _| format!("{:.0}%", x * 100.0)));
            });
            ui.checkbox(&mut v.e.comp.director.flash_safe, "Flash-safe: never use strobing effects");
            ui.checkbox(&mut v.e.comp.director.palettes, "Let Auto change the colour palette");
            ui.checkbox(&mut v.e.comp.director.show_mode, "Show mode: keep output and Auto on all night, and keep looking for the music")
                .on_hover_text("Saved with the show. When the app opens it switches output and Auto on and keeps trying to find your last music source (for example rekordbox) until it does.");
        });
        let groups = v.e.output.group_names();
        section_box(ui, "Groups", |ui| {
            if groups.is_empty() {
                ui.label(RichText::new("No groups yet, so Auto's motion covers all your lights. Make groups in Set up → Stage and each one gets its own motion.").color(t.weak).size(t.text_size - 2.0));
            }
            for g in &groups {
                let mut on = !v.e.comp.director.off_groups.contains(g);
                if ui.checkbox(&mut on, g).on_hover_text("Untick to keep Auto's motion off this group").changed() {
                    allow(&mut v.e.comp.director.off_groups, g, on);
                }
            }
        });
        section_box(ui, "Looks for each part of a song", |ui| {
            ui.label(RichText::new("Auto plays these in a shuffled order: every look once before any repeats. Untick the ones you don't want. Preview plays one now.").color(t.weak).size(t.text_size - 3.0));
            for s in Section::ALL {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(stage::section_color(s)));
                    ui.label(RichText::new(s.name()).font(theme::bold(t.text_size)));
                    ui.label(RichText::new(s.note()).color(t.weak).size(t.text_size - 3.0));
                });
                for l in playbook::looks_for(s) {
                    let mut on = !v.e.comp.director.off_looks.iter().any(|o| o == l.id);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, "").changed() {
                            allow(&mut v.e.comp.director.off_looks, l.id, on);
                        }
                        let now = v.e.comp.director_rt.look_name == l.id;
                        left_label(ui, 150.0, RichText::new(look_title(l.id)).color(if now { t.accent } else if on { t.text } else { t.weak }));
                        let moves: Vec<&str> = l.moves().map(|m| m.effect).collect();
                        ui.label(RichText::new(if moves.is_empty() { "dark".to_string() } else { moves.join(" + ") }).color(t.weak).size(t.text_size - 3.0));
                        if l.strobe() {
                            ui.colored_label(t.warn, "strobe");
                        }
                        if ui.small_button("Preview").clicked() {
                            if !v.e.comp.director.on {
                                toggle_auto(v);
                            }
                            v.e.comp.director_rt.preview = Some(l.id);
                        }
                    });
                }
            }
        });
        section_box(ui, "Effects Auto may use", |ui| {
            for (cat, note, list) in scenes::catalog() {
                ui.label(RichText::new(cat).font(theme::bold(t.text_size)));
                ui.label(RichText::new(note).color(t.weak).size(t.text_size - 3.0));
                for g in list {
                    let name = scenes::name(&g);
                    let tags = scenes::tags_for(&name);
                    let mut on = !v.e.comp.director.off_effects.contains(&name);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, "").changed() {
                            allow(&mut v.e.comp.director.off_effects, &name, on);
                        }
                        left_label(ui, 150.0, RichText::new(&name).color(if on { t.text } else { t.weak }));
                        ui.label(RichText::new(tags.role.name()).color(t.weak).size(t.text_size - 3.0));
                        energy_bar(ui, tags.energy);
                        for s in Section::ALL {
                            let col = if tags.suits(s) { stage::section_color(s) } else { t.line };
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                            ui.painter().circle_filled(r.center(), 3.5, col);
                        }
                        if tags.strobe {
                            ui.colored_label(t.warn, "strobe");
                        }
                    });
                }
                ui.add_space(4.0);
            }
            ui.label(RichText::new("Bar: how energetic the effect is. Dots: the parts of a song it suits (groove, melodic, build-up, pause, drop).").color(t.weak).size(t.text_size - 3.0));
        });
        let pals: Vec<(String, [[f32; 3]; 4])> = v.e.comp.palettes.list.iter().map(|p| (p.name.clone(), p.colors)).collect();
        section_box(ui, "Palettes Auto may use", |ui| {
            for (name, colors) in &pals {
                let mut on = !v.e.comp.director.off_palettes.contains(name);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, "").changed() {
                        allow(&mut v.e.comp.director.off_palettes, name, on);
                    }
                    left_label(ui, 110.0, RichText::new(name).color(if on { t.text } else { t.weak }));
                    for c in colors {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(26.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 3.0, Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8));
                    }
                    energy_bar(ui, scenes::palette_energy(name));
                });
            }
        });
    });
}

/// "groove-bass-wash" as "Bass wash".
fn look_title(id: &str) -> String {
    let rest = id.split_once('-').map_or(id, |(_, r)| r).replace('-', " ");
    let mut c = rest.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// A label of a fixed width, text to the left.
fn left_label(ui: &mut egui::Ui, width: f32, text: RichText) {
    ui.allocate_ui_with_layout(Vec2::new(width, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(text);
    });
}

fn energy_bar(ui: &mut egui::Ui, range: (f32, f32)) {
    let t = theme::current();
    let (r, _) = ui.allocate_exact_size(Vec2::new(60.0, 8.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, t.raised);
    let a = r.min.x + r.width() * range.0;
    let b = r.min.x + r.width() * range.1;
    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(a, r.min.y), egui::pos2(b.max(a + 2.0), r.max.y)), 2.0, t.accent);
}

fn section_box(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = theme::current();
    egui::Frame::new().fill(t.panel).corner_radius(t.radius + 2).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).font(theme::bold(t.text_size + 1.0)));
        ui.add_space(4.0);
        add(ui);
    });
    ui.add_space(8.0);
}
