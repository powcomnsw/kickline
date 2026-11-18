//! Fixture Editor: define pixel grids and their channel layout, or import
//! fixtures from the library (QLC+ `.qxf`, built-in or your own files).

use crate::{log, paths, View};
use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, Vec2};
use lumen_fixtures::library::{Library, Pin};
use lumen_fixtures::qxf::QxfFixture;
use lumen_fixtures::{color_order, ChannelFn, Fixture, FixtureType, Slot, COLOR_ORDERS};

#[derive(Default)]
pub struct State {
    pub selected: usize,
    /// Show the library browser instead of the type editor.
    pub browsing: bool,
    pub library: Option<Library>,
    pub query: String,
    pub entry: Option<usize>,
    pub opened: Option<Result<QxfFixture, String>>,
    pub mode: usize,
    pub message: Option<String>,
    /// Hide fixtures with no colour or dimmer channels.
    pub drivable_only: bool,
}

impl State {
    /// Load the library on first use (unpacks ~16 MB, takes a moment).
    pub fn library(&mut self) -> &mut Library {
        self.library.get_or_insert_with(|| {
            let lib = Library::load(&paths::fixture_folders());
            log::info("fixtures", format!("library loaded: {} fixtures", lib.entries.len()));
            for e in &lib.errors {
                log::warn("fixtures", e.clone());
            }
            lib
        })
    }

    pub fn select_entry(&mut self, i: usize) {
        self.entry = Some(i);
        self.mode = 0;
        self.opened = Some(self.library().open(i));
    }

    /// Import a file dropped on the window: copy it into ~/Documents/Kickline/Fixtures and show it.
    pub fn import_dropped(&mut self, path: &std::path::Path) {
        let dest = paths::fixtures_dir().join(path.file_name().unwrap_or_default());
        let target = if path.starts_with(paths::fixtures_dir()) {
            path.to_path_buf()
        } else {
            match std::fs::copy(path, &dest) {
                Ok(_) => dest,
                Err(e) => {
                    log::warn("fixtures", format!("couldn't copy {} to your Fixtures folder: {e}", path.display()));
                    path.to_path_buf()
                }
            }
        };
        let added = self.library().add_file(&target);
        match added {
            Some(i) => {
                self.browsing = true;
                self.query.clear();
                self.select_entry(i);
                log::info("fixtures", format!("imported {}", target.display()));
            }
            None => {
                let why = self.library().errors.last().cloned().unwrap_or_default();
                self.message = Some(format!("Couldn't import {}: {why}", path.display()));
                log::error("fixtures", self.message.clone().unwrap());
            }
        }
    }
}

fn channel_list(ui: &mut egui::Ui, id: &str, chans: &mut Vec<ChannelFn>) {
    let mut remove = None;
    for (i, c) in chans.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{:>2}", i + 1));
            func_combo(ui, (id, i), c);
            if ui.small_button("×").clicked() {
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

fn func_combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, c: &mut ChannelFn) {
    egui::ComboBox::from_id_salt(id).width(90.0).selected_text(c.name()).show_ui(ui, |ui| {
        for choice in ChannelFn::CHOICES {
            if ui.selectable_label(c.same_kind(choice), choice.name()).clicked() && !c.same_kind(choice) {
                *c = choice;
            }
        }
    });
    if let ChannelFn::Fixed(v) = c {
        ui.add(egui::DragValue::new(v).range(0..=255));
    }
}

/// What Kickline does with a channel, in words.
fn describe(func: ChannelFn, fine: bool) -> (String, bool) {
    match func {
        ChannelFn::Fixed(v) => (format!("held at {v}"), false),
        f if fine => (format!("{} (fine)", f.name()), true),
        f => (f.name().to_string(), true),
    }
}

/// Imported fixture: its exact channel list, editable per channel.
fn slot_table(ui: &mut egui::Ui, slots: &mut [Slot]) {
    egui::Grid::new("slots").num_columns(4).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
        ui.weak("ch");
        ui.weak("name");
        ui.weak("pixel");
        ui.weak("Kickline drives it as");
        ui.end_row();
        for (i, s) in slots.iter_mut().enumerate() {
            ui.monospace(format!("{:>3}", i + 1));
            ui.label(&s.name);
            ui.label(s.pixel.map(|p| (p + 1).to_string()).unwrap_or_else(|| "all".into()));
            ui.horizontal(|ui| {
                func_combo(ui, ("slot", i), &mut s.func);
                if !matches!(s.func, ChannelFn::Fixed(_)) {
                    ui.checkbox(&mut s.fine, "fine");
                }
            });
            ui.end_row();
        }
    });
}

