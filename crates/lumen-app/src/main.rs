mod advanced_output;
mod auto;
mod backdrop;
mod audio_panel;
mod clips;
mod control;
mod diagnostics;
mod dmx_monitor;
mod engine;
mod example_stages;
mod fixture_editor;
mod guide;
mod link;
mod log;
mod paths;
mod perform;
mod project;
mod stage;
mod theme;
mod util;
mod wave;

use eframe::egui::{self, Color32, Sense, Vec2};
use engine::{AudioSource, Engine};
use lumen_audio::{AppInfo, AudioInput, DeviceInfo};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};
use std::time::Duration;


/// Play: performing. Set up: outputs, fixtures, groups, music, diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Play,
    Setup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Output,
    Stage,
    Library,
    Auto,
    Music,
    Dmx,
    Controllers,
    Diagnostics,
}

impl Page {
    const ALL: [Page; 8] = [Page::Output, Page::Stage, Page::Auto, Page::Library, Page::Music, Page::Controllers, Page::Dmx, Page::Diagnostics];

    fn name(self) -> &'static str {
        match self {
            Page::Output => "Output",
            Page::Stage => "Stage",
            Page::Library => "Fixture library",
            Page::Auto => "Auto mode",
            Page::Music => "Music",
            Page::Dmx => "DMX values",
            Page::Controllers => "Controllers",
            Page::Diagnostics => "Diagnostics",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Page::Output => "Controllers, universes and where each fixture sits",
            Page::Stage => "Groups, looks for each part of a song, stage profiles",
            Page::Library => "Fixture types: search thousands, or make your own",
            Page::Auto => "Style of music, mood, and which effects and colours Auto may use",
            Page::Music => "Audio input and drum detection",
            Page::Dmx => "Every channel being sent, live",
            Page::Controllers => "MIDI controllers and OSC: faders, pads, learn",
            Page::Diagnostics => "Network checks, log and reports",
        }
    }
}

/// Buttons all over the app open a "window" by setting one of these; the
/// shell turns that into the matching Set up page.
pub struct Windows {
    pub advanced_output: bool,
    pub fixture_editor: bool,
    pub dmx_monitor: bool,
    pub audio: bool,
    pub diagnostics: bool,
}

/// UI-side state. The engine (composition, output, audio analysis, DMX)
/// lives in `engine` and runs on its own thread.
pub struct LumenApp {
    pub engine: Arc<Mutex<Engine>>,
    engine_thread: Option<std::thread::JoinHandle<()>>,
    /// Keeps the live audio stream running; dropping it stops capture.
    pub audio_input: Option<AudioInput>,
    pub audio_devices: Vec<DeviceInfo>,
    pub audio_apps: Vec<AppInfo>,
    pub audio_status: String,
    pub diag: diagnostics::State,
    export_after: Option<f64>,
    /// Audio source from last time, reopened once it is available.
    restore_audio: Option<String>,
    restore_tick: u32,
    export_probe_started: bool,
    pub windows: Windows,
    pub ao: advanced_output::State,
    pub fe: fixture_editor::State,
    pub dmx_view: dmx_monitor::State,
    /// Art-Net nodes heard on the network (polled in the background).
    pub nodes: lumen_output::NodeWatcher,
    /// The open show: file, undo history, autosave.
    pub session: project::Session,
    pub guide: guide::Guide,
    pub mode: Mode,
    pub page: Page,
    pub play: clips::State,
    pub stage: stage::State,
    pub backdrop: backdrop::State,
    pub control: control::Io,
    /// Recent taps of the TAP button.
    taps: Vec<std::time::Instant>,
    /// The last minute of music, for the strip on Play.
    pub wave: wave::History,
}

/// What every UI function gets: the locked engine (`app.e`) plus UI state
/// (reachable directly, e.g. `app.windows`).
pub struct View<'a> {
    pub e: &'a mut Engine,
    pub s: &'a mut LumenApp,
}

impl Deref for View<'_> {
    type Target = LumenApp;
    fn deref(&self) -> &LumenApp {
        self.s
    }
}

impl DerefMut for View<'_> {
    fn deref_mut(&mut self) -> &mut LumenApp {
        self.s
    }
}

