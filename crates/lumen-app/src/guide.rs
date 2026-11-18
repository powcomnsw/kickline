//! First-run setup guide: controller → lights → test → play.
//! Opens by itself the first time Kickline runs, and from the show menu.

use crate::{advanced_output, theme, View};
use eframe::egui::{self, RichText};
use lumen_fixtures::{Addressing, ArrayDirection, ArraySpec, Fixture, FixtureType, Lumiverse, Protocol, COLOR_ORDERS};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Step {
    #[default]
    Welcome,
    Controller,
    Lights,
    Test,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Kind {
    #[default]
    Strips,
    Matrix,
    Other,
}

pub struct Guide {
    pub is_open: bool,
    step: Step,
    /// When the controller step was first shown (to time the "can't find it" help).
    searching_since: Option<Instant>,
    picked: Option<usize>,
    manual: bool,
    manual_ip: String,
    manual_universe: u16,
    sacn: bool,
    kind: Kind,
    count: usize,
    leds: usize,
    order: usize,
    columns: bool,
    zigzag: bool,
    own_universe: bool,
    mw: usize,
    mh: usize,
    snake: bool,
    added: Vec<String>,
}

impl Default for Guide {
    fn default() -> Self {
        Guide {
            is_open: false,
            step: Step::Welcome,
            searching_since: None,
            picked: None,
            manual: false,
            manual_ip: "2.0.0.10".into(),
            manual_universe: 0,
            sacn: false,
            kind: Kind::Strips,
            count: 4,
            leds: 150,
            order: 2, // GRB, the most common for WS2812-type tape
            columns: false,
            zigzag: true,
            own_universe: true,
            mw: 16,
            mh: 16,
            snake: true,
            added: Vec::new(),
        }
    }
}

impl Guide {
    pub fn starting(open: bool) -> Self {
        Guide { is_open: open, ..Guide::default() }
    }
    pub fn open(&mut self) {
        *self = Guide { is_open: true, ..Guide::default() };
    }
    /// Polling for nodes is needed while the controller step can be reached.
    pub fn wants_nodes(&self) -> bool {
        self.is_open && matches!(self.step, Step::Welcome | Step::Controller)
    }
}

fn heading(ui: &mut egui::Ui, step: Step, title: &str) {
    let t = theme::current();
    let n = match step {
        Step::Welcome => 0,
        Step::Controller => 1,
        Step::Lights => 2,
        Step::Test => 3,
        Step::Done => 4,
    };
    if n > 0 {
        ui.label(RichText::new(format!("Step {n} of 4")).color(t.weak));
    }
    ui.label(RichText::new(title).font(theme::bold(t.text_size + 7.0)));
    ui.add_space(6.0);
}

fn primary(ui: &mut egui::Ui, text: &str) -> bool {
    let t = theme::current();
    ui.add(egui::Button::new(RichText::new(text).color(t.on_accent).font(theme::bold(t.text_size))).fill(t.accent).min_size(egui::vec2(120.0, 30.0))).clicked()
}

pub fn ui(v: &mut View, ctx: &egui::Context) {
    if !v.s.guide.is_open {
        return;
    }
    let mut open = true;
    egui::Window::new("Setup guide")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(560.0);
            match v.s.guide.step {
                Step::Welcome => welcome(v, ui),
                Step::Controller => controller(v, ui),
                Step::Lights => lights(v, ui),
                Step::Test => test(v, ui),
                Step::Done => done(v, ui),
            }
        });
    if !open {
        close(v);
    }
}

fn close(v: &mut View) {
    v.s.guide.is_open = false;
    v.s.session.first_run = false;
    v.e.test = None;
}

fn welcome(v: &mut View, ui: &mut egui::Ui) {
    heading(ui, Step::Welcome, "Let's get your lights going");
    ui.label("Four short steps: find your controller, tell Kickline what lights you have, check each one lights up the right way, then play. About five minutes.");
    ui.add_space(4.0);
    ui.weak("You'll need: your Art-Net or sACN controller plugged into this Mac and powered on, with the lights connected to it.");
    ui.add_space(12.0);
    let has_patch = !v.e.output.fixtures.is_empty() && !v.s.session.first_run;
    ui.horizontal(|ui| {
        if primary(ui, if has_patch { "Start a new show" } else { "Set up my lights" }) {
            if has_patch {
                crate::project::new_show(v);
                v.s.guide.is_open = true;
            } else {
                // First run: replace the demo patch, keep its fixture types.
                v.e.output.fixtures.clear();
                v.e.output.lumiverses = vec![Lumiverse::new("Lumiverse 1", 0)];
                v.s.ao = Default::default();
            }
            v.s.guide.step = Step::Controller;
        }
        if has_patch && ui.button("Add lights to this show").clicked() {
            v.s.guide.step = Step::Controller;
        }
        if ui.button("Just look around").on_hover_text("Close the guide and play with the demo. Reopen it from the show menu (top right).").clicked() {
            close(v);
        }
    });
}

