//! Fixture Editor: define pixel grids and their channel layout.

use crate::View;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};
use lumen_fixtures::{color_order, ChannelFn, FixtureType, COLOR_ORDERS};

#[derive(Default)]
pub struct State {
    pub selected: usize,
}

fn channel_list(ui: &mut egui::Ui, id: &str, chans: &mut Vec<ChannelFn>) {
    let mut remove = None;
    for (i, c) in chans.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{:>2}", i + 1));
            egui::ComboBox::from_id_salt((id, i))
                .width(90.0)
                .selected_text(c.name())
                .show_ui(ui, |ui| {
                    for choice in ChannelFn::CHOICES {
                        if ui
                            .selectable_label(c.same_kind(choice), choice.name())
                            .clicked()
                            && !c.same_kind(choice)
                        {
                            *c = choice;
                        }
                    }
                });
            if let ChannelFn::Fixed(v) = c {
                ui.add(egui::DragValue::new(v).range(0..=255));
            }
            if ui.small_button("✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        chans.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.small_button("+ Channel").clicked() {
            chans.push(ChannelFn::Fixed(0));
        }
    });
}

/// Draw the pixel grid numbered in DMX order.
fn preview(ui: &mut egui::Ui, t: &FixtureType) {
    let (cols, rows) = (t.columns.max(1), t.rows.max(1));
    let avail = ui.available_width().min(380.0);
    let cell = (avail / cols as f32)
        .min(200.0 / rows as f32)
        .clamp(4.0, 40.0);
    let (resp, painter) = ui.allocate_painter(
        Vec2::new(cell * cols as f32, cell * rows as f32),
        Sense::hover(),
    );
    let per = t.pixel.len().max(1);
    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            let rr = Rect::from_min_size(
                resp.rect.min + Vec2::new(c as f32 * cell, r as f32 * cell),
                Vec2::splat(cell),
            );
            let hue = idx as f32 / (cols * rows) as f32;
            let [cr, cg, cb] = lumen_core::color::hsv_to_rgb(hue * 0.8, 0.5, 0.45);
            painter.rect_filled(
                rr.shrink(1.0),
                2.0,
                Color32::from_rgb((cr * 255.0) as u8, (cg * 255.0) as u8, (cb * 255.0) as u8),
            );
            painter.rect_stroke(
                rr.shrink(1.0),
                2.0,
                Stroke::new(1.0, Color32::from_gray(90)),
                egui::StrokeKind::Inside,
            );
            if cell >= 22.0 {
                painter.text(
                    rr.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", idx + 1),
                    egui::FontId::monospace(10.0),
                    Color32::WHITE,
                );
            }
        }
    }
    ui.weak(format!(
        "Pixel 1 = ch {}-{}, last pixel ends at ch {}",
        t.header.len() + 1,
        t.header.len() + per,
        t.footprint()
    ));
}

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    let View { e, s } = app;
    let o = &mut e.output;
    let st = &mut s.fe;
    egui::Panel::left("fe_left")
        .default_size(220.0)
        .resizable(true)
        .show(ui, |ui| {
            ui.heading("Fixture types");
            for (i, t) in o.types.iter().enumerate() {
                let used = o.fixtures.iter().filter(|f| f.type_id == i).count();
                let text = format!(
                    "{}  ({}×{}, {} ch)",
                    t.name,
                    t.columns,
                    t.rows,
                    t.footprint()
                );
                if ui
                    .selectable_label(st.selected == i, text)
                    .on_hover_text(format!("Used by {used} fixture(s)"))
                    .clicked()
                {
                    st.selected = i;
                }
            }
            ui.horizontal(|ui| {
                if ui.button("+ New").clicked() {
                    o.types.push(FixtureType::new("New fixture", 10, 1, "RGB"));
                    st.selected = o.types.len() - 1;
                }
                if ui
                    .add_enabled(st.selected < o.types.len(), egui::Button::new("Duplicate"))
                    .clicked()
                {
                    let mut t = o.types[st.selected].clone();
                    t.name.push_str(" copy");
                    o.types.push(t);
                    st.selected = o.types.len() - 1;
                }
                let in_use = o.type_in_use(st.selected);
                if ui
                    .add_enabled(
                        st.selected < o.types.len() && !in_use,
                        egui::Button::new("Delete"),
                    )
                    .on_disabled_hover_text("Still used by fixtures in Advanced Output")
                    .clicked()
                {
                    o.remove_type(st.selected);
                    st.selected = st.selected.saturating_sub(1);
                }
            });
        });
    egui::CentralPanel::default().show(ui, |ui| {
        let Some(t) = o.types.get_mut(st.selected) else {
            ui.label("No fixture types yet.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("fe")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut t.name);
                    ui.end_row();
                    ui.label("Pixels");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut t.columns)
                                .range(1..=1024)
                                .prefix("columns "),
                        );
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(&mut t.rows)
                                .range(1..=256)
                                .prefix("rows "),
                        );
                    });
                    ui.end_row();
                    ui.label("Colour order");
                    let current = t.order_string();
                    egui::ComboBox::from_id_salt("order")
                        .selected_text(current.clone())
                        .show_ui(ui, |ui| {
                            for o in COLOR_ORDERS {
                                if ui.selectable_label(current == o, o).clicked() {
                                    t.pixel = color_order(o);
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Footprint");
                    ui.label(format!(
                        "{} channels ({} per pixel × {} pixels{})",
                        t.footprint(),
                        t.pixel.len(),
                        t.pixel_count(),
                        if t.header.is_empty() {
                            String::new()
                        } else {
                            format!(" + {} header", t.header.len())
                        }
                    ));
                    ui.end_row();
                });
            if t.footprint() > 512 {
                ui.weak(
                    "Larger than one universe: pixels continue on the next universe automatically.",
                );
            }
            ui.separator();
            ui.columns(2, |cols| {
                cols[0].strong("Channels per pixel");
                channel_list(&mut cols[0], "pix", &mut t.pixel);
                cols[1].strong("Header channels (sent once)");
                channel_list(&mut cols[1], "hdr", &mut t.header);
            });
            ui.separator();
            ui.strong("Pixel order");
            preview(ui, t);
        });
    });
}