impl LumenApp {
    fn new(gpu: String) -> Self {
        let (path, show, first_run) = project::startup();
        let engine = Arc::new(Mutex::new(Engine::new(show.composition, show.output)));
        let engine_thread = Some(engine::spawn(engine.clone()));
        // Follow rekordbox's tempo and beat by default; with nobody to follow
        // the clock simply runs on the BPM.
        if link::LinkSync::AVAILABLE {
            if let Ok(mut e) = engine.lock() {
                e.sync.link_enabled = true;
            }
        }
        LumenApp {
            engine,
            engine_thread,
            audio_input: None,
            audio_devices: Vec::new(),
            audio_apps: Vec::new(),
            audio_status: "No audio source".into(),
            diag: diagnostics::State::new(gpu),
            export_after: None,
            restore_audio: audio_panel::saved(),
            restore_tick: 0,
            export_probe_started: false,
            windows: Windows {
                advanced_output: false,
                fixture_editor: false,
                dmx_monitor: false,
                audio: false,
                diagnostics: false,
            },
            ao: Default::default(),
            fe: Default::default(),
            dmx_view: Default::default(),
            nodes: lumen_output::NodeWatcher::start(),
            session: project::Session::new(path, first_run),
            guide: guide::Guide::starting(first_run),
            mode: Mode::Play,
            page: Page::Output,
            play: Default::default(),
            stage: Default::default(),
            backdrop: Default::default(),
            control: Default::default(),
            taps: Vec::new(),
            wave: Default::default(),
        }
    }
}