fn controller(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    heading(ui, Step::Controller, "Find your controller");
    ui.label("Plug the controller into this Mac (an Ethernet port or a USB Ethernet adapter) and switch it on. Kickline is listening for it.");
    ui.add_space(6.0);
    let g = &mut v.s.guide;
    let since = *g.searching_since.get_or_insert_with(Instant::now);
    let nodes = v.s.nodes.nodes();
    if nodes.is_empty() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Looking for Art-Net controllers…");
        });
    }
    for (i, n) in nodes.iter().enumerate() {
        let unis = n.node.universes.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", ");
        let text = format!("{}  ·  {}  ·  universe {}", n.name(), n.node.ip, if unis.is_empty() { "?".into() } else { unis });
        ui.horizontal(|ui| {
            ui.colored_label(if n.online() { t.ok } else { t.err }, "●");
            if ui.radio(g.picked == Some(i) && !g.manual, text).clicked() {
                g.picked = Some(i);
                g.manual = false;
            }
        });
    }
    if nodes.len() == 1 && g.picked.is_none() && !g.manual {
        g.picked = Some(0);
    }
    if nodes.is_empty() && since.elapsed().as_secs() >= 6 {
        ui.add_space(6.0);
        egui::Frame::new().fill(t.raised).corner_radius(t.radius).inner_margin(10).show(ui, |ui| {
            ui.label(RichText::new("Not showing up?").font(theme::bold(t.text_size)));
            let ports = lumen_output::interfaces();
            if ports.is_empty() {
                ui.label("This Mac has no network connection right now. Check the cable or adapter.");
            } else {
                ui.label("This Mac's network addresses:");
                for p in &ports {
                    let hint = if p.ip.octets()[0] == 169 && p.ip.octets()[1] == 254 {
                        "  (no address given: the controller probably has a fixed one, see below)"
                    } else {
                        ""
                    };
                    ui.monospace(format!("  {}{hint}", p.label()));
                }
            }
            ui.add_space(4.0);
            ui.label("The Mac and the controller must be on the same network. Controllers often come set to 2.x.x.x or 192.168.x.x (check its label or manual). To match a controller at 2.0.0.10:");
            ui.weak("System Settings › Network › your adapter › Details › TCP/IP › Configure IPv4: Manually › IP address 2.0.0.100, Subnet mask 255.0.0.0.");
            ui.weak("Also allow Kickline in System Settings › Privacy & Security › Local Network.");
        });
    }
    ui.add_space(6.0);
    if ui.radio(g.manual, "My controller isn't listed: enter it by hand").clicked() {
        g.manual = true;
    }
    if g.manual {
        ui.indent("manual", |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut g.sacn, false, "Art-Net");
                ui.selectable_value(&mut g.sacn, true, "sACN (E1.31)");
            });
            if !g.sacn {
                ui.horizontal(|ui| {
                    ui.label("IP address");
                    ui.text_edit_singleline(&mut g.manual_ip).on_hover_text("Printed on the controller or shown on its web page. 255.255.255.255 sends to every device.");
                });
            }
            ui.horizontal(|ui| {
                ui.label("First universe");
                ui.add(egui::DragValue::new(&mut g.manual_universe).range(0..=32767)).on_hover_text("A universe is one cable's worth of data (170 RGB pixels). Kickline counts from 0; if your controller's first output says universe 1, try 1 here.");
            });
        });
    }
    ui.add_space(12.0);
    let ready = g.manual || g.picked.is_some();
    let mut next = false;
    let mut back = false;
    ui.horizontal(|ui| {
        next = ui.add_enabled(ready, egui::Button::new(RichText::new("Next").color(t.on_accent)).fill(t.accent).min_size(egui::vec2(120.0, 30.0))).clicked();
        back = ui.button("Back").clicked();
    });
    let (picked, manual, sacn, ip, uni) = (g.picked, g.manual, g.sacn, g.manual_ip.trim().to_string(), g.manual_universe);
    if back {
        v.s.guide.step = Step::Welcome;
    }
    if next {
        {
            let o = &mut v.e.output;
            if o.lumiverses.is_empty() {
                o.lumiverses.push(Lumiverse::new("Lumiverse 1", 0));
            }
            let l = &mut o.lumiverses[0];
            if manual {
                l.protocol = if sacn { Protocol::Sacn } else { Protocol::ArtNet };
                if !sacn {
                    l.target = ip;
                }
                l.universe = uni;
            } else if let Some(n) = picked.and_then(|i| nodes.get(i)) {
                l.protocol = Protocol::ArtNet;
                l.name = n.name().to_string();
                l.target = n.node.ip.to_string();
                l.universe = n.node.universes.first().copied().unwrap_or(0);
                l.interface = n.port.clone();
            }
            v.e.output_enabled = true;
            v.s.guide.step = Step::Lights;
        }
    }
}