/// Draw the pixel grid numbered in DMX order.
fn preview(ui: &mut egui::Ui, t: &FixtureType) {
    let (cols, rows) = (t.columns.max(1), t.rows.max(1));
    let avail = ui.available_width().min(380.0);
    let cell = (avail / cols as f32).min(200.0 / rows as f32).clamp(4.0, 40.0);
    let (resp, painter) = ui.allocate_painter(Vec2::new(cell * cols as f32, cell * rows as f32), Sense::hover());
    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            let rr = Rect::from_min_size(resp.rect.min + Vec2::new(c as f32 * cell, r as f32 * cell), Vec2::splat(cell));
            let hue = idx as f32 / (cols * rows) as f32;
            let [cr, cg, cb] = lumen_core::color::hsv_to_rgb(hue * 0.8, 0.5, 0.45);
            painter.rect_filled(rr.shrink(1.0), 2.0, Color32::from_rgb((cr * 255.0) as u8, (cg * 255.0) as u8, (cb * 255.0) as u8));
            painter.rect_stroke(rr.shrink(1.0), 2.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
            if cell >= 22.0 {
                painter.text(rr.center(), egui::Align2::CENTER_CENTER, format!("{}", idx + 1), egui::FontId::monospace(10.0), Color32::WHITE);
            }
        }
    }
    if t.slots.is_none() {
        let per = t.pixel.len().max(1);
        ui.weak(format!("Pixel 1 = ch {}-{}, last pixel ends at ch {}", t.header.len() + 1, t.header.len() + per, t.footprint()));
    }
}

fn type_editor(ui: &mut egui::Ui, t: &mut FixtureType) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("fe").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut t.name);
            ui.end_row();
            if let Some(src) = &t.source {
                ui.label("Imported from");
                ui.weak(src);
                ui.end_row();
            }
            ui.label("Pixels");
            if t.slots.is_some() {
                ui.label(format!("{} × {}", t.columns, t.rows));
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut t.columns).range(1..=1024).prefix("columns "));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut t.rows).range(1..=256).prefix("rows "));
                });
            }
            ui.end_row();
            if t.slots.is_none() {
                ui.label("Colour order");
                let current = t.order_string();
                egui::ComboBox::from_id_salt("order").selected_text(current.clone()).show_ui(ui, |ui| {
                    for o in COLOR_ORDERS {
                        if ui.selectable_label(current == o, o).clicked() {
                            t.pixel = color_order(o);
                        }
                    }
                });
                ui.end_row();
            }
            ui.label("Footprint");
            ui.label(match &t.slots {
                Some(_) => format!("{} channels", t.footprint()),
                None => format!(
                    "{} channels ({} per pixel × {} pixels{})",
                    t.footprint(),
                    t.pixel.len(),
                    t.pixel_count(),
                    if t.header.is_empty() { String::new() } else { format!(" + {} header", t.header.len()) }
                ),
            });
            ui.end_row();
        });
        if t.footprint() > 512 {
            ui.weak("Larger than one universe: pixels continue on the next universe automatically.");
        }
        ui.separator();
        match &mut t.slots {
            Some(slots) => {
                ui.strong("Channels");
                ui.weak("Colour and dimmer channels follow the image. Others are held at a fixed value you can change.");
                slot_table(ui, slots);
            }
            None => {
                ui.columns(2, |cols| {
                    cols[0].strong("Channels per pixel");
                    channel_list(&mut cols[0], "pix", &mut t.pixel);
                    cols[1].strong("Header channels (sent once)");
                    channel_list(&mut cols[1], "hdr", &mut t.header);
                });
            }
        }
        ui.separator();
        ui.strong("Pixel order");
        preview(ui, t);
    });
}

/// Colour for each kind of channel in the pinout strip.
pub fn pin_color(p: Pin) -> Color32 {
    match p {
        Pin::Dimmer => Color32::from_gray(200),
        Pin::Red => Color32::from_rgb(230, 60, 60),
        Pin::Green => Color32::from_rgb(60, 200, 90),
        Pin::Blue => Color32::from_rgb(70, 120, 240),
        Pin::White => Color32::from_gray(250),
        Pin::Amber => Color32::from_rgb(255, 170, 40),
        Pin::Uv => Color32::from_rgb(150, 70, 255),
        Pin::Cmy => Color32::from_rgb(0, 200, 200),
        Pin::Colour => Color32::from_rgb(230, 110, 200),
        Pin::Pan => Color32::from_rgb(40, 150, 140),
        Pin::Tilt => Color32::from_rgb(30, 110, 160),
        Pin::Gobo => Color32::from_rgb(170, 140, 90),
        Pin::Prism => Color32::from_rgb(140, 120, 190),
        Pin::Strobe => Color32::from_rgb(250, 230, 120),
        Pin::Beam => Color32::from_rgb(120, 140, 110),
        Pin::Speed | Pin::Effect | Pin::Other => Color32::from_gray(90),
    }
}