impl View<'_> {
    pub fn stop_audio(&mut self) {
        self.e.set_audio(AudioSource::Off);
        self.s.audio_input = None;
        self.s.audio_status = "No audio source".into();
    }

    pub fn start_demo_audio(&mut self) {
        self.s.audio_input = None;
        self.e.start_demo_audio();
        self.s.audio_status = "Demo beat at the composition BPM".into();
    }

    /// Hand a running capture to the engine and keep its stream alive here.
    pub fn use_input(&mut self, input: AudioInput) {
        let f = &input.feed;
        self.s.audio_status = format!("{} — {} Hz, {} ch", f.label(), f.sample_rate, f.channels);
        self.e.set_audio(AudioSource::Live(input.feed.clone()));
        self.s.audio_input = Some(input);
    }

    pub fn wake(&self) -> Arc<lumen_audio::Wake> {
        self.e.wake.clone()
    }

    /// Buttons anywhere can still "open a window": go to that Set up page.
    fn route_windows(&mut self) {
        let w = &mut self.s.windows;
        let page = if w.advanced_output {
            Some(Page::Output)
        } else if w.fixture_editor {
            Some(Page::Library)
        } else if w.audio {
            Some(Page::Music)
        } else if w.dmx_monitor {
            Some(Page::Dmx)
        } else if w.diagnostics {
            Some(Page::Diagnostics)
        } else {
            None
        };
        *w = Windows { advanced_output: false, fixture_editor: false, dmx_monitor: false, audio: false, diagnostics: false };
        if let Some(p) = page {
            self.s.page = p;
            self.s.mode = Mode::Setup;
        }
    }

    /// Tap tempo: the tempo from the last few taps, and the beat lined up with this one.
    pub(crate) fn tap(&mut self, shift: bool) {
        let now = std::time::Instant::now();
        let taps = &mut self.s.taps;
        if taps.last().is_some_and(|l| now.duration_since(*l).as_secs_f64() > 2.0) {
            taps.clear();
        }
        taps.push(now);
        if taps.len() > 8 {
            taps.remove(0);
        }
        if taps.len() >= 2 {
            let n = taps.len() - 1;
            let span = taps[n].duration_since(taps[0]).as_secs_f64();
            let bpm = 60.0 * n as f64 / span.max(1e-3);
            if (20.0..=300.0).contains(&bpm) {
                self.e.comp.bpm = (bpm * 10.0).round() / 10.0;
            }
        }
        let b = self.e.comp.beat;
        self.e.comp.beat = if shift { (b / 4.0).round() * 4.0 } else { b.round() };
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            // Tempo
            let synced = self.e.clock_out.source != lumen_core::ClockSource::Manual;
            ui.add_enabled(
                !synced,
                egui::DragValue::new(&mut self.e.comp.bpm).range(20.0..=300.0).speed(0.25).max_decimals(1).suffix(" BPM"),
            )
            .on_disabled_hover_text("Tempo comes from the music (or Ableton Link)");
            ui.toggle_value(&mut self.e.sync.link_enabled, "Link").on_hover_text(if link::LinkSync::AVAILABLE {
                "Follow tempo and beat from rekordbox via Ableton Link"
            } else {
                "This build was made without Ableton Link"
            });
            let tap = egui::Button::new(egui::RichText::new("TAP").font(theme::bold(t.text_size - 1.0)));
            if ui.add_enabled(!synced, tap).on_hover_text("Tap the beat to set the tempo and line the beat up. Shift-tap on the 1 to start the bar there.").clicked() {
                let shift = ui.input(|i| i.modifiers.shift);
                self.tap(shift);
            }
            util::beat_dots(ui, self.e.comp.beat);
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Master").color(t.weak));
            ui.spacing_mut().slider_width = 110.0;
            ui.add(egui::Slider::new(&mut self.e.comp.master, 0.0..=1.0).show_value(false));
            let bo = self.e.blackout;
            let b = egui::Button::new(egui::RichText::new("Blackout").font(theme::bold(t.text_size - 1.0)).color(if bo { Color32::WHITE } else { t.weak }))
                .fill(if bo { t.err } else { t.raised });
            if ui.add(b).on_hover_text("All lights dark, instantly (B)").clicked() {
                self.e.blackout = !bo;
            }
            ui.add_space(6.0);
            let on = self.e.output_enabled;
            let dead = self.e.health.state(on) == engine::OutputState::Dead;
            let label = egui::RichText::new(if dead { "DMX  DEAD" } else if on { "DMX ON" } else { "DMX OFF" }).font(theme::bold(t.text_size - 1.0));
            let b = egui::Button::new(if on { label.color(t.on_accent) } else { label.color(t.weak) }).fill(if dead { t.err } else if on { t.accent } else { t.raised });
            if ui.add(b).on_hover_text("Send to your lights").clicked() {
                self.e.output_enabled = !on;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.show_menu(ui);
                let can_redo = self.session.can_redo();
                if ui.add_enabled(can_redo, egui::Button::new("↷")).on_hover_text("Redo (⌘⇧Z)").clicked() {
                    project::redo_step(self);
                }
                let can_undo = self.session.can_undo();
                if ui.add_enabled(can_undo, egui::Button::new("↶")).on_hover_text("Undo (⌘Z)").clicked() {
                    project::undo_step(self);
                }
                ui.add_space(10.0);
                // Play | Set up
                for (m, name) in [(Mode::Setup, "Set up"), (Mode::Play, "Play")] {
                    let sel = self.s.mode == m;
                    let b = egui::Button::new(egui::RichText::new(name).font(theme::bold(t.text_size)).color(if sel { t.on_accent } else { t.weak }))
                        .fill(if sel { t.accent } else { t.raised })
                        .min_size(Vec2::new(70.0, 26.0));
                    if ui.add(b).clicked() {
                        self.s.mode = m;
                    }
                }
                ui.add_space(10.0);
                self.output_badge(ui);
            });
        });
        ui.add_space(5.0);
    }

    /// Play screen, right side: the inspector when something is selected,
    /// otherwise what the lights get, the music and the outputs.
    fn play_side(&mut self, ui: &mut egui::Ui) {
        let groups = self.e.output.group_names();
        if self.s.play.inspect.is_some() {
            if !clips::inspector(self, ui, &groups) {
                self.s.play.inspect = None;
                self.s.play.browsing = false;
            }
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            perform::state_line(self, ui);
            card(ui, "Auto", |ui| auto::card(self, ui));
            card(ui, "Mix", |ui| perform::mix_card(self, ui));
            card(ui, "What the lights get", |ui| {
                let w = ui.available_width();
                stage::paint_lights(ui, self, Vec2::new(w, w), &|_| None);
                let t = theme::current();
                let n = self.e.output.fixtures.len();
                let px: usize = self.e.lights.iter().map(|l| l.len()).sum();
                ui.label(egui::RichText::new(format!("{n} fixtures · {px} pixels · {} universes", self.e.dmx.len())).color(t.weak).size(t.text_size - 2.0));
            });
            card(ui, "Set so far", |ui| perform::set_strip(self, ui));
            card(ui, "Music", |ui| self.music_card(ui));
            card(ui, "Lights", |ui| self.lights_card(ui));
        });
    }

    fn music_card(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.e.audio.label()).color(t.weak));
            util::hit_dots(ui, &self.e.bands);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Audio…").clicked() {
                    self.s.windows.audio = true;
                }
            });
        });
        if let Some(st) = &self.e.link_status {
            if st.peers > 0 {
                ui.colored_label(t.ok, format!("Link: {} peer(s), {:.1} BPM", st.peers, st.tempo));
            } else {
                ui.colored_label(t.warn, "Link: waiting for rekordbox");
            }
        }
        if let Some(note) = self.e.clock_out.note.clone() {
            ui.colored_label(t.warn, note);
        }
        ui.add_space(4.0);
        stage::section_card(self, ui, false);
        stage::timeline(self, ui);
    }

    fn lights_card(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        if let Some(tst) = self.e.test {
            let name = self.e.output.fixtures.get(tst.fixture).map(|f| f.name.clone()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.colored_label(t.warn, format!("Testing {name}"));
                if ui.small_button("Stop test").clicked() {
                    self.e.test = None;
                }
            });
        }
        let seen = self.nodes.nodes();
        for i in 0..self.e.output.lumiverses.len() {
            let l = &self.e.output.lumiverses[i];
            let used = self.e.output.universes_used(i);
            let (dot, note) = match advanced_output::node_status(&self.e.output, i, &seen) {
                advanced_output::NodeStatus::Online(n) => (t.ok, format!("{} answering", n)),
                advanced_output::NodeStatus::Lost { name, .. } => (t.err, format!("{name} not answering")),
                _ if self.e.output_enabled && l.enabled => (t.accent, "sending".to_string()),
                _ => (t.line, if l.enabled { "output off".to_string() } else { "disabled".to_string() }),
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("●").color(dot));
                ui.label(&l.name);
                let unis = if used.is_empty() { "no fixtures".to_string() } else { format!("U{}", engine::universe_ranges(used)) };
                ui.label(egui::RichText::new(format!("{unis} · {note}")).color(t.weak).size(t.text_size - 2.0));
            });
        }
        if self.e.output.lumiverses.is_empty() {
            ui.weak("No outputs yet.");
        }
        if ui.small_button("Set up outputs…").clicked() {
            self.s.mode = Mode::Setup;
            self.s.page = Page::Output;
        }
    }

    /// Set up: the list of pages on the left.
    fn setup_nav(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        ui.add_space(10.0);
        let (_, errs) = log::counts();
        for p in Page::ALL {
            let sel = self.s.page == p;
            let name = if p == Page::Diagnostics && errs > 0 { format!("{} ({errs})", p.name()) } else { p.name().to_string() };
            let r = egui::Frame::new()
                .fill(if sel { t.raised } else { Color32::TRANSPARENT })
                .corner_radius(t.radius)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(name).font(theme::bold(t.text_size)).color(if sel { t.text } else { t.weak }));
                    ui.label(egui::RichText::new(p.note()).color(t.weak).size(t.text_size - 3.0));
                })
                .response
                .interact(Sense::click());
            if sel {
                let r = r.rect;
                ui.painter().rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 1.0, t.accent);
            }
            if r.clicked() {
                self.s.page = p;
            }
            ui.add_space(2.0);
        }
    }

    fn setup_page(&mut self, ui: &mut egui::Ui) {
        match self.s.page {
            Page::Output => advanced_output::ui(self, ui),
            Page::Stage => stage::ui(self, ui),
            Page::Auto => auto::page(self, ui),
            Page::Library => fixture_editor::ui(self, ui),
            Page::Music => {
                stage::music_sections(self, ui);
                audio_panel::ui(self, ui);
            }
            Page::Dmx => dmx_monitor::ui(self, ui),
            Page::Controllers => control::page(self, ui),
            Page::Diagnostics => diagnostics::ui(self, ui),
        }
    }

    /// Output health: an unmissable badge, then universes, rate and count.
    fn output_badge(&mut self, ui: &mut egui::Ui) {
        use engine::OutputState::*;
        let t = theme::current();
        let state = self.e.health.state(self.e.output_enabled);
        let blink = (ui.input(|i| i.time) * 2.0).fract() < 0.6;
        let (label, fill, fg) = match state {
            Off => ("OUTPUT OFF", t.raised, t.weak),
            Idle => ("NOTHING TO SEND", t.raised, t.warn),
            Live => ("OUTPUT LIVE", theme::dim(t.ok, 0.35), t.ok),
            Partial => ("OUTPUT PARTLY FAILING", theme::dim(t.warn, 0.35), t.warn),
            Dead => ("OUTPUT DEAD", if blink { t.err } else { theme::dim(t.err, 0.5) }, Color32::WHITE),
        };
        let mut detail = String::new();
        if matches!(state, Live | Partial) {
            let art: Vec<u16> = self.e.health.universes.iter().filter(|(sacn, _)| !sacn).map(|(_, u)| *u).collect();
            let sacn: Vec<u16> = self.e.health.universes.iter().filter(|(sacn, _)| *sacn).map(|(_, u)| *u + 1).collect();
            let mut parts = Vec::new();
            if !art.is_empty() {
                let first = art.iter().min().copied().unwrap_or(0);
                parts.push(format!("Art-Net U{} (net {} sub {})", engine::universe_ranges(art.clone()), first >> 8, (first >> 4) & 15));
            }
            if !sacn.is_empty() {
                parts.push(format!("sACN U{}", engine::universe_ranges(sacn.clone())));
            }
            let n = art.len() + sacn.len();
            detail = format!("\n\n{} · {n} universe{} · {:.0} Hz", parts.join(" + "), if n == 1 { "" } else { "s" }, self.e.metrics.output_hz());
        }
        let r = ui.add(egui::Button::new(egui::RichText::new(label).font(theme::bold(t.text_size - 2.0)).color(fg)).fill(fill).corner_radius(t.radius));
        let tip = match state {
            Off => "DMX output is off. Click DMX in the top bar to send to your lights.".to_string(),
            Idle => "Output is on, but no fixtures or universes are set up to send yet.".to_string(),
            Live => "Packets are leaving this computer for every universe. Art-Net doesn't confirm delivery: \"answering\" in the Lights card is the proof the controller is there (some cheap controllers never answer).".to_string(),
            Partial | Dead => {
                let e = self.e.health.last_error.as_ref().map(|(_, e)| e.clone()).unwrap_or_default();
                format!("{e}\nClick for Diagnostics.")
            }
        };
        if r.on_hover_text(format!("{tip}{detail}")).clicked() && matches!(state, Partial | Dead) {
            self.s.windows.diagnostics = true;
        }
        if let Some(sw) = self.e.sweep {
            let base = self.e.output.lumiverses.get(sw.lumiverse).map(|l| l.universe).unwrap_or(0);
            ui.colored_label(t.warn, format!("Sweep U{}", base + sw.current()));
            if ui.small_button("Stop sweep").clicked() {
                self.e.sweep = None;
            }
        }
    }

    /// The show's name, save state and file actions.
    fn show_menu(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        let name = self.session.name();
        let (mark, state) = if self.session.save_error.is_some() {
            ("⚠", "Not saved! Open this menu for details.")
        } else if self.session.dirty() {
            ("•", "Saving…")
        } else {
            ("✓", "All changes saved")
        };
        let short: String = if name.chars().count() > 16 { name.chars().take(15).chain(['…']).collect() } else { name.clone() };
        let label = egui::RichText::new(format!("{short} {mark}"));
        let label = if self.session.save_error.is_some() { label.color(t.err) } else { label };
        let mut action = None;
        let mut open_recent: Option<std::path::PathBuf> = None;
        ui.menu_button(label, |ui| {
            ui.weak("Changes save automatically.");
            if let Some(e) = &self.session.save_error {
                ui.colored_label(t.err, format!("Last save failed: {e}"));
            }
            ui.separator();
            if ui.button("New show").clicked() {
                action = Some(0);
            }
            if ui.button("Open…").clicked() {
                action = Some(1);
            }
            let recent: Vec<_> = paths::recent_projects().into_iter().filter(|p| *p != self.session.path).collect();
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button("Open recent", |ui| {
                    for p in recent {
                        let name = p.file_stem().map(|s| s.to_string_lossy().replace("kickline_project", "My show")).unwrap_or_default();
                        if ui.button(name).on_hover_text(p.display().to_string()).clicked() {
                            open_recent = Some(p);
                        }
                    }
                });
            });
            if ui.button("Save as…").on_hover_text("Save a copy under a new name and keep working in the copy").clicked() {
                action = Some(2);
            }
            if ui.button("Save now  ⌘S").clicked() {
                action = Some(3);
            }
            ui.separator();
            if ui.button("Setup guide…").clicked() {
                action = Some(4);
            }
            if ui.button("Show backups").on_hover_text("Kickline keeps the last 10 versions of each show, saved every few minutes").clicked() {
                action = Some(5);
            }
            ui.weak(self.session.path.display().to_string());
        })
        .response
        .on_hover_text(format!("{name}: {state}. Changes save automatically."));
        match action {
            Some(0) => {
                project::new_show(self);
                self.s.guide.open();
            }
            Some(1) => project::open_dialog(self),
            Some(2) => project::save_as_dialog(self),
            Some(3) => project::save_now(self),
            Some(4) => self.s.guide.open(),
            Some(5) => {
                let dir = paths::backups_dir(&self.session.path);
                let _ = std::fs::create_dir_all(&dir);
                paths::reveal(&dir);
            }
            _ => {}
        }
        if let Some(p) = open_recent {
            project::open_path(self, p);
        }
    }

}