fn lights(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    heading(ui, Step::Lights, "What lights do you have?");
    let g = &mut v.s.guide;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut g.kind, Kind::Strips, "LED strips / pixel tape");
        ui.selectable_value(&mut g.kind, Kind::Matrix, "Pixel panel / matrix");
        ui.selectable_value(&mut g.kind, Kind::Other, "Pars, bars, other lights");
    });
    ui.add_space(8.0);
    let mut add = false;
    match g.kind {
        Kind::Strips => {
            egui::Grid::new("strips").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("How many strips");
                ui.add(egui::DragValue::new(&mut g.count).range(1..=128));
                ui.end_row();
                ui.label("LEDs per strip").on_hover_text("Count the pixels on one strip. 5 m of 30/m tape is 150.");
                ui.add(egui::DragValue::new(&mut g.leds).range(1..=2000));
                ui.end_row();
                ui.label("Colour order").on_hover_text("The order your tape expects colours in. Most WS2812/WS2811 tape is GRB. Not sure? Leave it: the test step will tell you.");
                egui::ComboBox::from_id_salt("g_order").selected_text(COLOR_ORDERS[g.order]).show_ui(ui, |ui| {
                    for (i, o) in COLOR_ORDERS.iter().enumerate() {
                        ui.selectable_value(&mut g.order, i, *o);
                    }
                });
                ui.end_row();
                ui.label("Laid out");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut g.columns, false, "Side by side, across");
                    ui.selectable_value(&mut g.columns, true, "Standing up");
                });
                ui.end_row();
                ui.label("Wiring");
                ui.checkbox(&mut g.zigzag, "Zig-zag (each strip runs back the other way)");
                ui.end_row();
                ui.label("Universes");
                ui.checkbox(&mut g.own_universe, "Each strip starts on its own universe")
                    .on_hover_text("Usual when each strip is plugged into its own controller output. Untick if the strips are chained on one output.");
                ui.end_row();
            });
            add = primary(ui, &format!("Add {} strip(s)", g.count));
        }
        Kind::Matrix => {
            egui::Grid::new("matrix").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Pixels across");
                ui.add(egui::DragValue::new(&mut g.mw).range(1..=512));
                ui.end_row();
                ui.label("Pixels down");
                ui.add(egui::DragValue::new(&mut g.mh).range(1..=512));
                ui.end_row();
                ui.label("Colour order");
                egui::ComboBox::from_id_salt("g_order_m").selected_text(COLOR_ORDERS[g.order]).show_ui(ui, |ui| {
                    for (i, o) in COLOR_ORDERS.iter().enumerate() {
                        ui.selectable_value(&mut g.order, i, *o);
                    }
                });
                ui.end_row();
                ui.label("Wiring");
                ui.checkbox(&mut g.snake, "Snake (rows go back and forth)");
                ui.end_row();
            });
            add = primary(ui, "Add panel");
        }
        Kind::Other => {
            ui.label("Pick them from the fixture library: 1,781 lights by brand and model. Search, choose the mode your light is set to, and add it. Then come back here.");
            ui.add_space(4.0);
            if ui.button("Open the fixture library").clicked() {
                v.s.windows.fixture_editor = true;
                v.s.fe.browsing = true;
            }
        }
    }
    if add {
        let g = &v.s.guide;
        let order = COLOR_ORDERS[g.order];
        let o = &mut v.e.output;
        let lv = 0;
        let (name, made) = match g.kind {
            Kind::Strips => {
                let ty = FixtureType::new(&format!("Strip {} {order}", g.leds), g.leds, 1, order);
                o.types.push(ty);
                let type_id = o.types.len() - 1;
                // Keep new strips clear of anything already placed.
                let n_before = o.fixtures.len();
                let top = if n_before == 0 { 0.1 } else { 0.55 };
                let r = o.add_array(&ArraySpec {
                    name: "Strip".into(),
                    type_id,
                    lumiverse: lv,
                    count: g.count,
                    direction: if g.columns { ArrayDirection::Columns } else { ArrayDirection::Rows },
                    area: if g.columns { (0.05, 0.05, 0.95, 0.95) } else { (0.05, top, 0.95, 0.95) },
                    zigzag: g.zigzag,
                    addressing: if g.own_universe { Addressing::UniversePerFixture } else { Addressing::Continue },
                    thickness: 0.03,
                });
                (format!("{} strips of {} LEDs ({order})", g.count, g.leds), r.len())
            }
            Kind::Matrix => {
                o.types.push(FixtureType::new(&format!("Panel {}x{} {order}", g.mw, g.mh), g.mw, g.mh, order));
                let type_id = o.types.len() - 1;
                let (universe_offset, address) = o.next_free_slot(lv);
                let aspect = o.aspect.max(0.01);
                let h = 0.8;
                let w = (h * g.mw as f32 / g.mh.max(1) as f32 / aspect).min(0.9);
                o.fixtures.push(Fixture {
                    name: "Panel".into(),
                    type_id,
                    lumiverse: lv,
                    address,
                    x: 0.5,
                    y: 0.5,
                    w,
                    h,
                    rotation: 0.0,
                    snake: g.snake,
                    reverse: false,
                    universe_offset,
                    white: Default::default(),
                });
                (format!("a {}×{} panel ({order})", g.mw, g.mh), 1)
            }
            Kind::Other => (String::new(), 0),
        };
        if made > 0 {
            v.s.guide.added.push(name);
        }
    }
    ui.add_space(8.0);
    let n = v.e.output.fixtures.len();
    if n > 0 {
        ui.colored_label(t.ok, format!("{n} light(s) in your show so far"));
        for a in &v.s.guide.added {
            ui.weak(format!("  + {a}"));
        }
        ui.weak("Add more, or move on. You can drag them around later in Advanced Output.");
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(n > 0, egui::Button::new(RichText::new("Next: test them").color(t.on_accent)).fill(t.accent).min_size(egui::vec2(140.0, 30.0))).clicked() {
            v.s.guide.step = Step::Test;
        }
        if ui.button("Back").clicked() {
            v.s.guide.step = Step::Controller;
        }
    });
}

