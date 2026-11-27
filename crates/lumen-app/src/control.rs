//! MIDI controllers and OSC: the ports, the learn page, and what a control
//! does to the show. The mapping itself is `lumen_core::controls`.

use crate::engine::Engine;
use crate::{theme, View};
use eframe::egui::{self, RichText};
use lumen_core::controls::{ControlMap, Event, Message, Target};
use lumen_core::Section;
use std::net::UdpSocket;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// Open ports and the queue of what has arrived.
pub struct Io {
    tx: Sender<Message>,
    rx: Receiver<Message>,
    midi: Option<(String, midir::MidiInputConnection<()>)>,
    /// MIDI inputs seen at the last scan.
    pub ports: Vec<String>,
    scanned: Option<Instant>,
    osc_in: Option<(u16, UdpSocket)>,
    osc_out: Option<(String, UdpSocket)>,
    sent: Vec<(String, f32)>,
    sent_at: Option<Instant>,
    pub error: Option<String>,
    /// The last thing that arrived, for the page to show.
    pub last: Option<(String, Instant)>,
}

impl Default for Io {
    fn default() -> Self {
        let (tx, rx) = channel();
        Io { tx, rx, midi: None, ports: Vec::new(), scanned: None, osc_in: None, osc_out: None, sent: Vec::new(), sent_at: None, error: None, last: None }
    }
}

fn list_ports() -> Vec<String> {
    let Ok(input) = midir::MidiInput::new("Kickline scan") else { return Vec::new() };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
}

fn open_midi(name: &str, tx: Sender<Message>) -> Result<midir::MidiInputConnection<()>, String> {
    let input = midir::MidiInput::new("Kickline").map_err(|e| e.to_string())?;
    let port = input.ports().into_iter().find(|p| input.port_name(p).as_deref() == Ok(name)).ok_or_else(|| format!("{name} is not connected"))?;
    input
        .connect(
            &port,
            "kickline-in",
            move |_, bytes, _| {
                if let Some(m) = Message::from_midi(bytes) {
                    let _ = tx.send(m);
                }
            },
            (),
        )
        .map_err(|e| e.to_string())
}

/// The first argument of an OSC message as a number.
fn osc_value(args: &[rosc::OscType]) -> f32 {
    use rosc::OscType::*;
    match args.first() {
        Some(Float(f)) => *f,
        Some(Double(d)) => *d as f32,
        Some(Int(i)) => *i as f32,
        Some(Long(i)) => *i as f32,
        Some(Bool(b)) => *b as u8 as f32,
        _ => 1.0,
    }
}

fn flatten(p: rosc::OscPacket, out: &mut Vec<Message>) {
    match p {
        rosc::OscPacket::Message(m) => out.push(Message::Osc { value: osc_value(&m.args), address: m.addr }),
        rosc::OscPacket::Bundle(b) => b.content.into_iter().for_each(|p| flatten(p, out)),
    }
}

