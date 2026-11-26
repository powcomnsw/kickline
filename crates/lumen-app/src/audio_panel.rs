//! Audio window: pick the input, watch each band, tune the detectors and
//! see which layers react to what.

use crate::engine::AudioSource;
use crate::util::band_color;
use crate::View;
use eframe::egui::{self, Color32, Rect, Sense, Vec2};
use lumen_audio::{list_devices, AudioInput, KickResponse};
use lumen_core::{AudioBand, Effect, Generator};

fn meter(ui: &mut egui::Ui, band: AudioBand, v: f32, hits: Option<u64>) {
    ui.horizontal(|ui| {
        ui.add_sized([64.0, 18.0], egui::Label::new(band.name()));
        let (resp, p) = ui.allocate_painter(Vec2::new(300.0, 16.0), Sense::hover());
        let r = resp.rect;
        p.rect_filled(r, 3.0, Color32::from_gray(28));
        let fill = Rect::from_min_size(r.min, Vec2::new(r.width() * v.clamp(0.0, 1.0), r.height()));
        p.rect_filled(fill, 3.0, band_color(band, 0.35 + 0.65 * v));
        match hits {
            Some(h) => ui.monospace(format!("{h} hits")),
            None => ui.monospace(format!("{:.0}%", v * 100.0)),
        };
    });
}

/// Which layers use which band — the "routing" at a glance.
fn routing(app: &View) -> Vec<(String, AudioBand)> {
    let mut out = Vec::new();
    for l in &app.e.comp.layers {
        if let Some(link) = l.audio_opacity {
            out.push((format!("{}: opacity", l.name), link.band));
        }
        if let Some(b) = l.advance_on {
            out.push((format!("{}: next clip", l.name), b));
        }
        for e in &l.effects {
            if let Effect::AudioBrightness { band, .. } | Effect::AudioHue { band, .. } = e {
                out.push((format!("{}: {}", l.name, e.name()), *band));
            }
        }
        for c in l.clips.iter().flatten() {
            if let Generator::AudioFlash { band, .. } = c.generator {
                out.push((format!("{}: clip \"{}\"", l.name, c.name), band));
            }
        }
    }
    out
}

enum Pick {
    Off,
    Demo,
    App(usize),
    Device(usize),
}

fn refresh_sources(app: &mut View) {
    app.audio_devices = list_devices();
    app.audio_apps = lumen_audio::mac_tap::list_audio_apps();
}

fn source_file() -> std::path::PathBuf {
    crate::paths::data_dir().join("audio_source.txt")
}

/// Remember the source for next launch: "off", "demo", "app:<bundle id>" or "device:<id>".
fn remember(what: &str) {
    let _ = std::fs::write(source_file(), what);
}

/// The source used last time, if any.
pub fn saved() -> Option<String> {
    let s = std::fs::read_to_string(source_file()).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty() && s != "off").then_some(s)
}

/// Reopen the source from last time. Returns true when finished (opened, or
/// not worth trying again); false when the app isn't running yet.
pub fn restore(app: &mut View, what: &str) -> bool {
    refresh_sources(app);
    if what == "demo" {
        app.start_demo_audio();
        return true;
    }
    if let Some(bundle) = what.strip_prefix("app:") {
        let b = bundle.to_ascii_lowercase();
        let found = app.audio_apps.iter().position(|a| a.bundle_id.to_ascii_lowercase() == b).or_else(|| {
            if b.contains("rekordbox") {
                lumen_audio::mac_tap::find_rekordbox(&app.audio_apps)
                    .and_then(|r| app.audio_apps.iter().position(|a| a.pid == r.pid))
            } else {
                None
            }
        });
        return match found {
            Some(i) => {
                start(app, Pick::App(i));
                true
            }
            None => false,
        };
    }
    if let Some(id) = what.strip_prefix("device:") {
        if let Some(i) = app.audio_devices.iter().position(|d| d.id == id) {
            start(app, Pick::Device(i));
        } else {
            app.audio_status = "Last audio device isn't connected".into();
        }
        return true;
    }
    true
}

fn start(app: &mut View, pick: Pick) {
    match pick {
        Pick::Off => {
            app.stop_audio();
            remember("off");
        }
        Pick::Demo => {
            app.start_demo_audio();
            remember("demo");
        }
        Pick::App(i) => {
            let info = app.audio_apps[i].clone();
            app.audio_input = None; // stop the old stream first
            match lumen_audio::mac_tap::start_app_capture(&info, Some(app.wake())) {
                Ok(input) => {
                    app.use_input(input);
                    remember(&format!("app:{}", info.bundle_id));
                }
                Err(e) => {
                    app.audio_status = format!("Couldn't capture {}: {e}", info.name);
                    crate::log::error("audio", app.audio_status.clone());
                }
            }
        }
        Pick::Device(i) => {
            let d = app.audio_devices[i].clone();
            app.audio_input = None;
            match AudioInput::start(Some(&d.id), Some(app.wake())) {
                Ok(input) => {
                    app.use_input(input);
                    remember(&format!("device:{}", d.id));
                }
                Err(e) => {
                    app.audio_status = format!("Couldn't open device: {e}");
                    crate::log::error("audio", format!("couldn't open {}: {e}", d.label()));
                }
            }
        }
    }
}