fn test(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    heading(ui, Step::Test, "Check each light");
    ui.label("Press Test on each light and look at the real thing. Everything else goes dark while you test.");
    ui.add_space(6.0);
    let names: Vec<String> = v.e.output.fixtures.iter().map(|f| f.name.clone()).collect();
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        for (i, name) in names.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).font(theme::bold(t.text_size)));
                ui.weak(v.e.output.range_label(&v.e.output.fixtures[i]));
            });
            advanced_output::test_row(&mut v.e.test, &mut v.e.output_enabled, i, ui);
            ui.add_space(4.0);
        }
    });
    ui.add_space(6.0);
    ui.weak("Nothing lights up? Check the controller's address and universe in Advanced Output, and Diagnostics for network problems.");
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if primary(ui, "Next") {
            v.e.test = None;
            v.s.guide.step = Step::Done;
        }
        if ui.button("Back").clicked() {
            v.e.test = None;
            v.s.guide.step = Step::Lights;
        }
    });
}

fn done(v: &mut View, ui: &mut egui::Ui) {
    heading(ui, Step::Done, "You're ready to play");
    ui.label("Your lights now show the picture on the right. A few things to try:");
    ui.add_space(4.0);
    ui.label("•  Click a clip to launch it. Each row is a layer; layers mix on top of each other.");
    ui.label("•  Click a palette above the clips to recolour everything.");
    ui.label("•  Feed it music so the lights react to the kick, snare and bass.");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Connect music (rekordbox)").clicked() {
            v.s.windows.audio = true;
        }
        if ui.button("Arrange lights on the canvas").clicked() {
            v.s.windows.advanced_output = true;
        }
    });
    ui.add_space(10.0);
    ui.weak("Your show saves automatically. ⌘Z undoes anything.");
    ui.add_space(6.0);
    if primary(ui, "Finish") {
        close(v);
    }
}
