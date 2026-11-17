//! Live view of the 512 channels of one universe.

use crate::View;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};

#[derive(Default)]
pub struct State {
    pub key: Option<(usize, u16)>,
}

pub fn ui(view: &mut View, ui: &mut egui::Ui) {
    let View { e: app, s } = view;
    let keys: Vec<(usize, u16)> = app.dmx.keys().copied().collect();
    if keys.is_empty() {
        ui.label("Nothing patched yet.");
        return;
    }
    let st = &mut s.dmx_view;
    if st.key.is_none_or(|k| !keys.contains(&k)) {
        st.key = Some(keys[0]);
    }
    let name = |k: (usize, u16)| {
        let lv = app
            .output
            .lumiverses
            .get(k.0)
            .map(|l| l.name.as_str())
            .unwrap_or("?");
        format!("{lv} — universe {}", k.1)
    };
    ui.horizontal(|ui| {
        ui.label("Universe");
        egui::ComboBox::from_id_salt("dmxkey")
            .width(220.0)
            .selected_text(name(st.key.unwrap()))
            .show_ui(ui, |ui| {
                for k in &keys {
                    ui.selectable_value(&mut st.key, Some(*k), name(*k));
                }
            });
        ui.weak(if app.output_enabled {
            "live, sending"
        } else {
            "preview, DMX out is off"
        });
    });

    let key = st.key.unwrap();
    let data = app.dmx[&key];

    // Which fixture owns each channel.
    let mut owner: Vec<Option<usize>> = vec![None; 512];
    if let Some(lv) = app.output.lumiverses.get(key.0) {
        for (i, fx) in app
            .output
            .fixtures
            .iter()
            .enumerate()
            .filter(|(_, f)| f.lumiverse == key.0)
        {
            for s in app.output.layout(fx) {
                if lv.universe + s.universe_offset == key.1 {
                    for ch in s.start..s.start + s.len {
                        owner[ch] = Some(i);
                    }
                }
            }
        }
    }

    let cols = 32;
    let cell = ((ui.available_width() - 4.0) / cols as f32).clamp(14.0, 28.0);
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(cell * cols as f32, cell * 16.0), Sense::hover());
    let hover = resp.hover_pos();
    let mut hovered = None;
    for ch in 0..512 {
        let (c, r) = (ch % cols, ch / cols);
        let rr = Rect::from_min_size(
            resp.rect.min + Vec2::new(c as f32 * cell, r as f32 * cell),
            Vec2::splat(cell),
        );
        let v = data[ch];
        let bg = if owner[ch].is_some() {
            Color32::from_rgb(v / 2 + 20, v / 2 + 20, (v as u16 * 3 / 4) as u8 + 30)
        } else {
            Color32::from_gray(22)
        };
        painter.rect_filled(rr.shrink(0.5), 1.0, bg);
        // start of each fixture
        if owner[ch].is_some() && (ch == 0 || owner[ch - 1] != owner[ch]) {
            painter.line_segment(
                [rr.left_top(), rr.left_bottom()],
                Stroke::new(2.0, crate::theme::current().accent),
            );
        }
        if cell >= 20.0 && owner[ch].is_some() {
            let tc = if v > 150 {
                Color32::BLACK
            } else {
                Color32::from_gray(220)
            };
            painter.text(
                rr.center(),
                egui::Align2::CENTER_CENTER,
                v.to_string(),
                egui::FontId::monospace(8.5),
                tc,
            );
        }
        if hover.is_some_and(|p| rr.contains(p)) {
            hovered = Some(ch);
            painter.rect_stroke(
                rr,
                1.0,
                Stroke::new(1.5, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
    }
    ui.add_space(4.0);
    match hovered {
        Some(ch) => {
            let who = owner[ch]
                .map(|i| app.output.fixtures[i].name.as_str())
                .unwrap_or("unpatched");
            ui.label(format!("Channel {}  =  {}   ({who})", ch + 1, data[ch]));
        }
        None => {
            let used = owner.iter().filter(|o| o.is_some()).count();
            ui.weak(format!("{used} of 512 channels patched. Green bars mark where each fixture starts. Hover for details."));
        }
    }
}
