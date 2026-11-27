//! Advanced Output window: lumiverses on the left, the composition with
//! fixtures drawn on it in the middle, the selected fixture's settings below.

use crate::util::{paint_frame, to_canvas, to_color, to_screen};
use crate::View;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use lumen_fixtures::{AdvancedOutput, Fixture, Lumiverse, Protocol};
use lumen_output::SeenNode;

#[derive(Default)]
pub struct State {
    pub selected: Option<usize>,
    pub lumiverse: usize,
    drag: Option<Drag>,
    add_type: usize,
    show_pixels: bool,
    array_open: bool,
    array: Option<ArrayForm>,
    /// Network ports, refreshed every few seconds.
    ports: Vec<lumen_output::NetIface>,
    ports_at: Option<std::time::Instant>,
    find_nodes: bool,
    sweep_count: u16,
    /// "Add a controller" form, when open.
    pub controller: Option<lumen_fixtures::ControllerSetup>,
    controller_error: Option<String>,
}

struct ArrayForm {
    count: usize,
    columns: bool,
    zigzag: bool,
    own_universe: bool,
    area: [f32; 4],
}

impl Default for ArrayForm {
    fn default() -> Self {
        ArrayForm { count: 8, columns: false, zigzag: true, own_universe: true, area: [0.05, 0.1, 0.95, 0.9] }
    }
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

/// What we know about the node(s) behind a lumiverse, from ArtPoll replies.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeStatus {
    /// sACN or disabled: ArtPoll doesn't apply.
    NotArtNet,
    /// No node has answered for this lumiverse (yet).
    Waiting,
    Online(String),
    /// The node answers but doesn't output some of the universes we send.
    Missing { name: String, missing: Vec<u16>, has: Vec<u16> },
    /// The node was answering and has stopped.
    Lost { name: String, secs: u64 },
}

impl NodeStatus {
    pub fn color(&self, t: &crate::theme::Theme) -> Color32 {
        match self {
            NodeStatus::Online(_) => t.ok,
            NodeStatus::Missing { .. } => t.warn,
            NodeStatus::Lost { .. } => t.err,
            _ => t.line,
        }
    }

    pub fn describe(&self) -> String {
        let list = |v: &[u16]| v.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", ");
        match self {
            NodeStatus::NotArtNet => String::new(),
            NodeStatus::Waiting => "No node has answered yet. Some nodes don't reply to ArtPoll; output is sent anyway.".into(),
            NodeStatus::Online(n) => format!("{n} is answering"),
            NodeStatus::Missing { name, missing, has } => format!(
                "{name} answers, but it outputs universe {} and you send {}. Some nodes count universes from 1, Kickline counts from 0.",
                list(has),
                list(missing)
            ),
            NodeStatus::Lost { name, secs } => format!("{name} stopped answering {secs} s ago. Kickline keeps sending."),
        }
    }
}

fn is_broadcast(target: &str) -> bool {
    target.trim().ends_with(".255")
}

pub fn node_status(o: &AdvancedOutput, lv: usize, nodes: &[SeenNode]) -> NodeStatus {
    let Some(l) = o.lumiverses.get(lv) else { return NodeStatus::NotArtNet };
    if l.protocol != Protocol::ArtNet || !l.enabled {
        return NodeStatus::NotArtNet;
    }
    let used = o.universes_used(lv);
    let pick = |cands: Vec<&SeenNode>| -> NodeStatus {
        if let Some(n) = cands.iter().find(|n| n.online()) {
            let names: Vec<&str> = cands.iter().filter(|n| n.online()).map(|n| n.name()).collect();
            let missing: Vec<u16> = used.iter().copied().filter(|u| !cands.iter().filter(|n| n.online()).any(|n| n.node.universes.contains(u))).collect();
            if !missing.is_empty() && !n.node.universes.is_empty() && !is_broadcast(&l.target) {
                return NodeStatus::Missing { name: n.name().to_string(), missing, has: n.node.universes.clone() };
            }
            return NodeStatus::Online(names.join(", "));
        }
        match cands.iter().max_by_key(|n| n.last_seen) {
            Some(n) => NodeStatus::Lost { name: n.name().to_string(), secs: n.last_seen.elapsed().as_secs() },
            None => NodeStatus::Waiting,
        }
    };
    if is_broadcast(&l.target) {
        pick(nodes.iter().filter(|n| n.node.universes.iter().any(|u| used.contains(u))).collect())
    } else {
        let ip = l.target.trim();
        pick(nodes.iter().filter(|n| n.node.ip.to_string() == ip).collect())
    }
}

