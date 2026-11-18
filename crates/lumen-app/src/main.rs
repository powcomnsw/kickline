mod advanced_output;
mod audio_panel;
mod clips;
mod diagnostics;
mod dmx_monitor;
mod engine;
mod fixture_editor;
mod guide;
mod link;
mod log;
mod paths;
mod project;
mod theme;
mod util;

use eframe::egui::{self, Color32, Sense, Stroke, Vec2};
use engine::{AudioSource, Engine};
use lumen_audio::{AppInfo, AudioInput, DeviceInfo};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};
use std::time::Duration;


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
        LumenApp {
            engine,
            engine_thread,
            audio_input: None,
            audio_devices: Vec::new(),
            audio_apps: Vec::new(),
            audio_status: "No audio source".into(),
            diag: diagnostics::State::new(gpu),
            export_after: None,
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

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Tempo
            let synced = self.e.link_status.as_ref().is_some_and(|s| s.peers > 0);
            ui.add_enabled(
                !synced,
                egui::DragValue::new(&mut self.e.comp.bpm).range(20.0..=300.0).speed(0.25).max_decimals(1).suffix(" BPM"),
            )
            .on_disabled_hover_text("Tempo comes from Ableton Link");
            ui.toggle_value(&mut self.e.sync.link_enabled, "Link").on_hover_text(if link::LinkSync::AVAILABLE {
                "Follow tempo and beat from rekordbox via Ableton Link"
            } else {
                "This build was made without Ableton Link"
            });
            util::beat_dots(ui, self.e.comp.beat);
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Master").color(t.weak));
            ui.spacing_mut().slider_width = 130.0;
            ui.add(egui::Slider::new(&mut self.e.comp.master, 0.0..=1.0).show_value(false));
            ui.add_space(14.0);
            let on = self.e.output_enabled;
            let dead = self.e.health.state(on) == engine::OutputState::Dead;
            let label = egui::RichText::new(if dead { "DMX  DEAD" } else if on { "DMX OUT  ON" } else { "DMX OUT  OFF" }).font(theme::bold(t.text_size - 1.0));
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
                ui.add_space(12.0);
                let (_, errs) = log::counts();
                // Short labels when the window is narrow, so nothing overlaps.
                let compact = ui.available_width() < 560.0;
                let w = &mut self.s.windows;
                let diag_name = if compact { "Diag." } else { "Diagnostics" };
                let diag = if errs > 0 { format!("{diag_name} ({errs})") } else { diag_name.into() };
                ui.toggle_value(&mut w.diagnostics, diag).on_hover_text("Diagnostics");
                ui.toggle_value(&mut w.audio, "Audio");
                ui.toggle_value(&mut w.dmx_monitor, if compact { "DMX" } else { "DMX Monitor" }).on_hover_text("DMX Monitor");
                ui.toggle_value(&mut w.fixture_editor, "Fixtures");
                ui.toggle_value(&mut w.advanced_output, if compact { "Output" } else { "Advanced Output" }).on_hover_text("Advanced Output");
            });
        });
        ui.add_space(4.0);
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
        let r = ui.add(egui::Button::new(egui::RichText::new(label).font(theme::bold(t.text_size - 2.0)).color(fg)).fill(fill).corner_radius(t.radius));
        let tip = match state {
            Off => "DMX output is off. Click DMX OUT in the top bar to send to your lights.".to_string(),
            Idle => "Output is on, but no fixtures or universes are set up to send yet.".to_string(),
            Live => "Packets are leaving this computer for every universe. Art-Net doesn't confirm delivery: a green \"node answering\" next to this is the proof the controller is there (some cheap controllers never answer).".to_string(),
            Partial | Dead => {
                let e = self.e.health.last_error.as_ref().map(|(_, e)| e.clone()).unwrap_or_default();
                format!("{e}\nClick for Diagnostics.")
            }
        };
        if r.on_hover_text(tip).clicked() && matches!(state, Partial | Dead) {
            self.s.windows.diagnostics = true;
        }
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
            ui.label(egui::RichText::new(format!("{} · {n} universe{} · {:.0} Hz", parts.join(" + "), if n == 1 { "" } else { "s" }, self.e.metrics.output_hz())).color(t.weak));
        }
        if let Some(sw) = self.e.sweep {
            let base = self.e.output.lumiverses.get(sw.lumiverse).map(|l| l.universe).unwrap_or(0);
            ui.separator();
            ui.colored_label(t.warn, format!("Sweeping: universe {} lit", base + sw.current()));
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        ui.horizontal(|ui| {
            self.output_badge(ui);
            if let Some(tst) = self.e.test {
                let name = self.e.output.fixtures.get(tst.fixture).map(|f| f.name.clone()).unwrap_or_default();
                ui.separator();
                ui.colored_label(t.warn, format!("Testing {name}: other lights are dark"));
                if ui.small_button("Stop test").clicked() {
                    self.e.test = None;
                }
            }
            if self.e.output_enabled {
                let seen = self.nodes.nodes();
                let mut lost = Vec::new();
                let mut online = 0;
                for i in 0..self.e.output.lumiverses.len() {
                    match advanced_output::node_status(&self.e.output, i, &seen) {
                        advanced_output::NodeStatus::Lost { name, .. } => lost.push(format!("{} ({name})", self.e.output.lumiverses[i].name)),
                        advanced_output::NodeStatus::Online(_) => online += 1,
                        _ => {}
                    }
                }
                ui.separator();
                if !lost.is_empty() {
                    ui.colored_label(t.err, format!("Node not answering: {}", lost.join(", ")))
                        .on_hover_text("Kickline keeps sending. Check the node's power and cable.");
                } else if online > 0 {
                    let n = seen.iter().filter(|n| n.online()).count();
                    ui.colored_label(t.ok, format!("{n} node(s) answering"));
                }
            }
            ui.separator();
            ui.label(egui::RichText::new(format!("Audio: {}", self.e.audio.label())).color(t.weak));
            util::hit_dots(ui, &self.e.bands);
            if let Some(st) = &self.e.link_status {
                ui.separator();
                if st.peers > 0 {
                    ui.colored_label(t.ok, format!("Link: {} peer(s), {:.1} BPM", st.peers, st.tempo));
                } else {
                    ui.colored_label(t.warn, "Link: waiting for rekordbox");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut i = theme::index();
                egui::ComboBox::from_id_salt("theme").selected_text(theme::PRESETS[i].name).show_ui(ui, |ui| {
                    for (k, th) in theme::PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut i, k, th.name).on_hover_text(th.note);
                    }
                });
                if i != theme::index() {
                    theme::choose(ui.ctx(), i);
                }
                ui.label(egui::RichText::new("Theme").color(t.weak));
            });
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let t = theme::current();
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Composition").font(theme::bold(t.text_size)));
        let w = ui.available_width();
        let frame = &self.e.frame;
        let aspect = frame.width as f32 / frame.height as f32;
        let (resp, painter) = ui.allocate_painter(Vec2::new(w, w / aspect), Sense::hover());
        util::paint_frame(&painter, resp.rect, frame);
        let aspect = self.e.output.aspect;
        // Strip clips aren't part of the picture: show them as the pixels the
        // lights will actually get.
        let stage = self.e.comp.strip_stage();
        if !stage.is_empty() {
            let n = self.e.output.fixtures.len();
            let dark = painter.clone();
            dark.rect_filled(resp.rect, 0.0, Color32::from_black_alpha(150));
            for (fi, fx) in self.e.output.fixtures.iter().enumerate() {
                let Some(ty) = self.e.output.types.get(fx.type_id) else { continue };
                let cells = fx.pixel_local(ty);
                let pos = fx.pixel_positions(aspect, ty);
                let count = cells.len();
                let r = (resp.rect.width() / 260.0).clamp(1.2, 3.5);
                for (i, ((lx, ly), (u, v))) in cells.iter().zip(pos).enumerate() {
                    let base = fx.sample_cell(aspect, ty, frame, *lx, *ly);
                    let p = lumen_core::StripPixel { fixture: fi, fixtures: n, index: i, count, x: lx + 0.5, y: ly + 0.5 };
                    let c = stage.shade(base, p);
                    let col = Color32::from_rgb((c.r.clamp(0.0, 1.0) * 255.0) as u8, (c.g.clamp(0.0, 1.0) * 255.0) as u8, (c.b.clamp(0.0, 1.0) * 255.0) as u8);
                    painter.circle_filled(util::to_screen(resp.rect, u, v), r, col);
                }
            }
        }
        for fx in &self.e.output.fixtures {
            let pts: Vec<_> = fx
                .corners(aspect)
                .iter()
                .map(|&(u, v)| util::to_screen(resp.rect, u, v))
                .collect();
            painter.add(egui::Shape::closed_line(
                pts,
                Stroke::new(1.0, Color32::from_white_alpha(140)),
            ));
        }
        ui.add_space(6.0);
        ui.label(format!(
            "{} fixtures on {} lumiverse(s), {} universe(s) in use",
            self.e.output.fixtures.len(),
            self.e.output.lumiverses.len(),
            self.e.dmx.len()
        ));
    }
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

        // Poll for nodes while output is on or the patch is open.
        v.s.nodes.set_active(v.e.output_enabled || v.s.windows.advanced_output || v.s.guide.wants_nodes());
        let was_open = [v.windows.advanced_output, v.windows.fixture_editor, v.windows.dmx_monitor, v.windows.audio, v.windows.diagnostics];
        egui::Panel::top("top").show(ui, |ui| v.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| v.status_bar(ui));
        egui::Panel::right("preview").default_size(380.0).resizable(true).show(ui, |ui| v.preview(ui));
        egui::CentralPanel::default().show(ui, |ui| clips::ui(&mut v, ui));

        let mut open = v.windows.advanced_output;
        egui::Window::new("Advanced Output")
            .open(&mut open)
            .default_pos([20.0, 70.0])
            .default_size([980.0, 640.0])
            .show(&ctx, |ui| advanced_output::ui(&mut v, ui));
        v.windows.advanced_output = open;

        let mut open = v.windows.fixture_editor;
        egui::Window::new("Fixture Editor")
            .open(&mut open)
            .default_pos([120.0, 110.0])
            .default_size([900.0, 640.0])
            .show(&ctx, |ui| fixture_editor::ui(&mut v, ui));
        v.windows.fixture_editor = open;

        let mut open = v.windows.dmx_monitor;
        egui::Window::new("DMX Monitor")
            .open(&mut open)
            .default_pos([200.0, 150.0])
            .default_size([700.0, 460.0])
            .show(&ctx, |ui| dmx_monitor::ui(&mut v, ui));
        v.windows.dmx_monitor = open;

        let mut open = v.windows.audio;
        egui::Window::new("Audio")
            .open(&mut open)
            .default_pos([300.0, 70.0])
            .default_size([520.0, 700.0])
            .show(&ctx, |ui| audio_panel::ui(&mut v, ui));
        v.windows.audio = open;

        let mut open = v.windows.diagnostics;
        egui::Window::new("Diagnostics")
            .open(&mut open)
            .default_pos([180.0, 70.0])
            .default_size([760.0, 680.0])
            .show(&ctx, |ui| diagnostics::ui(&mut v, ui));
        v.windows.diagnostics = open;

        guide::ui(&mut v, &ctx);
        recent_card(&mut v, &ctx);

        // A window that was just opened (from a button anywhere) always comes to the front.
        let now_open = [v.windows.advanced_output, v.windows.fixture_editor, v.windows.dmx_monitor, v.windows.audio, v.windows.diagnostics];
        for (i, title) in ["Advanced Output", "Fixture Editor", "DMX Monitor", "Audio", "Diagnostics"].iter().enumerate() {
            if now_open[i] && !was_open[i] {
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(*title)));
            }
        }
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
        } else if arg == "--test-crash" {
            panic!("test crash requested with --test-crash");
        }
    }
}