/// A row of small coloured ticks, one per channel.
pub fn pinout(ui: &mut egui::Ui, pins: &[Pin], max: usize) -> egui::Response {
    let n = pins.len().min(max);
    let (w, h) = (6.0, 9.0);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(n as f32 * (w + 1.0) + if pins.len() > max { 12.0 } else { 0.0 }, h), Sense::hover());
    for (i, p) in pins.iter().take(n).enumerate() {
        let x = r.min.x + i as f32 * (w + 1.0);
        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x, r.min.y), egui::vec2(w, h)), 1.0, pin_color(*p));
    }
    if pins.len() > max {
        ui.painter().text(egui::pos2(r.max.x - 10.0, r.center().y), egui::Align2::LEFT_CENTER, "…", egui::FontId::proportional(10.0), crate::theme::current().weak);
    }
    resp.on_hover_ui(|ui| {
        let mut counts: Vec<(Pin, usize)> = Vec::new();
        for p in pins {
            match counts.iter_mut().find(|(q, _)| q == p) {
                Some(c) => c.1 += 1,
                None => counts.push((*p, 1)),
            }
        }
        ui.label(format!("{} channels", pins.len()));
        for (p, c) in counts {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(r, 1.0, pin_color(p));
                ui.label(if c > 1 { format!("{} ×{c}", p.name()) } else { p.name().to_string() });
            });
        }
    })
}

/// One fixture in the library list: name, type, channel counts per mode, and
/// the pinout of the mode a search points at (or the first mode).
fn lib_row(ui: &mut egui::Ui, lib: &Library, i: usize, selected: bool, query: &str) -> egui::Response {
    let t = crate::theme::current();
    let e = &lib.entries[i];
    let fill = if selected { t.raised } else { Color32::TRANSPARENT };
    let inner = egui::Frame::new().fill(fill).corner_radius(t.radius).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let mut name = RichText::new(format!("{} — {}", e.manufacturer, e.model));
        if e.origin() == "your files" {
            name = name.color(t.accent);
        }
        ui.label(name);
        if let Some(m) = &e.meta {
            ui.horizontal(|ui| {
                let counts = e.channel_counts().iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" / ");
                ui.label(RichText::new(format!("{} · {counts} ch", m.kind_label())).color(t.weak).size(t.text_size - 2.0));
                let mode = lib.preferred_mode(i, query).unwrap_or(0);
                if let Some(md) = m.modes.get(mode) {
                    pinout(ui, &md.pins, 32);
                }
                if !m.drivable() {
                    ui.label(RichText::new("not driven yet").color(t.warn).size(t.text_size - 3.0));
                }
            });
        }
    });
    let resp = ui.interact(inner.response.rect, ui.scope_id().with(("librow", i)), Sense::click());
    if resp.hovered() && !selected {
        ui.painter().rect_stroke(inner.response.rect, t.radius, egui::Stroke::new(1.0, t.line), egui::StrokeKind::Inside);
    }
    resp
}