/// The "Find nodes" list: every node that answered, with one-click setup.
fn nodes_list(o: &mut AdvancedOutput, st: &mut State, nodes: &[SeenNode], watcher: &lumen_output::NodeWatcher, ui: &mut egui::Ui) {
    let t = crate::theme::current();
    egui::Frame::new().fill(t.panel).corner_radius(t.radius).inner_margin(8).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Nodes on the network").font(crate::theme::bold(t.text_size)));
            if ui.small_button("Search again").clicked() {
                watcher.poll_now();
            }
        });
        if let Some(e) = watcher.error() {
            ui.colored_label(t.warn, e);
        }
        if nodes.is_empty() {
            ui.weak("Listening for Art-Net nodes… If nothing appears: check the node is powered and cabled, and that this computer has an IP on the node's network (often 2.x.x.x or 192.168.x.x).");
        }
        let target_lv = st.lumiverse;
        let lv_name = o.lumiverses.get(target_lv).map(|l| l.name.clone()).unwrap_or_default();
        for n in nodes {
            ui.separator();
            ui.horizontal(|ui| {
                ui.colored_label(if n.online() { t.ok } else { t.err }, "●");
                ui.label(egui::RichText::new(n.name()).font(crate::theme::bold(t.text_size)));
            });
            let unis = if n.node.universes.is_empty() { "no outputs reported".to_string() } else { format!("universe {}", n.node.universes.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", ")) };
            let port = n.port.as_deref().map(|p| format!(" · via {p}")).unwrap_or_default();
            let ago = if n.online() { String::new() } else { format!(" · last heard {} s ago", n.last_seen.elapsed().as_secs()) };
            ui.weak(format!("{} · {unis}{port}{ago}", n.node.ip));
            ui.horizontal(|ui| {
                let first = n.node.universes.first().copied();
                if ui.small_button(format!("Use for {lv_name}")).on_hover_text("Send this lumiverse straight to this node, on its first universe and network port").clicked() {
                    if let Some(l) = o.lumiverses.get_mut(target_lv) {
                        l.protocol = Protocol::ArtNet;
                        l.target = n.node.ip.to_string();
                        if let Some(u) = first {
                            l.universe = u;
                        }
                        if n.port.is_some() {
                            l.interface = n.port.clone();
                        }
                    }
                }
                if ui.small_button("New lumiverse").clicked() {
                    let mut l = Lumiverse::new(n.name(), first.unwrap_or(0));
                    l.target = n.node.ip.to_string();
                    l.interface = n.port.clone();
                    o.lumiverses.push(l);
                    st.lumiverse = o.lumiverses.len() - 1;
                }
            });
        }
    });
}

/// Art-Net's own way of writing a universe: Net : Sub-Net : Universe.
pub fn artnet_address(u: u16) -> String {
    format!("{}:{}:{}", u >> 8, (u >> 4) & 15, u & 15)
}