fn timing(app: &mut View, ui: &mut egui::Ui) {
    let link_ok = crate::link::LinkSync::AVAILABLE;
    ui.add_enabled_ui(link_ok, |ui| {
        ui.checkbox(
            &mut app.e.sync.link_enabled,
            "Follow rekordbox via Ableton Link",
        )
        .on_hover_text(
            "rekordbox Performance mode: press LINK in the global section and on the decks.",
        );
    });
    if !link_ok {
        ui.weak("This build was made without Ableton Link.");
    }
    if let Some(st) = &app.e.link_status {
        if st.peers > 0 {
            ui.colored_label(
                crate::theme::current().ok,
                format!(
                    "Synced: {} peer(s), {:.2} BPM, beat {:.2}",
                    st.peers, st.tempo, st.beat
                ),
            );
        } else {
            ui.colored_label(
                crate::theme::current().warn,
                "Waiting for rekordbox. Turn on LINK in rekordbox, same Wi-Fi/LAN.",
            );
        }
    }
    ui.add_enabled(
        app.e.sync.link_enabled,
        egui::Slider::new(&mut app.e.sync.lookahead_ms, 0.0..=100.0)
            .suffix(" ms")
            .text("fire early (Link)"),
    )
    .on_hover_text(
        "Beat and Bar triggers fire this much before the beat, to cancel node and fixture delay.",
    );
    ui.add(
        egui::Slider::new(&mut app.e.sync.send_rate_hz, 20.0..=120.0)
            .suffix(" Hz")
            .text("DMX refresh"),
    )
    .on_hover_text("44 Hz matches a full DMX universe. Higher helps pixel controllers.");
    ui.checkbox(
        &mut app.e.sync.send_on_hit,
        "Send immediately on kick / snare / hat",
    );
    ui.horizontal(|ui| {
        ui.label("Kick response");
        egui::ComboBox::from_id_salt("kickresp")
            .selected_text(app.e.audio_settings.kick_response.name())
            .show_ui(ui, |ui| {
                for k in KickResponse::ALL {
                    ui.selectable_value(&mut app.e.audio_settings.kick_response, k, k.name());
                }
            });
    });
    let m = &app.e.metrics;
    let (hit, n) = m.hit_latency();
    egui::Grid::new("lat")
        .num_columns(2)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            ui.weak("Audio input");
            ui.label(if m.input_latency_ms > 0.0 {
                format!("{:.1} ms", m.input_latency_ms)
            } else {
                "—".into()
            });
            ui.end_row();
            ui.weak("Hit → DMX sent");
            ui.label(if n > 0 {
                format!("{hit:.1} ms")
            } else {
                "—".into()
            });
            ui.end_row();
            ui.weak("DMX rate");
            ui.label(format!("{:.0} Hz", m.output_hz()));
            ui.end_row();
        });
}

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    if app.audio_devices.is_empty()
        && app.audio_apps.is_empty()
        && ui.ctx().cumulative_frame_nr() % 300 == 0
    {
        refresh_sources(app);
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut pick: Option<Pick> = None;
        ui.horizontal(|ui| {
            ui.label("Source");
            egui::ComboBox::from_id_salt("audiosrc").width(280.0).selected_text(app.e.audio.label()).show_ui(ui, |ui| {
                if ui.selectable_label(matches!(app.e.audio, AudioSource::Off), "Off").clicked() {
                    pick = Some(Pick::Off);
                }
                if ui.selectable_label(matches!(app.e.audio, AudioSource::Demo { .. }), "Demo beat (built-in)").clicked() {
                    pick = Some(Pick::Demo);
                }
                if !app.audio_apps.is_empty() {
                    ui.separator();
                    ui.weak("Apps — captured directly");
                    for (i, a) in app.audio_apps.iter().enumerate() {
                        if ui.selectable_label(false, a.label()).clicked() {
                            pick = Some(Pick::App(i));
                        }
                    }
                }
                if !app.audio_devices.is_empty() {
                    ui.separator();
                    ui.weak("Devices");
                    for (i, d) in app.audio_devices.iter().enumerate() {
                        if ui.selectable_label(false, d.label()).clicked() {
                            pick = Some(Pick::Device(i));
                        }
                    }
                }
            });
            if ui.button("Refresh").clicked() {
                refresh_sources(app);
            }
        });
        // One-click rekordbox.
        match lumen_audio::mac_tap::supported() {
            Ok(()) => match lumen_audio::mac_tap::find_rekordbox(&app.audio_apps).map(|r| r.pid) {
                Some(pid) => {
                    let i = app.audio_apps.iter().position(|a| a.pid == pid).unwrap();
                    let already = matches!(&app.e.audio, AudioSource::Live(f) if f.kind == lumen_audio::FeedKind::App);
                    if !already && ui.button("Use rekordbox (direct)").clicked() {
                        pick = Some(Pick::App(i));
                    }
                }
                None => {
                    ui.weak("rekordbox not found. Start it and play a track, then press Refresh.");
                }
            },
            Err(why) => {
                ui.weak(why);
            }
        }
        if let Some(p) = pick {
            start(app, p);
        }
        ui.weak(&app.audio_status);
        ui.collapsing("Getting rekordbox audio in", |ui| {
            ui.label("Kickline listens to what rekordbox plays and splits it into bands itself.");
            ui.label("• macOS 14.2+: \"Use rekordbox (direct)\". The first time, allow Kickline under Privacy & Security › Screen & System Audio Recording.");
            ui.label("• Windows: choose the device rekordbox outputs to, marked (loopback).");
            ui.label("• Or run a cable from the controller's booth out into an audio interface input.");
            ui.label("• BlackHole still works if you prefer it.");
        });
        ui.collapsing("Timing & sync", |ui| timing(app, ui));

        ui.separator();
        ui.add(egui::Slider::new(&mut app.e.audio_settings.gain, 0.1..=8.0).logarithmic(true).text("Input gain"));
        ui.add_space(4.0);
        ui.strong("Frequency bands");
        for (i, b) in AudioBand::ALL[..6].iter().enumerate() {
            meter(ui, *b, app.e.bands.values[i], None);
        }
        ui.add_space(4.0);
        ui.strong("Drum hits");
        for (i, b) in AudioBand::DRUMS.iter().enumerate() {
            meter(ui, *b, app.e.bands.get(*b), Some(app.e.bands.hits[i]));
        }
        drum_tuning(app, ui);
        ui.strong(if app.e.link_status.as_ref().is_some_and(|s| s.peers > 0) { "Clock (Ableton Link)" } else { "Clock (BPM)" });
        for b in [AudioBand::Beat, AudioBand::Bar] {
            meter(ui, b, app.e.bands.get(b), None);
        }

        ui.collapsing("Tuning", |ui| {
            egui::Grid::new("tune").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("");
                ui.weak("flash fade (s)");
                ui.end_row();
                for (i, b) in AudioBand::DRUMS.iter().enumerate() {
                    ui.label(b.name());
                    ui.add(egui::Slider::new(&mut app.e.audio_settings.pulse_decay[i], 0.02..=0.6));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            for (i, b) in AudioBand::ALL[..6].iter().enumerate() {
                ui.add(egui::Slider::new(&mut app.e.audio_settings.band_gain[i], 0.0..=3.0).text(format!("{} gain", b.name())));
            }
        });

        ui.separator();
        ui.strong("Routing");
        let r = routing(app);
        if r.is_empty() {
            ui.weak("No layer reacts to audio yet. Use \"Opacity follows\", \"Next clip on\", audio effects or Audio flash clips.");
        }
        egui::Grid::new("routing").num_columns(2).striped(true).spacing([12.0, 3.0]).show(ui, |ui| {
            for (what, band) in r {
                ui.colored_label(band_color(band, 1.0), format!("● {}", band.name()));
                ui.label(what);
                ui.end_row();
            }
        });
    });
}

