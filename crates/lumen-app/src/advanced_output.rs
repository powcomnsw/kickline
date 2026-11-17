//! Advanced Output window: lumiverses on the left, the composition with
//! fixtures drawn on it in the middle, the selected fixture's settings below.

use crate::util::{paint_frame, to_canvas, to_color, to_screen};
use crate::View;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use lumen_fixtures::{AdvancedOutput, Fixture, Lumiverse, Protocol};

#[derive(Default)]
pub struct State {
    pub selected: Option<usize>,
    pub lumiverse: usize,
    drag: Option<Drag>,
    add_type: usize,
    show_pixels: bool,
}

#[derive(Clone, Copy)]
enum Drag {
    Move(usize),
    Resize(usize),
    Rotate(usize),
}

const HANDLE: f32 = 7.0;

fn handles(fx: &Fixture, aspect: f32, rect: Rect) -> (Pos2, Pos2) {
    let (u, v) = fx.to_canvas(aspect, 0.5, 0.5);
    let resize = to_screen(rect, u, v);
    let top = fx.to_canvas(aspect, 0.0, -0.5);
    let centre = to_screen(rect, fx.x, fx.y);
    let top = to_screen(rect, top.0, top.1);
    let dir = (top - centre).normalized();
    let rotate = top + dir * 18.0;
    (resize, rotate)
}