/// "Sweep universes": light each universe in turn, no fixtures needed.
fn sweep_panel(o: &AdvancedOutput, lv: usize, sweep: &mut Option<crate::engine::Sweep>, out_on: &mut bool, count: &mut u16, ui: &mut egui::Ui) {
    let t = crate::theme::current();
    let Some(l) = o.lumiverses.get(lv) else { return };
    if *count == 0 {
        *count = 4;
    }
    let mine = sweep.as_ref().is_some_and(|s| s.lumiverse == lv);
    ui.add_space(4.0);
    if !mine {
        ui.horizontal(|ui| {
            if ui
                .button("Sweep universes")
                .on_hover_text("Check the cable, controller and lights before adding fixtures: a short chase runs across one universe at a time, moving on every 1.5 s. Whatever flickers is listening to that universe. Everything else on this lumiverse is dark while it runs.")
                .clicked()
            {
                *sweep = Some(crate::engine::Sweep::new(lv, *count));
                *out_on = true;
            }
            ui.label("first");
            ui.add(egui::DragValue::new(count).range(1..=64));
            ui.label("universes");
        });
        return;
    }
    let mut stop = false;
    let sw = sweep.as_mut().expect("mine");
    let cur = sw.current();
    let u = l.universe + cur;
    egui::Frame::new().fill(t.raised).corner_radius(t.radius).inner_margin(8).show(ui, |ui| {
        ui.label(egui::RichText::new(format!("Universe {u} is on now ({} of {})", cur + 1, sw.count)).font(crate::theme::bold(t.text_size + 1.0)));
        ui.weak(format!("Art-Net {} · sACN {}", artnet_address(u), u + 1));
        ui.weak("Whatever lights up is listening to this universe. Nothing at all? Check the IP, network port and that the controller is in Art-Net/sACN mode. Lights up one step late or early? The controller counts universes from 1: change Universe above.");
        ui.label(egui::RichText::new(if sw.all {
            "All channels are held on: moving heads and smart fixtures may move, strobe or react to control channels. Use the chase for those."
        } else {
            "A short chase runs across the universe: pixels show a running dot (and which way the data goes); moving heads may twitch."
        }).color(t.warn));
        ui.horizontal(|ui| {
            if ui.button("◀").clicked() {
                sw.hold = Some((cur + sw.count - 1) % sw.count);
            }
            if ui.button("▶").clicked() {
                sw.hold = Some((cur + 1) % sw.count);
            }
            if sw.hold.is_some() {
                if ui.button("Auto").on_hover_text("Move on by itself every 1.5 s").clicked() {
                    sw.hold = None;
                    sw.started = std::time::Instant::now() - std::time::Duration::from_secs_f32(crate::engine::Sweep::STEP * cur as f32);
                }
            } else if ui.button("Hold").on_hover_text("Stay on this universe").clicked() {
                sw.hold = Some(cur);
            }
            ui.selectable_value(&mut sw.all, false, "Chase").on_hover_text("Safe for any fixture: no channel is held on");
            ui.selectable_value(&mut sw.all, true, "All channels").on_hover_text("Every channel at half: easiest to see on pixels and pars, but avoid with moving heads");
        });
        if ui.add(egui::Button::new(egui::RichText::new("Stop sweep").color(t.on_accent)).fill(t.accent)).clicked() {
            stop = true;
        }
    });
    if stop {
        *sweep = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn lumiverse_panel(
    o: &mut AdvancedOutput,
    st: &mut State,
    routes: &[lumen_output::RouteState],
    nodes: &[SeenNode],
    watcher: &lumen_output::NodeWatcher,
    sweep: &mut Option<crate::engine::Sweep>,
    out_on: &mut bool,
    ui: &mut egui::Ui,
    list: bool,
    settings: bool,
) {
    if st.ports_at.is_none_or(|t| t.elapsed().as_secs() >= 3) {
        st.ports = lumen_output::interfaces();
        st.ports_at = Some(std::time::Instant::now());
    }
    let t = crate::theme::current();
    let statuses: Vec<NodeStatus> = (0..o.lumiverses.len()).map(|i| node_status(o, i, nodes)).collect();
    if list {
    ui.horizontal(|ui| {
        ui.heading("Lumiverses").on_hover_text("A lumiverse is one output: where the data goes (a controller's IP), how (Art-Net or sACN) and from which universe. Most rigs need one per controller.");
        if ui.add(egui::Button::new("Find nodes").selected(st.find_nodes)).on_hover_text("List the Art-Net nodes on your network").clicked() {
            st.find_nodes = !st.find_nodes;
            watcher.poll_now();
        }
    });
    if st.find_nodes {
        nodes_list(o, st, nodes, watcher, ui);
        ui.add_space(4.0);
    }
    let mut remove = None;
    for (i, l) in o.lumiverses.iter().enumerate() {
        ui.horizontal(|ui| {
            let ns = &statuses[i];
            if *ns != NodeStatus::NotArtNet {
                ui.colored_label(ns.color(&t), "●").on_hover_text(ns.describe());
            }
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
    if ui.button("+ Controller…").on_hover_text("Pick your pixel controller and how many LEDs are on each output: Kickline sets up the universes and addresses to match").clicked() {
        st.controller = Some(lumen_fixtures::ControllerSetup::new(0, st.lumiverse));
        st.controller_error = None;
        st.selected = None;
    }
    }
    if settings {
    let mut sweep_here = false;
    if let Some(l) = o.lumiverses.get_mut(st.lumiverse) {
        ui.add_space(4.0);
        egui::Grid::new("lv")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut l.name);
                ui.end_row();
                ui.label("Protocol").on_hover_text("How the data travels. Art-Net is the most common for LED controllers; sACN (E1.31) is common on consoles and some controllers. Use what your controller is set to.");
                egui::ComboBox::from_id_salt("lvproto")
                    .selected_text(l.protocol.name())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut l.protocol, Protocol::ArtNet, "Art-Net");
                        ui.selectable_value(&mut l.protocol, Protocol::Sacn, "sACN");
                    });
                ui.end_row();
                ui.label("Universe").on_hover_text("A universe is 512 channels: about 170 RGB pixels, or one controller output. Kickline counts from 0; if your controller counts from 1, add one.");
                ui.add(egui::DragValue::new(&mut l.universe).range(0..=32767));
                ui.end_row();
                ui.label("Target IP").on_hover_text("The controller's network address. 255.255.255.255 sends to everything on the network (easy, but busier); the controller's own IP is better. Find nodes fills this in.");
                ui.add_enabled(
                    l.protocol == Protocol::ArtNet,
                    egui::TextEdit::singleline(&mut l.target).desired_width(130.0),
                );
                ui.end_row();
                ui.label("Network port").on_hover_text("Which of this Mac's network connections to send through, e.g. a USB Ethernet adapter. Automatic lets macOS choose.");
                let label = match &l.interface {
                    None => "Automatic".to_string(),
                    Some(n) => st.ports.iter().find(|p| &p.name == n).map(|p| p.label()).unwrap_or_else(|| format!("{n} (not connected)")),
                };
                egui::ComboBox::from_id_salt("lvport").width(170.0).selected_text(label).show_ui(ui, |ui| {
                    ui.selectable_value(&mut l.interface, None, "Automatic").on_hover_text("Let the computer choose");
                    for p in &st.ports {
                        ui.selectable_value(&mut l.interface, Some(p.name.clone()), p.label());
                    }
                });
                ui.end_row();
                if l.protocol == Protocol::ArtNet {
                    let bcast = is_broadcast(&l.target);
                    ui.label("Frame sync").on_hover_text("ArtSync: after each frame, tells the controller to show all its universes at the same instant, so long runs of pixels don't tear. Controllers that don't support it just ignore it.");
                    ui.add_enabled(!bcast, egui::Checkbox::new(&mut l.sync, if bcast { "needs the controller's IP" } else { "ArtSync" }))
                        .on_disabled_hover_text("Frame sync only works when sending straight to a controller, not to 255.255.255.255.");
                    ui.end_row();
                }
                ui.label("Refresh").on_hover_text("Some cheap pixel controllers glitch or flicker when sent frames too fast. If yours does, cap it here (30 Hz is a safe start).");
                let name = |h: Option<u16>| match h {
                    None => "As fast as Kickline runs".to_string(),
                    Some(h) => format!("At most {h} Hz"),
                };
                egui::ComboBox::from_id_salt("lvhz").selected_text(name(l.max_hz)).show_ui(ui, |ui| {
                    for h in [None, Some(40), Some(30), Some(25), Some(20)] {
                        ui.selectable_value(&mut l.max_hz, h, name(h));
                    }
                });
                ui.end_row();
                ui.label("Enabled");
                ui.checkbox(&mut l.enabled, "");
                ui.end_row();
            });
        match &statuses[st.lumiverse] {
            NodeStatus::NotArtNet => {}
            NodeStatus::Waiting => {
                ui.weak(statuses[st.lumiverse].describe());
            }
            ns => {
                ui.colored_label(ns.color(&t), ns.describe());
            }
        }
        match routes.get(st.lumiverse) {
            Some(lumen_output::RouteState::Up(i)) => {
                ui.colored_label(t.ok, format!("Sending through {} ({})", i.name, i.ip));
            }
            Some(lumen_output::RouteState::Waiting(n)) => {
                ui.colored_label(t.warn, format!("{n} isn't connected. Kickline keeps trying and resumes by itself."));
            }
            _ => {}
        }
        sweep_here = true;
        if let (Some(n), Ok(ip)) = (&l.interface, l.target.trim().parse::<std::net::Ipv4Addr>()) {
            if let Some(p) = st.ports.iter().find(|p| &p.name == n) {
                if ip != std::net::Ipv4Addr::BROADCAST && !p.contains(ip) && l.protocol == Protocol::ArtNet {
                    ui.colored_label(t.warn, format!("{ip} isn't on {}'s network ({}/{}).", p.name, p.ip, p.prefix()));
                }
                if ip == std::net::Ipv4Addr::BROADCAST {
                    ui.weak(format!("Broadcast goes to {} on this port.", p.broadcast));
                }
            }
        }
        if l.protocol == Protocol::Sacn {
            ui.weak(format!(
                "Multicast to 239.255.{}.{}",
                (l.universe + 1) >> 8,
                (l.universe + 1) & 0xff
            ));
        }
    }
    if sweep_here {
        let lv = st.lumiverse;
        sweep_panel(o, lv, sweep, out_on, &mut st.sweep_count, ui);
    }

    }
    if !list {
        return;
    }
    ui.add_space(8.0);
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
                address: o.next_free_slot(st.lumiverse).1,
                x: 0.5,
                y: 0.5,
                w: if strip { 0.6 } else { 0.15 },
                h: if strip { 0.04 } else { 0.2 },
                rotation: 0.0,
                snake: t.rows > 1,
                reverse: false,
                universe_offset: o.next_free_slot(st.lumiverse).0,
                path_order: 0,
                path_reverse: false,
                white: Default::default(),
                groups: Vec::new(),
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
            (f.universe_offset, f.address) = o.next_free_slot(f.lumiverse);
            o.fixtures.push(f);
            st.selected = Some(o.fixtures.len() - 1);
        }
        if ui.add_enabled(has, egui::Button::new("Delete")).clicked() {
            o.fixtures.remove(st.selected.unwrap());
            st.selected = None;
        }
        ui.toggle_value(&mut st.array_open, "Add array…").on_hover_text("Lay out many strips at once");
    });
    if st.array_open {
        array_form(o, st, ui);
    }
}