/// The library browser. Returns a type to add, and whether to also place it.
fn browser(st: &mut State, ui: &mut egui::Ui) -> Option<(FixtureType, bool)> {
    let mut result = None;
    let total = st.library().entries.len();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.query)
                .hint_text(format!("Search {total} fixtures: brand, model, type or channels"))
                .desired_width(320.0),
        )
        .on_hover_text("Try \"16 channel moving head\", \"par 7ch\", \"pixel bar\" or \"chauvet slimpar\".");
        ui.checkbox(&mut st.drivable_only, "Only lights Kickline can drive")
            .on_hover_text("Hide fixtures with no colour or dimmer channels (smoke machines, lasers, pan/tilt-only…). Moving-head control is planned.");
        if ui.button("Rescan folders").clicked() {
            st.library = None;
            st.entry = None;
            st.opened = None;
        }
    });
    let q = st.query.clone();
    let drivable_only = st.drivable_only;
    let hits = st.library().search_filtered(&q, 300, drivable_only);
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        egui::ScrollArea::vertical().id_salt("libhits").max_height(420.0).show(ui, |ui| {
            let mut pick = None;
            {
                let lib = st.library.as_ref().unwrap();
                for &i in &hits {
                    if lib_row(ui, lib, i, st.entry == Some(i), &q).clicked() {
                        pick = Some(i);
                    }
                }
            }
            if let Some(i) = pick {
                st.select_entry(i);
                // Searching "16 ch" opens the 16-channel mode.
                if let Some(m) = st.library.as_ref().and_then(|l| l.preferred_mode(i, &q)) {
                    st.mode = m;
                }
            }
            if hits.is_empty() {
                ui.weak("Nothing found.");
            } else if hits.len() == 300 {
                ui.weak("Showing the first 300; type more to narrow it down.");
            }
        });

        let ui = &mut cols[1];
        match &st.opened {
            None => {
                ui.weak("Pick a fixture on the left.");
            }
            Some(Err(e)) => {
                ui.colored_label(crate::theme::current().err, format!("Can't read this fixture: {e}"));
            }
            Some(Ok(f)) => {
                ui.heading(f.title());
                ui.weak(&f.kind);
                st.mode = st.mode.min(f.modes.len() - 1);
                let label = |m: usize| format!("{} ({} ch)", f.modes[m].name, f.modes[m].channels.len());
                egui::ComboBox::from_id_salt("qxfmode").width(260.0).selected_text(label(st.mode)).show_ui(ui, |ui| {
                    for m in 0..f.modes.len() {
                        ui.selectable_value(&mut st.mode, m, label(m));
                    }
                });
                let src = st.library.as_ref().map(|l| l.source_label(st.entry.unwrap_or(0))).unwrap_or_default();
                match f.to_type(st.mode, &src) {
                    Ok(t) => {
                        if let Some(md) = st.entry.and_then(|i| st.library.as_ref()?.entries.get(i)?.meta.as_ref()?.modes.get(st.mode)) {
                            pinout(ui, &md.pins, 64);
                        }
                        let driven = f.driven_channels(st.mode);
                        ui.label(format!("{} pixel(s), {} of {} channels follow the image", t.pixel_count(), driven, t.footprint()));
                        if driven == 0 {
                            ui.colored_label(
                                crate::theme::current().warn,
                                "No colour or dimmer channels in this mode: Kickline can't light it yet (moving-head control is planned).",
                            );
                        }
                        egui::ScrollArea::vertical().id_salt("modech").max_height(330.0).show(ui, |ui| {
                            egui::Grid::new("modech_g").num_columns(3).striped(true).spacing([10.0, 2.0]).show(ui, |ui| {
                                for (i, s) in t.slots.as_ref().unwrap().iter().enumerate() {
                                    ui.monospace(format!("{:>3}", i + 1));
                                    ui.label(&s.name);
                                    let (what, lit) = describe(s.func, s.fine);
                                    let what = match s.pixel {
                                        Some(p) if t.pixel_count() > 1 => format!("{what} · pixel {}", p + 1),
                                        _ => what,
                                    };
                                    if lit {
                                        ui.colored_label(crate::theme::current().ok, what);
                                    } else {
                                        ui.weak(what);
                                    }
                                    ui.end_row();
                                }
                            });
                        });
                        ui.horizontal(|ui| {
                            if ui.button("+ Add to fixture types").clicked() {
                                result = Some((t.clone(), false));
                            }
                            if ui.button("+ Add and place in Advanced Output").clicked() {
                                result = Some((t, true));
                            }
                        });
                    }
                    Err(e) => {
                        ui.colored_label(crate::theme::current().err, e);
                    }
                }
            }
        }
    });
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.weak(format!("Your fixtures: {}", paths::fixtures_dir().display()));
        if ui.small_button("Show folder").clicked() {
            paths::reveal(&paths::fixtures_dir());
        }
    });
    ui.weak(
        "Missing one? Download it from open-fixture-library.org as a \"QLC+ fixture\" and drag the file onto Kickline. \
         Built-in fixtures come from the QLC+ project (Apache-2.0).",
    );
    result
}

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    let View { e, s } = app;
    let o = &mut e.output;
    let lumiverse = s.ao.lumiverse;
    let st = &mut s.fe;
    let mut open_output = false;
    egui::Panel::left("fe_left").default_size(240.0).resizable(true).show(ui, |ui| {
        ui.heading("Fixture types");
        egui::ScrollArea::vertical().id_salt("types").max_height(400.0).show(ui, |ui| {
            for (i, t) in o.types.iter().enumerate() {
                let used = o.fixtures.iter().filter(|f| f.type_id == i).count();
                let text = format!("{}  ({}×{}, {} ch)", t.name, t.columns, t.rows, t.footprint());
                if ui.selectable_label(!st.browsing && st.selected == i, text).on_hover_text(format!("Used by {used} fixture(s)")).clicked() {
                    st.selected = i;
                    st.browsing = false;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.toggle_value(&mut st.browsing, "Library");
            if ui.button("+ New").clicked() {
                o.types.push(FixtureType::new("New fixture", 10, 1, "RGB"));
                st.selected = o.types.len() - 1;
                st.browsing = false;
            }
            if ui.add_enabled(st.selected < o.types.len(), egui::Button::new("Duplicate")).clicked() {
                let mut t = o.types[st.selected].clone();
                t.name.push_str(" copy");
                o.types.push(t);
                st.selected = o.types.len() - 1;
                st.browsing = false;
            }
            let in_use = o.type_in_use(st.selected);
            if ui
                .add_enabled(st.selected < o.types.len() && !in_use, egui::Button::new("Delete"))
                .on_disabled_hover_text("Still used by fixtures in Advanced Output")
                .clicked()
            {
                o.remove_type(st.selected);
                st.selected = st.selected.saturating_sub(1);
            }
        });
    });
    egui::CentralPanel::default().show(ui, |ui| {
        if let Some(m) = st.message.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(crate::theme::current().ok, &m);
                if ui.small_button("×").clicked() {
                    st.message = None;
                }
            });
        }
        if st.browsing {
            if let Some((t, place)) = browser(st, ui) {
                let name = t.name.clone();
                let strip = t.rows <= 1 && t.columns > 1;
                o.types.push(t);
                st.selected = o.types.len() - 1;
                st.browsing = false;
                if place && !o.lumiverses.is_empty() {
                    let lv = lumiverse.min(o.lumiverses.len() - 1);
                    let address = o.next_free_address(lv);
                    o.fixtures.push(Fixture {
                        name: name.clone(),
                        type_id: st.selected,
                        lumiverse: lv,
                        address,
                        x: 0.5,
                        y: 0.5,
                        w: if strip { 0.5 } else { 0.12 },
                        h: if strip { 0.05 } else { 0.16 },
                        rotation: 0.0,
                        snake: false,
                        reverse: false,
                        universe_offset: 0,
                        white: Default::default(),
                        groups: Vec::new(),
                    });
                    open_output = true;
                    st.message = Some(format!("Added \"{name}\" and placed it at address {address} on {}.", o.lumiverses[lv].name));
                } else {
                    st.message = Some(format!("Added \"{name}\". Place it from Advanced Output."));
                }
                log::info("fixtures", st.message.clone().unwrap());
            }
            return;
        }
        match o.types.get_mut(st.selected) {
            Some(t) => type_editor(ui, t),
            None => {
                ui.label("No fixture types yet. Open Library or click + New.");
            }
        }
    });
    if open_output {
        s.windows.advanced_output = true;
        s.ao.selected = Some(e.output.fixtures.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dropping a .qxf copies it into the Fixtures folder and opens it in the library.
    #[test]
    fn import_dropped_file() {
        let mut st = State::default();
        let i = st.library().search("chauvet slimpar 56", 1)[0];
        let xml = st.library().read(i).unwrap();
        let tmp = std::env::temp_dir().join(format!("lumen-drop-{}.qxf", std::process::id()));
        std::fs::write(&tmp, xml).unwrap();
        st.import_dropped(&tmp);
        assert!(st.browsing);
        let f = st.opened.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(f.model, "SlimPAR 56");
        let copied = paths::fixtures_dir().join(tmp.file_name().unwrap());
        assert!(copied.exists());
        std::fs::remove_file(copied).ok();
        std::fs::remove_file(tmp).ok();

        // A broken file is reported, not imported.
        let bad = std::env::temp_dir().join(format!("lumen-bad-{}.qxf", std::process::id()));
        std::fs::write(&bad, "not xml").unwrap();
        st.message = None;
        st.import_dropped(&bad);
        assert!(st.message.as_ref().unwrap().contains("Couldn't import"));
        std::fs::remove_file(paths::fixtures_dir().join(bad.file_name().unwrap())).ok();
        std::fs::remove_file(bad).ok();
    }
}