fn lumiverse_panel(o: &mut AdvancedOutput, st: &mut State, ui: &mut egui::Ui) {
    ui.heading("Lumiverses");
    let mut remove = None;
    for (i, l) in o.lumiverses.iter().enumerate() {
        ui.horizontal(|ui| {
            let used = o.fixtures.iter().filter(|f| f.lumiverse == i).count();
            let text = format!("{}  ·  U{}  ·  {}", l.name, l.universe, l.protocol.name());
            if ui
                .selectable_label(st.lumiverse == i, text)
                .on_hover_text(format!("{used} fixture(s)"))
                .clicked()
            {
                st.lumiverse = i;
            }
            if o.lumiverses.len() > 1 && ui.small_button("×").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        o.remove_lumiverse(i);
        st.lumiverse = st.lumiverse.min(o.lumiverses.len() - 1);
    }
    if ui.button("+ Lumiverse").clicked() {
        let next = o
            .lumiverses
            .iter()
            .map(|l| l.universe + 1)
            .max()
            .unwrap_or(0);
        o.lumiverses.push(Lumiverse::new(
            &format!("Lumiverse {}", o.lumiverses.len() + 1),
            next,
        ));
        st.lumiverse = o.lumiverses.len() - 1;
    }
    if let Some(l) = o.lumiverses.get_mut(st.lumiverse) {
        ui.add_space(4.0);
        egui::Grid::new("lv")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut l.name);
                ui.end_row();
                ui.label("Protocol");
                egui::ComboBox::from_id_salt("lvproto")
                    .selected_text(l.protocol.name())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut l.protocol, Protocol::ArtNet, "Art-Net");
                        ui.selectable_value(&mut l.protocol, Protocol::Sacn, "sACN");
                    });
                ui.end_row();
                ui.label("Universe");
                ui.add(egui::DragValue::new(&mut l.universe).range(0..=32767));
                ui.end_row();
                ui.label("Target IP");
                ui.add_enabled(
                    l.protocol == Protocol::ArtNet,
                    egui::TextEdit::singleline(&mut l.target).desired_width(130.0),
                );
                ui.end_row();
                ui.label("Enabled");
                ui.checkbox(&mut l.enabled, "");
                ui.end_row();
            });
        if l.protocol == Protocol::Sacn {
            ui.weak(format!(
                "Multicast to 239.255.{}.{}",
                (l.universe + 1) >> 8,
                (l.universe + 1) & 0xff
            ));
        }
    }

    ui.separator();
    ui.heading("Fixtures");
    let conflicts = o.conflicts();
    egui::ScrollArea::vertical()
        .id_salt("fxlist")
        .max_height(220.0)
        .show(ui, |ui| {
            for i in 0..o.fixtures.len() {
                let fx = &o.fixtures[i];
                let bad = conflicts.iter().any(|&(a, b)| a == i || b == i);
                let mut text = egui::RichText::new(format!("{}   {}", fx.name, o.range_label(fx)));
                if bad {
                    text = text.color(crate::theme::current().err);
                }
                if ui.selectable_label(st.selected == Some(i), text).clicked() {
                    st.selected = Some(i);
                    st.lumiverse = fx.lumiverse;
                }
            }
        });
    if !conflicts.is_empty() {
        ui.colored_label(
            crate::theme::current().err,
            format!("⚠ {} overlapping address range(s)", conflicts.len()),
        );
    }
    ui.horizontal(|ui| {
        st.add_type = st.add_type.min(o.types.len().saturating_sub(1));
        let name = o
            .types
            .get(st.add_type)
            .map(|t| t.name.as_str())
            .unwrap_or("no types");
        egui::ComboBox::from_id_salt("addtype")
            .width(140.0)
            .selected_text(name)
            .show_ui(ui, |ui| {
                for (i, t) in o.types.iter().enumerate() {
                    ui.selectable_value(&mut st.add_type, i, &t.name);
                }
            });
        if ui
            .add_enabled(
                !o.types.is_empty() && !o.lumiverses.is_empty(),
                egui::Button::new("+ Add"),
            )
            .clicked()
        {
            let t = &o.types[st.add_type];
            let strip = t.rows <= 1 && t.columns > 1;
            let fx = Fixture {
                name: format!("{} {}", t.name, o.fixtures.len() + 1),
                type_id: st.add_type,
                lumiverse: st.lumiverse,
                address: o.next_free_address(st.lumiverse),
                x: 0.5,
                y: 0.5,
                w: if strip { 0.6 } else { 0.15 },
                h: if strip { 0.04 } else { 0.2 },
                rotation: 0.0,
                snake: t.rows > 1,
                reverse: false,
            };
            o.fixtures.push(fx);
            st.selected = Some(o.fixtures.len() - 1);
        }
    });
    ui.horizontal(|ui| {
        let has = st.selected.is_some_and(|i| i < o.fixtures.len());
        if ui
            .add_enabled(has, egui::Button::new("Duplicate"))
            .clicked()
        {
            let mut f = o.fixtures[st.selected.unwrap()].clone();
            f.name.push_str(" copy");
            f.y = (f.y + 0.05).min(1.0);
            f.address = o.next_free_address(f.lumiverse);
            o.fixtures.push(f);
            st.selected = Some(o.fixtures.len() - 1);
        }
        if ui.add_enabled(has, egui::Button::new("Delete")).clicked() {
            o.fixtures.remove(st.selected.unwrap());
            st.selected = None;
        }
    });
}