/// Lay out N strips/bars at once, wired and addressed for you.
fn array_form(o: &mut AdvancedOutput, st: &mut State, ui: &mut egui::Ui) {
    let t = crate::theme::current();
    if o.types.is_empty() || o.lumiverses.is_empty() {
        ui.weak("Add a fixture type and a lumiverse first.");
        return;
    }
    let f = st.array.get_or_insert_with(Default::default);
    egui::Frame::new().fill(t.raised).corner_radius(t.radius).inner_margin(8).show(ui, |ui| {
        ui.strong("Array");
        egui::Grid::new("arr").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Type");
            ui.label(&o.types[st.add_type.min(o.types.len() - 1)].name);
            ui.end_row();
            ui.label("How many");
            ui.add(egui::DragValue::new(&mut f.count).range(1..=256));
            ui.end_row();
            ui.label("Layout");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut f.columns, false, "Rows");
                ui.selectable_value(&mut f.columns, true, "Columns");
            });
            ui.end_row();
            ui.label("Area");
            ui.horizontal(|ui| {
                for (i, l) in ["x", "y", "to x", "to y"].iter().enumerate() {
                    ui.add(egui::DragValue::new(&mut f.area[i]).range(0.0..=1.0).speed(0.005).max_decimals(2).prefix(format!("{l} ")));
                }
            });
            ui.end_row();
            ui.label("Wiring");
            ui.checkbox(&mut f.zigzag, "Zig-zag (every other one reversed)");
            ui.end_row();
            ui.label("Addressing");
            ui.vertical(|ui| {
                ui.radio_value(&mut f.own_universe, true, "Each on its own universe");
                ui.radio_value(&mut f.own_universe, false, "Continue on from the last one");
            });
            ui.end_row();
        });
        let spec = lumen_fixtures::ArraySpec {
            name: o.types[st.add_type.min(o.types.len() - 1)].name.clone(),
            type_id: st.add_type.min(o.types.len() - 1),
            lumiverse: st.lumiverse.min(o.lumiverses.len() - 1),
            count: f.count,
            direction: if f.columns { lumen_fixtures::ArrayDirection::Columns } else { lumen_fixtures::ArrayDirection::Rows },
            area: (f.area[0].min(f.area[2]), f.area[1].min(f.area[3]), f.area[0].max(f.area[2]), f.area[1].max(f.area[3])),
            zigzag: f.zigzag,
            addressing: if f.own_universe {
                lumen_fixtures::Addressing::UniversePerFixture
            } else {
                lumen_fixtures::Addressing::Continue
            },
            thickness: 0.035,
        };
        // Preview by trying it on a copy.
        let mut trial = o.clone();
        let r = trial.add_array(&spec);
        let pixels: usize = trial.fixtures[r.clone()].iter().filter_map(|x| trial.types.get(x.type_id)).map(|t| t.pixel_count()).sum();
        let unis = trial.universes_of(r.clone()).map(|(a, b)| if a == b { format!("universe {a}") } else { format!("universes {a}–{b}") }).unwrap_or_default();
        ui.label(format!("{} fixtures, {} pixels, {} on {}", f.count, pixels, unis, o.lumiverses[spec.lumiverse].name));
        // Tell the user about universes we stepped over.
        let taken = o.universes_taken_by_others(spec.lumiverse);
        if let Some((a, b)) = trial.universes_of(r.clone()) {
            let mut skipped: Vec<u16> = (a..=b).filter(|u| taken.contains(u)).collect();
            skipped.dedup();
            if !skipped.is_empty() {
                let list = skipped.iter().map(|u| format!("U{u}")).collect::<Vec<_>>().join(", ");
                ui.label(egui::RichText::new(format!("Skips {list}: another lumiverse already sends there.")).color(crate::theme::current().weak));
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Create").clicked() {
                let r = o.add_array(&spec);
                st.selected = Some(r.start);
                st.array_open = false;
            }
            if ui.button("Cancel").clicked() {
                st.array_open = false;
            }
        });
    });
}