/// Sensitivity per drum, with a live "how close to a hit" bar: the bar is the
/// latest rise in that band; reaching the line counts as a hit.
fn drum_tuning(app: &mut View, ui: &mut egui::Ui) {
    let t = crate::theme::current();
    ui.add_space(2.0);
    ui.weak("Sensitivity: move right if hits are missed, left if it fires on the wrong things. Watch the bar: crossing the line is a hit. Works the same at any volume.");
    egui::Grid::new("drum_sens").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
        for (i, b) in AudioBand::DRUMS.iter().enumerate() {
            ui.label(b.name());
            ui.add(egui::Slider::new(&mut app.e.audio_settings.sensitivity[i], 0.3..=3.0).show_value(false));
            let (rise, need) = app.e.onset[i];
            let (r, resp) = ui.allocate_exact_size(egui::vec2(120.0, 14.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 3.0, t.raised);
            let frac = if need > 0.0 { (rise.max(0.0) / need / 1.5).clamp(0.0, 1.0) } else { 0.0 };
            let lit = app.e.bands.get(*b) > 0.5;
            let fill = if lit { t.accent } else { band_color(*b, 1.0) };
            p.rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(r.width() * frac, r.height())), 3.0, fill);
            let x = r.min.x + r.width() / 1.5;
            p.line_segment([egui::pos2(x, r.min.y - 2.0), egui::pos2(x, r.max.y + 2.0)], egui::Stroke::new(1.5, t.text));
            resp.on_hover_text(format!("Rise {rise:.1} dB, needs {need:.1} dB"));
            ui.end_row();
        }
    });
}