fn fixture_props(o: &mut AdvancedOutput, idx: usize, ui: &mut egui::Ui) {
    let range = o.range_label(&o.fixtures[idx]);
    let types: Vec<String> = o.types.iter().map(|t| t.name.clone()).collect();
    let lvs: Vec<String> = o
        .lumiverses
        .iter()
        .map(|l| format!("{} (U{})", l.name, l.universe))
        .collect();
    let fx = &mut o.fixtures[idx];
    ui.horizontal_wrapped(|ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut fx.name).desired_width(140.0));
        ui.label("Type");
        egui::ComboBox::from_id_salt("fxtype")
            .selected_text(types.get(fx.type_id).cloned().unwrap_or_default())
            .show_ui(ui, |ui| {
                for (i, t) in types.iter().enumerate() {
                    ui.selectable_value(&mut fx.type_id, i, t);
                }
            });
        ui.label("Lumiverse");
        egui::ComboBox::from_id_salt("fxlv")
            .selected_text(lvs.get(fx.lumiverse).cloned().unwrap_or_default())
            .show_ui(ui, |ui| {
                for (i, l) in lvs.iter().enumerate() {
                    ui.selectable_value(&mut fx.lumiverse, i, l);
                }
            });
        ui.label("Address");
        ui.add(egui::DragValue::new(&mut fx.address).range(1..=512));
        ui.weak(range);
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("X");
        ui.add(
            egui::DragValue::new(&mut fx.x)
                .speed(0.002)
                .range(-0.5..=1.5)
                .max_decimals(3),
        );
        ui.label("Y");
        ui.add(
            egui::DragValue::new(&mut fx.y)
                .speed(0.002)
                .range(-0.5..=1.5)
                .max_decimals(3),
        );
        ui.label("W");
        ui.add(
            egui::DragValue::new(&mut fx.w)
                .speed(0.002)
                .range(0.005..=2.0)
                .max_decimals(3),
        );
        ui.label("H");
        ui.add(
            egui::DragValue::new(&mut fx.h)
                .speed(0.002)
                .range(0.005..=2.0)
                .max_decimals(3),
        );
        ui.label("Rotation");
        ui.add(
            egui::DragValue::new(&mut fx.rotation)
                .speed(0.5)
                .range(-360.0..=360.0)
                .max_decimals(1)
                .suffix("°"),
        );
        ui.checkbox(&mut fx.snake, "Snake");
        ui.checkbox(&mut fx.reverse, "Reverse");
    });
}

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    let View { e, s } = app;
    let (output, frame, st) = (&mut e.output, &e.frame, &mut s.ao);
    let aspect = output.aspect;
    egui::Panel::left("ao_left")
        .default_size(290.0)
        .resizable(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| lumiverse_panel(output, st, ui));
        });

    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Drag to move · corner handle to resize · round handle to rotate");
            ui.checkbox(&mut st.show_pixels, "Show pixel colours");
        });
        let avail = ui.available_size() - Vec2::new(0.0, 70.0);
        let w = avail.x.min(avail.y.max(100.0) * aspect);
        let (resp, painter) =
            ui.allocate_painter(Vec2::new(w, w / aspect), Sense::click_and_drag());
        let rect = resp.rect;
        paint_frame(&painter, rect, frame);
        painter.rect_filled(
            rect,
            0.0,
            Color32::from_black_alpha(if st.show_pixels { 160 } else { 60 }),
        );

        // --- interaction
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.drag_started() || resp.clicked() {
                // Hit-test where the button went down, not where the drag was
                // recognised: thin strips are easy to slip off otherwise.
                let pos = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                let (u, v) = to_canvas(rect, pos);
                st.drag = None;
                if let Some(i) = st.selected.filter(|&i| i < output.fixtures.len()) {
                    let (rs, rot) = handles(&output.fixtures[i], aspect, rect);
                    if rs.distance(pos) < HANDLE * 1.6 {
                        st.drag = Some(Drag::Resize(i));
                    } else if rot.distance(pos) < HANDLE * 1.6 {
                        st.drag = Some(Drag::Rotate(i));
                    }
                }
                if st.drag.is_none() {
                    let hit = (0..output.fixtures.len())
                        .rev()
                        .find(|&i| output.fixtures[i].contains(aspect, u, v));
                    st.selected = hit;
                    if let Some(i) = hit {
                        st.lumiverse = output.fixtures[i].lumiverse;
                        st.drag = Some(Drag::Move(i));
                    }
                }
            }
            if resp.dragged() {
                let (u, v) = to_canvas(rect, pos);
                match st.drag {
                    Some(Drag::Move(i)) => {
                        let d = resp.drag_delta();
                        let fx = &mut output.fixtures[i];
                        fx.x += d.x / rect.width();
                        fx.y += d.y / rect.height();
                    }
                    Some(Drag::Resize(i)) => {
                        let fx = &mut output.fixtures[i];
                        let a = fx.rotation.to_radians();
                        let (dx, dy) = ((u - fx.x) * aspect, v - fx.y);
                        let (rx, ry) = (dx * a.cos() + dy * a.sin(), -dx * a.sin() + dy * a.cos());
                        fx.w = (2.0 * rx.abs() / aspect).max(0.005);
                        fx.h = (2.0 * ry.abs()).max(0.005);
                    }
                    Some(Drag::Rotate(i)) => {
                        let fx = &mut output.fixtures[i];
                        let (dx, dy) = ((u - fx.x) * aspect, v - fx.y);
                        let mut deg = dx.atan2(-dy).to_degrees();
                        if ui.input(|i| i.modifiers.shift) {
                            deg = (deg / 15.0).round() * 15.0;
                        }
                        fx.rotation = deg;
                    }
                    None => {}
                }
            }
        }
        if resp.drag_stopped() {
            st.drag = None;
        }

        // --- drawing
        let conflicts = output.conflicts();
        for (i, fx) in output.fixtures.iter().enumerate() {
            let Some(t) = output.types.get(fx.type_id) else {
                continue;
            };
            let sel = st.selected == Some(i);
            let bad = conflicts.iter().any(|&(a, b)| a == i || b == i);
            let pts: Vec<Pos2> = fx
                .corners(aspect)
                .iter()
                .map(|&(u, v)| to_screen(rect, u, v))
                .collect();
            let col = if bad {
                crate::theme::current().err
            } else if sel {
                crate::theme::current().accent
            } else {
                Color32::from_white_alpha(170)
            };
            painter.add(egui::Shape::convex_polygon(
                pts.clone(),
                Color32::from_black_alpha(40),
                Stroke::NONE,
            ));
            painter.add(egui::Shape::closed_line(
                pts.clone(),
                Stroke::new(if sel { 2.0 } else { 1.0 }, col),
            ));
            let positions = fx.pixel_positions(aspect, t);
            let cell = (rect.width() * fx.w / t.columns.max(1) as f32)
                .min(rect.height() * fx.h / t.rows.max(1) as f32);
            let r = (cell * 0.35).clamp(1.2, 6.0);
            for (k, (lx, ly)) in fx.pixel_local(t).into_iter().enumerate() {
                let p = to_screen(rect, positions[k].0, positions[k].1);
                if st.show_pixels {
                    let c = fx.sample_cell(aspect, t, frame, lx, ly);
                    painter.circle_filled(p, r, to_color(c));
                } else {
                    painter.circle_stroke(p, r, Stroke::new(1.0, col.gamma_multiply(0.8)));
                }
            }
            // first pixel marker shows wiring start
            if let Some(&(u, v)) = positions.first() {
                painter.circle_filled(
                    to_screen(rect, u, v),
                    (r * 0.6).max(1.5),
                    crate::theme::current().accent,
                );
            }
            let label_pos = pts[0] + Vec2::new(2.0, -14.0);
            painter.text(
                label_pos,
                egui::Align2::LEFT_TOP,
                &fx.name,
                egui::FontId::proportional(11.0),
                col,
            );
            if sel {
                let (rs, rot) = handles(fx, aspect, rect);
                painter.rect_filled(
                    Rect::from_center_size(rs, Vec2::splat(HANDLE * 1.6)),
                    2.0,
                    col,
                );
                let top = fx.to_canvas(aspect, 0.0, -0.5);
                painter.line_segment([to_screen(rect, top.0, top.1), rot], Stroke::new(1.0, col));
                painter.circle_filled(rot, HANDLE, col);
            }
        }

        ui.add_space(4.0);
        match st.selected.filter(|&i| i < output.fixtures.len()) {
            Some(i) => fixture_props(output, i, ui),
            None => {
                ui.weak("Click a fixture to edit it, or add one from the list on the left.");
            }
        }
    });
}