/// The selected fixture's channels as they're being sent right now.
fn live_channels(o: &AdvancedOutput, dmx: &std::collections::BTreeMap<(usize, u16), [u8; lumen_fixtures::UNIVERSE_SIZE]>, out_on: bool, idx: usize, ui: &mut egui::Ui) {
    let t = crate::theme::current();
    let fx = &o.fixtures[idx];
    let map = o.channel_map(fx);
    if map.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(egui::RichText::new(format!("Live channels ({})", map.len())).font(crate::theme::bold(t.text_size)))
        .id_salt("live_ch")
        .default_open(true)
        .show(ui, |ui| {
            if !out_on {
                ui.weak("DMX output is off: these are the values that would be sent.");
            }
            const SHOW: usize = 48;
            egui::ScrollArea::vertical().max_height(190.0).id_salt("live_ch_scroll").show(ui, |ui| {
                egui::Grid::new("live_ch_grid").num_columns(4).spacing([10.0, 2.0]).striped(true).show(ui, |ui| {
                    for ch in map.iter().take(SHOW) {
                        let v = dmx.get(&(fx.lumiverse, ch.universe)).map(|u| u[ch.address as usize - 1]).unwrap_or(0);
                        ui.monospace(format!("U{} {:>3}", ch.universe, ch.address));
                        ui.label(&ch.label);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(110.0, 10.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, t.raised);
                        let col = match ch.func {
                            lumen_fixtures::ChannelFn::Red => Color32::from_rgb(230, 60, 60),
                            lumen_fixtures::ChannelFn::Green => Color32::from_rgb(60, 200, 90),
                            lumen_fixtures::ChannelFn::Blue => Color32::from_rgb(70, 120, 240),
                            lumen_fixtures::ChannelFn::White => Color32::from_gray(230),
                            lumen_fixtures::ChannelFn::Amber => Color32::from_rgb(255, 170, 40),
                            lumen_fixtures::ChannelFn::Fixed(_) => t.weak,
                            _ => t.accent,
                        };
                        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(r.width() * v as f32 / 255.0, r.height())), 2.0, col);
                        ui.monospace(format!("{v:>3}"));
                        ui.end_row();
                    }
                });
                if map.len() > SHOW {
                    ui.weak(format!("…and {} more channels. The DMX Monitor shows every universe in full.", map.len() - SHOW));
                }
            });
        });
}

/// "Test this light": solo one fixture and walk through colours and wiring.
pub fn test_row(test: &mut Option<crate::engine::FixtureTest>, output_on: &mut bool, fixture: usize, ui: &mut egui::Ui) {
    let t = crate::theme::current();
    let mine = test.is_some_and(|x| x.fixture == fixture);
    if !mine {
        if ui
            .button("Test this light")
            .on_hover_text("Lights only this fixture: red, green, blue, white, then a dot running from its first pixel. Everything else goes dark until you stop.")
            .clicked()
        {
            *test = Some(crate::engine::FixtureTest::new(fixture));
            *output_on = true;
        }
        return;
    }
    let x = test.expect("mine");
    egui::Frame::new().fill(t.raised).corner_radius(t.radius).inner_margin(8).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(egui::RichText::new("Stop test").color(t.on_accent)).fill(t.accent)).clicked() {
                *test = None;
            }
            ui.label(egui::RichText::new(format!("The light should be {}", x.step().expect())).font(crate::theme::bold(t.text_size)));
        });
        ui.weak(x.step().wrong());
    });
}