impl Io {
    /// Keep the ports in line with the settings and return what arrived.
    pub fn poll(&mut self, map: &ControlMap) -> Vec<Message> {
        let now = Instant::now();
        // MIDI: look for the chosen controller about once a second, so
        // unplugging and replugging it just works.
        if self.scanned.is_none_or(|t| now.duration_since(t) > Duration::from_millis(1500)) {
            self.scanned = Some(now);
            self.ports = list_ports();
            match &map.midi_port {
                Some(want) if self.midi.as_ref().is_none_or(|(n, _)| n != want) => {
                    self.midi = None;
                    if self.ports.contains(want) {
                        match open_midi(want, self.tx.clone()) {
                            Ok(c) => {
                                self.midi = Some((want.clone(), c));
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                }
                None => self.midi = None,
                _ => {
                    if self.midi.is_some() && map.midi_port.as_ref().is_some_and(|w| !self.ports.contains(w)) {
                        self.midi = None;
                    }
                }
            }
            // OSC in.
            if self.osc_in.as_ref().map(|(p, _)| *p) != map.osc_in {
                self.osc_in = None;
                if let Some(port) = map.osc_in {
                    match UdpSocket::bind(("0.0.0.0", port)).and_then(|s| s.set_nonblocking(true).map(|_| s)) {
                        Ok(s) => {
                            self.osc_in = Some((port, s));
                            self.error = None;
                        }
                        Err(e) => self.error = Some(format!("OSC port {port}: {e}")),
                    }
                }
            }
            // OSC out.
            if self.osc_out.as_ref().map(|(a, _)| a) != map.osc_out.as_ref() {
                self.osc_out = None;
                self.sent.clear();
                if let Some(addr) = &map.osc_out {
                    match UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| s.connect(addr.as_str()).map(|_| s)) {
                        Ok(s) => self.osc_out = Some((addr.clone(), s)),
                        Err(e) => self.error = Some(format!("OSC out {addr}: {e}")),
                    }
                }
            }
        }
        let mut got: Vec<Message> = Vec::new();
        if let Some((_, sock)) = &self.osc_in {
            let mut buf = [0u8; 2048];
            while let Ok((n, _)) = sock.recv_from(&mut buf) {
                if let Ok((_, p)) = rosc::decoder::decode_udp(&buf[..n]) {
                    flatten(p, &mut got);
                }
            }
        }
        got.extend(self.rx.try_iter());
        if let Some(m) = got.last() {
            self.last = Some((format!("{m:?}"), now));
        }
        got
    }

    /// Tell a remote (TouchOSC and the like) where the faders are.
    pub fn send_state(&mut self, e: &Engine) {
        let Some((_, sock)) = &self.osc_out else { return };
        let now = Instant::now();
        if self.sent_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(100)) {
            return;
        }
        self.sent_at = Some(now);
        let state = state_of(e);
        for (addr, v) in state {
            if self.sent.iter().find(|(a, _)| *a == addr).is_some_and(|(_, old)| (old - v).abs() < 0.004) {
                continue;
            }
            let pkt = rosc::OscPacket::Message(rosc::OscMessage { addr: addr.clone(), args: vec![rosc::OscType::Float(v)] });
            if let Ok(bytes) = rosc::encoder::encode(&pkt) {
                let _ = sock.send(&bytes);
            }
            match self.sent.iter_mut().find(|(a, _)| *a == addr) {
                Some(s) => s.1 = v,
                None => self.sent.push((addr, v)),
            }
        }
    }
}

/// The values a remote shows: faders and switches, by OSC address.
pub fn state_of(e: &Engine) -> Vec<(String, f32)> {
    vec![
        (Target::Master.osc_address(), e.comp.master),
        (Target::Left.osc_address(), e.comp.gains.left),
        (Target::Centre.osc_address(), e.comp.gains.centre),
        (Target::Right.osc_address(), e.comp.gains.right),
        (Target::Crossfade.osc_address(), e.comp.crossfade),
        (Target::Blackout.osc_address(), e.blackout as u8 as f32),
        (Target::Auto.osc_address(), e.comp.director.on as u8 as f32),
    ]
}

/// Do what a control asked. Returns true when it was the tap button (the
/// caller owns the tap timing).
pub fn apply(e: &mut Engine, ev: Event) -> bool {
    match ev {
        Event::Set(t, x) => {
            let x = x.clamp(0.0, 1.0);
            match t {
                Target::Master => e.comp.master = x,
                Target::Left => e.comp.gains.left = x,
                Target::Centre => e.comp.gains.centre = x,
                Target::Right => e.comp.gains.right = x,
                Target::Crossfade => e.comp.crossfade = x,
                _ => {}
            }
        }
        Event::Press(t) => match t {
            Target::Blackout => e.blackout = !e.blackout,
            Target::Auto => {
                e.comp.director.on = !e.comp.director.on;
                e.comp.director_rt.hold = false;
            }
            Target::Drop => e.comp.sections.force(Section::BuildUp, 64),
            Target::Column(c) => e.comp.trigger_column(c as usize),
            Target::Tap => return true,
            _ => {}
        },
        Event::Release(Target::Drop) => e.comp.sections.force(Section::Drop, 16),
        Event::Release(_) => {}
    }
    false
}

/// Run once a frame: read the ports, act, and answer a remote.
pub fn tick(v: &mut View) {
    let map = v.e.comp.controls.clone();
    let msgs = v.s.control.poll(&map);
    let mut tap = false;
    for m in msgs {
        let events = v.e.comp.controls.handle(&m);
        for ev in events {
            tap |= apply(v.e, ev);
        }
    }
    if tap {
        v.tap(false);
    }
    v.s.control.send_state(v.e);
}

/// Set up → Controllers.
pub fn page(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(8.0);
        ui.label(RichText::new("Controllers").font(theme::bold(t.text_size + 4.0)));
        ui.label(RichText::new("Drive the faders, buttons and columns from a MIDI controller or an OSC app. Pick a control, press Learn, then move a knob or hit a pad.").color(t.weak));
        ui.add_space(10.0);
        // MIDI input.
        ui.label(RichText::new("MIDI").font(theme::bold(t.text_size)));
        let ports = v.s.control.ports.clone();
        let current = v.e.comp.controls.midi_port.clone();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("midi_port")
                .width(260.0)
                .selected_text(current.clone().unwrap_or_else(|| "No controller".into()))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "No controller").clicked() {
                        v.e.comp.controls.midi_port = None;
                    }
                    for p in &ports {
                        if ui.selectable_label(current.as_deref() == Some(p), p).clicked() {
                            v.e.comp.controls.midi_port = Some(p.clone());
                        }
                    }
                });
            let live = v.s.control.midi.is_some();
            match (&current, live) {
                (Some(_), true) => ui.colored_label(t.ok, "connected"),
                (Some(_), false) => ui.colored_label(t.warn, "not found: plug it in and it will connect"),
                _ => ui.label(RichText::new("off").color(t.weak)),
            };
        });
        if ports.is_empty() {
            ui.label(RichText::new("No MIDI inputs found.").color(t.weak));
        }
        ui.add_space(10.0);
        // OSC.
        ui.label(RichText::new("OSC").font(theme::bold(t.text_size)));
        ui.horizontal(|ui| {
            let mut on = v.e.comp.controls.osc_in.is_some();
            if ui.checkbox(&mut on, "Listen on port").changed() {
                v.e.comp.controls.osc_in = on.then_some(9000);
            }
            if let Some(p) = &mut v.e.comp.controls.osc_in {
                ui.add(egui::DragValue::new(p).range(1024..=65535));
            }
        });
        ui.horizontal(|ui| {
            let mut on = v.e.comp.controls.osc_out.is_some();
            if ui.checkbox(&mut on, "Send fader positions to").on_hover_text("So a TouchOSC or similar app follows the show. Use host:port, like 192.168.1.20:9001").changed() {
                v.e.comp.controls.osc_out = on.then(|| "127.0.0.1:9001".to_string());
            }
            if let Some(a) = &mut v.e.comp.controls.osc_out {
                ui.add(egui::TextEdit::singleline(a).desired_width(180.0));
            }
        });
        ui.label(RichText::new("Addresses that work with no setup: /kickline/master, /left, /centre, /right, /crossfade, /blackout, /drop, /tap, /auto, /column/1…").color(t.weak).size(t.text_size - 1.0));
        if let Some(e) = &v.s.control.error {
            ui.colored_label(t.err, e);
        }
        if let Some((m, at)) = &v.s.control.last {
            if at.elapsed() < Duration::from_secs(4) {
                ui.label(RichText::new(format!("Heard: {m}")).color(t.weak).size(t.text_size - 1.0));
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new("Controls").font(theme::bold(t.text_size)));
        let cols = v.e.comp.columns;
        egui::Grid::new("controls_grid").num_columns(3).spacing([14.0, 8.0]).show(ui, |ui| {
            for tg in Target::all(cols) {
                ui.label(tg.name());
                let learning = v.e.comp.controls.learn == Some(tg);
                let bound = v.e.comp.controls.source_of(tg).map(|s| s.describe());
                ui.label(RichText::new(if learning { "Waiting for a control…".to_string() } else { bound.clone().unwrap_or_else(|| "not set".into()) }).color(if learning { t.warn } else { t.weak }));
                ui.horizontal(|ui| {
                    if ui.selectable_label(learning, "Learn").clicked() {
                        v.e.comp.controls.learn = if learning { None } else { Some(tg) };
                    }
                    if bound.is_some() && ui.small_button("Clear").clicked() {
                        v.e.comp.controls.clear(tg);
                    }
                });
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::controls::Source;

    fn engine() -> Engine {
        let p = crate::stage::example_show();
        Engine::new(p.composition, p.output)
    }

    #[test]
    fn faders_buttons_and_columns_drive_the_show() {
        let mut e = engine();
        let mut map = ControlMap::default();
        map.bind(Source::Cc { channel: 0, number: 1 }, Target::Master);
        map.bind(Source::Cc { channel: 0, number: 2 }, Target::Left);
        map.bind(Source::Note { channel: 0, number: 36 }, Target::Blackout);
        map.bind(Source::Note { channel: 0, number: 37 }, Target::Drop);
        let send = |e: &mut Engine, map: &mut ControlMap, m: Message| {
            for ev in map.handle(&m) {
                apply(e, ev);
            }
        };
        send(&mut e, &mut map, Message::Cc { channel: 0, number: 1, value: 0 });
        assert_eq!(e.comp.master, 0.0);
        send(&mut e, &mut map, Message::Cc { channel: 0, number: 2, value: 127 });
        assert_eq!(e.comp.gains.left, 1.0);
        send(&mut e, &mut map, Message::NoteOn { channel: 0, number: 36, velocity: 100 });
        assert!(e.blackout);
        send(&mut e, &mut map, Message::NoteOff { channel: 0, number: 36 });
        assert!(e.blackout, "releasing a toggle does nothing");
        send(&mut e, &mut map, Message::NoteOn { channel: 0, number: 37, velocity: 100 });
        assert_eq!(e.comp.sections.current(), Section::BuildUp);
        send(&mut e, &mut map, Message::NoteOff { channel: 0, number: 37 });
        assert_eq!(e.comp.sections.current(), Section::Drop);
        // OSC needs no setup.
        send(&mut e, &mut map, Message::Osc { address: "/kickline/crossfade".into(), value: 0.9 });
        assert!((e.comp.crossfade - 0.9).abs() < 1e-6);
        send(&mut e, &mut map, Message::Osc { address: "/kickline/auto".into(), value: 1.0 });
        assert!(e.comp.director.on);
        assert!(apply(&mut e, Event::Press(Target::Tap)), "tap is handed back to the caller");
    }

    #[test]
    fn osc_packets_are_read_including_bundles() {
        let msg = |a: &str, args| rosc::OscPacket::Message(rosc::OscMessage { addr: a.into(), args });
        let bundle = rosc::OscPacket::Bundle(rosc::OscBundle {
            timetag: (0, 1).into(),
            content: vec![msg("/kickline/master", vec![rosc::OscType::Float(0.5)]), msg("/kickline/tap", vec![])],
        });
        let bytes = rosc::encoder::encode(&bundle).unwrap();
        let (_, p) = rosc::decoder::decode_udp(&bytes).unwrap();
        let mut out = Vec::new();
        flatten(p, &mut out);
        assert_eq!(out, vec![Message::Osc { address: "/kickline/master".into(), value: 0.5 }, Message::Osc { address: "/kickline/tap".into(), value: 1.0 }]);
    }

    #[test]
    fn the_state_a_remote_sees_follows_the_faders() {
        let mut e = engine();
        e.comp.gains.centre = 0.25;
        e.blackout = true;
        let s = state_of(&e);
        assert!(s.contains(&("/kickline/centre".to_string(), 0.25)));
        assert!(s.contains(&("/kickline/blackout".to_string(), 1.0)));
    }

    #[test]
    fn osc_arrives_over_a_real_socket_and_the_state_goes_back_out() {
        // Listen on a free port, send a packet to it, read it through Io.
        let free = UdpSocket::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let mut io = Io::default();
        let mut map = ControlMap::default();
        map.osc_in = Some(free);
        assert!(io.poll(&map).is_empty());
        let tx = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let pkt = rosc::OscPacket::Message(rosc::OscMessage { addr: "/kickline/left".into(), args: vec![rosc::OscType::Float(0.3)] });
        tx.send_to(&rosc::encoder::encode(&pkt).unwrap(), ("127.0.0.1", free)).unwrap();
        let mut got = Vec::new();
        for _ in 0..50 {
            got = io.poll(&map);
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, vec![Message::Osc { address: "/kickline/left".into(), value: 0.3 }]);

        // Out: a listener receives the master fader.
        let rx = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        rx.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        map.osc_out = Some(rx.local_addr().unwrap().to_string());
        io.scanned = None;
        io.poll(&map);
        let mut e = engine();
        e.comp.master = 0.6;
        io.send_state(&e);
        let mut buf = [0u8; 512];
        let n = rx.recv(&mut buf).unwrap();
        let (_, p) = rosc::decoder::decode_udp(&buf[..n]).unwrap();
        let mut seen = Vec::new();
        flatten(p, &mut seen);
        assert!(matches!(&seen[0], Message::Osc { address, .. } if address.starts_with("/kickline/")));
    }
}