/// A titled card on the Play screen's side.
fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = theme::current();
    egui::Frame::new().fill(t.panel).corner_radius(t.radius + 2).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(title).font(theme::bold(t.text_size)));
        ui.add_space(4.0);
        add(ui);
    });
    ui.add_space(8.0);
}

impl eframe::App for LumenApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dt = ctx.input(|i| i.unstable_dt).clamp(0.001, 0.25);
        let engine = self.engine.clone();
        let Ok(mut guard) = engine.lock() else {
            log::error("app", "engine lock poisoned");
            return;
        };
        guard.metrics.frame(dt * 1000.0);
        let mut v = View {
            e: &mut guard,
            s: self,
        };
        diagnostics::poll(&mut v);
        // Reopen last time's audio source (the app may still be starting, so
        // keep trying every couple of seconds for a few minutes).
        if v.restore_audio.is_some() {
            if !matches!(v.e.audio, AudioSource::Off) {
                v.restore_audio = None;
            } else if v.restore_tick % 120 == 0 {
                if let Some(what) = v.restore_audio.clone() {
                    if audio_panel::restore(&mut v, &what) {
                        v.restore_audio = None;
                    }
                }
            }
            v.restore_tick += 1;
            if v.restore_tick > 120 * 150 && !v.e.comp.director.show_mode {
                v.restore_audio = None;
            }
        }
        // Show mode: all night, keep output and Auto on and keep looking for the music.
        if v.e.comp.director.show_mode && v.restore_tick % 120 == 60 && v.e.keep_show_going() && v.restore_audio.is_none() {
            v.restore_audio = audio_panel::saved();
        }
        if v.e.comp.director.show_mode {
            v.restore_tick = v.restore_tick.wrapping_add(1);
        }
        project::tick(&mut v, &ctx);
        if let Some(at) = v.export_after {
            let up = log::uptime();
            if up > at - 3.0 && !v.export_probe_started && !v.diag.running() {
                diagnostics::run_probe(&mut v);
                v.export_probe_started = true;
            } else if up > at && v.export_probe_started && !v.diag.running() {
                match diagnostics::export(&mut v) {
                    Ok(p) => println!("report: {}", p.display()),
                    Err(e) => eprintln!("export failed: {e}"),
                }
                v.export_after = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        backdrop::sync(&mut v, &ctx);
        control::tick(&mut v);
        v.route_windows();
        // Poll for nodes while output is on or the patch is open.
        let patching = v.mode == Mode::Setup && matches!(v.page, Page::Output | Page::Diagnostics);
        v.s.nodes.set_active(v.e.output_enabled || patching || v.s.guide.wants_nodes());
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                v.s.play.inspect = None;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::B) && !i.modifiers.command) {
                v.e.blackout = !v.e.blackout;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::A) && !i.modifiers.command) {
                auto::toggle_auto(&mut v);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::H) && !i.modifiers.command) {
                v.e.comp.director_rt.hold = !v.e.comp.director_rt.hold;
            }
            // While recording a session, 1 to 5 mark which part of the song is playing.
            if v.e.recorder.is_some() {
                use egui::Key::*;
                for (i, k) in [Num0, Num1, Num2, Num3, Num4, Num5].into_iter().enumerate() {
                    if ctx.input(|inp| inp.key_pressed(k)) {
                        v.e.mark_section(i as u8);
                    }
                }
            }
        }
        egui::Panel::top("top").show(ui, |ui| v.top_bar(ui));
        match v.mode {
            Mode::Play => {
                egui::Panel::right("play_side").exact_size(340.0).resizable(false).show(ui, |ui| v.play_side(ui));
                egui::CentralPanel::default().show(ui, |ui| {
                    wave::strip(&mut v, ui);
                    clips::ui(&mut v, ui)
                });
            }
            Mode::Setup => {
                egui::Panel::left("nav").exact_size(200.0).resizable(false).show(ui, |ui| {
                    v.setup_nav(ui);
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.add_space(8.0);
                        let mut i = theme::index();
                        egui::ComboBox::from_id_salt("theme").selected_text(theme::PRESETS[i].name).show_ui(ui, |ui| {
                            for (k, th) in theme::PRESETS.iter().enumerate() {
                                ui.selectable_value(&mut i, k, th.name).on_hover_text(th.note);
                            }
                        });
                        if i != theme::index() {
                            theme::choose(ui.ctx(), i);
                        }
                        ui.weak("Theme");
                    });
                });
                egui::CentralPanel::default().show(ui, |ui| v.setup_page(ui));
            }
        }
        guide::ui(&mut v, &ctx);
        recent_card(&mut v, &ctx);
        v.route_windows();
        drop(guard);
        // The preview only needs ~30 fps; DMX timing is the engine thread's job.
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn on_exit(&mut self) {
        let engine = self.engine.clone();
        if let Ok(mut e) = engine.lock() {
            let mut v = View { e: &mut e, s: self };
            project::save_now(&mut v);
        }
        if let Ok(mut e) = self.engine.lock() {
            e.shutdown = true;
            e.wake.notify();
        }
        self.audio_input = None;
        if let Some(h) = self.engine_thread.take() {
            let _ = h.join();
        }
    }
}