/// The selected fixture in the inspector: test and live values first, the
/// rest in folding sections. Returns true when "Edit type" was clicked.
fn fixture_inspector(
    o: &mut AdvancedOutput,
    idx: usize,
    test: &mut Option<crate::engine::FixtureTest>,
    out_on: &mut bool,
    dmx: &std::collections::BTreeMap<(usize, u16), [u8; lumen_fixtures::UNIVERSE_SIZE]>,
    ui: &mut egui::Ui,
) -> bool {
    let t = crate::theme::current();
    let mut edit = false;
    let range = o.range_label(&o.fixtures[idx]);
    let type_name = o.types.get(o.fixtures[idx].type_id).map(|t| t.name.clone()).unwrap_or_default();
    let lv_name = o.lumiverses.get(o.fixtures[idx].lumiverse).map(|l| l.name.clone()).unwrap_or_default();
    let all_groups = o.group_names();
    ui.add(egui::TextEdit::singleline(&mut o.fixtures[idx].name).font(crate::theme::bold(t.text_size + 4.0)).frame(egui::Frame::NONE));
    ui.weak(format!("{type_name} · {lv_name} · {range}"));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        test_row(test, out_on, idx, ui);
        if test.is_none_or(|x| x.fixture != idx) && ui.button("Edit type").on_hover_text("Open this fixture's type in the Fixture library").clicked() {
            edit = true;
        }
    });
    if test.is_some_and(|x| x.fixture == idx) {
        // test_row shows the instructions under the stop button when running
    }
    ui.add_space(6.0);
    live_channels(o, dmx, *out_on, idx, ui);
    ui.add_space(4.0);
    let base = o.lumiverses.get(o.fixtures[idx].lumiverse).map(|l| l.universe).unwrap_or(0);
    let has_white = o.types.get(o.fixtures[idx].type_id).is_some_and(|t| {
        t.pixel.contains(&lumen_fixtures::ChannelFn::White) || t.slots.as_ref().is_some_and(|s| s.iter().any(|x| x.func == lumen_fixtures::ChannelFn::White))
    });
    let types: Vec<String> = o.types.iter().map(|t| t.name.clone()).collect();
    let lvs: Vec<String> = o.lumiverses.iter().map(|l| format!("{} (U{})", l.name, l.universe)).collect();
    let fx = &mut o.fixtures[idx];
    egui::CollapsingHeader::new(egui::RichText::new("Address").font(crate::theme::bold(t.text_size)))
        .id_salt("insp_addr")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("insp_addr_g").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                ui.label("Output").on_hover_text("Which output (controller) this light is connected to.");
                egui::ComboBox::from_id_salt("fxlv").selected_text(lvs.get(fx.lumiverse).cloned().unwrap_or_default()).show_ui(ui, |ui| {
                    for (i, l) in lvs.iter().enumerate() {
                        ui.selectable_value(&mut fx.lumiverse, i, l);
                    }
                });
                ui.end_row();
                ui.label("Universe").on_hover_text("A universe is 512 channels: about 170 RGB pixels, or one controller output. Kickline counts from 0; if your controller counts from 1, add one.");
                let mut abs = base + fx.universe_offset;
                if ui.add(egui::DragValue::new(&mut abs).range(base..=32767)).changed() {
                    fx.universe_offset = abs.saturating_sub(base);
                }
                ui.end_row();
                ui.label("Address").on_hover_text("The DMX start channel (1–512) within the universe: the first channel this light listens to.");
                ui.add(egui::DragValue::new(&mut fx.address).range(1..=512));
                ui.end_row();
                ui.label("Type");
                egui::ComboBox::from_id_salt("fxtype").selected_text(types.get(fx.type_id).cloned().unwrap_or_default()).show_ui(ui, |ui| {
                    for (i, t) in types.iter().enumerate() {
                        ui.selectable_value(&mut fx.type_id, i, t);
                    }
                });
                ui.end_row();
            });
        });
    egui::CollapsingHeader::new(egui::RichText::new("Position & size").font(crate::theme::bold(t.text_size)))
        .id_salt("insp_pos")
        .show(ui, |ui| {
            egui::Grid::new("insp_pos_g").num_columns(4).spacing([8.0, 5.0]).show(ui, |ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(&mut fx.x).speed(0.002).range(-0.5..=1.5).max_decimals(3));
                ui.label("Y");
                ui.add(egui::DragValue::new(&mut fx.y).speed(0.002).range(-0.5..=1.5).max_decimals(3));
                ui.end_row();
                ui.label("W");
                ui.add(egui::DragValue::new(&mut fx.w).speed(0.002).range(0.005..=2.0).max_decimals(3));
                ui.label("H");
                ui.add(egui::DragValue::new(&mut fx.h).speed(0.002).range(0.005..=2.0).max_decimals(3));
                ui.end_row();
                ui.label("Rotation");
                ui.add(egui::DragValue::new(&mut fx.rotation).speed(0.5).suffix("°"));
                ui.end_row();
            });
        });
    egui::CollapsingHeader::new(egui::RichText::new("Wiring & colour").font(crate::theme::bold(t.text_size)))
        .id_salt("insp_wire")
        .show(ui, |ui| {
            ui.checkbox(&mut fx.snake, "Snake").on_hover_text("For panels wired back and forth: every other row runs the opposite way.");
            ui.checkbox(&mut fx.reverse, "Reverse").on_hover_text("The data enters at the other end. Tick it if effects run the wrong way along the strip.");
            if has_white {
                ui.horizontal(|ui| {
                    ui.label("White").on_hover_text("For RGBW lights: how white and pale colours are made.");
                    egui::ComboBox::from_id_salt("fxwhite").selected_text(fx.white.name()).show_ui(ui, |ui| {
                        for m in lumen_fixtures::WhiteMode::ALL {
                            ui.selectable_value(&mut fx.white, m, m.name());
                        }
                    });
                });
            }
        });
    egui::CollapsingHeader::new(egui::RichText::new(format!("Groups ({})", fx.groups.len())).font(crate::theme::bold(t.text_size)))
        .id_salt("insp_groups")
        .default_open(true)
        .show(ui, |ui| {
            if all_groups.is_empty() {
                ui.weak("No groups yet. Make them in Set up → Stage.");
            }
            ui.horizontal_wrapped(|ui| {
                for g in &all_groups {
                    let mut on = fx.groups.contains(g);
                    if ui.toggle_value(&mut on, g).changed() {
                        if on {
                            fx.groups.push(g.clone());
                        } else {
                            fx.groups.retain(|x| x != g);
                        }
                    }
                }
            });
        });
    edit
}