/// For a few seconds after launch: which show was reopened, and one-click
/// access to the other recent ones.
fn recent_card(v: &mut View, ctx: &egui::Context) {
    let Some(until) = v.s.session.recent_card_until else { return };
    let others: Vec<_> = paths::recent_projects().into_iter().filter(|p| *p != v.s.session.path).take(5).collect();
    if std::time::Instant::now() > until || others.is_empty() || v.s.guide.is_open {
        v.s.session.recent_card_until = None;
        return;
    }
    let t = theme::current();
    let mut pick = None;
    let mut close = false;
    egui::Area::new(egui::Id::unique("recent_card"))
        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 52.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(12).show(ui, |ui| {
                ui.set_width(260.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("Opened {}", v.s.session.name())).font(theme::bold(t.text_size)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").clicked() {
                            close = true;
                        }
                    });
                });
                ui.weak("Recent shows");
                for p in &others {
                    let name = p.file_stem().map(|s| s.to_string_lossy().replace("kickline_project", "My show")).unwrap_or_default();
                    if ui.button(name).on_hover_text(p.display().to_string()).clicked() {
                        pick = Some(p.clone());
                    }
                }
            });
        });
    if close {
        v.s.session.recent_card_until = None;
    }
    if let Some(p) = pick {
        v.s.session.recent_card_until = None;
        project::open_path(v, p);
    }
}