/// Pick a controller, say how many LEDs are on each output, and Kickline
/// adds one strip per output, addressed the way the controller counts.
fn controller_form(o: &mut AdvancedOutput, st: &mut State, ui: &mut egui::Ui) {
    use lumen_fixtures::{ControllerSetup, CONTROLLERS};
    let t = crate::theme::current();
    let Some(cs) = st.controller.as_mut() else { return };
    ui.label(egui::RichText::new("Add a controller").font(crate::theme::bold(t.text_size + 4.0)));
    ui.add_space(4.0);
    let mut pick = None;
    egui::Grid::new("ctrl_g").num_columns(2).spacing([10.0, 7.0]).show(ui, |ui| {
        ui.label("Controller");
        egui::ComboBox::from_id_salt("ctrl_kind").selected_text(cs.controller().name).show_ui(ui, |ui| {
            for (i, c) in CONTROLLERS.iter().enumerate() {
                if ui.selectable_label(cs.controller == i, c.name).clicked() {
                    pick = Some(i);
                }
            }
        });
        ui.end_row();
        ui.label("Name");
        ui.text_edit_singleline(&mut cs.name);
        ui.end_row();
        ui.label("Sends on");
        egui::ComboBox::from_id_salt("ctrl_lv").selected_text(o.lumiverses.get(cs.lumiverse).map(|l| l.name.clone()).unwrap_or_default()).show_ui(ui, |ui| {
            for (i, l) in o.lumiverses.iter().enumerate() {
                ui.selectable_value(&mut cs.lumiverse, i, &l.name);
            }
        });
        ui.end_row();
        if let Some(l) = o.lumiverses.get_mut(cs.lumiverse) {
            ui.label("Start universe").on_hover_text("The universe the controller's output 1 listens on (its \"start universe\").");
            ui.add(egui::DragValue::new(&mut l.universe).range(0..=32767));
            ui.end_row();
        }
        ui.label("LEDs");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut cs.rgbw, false, "RGB");
            ui.selectable_value(&mut cs.rgbw, true, "RGBW");
        });
        ui.end_row();
    });
    if let Some(i) = pick {
        let (name, lv) = (cs.name.clone(), cs.lumiverse);
        let old = cs.leds.clone();
        *cs = ControllerSetup::new(i, lv);
        for (k, n) in old.into_iter().enumerate().take(cs.leds.len()) {
            cs.leds[k] = n;
        }
        if !CONTROLLERS.iter().any(|c| c.name.starts_with(name.as_str())) {
            cs.name = name;
        }
    }
    ui.label(egui::RichText::new(cs.controller().note).color(t.weak).size(t.text_size - 2.0));
    ui.add_space(6.0);
    ui.label(egui::RichText::new("LEDs on each output").font(crate::theme::bold(t.text_size)));
    egui::Grid::new("ctrl_leds").num_columns(4).spacing([10.0, 5.0]).show(ui, |ui| {
        for k in 0..cs.leds.len() {
            ui.label(format!("Out {}", k + 1));
            ui.add(egui::DragValue::new(&mut cs.leds[k]).range(0..=8192).speed(1.0));
            if k % 2 == 1 {
                ui.end_row();
            }
        }
    });
    ui.add_space(4.0);
    let suggested = cs.suggested_universes();
    if cs.universes_per_output < suggested {
        cs.universes_per_output = suggested;
    }
    let max_u = cs.max_universes().max(1);
    ui.horizontal(|ui| {
        ui.label("Universes per output");
        ui.add(egui::DragValue::new(&mut cs.universes_per_output).range(1..=max_u));
        if cs.universes_per_output != suggested && ui.small_button(format!("Fewest ({suggested})")).clicked() {
            cs.universes_per_output = suggested;
        }
    })
    .response
    .on_hover_text("Set the same number on the controller. Output 2 starts right after output 1's universes, and so on.");
    ui.add_space(6.0);
    let plan = o.controller_plan(cs);
    egui::Grid::new("ctrl_plan").num_columns(3).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
        ui.label(egui::RichText::new("Output").color(t.weak));
        ui.label(egui::RichText::new("Universes").color(t.weak));
        ui.label(egui::RichText::new("").color(t.weak));
        ui.end_row();
        for r in plan.iter().filter(|r| r.leds > 0) {
            ui.label(format!("Out {} · {} LEDs", r.output, r.leds));
            ui.label(if r.universes.0 == r.universes.1 { format!("{}", r.universes.0) } else { format!("{} – {}", r.universes.0, r.universes.1) });
            if r.fits {
                ui.colored_label(t.ok, "✓");
            } else {
                ui.colored_label(t.err, "too long");
            }
            ui.end_row();
        }
    });
    if plan.iter().all(|r| r.leds == 0) {
        ui.weak("Enter the LED count for each output you use.");
    }
    if let Some(e) = &st.controller_error {
        ui.colored_label(t.err, e);
    }
    ui.add_space(8.0);
    let mut close = false;
    ui.horizontal(|ui| {
        let any = cs.leds.iter().any(|&n| n > 0);
        if ui.add_enabled(any, egui::Button::new(egui::RichText::new("Add strips").color(t.on_accent)).fill(t.accent)).clicked() {
            match o.add_controller(cs) {
                Ok(r) => {
                    st.selected = Some(r.start);
                    st.lumiverse = cs.lumiverse;
                    close = true;
                }
                Err(e) => st.controller_error = Some(e),
            }
        }
        if ui.button("Cancel").clicked() {
            close = true;
        }
    });
    ui.weak("Each output becomes one strip in a group named after the controller. Move and shape them on the canvas afterwards.");
    if close {
        st.controller = None;
    }
}


pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    let View { e, s } = app;
    let (output, frame, st, routes, dmx) = (&mut e.output, &e.frame, &mut s.ao, &e.routes, &e.dmx);
    let (windows, fe) = (&mut s.windows, &mut s.fe);
    let (test, out_on, sweep) = (&mut e.test, &mut e.output_enabled, &mut e.sweep);
    let watcher = s.nodes.clone();
    let nodes = watcher.nodes();
    let mut edit_type = None;
    let aspect = output.aspect;
    egui::Panel::left("ao_left")
        .default_size(290.0)
        .resizable(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| lumiverse_panel(output, st, routes, &nodes, &watcher, sweep, out_on, ui, true, false));
        });

    // Inspector: the selected fixture, or the selected output's settings.
    egui::Panel::right("ao_inspector")
        .default_size(310.0)
        .resizable(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("ao_insp").show(ui, |ui| match st.selected.filter(|&i| i < output.fixtures.len()) {
                _ if st.controller.is_some() => controller_form(output, st, ui),
                Some(i) => {
                    if fixture_inspector(output, i, test, out_on, dmx, ui) {
                        edit_type = Some(output.fixtures[i].type_id);
                    }
                    ui.add_space(8.0);
                    if ui.small_button("Show this output's settings").clicked() {
                        st.lumiverse = output.fixtures[i].lumiverse;
                        st.selected = None;
                    }
                }
                None => {
                    let t = crate::theme::current();
                    if let Some(l) = output.lumiverses.get(st.lumiverse) {
                        ui.label(egui::RichText::new(&l.name).font(crate::theme::bold(t.text_size + 4.0)));
                        ui.weak(format!("{} · universe {} · {}", l.protocol.name(), l.universe, l.target));
                        ui.add_space(6.0);
                    }
                    lumiverse_panel(output, st, routes, &nodes, &watcher, sweep, out_on, ui, false, true);
                    ui.add_space(8.0);
                    ui.weak("Click a fixture on the canvas or in the list to edit it.");
                }
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Drag to move · corner handle to resize · round handle to rotate");
            ui.checkbox(&mut st.show_pixels, "Show pixel colours");
        });
        let avail = ui.available_size() - Vec2::new(0.0, 8.0);
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

    });
    if let Some(t) = edit_type {
        windows.fixture_editor = true;
        fe.selected = t;
        fe.browsing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_output::ArtNode;
    use std::time::{Duration, Instant};

    fn seen(ip: [u8; 4], unis: &[u16], ago: u64) -> SeenNode {
        SeenNode {
            node: ArtNode { ip: ip.into(), short_name: "Pix".into(), long_name: "PixLite".into(), universes: unis.to_vec(), mac: None },
            port: Some("en5".into()),
            last_seen: Instant::now() - Duration::from_secs(ago),
        }
    }

    #[test]
    fn node_status_rules() {
        let mut o = AdvancedOutput::demo();
        let lv = 0;
        o.lumiverses[lv].target = "2.0.0.50".into();
        let used = o.universes_used(lv);
        // Nobody answered yet.
        assert_eq!(node_status(&o, lv, &[]), NodeStatus::Waiting);
        // Right node, right universes.
        assert_eq!(node_status(&o, lv, &[seen([2, 0, 0, 50], &used, 0)]), NodeStatus::Online("PixLite".into()));
        // Node counts from 1: everything shifted.
        let shifted: Vec<u16> = used.iter().map(|u| u + 1).collect();
        assert!(matches!(node_status(&o, lv, &[seen([2, 0, 0, 50], &shifted, 0)]), NodeStatus::Missing { .. }));
        // Went quiet.
        assert!(matches!(node_status(&o, lv, &[seen([2, 0, 0, 50], &used, 30)]), NodeStatus::Lost { secs: 30, .. }));
        // Broadcast: matched by universe, not IP.
        o.lumiverses[lv].target = "255.255.255.255".into();
        assert_eq!(node_status(&o, lv, &[seen([2, 0, 0, 77], &used, 0)]), NodeStatus::Online("PixLite".into()));
        assert_eq!(node_status(&o, lv, &[seen([2, 0, 0, 77], &[99], 0)]), NodeStatus::Waiting);
        // sACN: not applicable.
        o.lumiverses[lv].protocol = Protocol::Sacn;
        assert_eq!(node_status(&o, lv, &[]), NodeStatus::NotArtNet);
    }
}