fn main() -> eframe::Result {
    log::install_panic_hook();
    log::info(
        "app",
        format!(
            "Kickline {} starting on {}/{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    let crashes = log::previous_crashes();
    if !crashes.is_empty() {
        log::warn(
            "app",
            format!(
                "{} crash report(s) from earlier sessions — see Diagnostics",
                crashes.len()
            ),
        );
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Kickline"),
        ..Default::default()
    };
    eframe::run_native(
        "Kickline",
        options,
        Box::new(move |cc| {
            let gpu = cc
                .wgpu_render_state
                .as_ref()
                .map(|rs| {
                    let i = rs.adapter.get_info();
                    format!(
                        "{} — {:?} ({:?}), driver {} {}",
                        i.name, i.backend, i.device_type, i.driver, i.driver_info
                    )
                })
                .unwrap_or_default();
            log::info("app", format!("graphics: {gpu}"));
            theme::install_fonts(&cc.egui_ctx);
            if let Some(i) = args.iter().find_map(|a| a.strip_prefix("--theme=")).and_then(|s| s.parse().ok()) {
                theme::set(&cc.egui_ctx, i);
            } else {
                theme::set(&cc.egui_ctx, theme::saved().unwrap_or(0));
            }
            let mut app = LumenApp::new(gpu);
            apply_args(&mut app, &args);
            Ok(Box::new(app))
        }),
    )
}

/// Handy for testing: --open=advanced,fixtures,dmx,audio,diagnostics  --audio=demo  --dmx-out
/// --target=IP  --select=N  --link  --export-report-after=SECS  --test-crash
fn apply_args(app: &mut LumenApp, args: &[String]) {
    let engine = app.engine.clone();
    let mut guard = engine.lock().expect("engine");
    let mut v = View {
        e: &mut guard,
        s: app,
    };
    for arg in args {
        if let Some(list) = arg.strip_prefix("--open=") {
            for w in list.split(',') {
                match w {
                    "advanced" => v.windows.advanced_output = true,
                    "fixtures" => v.windows.fixture_editor = true,
                    "dmx" => v.windows.dmx_monitor = true,
                    "audio" => v.windows.audio = true,
                    "diagnostics" => v.windows.diagnostics = true,
                    _ => {}
                }
            }
        } else if arg == "--dmx-out" {
            v.e.output_enabled = true;
        } else if arg == "--link" {
            v.e.sync.link_enabled = true;
        } else if let Some(ip) = arg.strip_prefix("--target=") {
            for l in &mut v.e.output.lumiverses {
                l.target = ip.to_string();
            }
        } else if let Some(q) = arg.strip_prefix("--library=") {
            v.s.windows.fixture_editor = true;
            v.s.fe.browsing = true;
            v.s.fe.query = q.to_string();
            if let Some(&i) = v.s.fe.library().search(q, 1).first() {
                v.s.fe.select_entry(i);
            }
        } else if arg == "--audio=demo" {
            v.start_demo_audio();
        } else if let Some(i) = arg.strip_prefix("--select=") {
            v.s.ao.selected = i.parse().ok();
            if let Some(f) = v.s.ao.selected.and_then(|i| v.e.output.fixtures.get(i)) {
                v.s.ao.lumiverse = f.lumiverse;
            }
        } else if let Some(s) = arg.strip_prefix("--export-report-after=") {
            v.s.export_after = s.parse().ok();
        } else if arg == "--example-face" {
            project::new_show_from(&mut v, "Face stage", example_stages::face_show());
        } else if arg == "--example-dragon" {
            project::new_show_from(&mut v, "Dragon stage", example_stages::dragon_show());
        } else if arg == "--example-stage" {
            project::new_show_from(&mut v, "DJ stage", stage::example_show());
        } else if let Some(p) = arg.strip_prefix("--page=") {
            v.s.mode = Mode::Setup;
            v.s.page = match p {
"stage" => Page::Stage,
"controllers" => Page::Controllers,
                "auto" => Page::Auto,
                "library" => Page::Library,
                "music" => Page::Music,
                "dmx" => Page::Dmx,
                "diagnostics" => Page::Diagnostics,
                _ => Page::Output,
            };
        } else if let Some(x) = arg.strip_prefix("--inspect=") {
            let nums: Vec<usize> = x.split([':', ',']).filter_map(|n| n.parse().ok()).collect();
            v.s.play.inspect = match (x.split(':').next(), nums.as_slice()) {
                (Some("layer"), [l]) => Some(clips::Inspect::Layer(*l)),
                (Some("clip"), [l, c]) => Some(clips::Inspect::Clip(*l, *c)),
                (Some("browse"), [l, c]) => {
                    v.s.play.browsing = true;
                    Some(clips::Inspect::Clip(*l, *c))
                }
                (Some("palette"), _) => Some(clips::Inspect::Palette),
                _ => None,
            };
        } else if let Some(n) = arg.strip_prefix("--category=") {
            v.s.play.category = n.parse().unwrap_or(0);
        } else if arg == "--controller" {
            let mut cs = lumen_fixtures::ControllerSetup::new(0, 0);
            cs.leds = vec![512, 512, 512, 180, 120, 0, 0, 0];
            v.s.ao.controller = Some(cs);
        } else if arg == "--auto" {
            v.e.comp.director.on = true;
        } else if let Some(p) = arg.strip_prefix("--backdrop=") {
            v.e.output.backdrop = Some(lumen_fixtures::Backdrop::new(p));
        } else if arg == "--straighten" {
            v.s.backdrop.editing = true;
        } else if arg == "--blackout" {
            v.e.blackout = true;
        } else if arg == "--test-crash" {
            panic!("test crash requested with --test-crash");
        }
    }
}
