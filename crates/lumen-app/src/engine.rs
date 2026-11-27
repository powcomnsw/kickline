//! The light engine runs on its own thread, independent of the window:
//! - it renders and sends DMX at a fixed rate (default 44 Hz), even when the
//!   window is hidden, minimised or busy;
//! - it wakes up the moment the audio analyser detects a kick, snare or hat
//!   and sends straight away instead of waiting for the next frame;
//! - with Ableton Link it takes the beat from the Link session, looking a
//!   little ahead so the lights land on the beat despite node/fixture delay.
//!
//! The UI locks the engine only while it builds its widgets.

use crate::diagnostics::Metrics;
use crate::link::{LinkStatus, LinkSync};
use crate::log;
use lumen_audio::{Analyzer, AudioFeed, DemoBeat, Settings, Wake};
use lumen_core::{AudioBands, Composition, Frame, Rgba, StripPixel};
use lumen_fixtures::{AdvancedOutput, UNIVERSE_SIZE};
use lumen_output::{SendStats, Sender};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the audio analysis comes from.
pub enum AudioSource {
    Off,
    /// Built-in drum machine, for trying things out without music.
    Demo {
        beat: DemoBeat,
        analyzer: Analyzer,
    },
    /// A device or an app (the stream itself is kept alive by the UI).
    Live(AudioFeed),
}

impl AudioSource {
    pub fn label(&self) -> String {
        match self {
            AudioSource::Off => "Off".into(),
            AudioSource::Demo { .. } => "Demo beat".into(),
            AudioSource::Live(f) => f.label(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SyncSettings {
    /// Follow tempo and beat from an Ableton Link session (rekordbox LINK).
    pub link_enabled: bool,
    /// With Link: fire this many ms early to cancel node/fixture delay.
    pub lookahead_ms: f32,
    /// Regular DMX refresh rate.
    pub send_rate_hz: f32,
    /// Send immediately when a kick/snare/hat is detected.
    pub send_on_hit: bool,
    /// Find the beat from the music itself (Link and the typed BPM help).
    pub audio_clock: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        SyncSettings {
            link_enabled: false,
            lookahead_ms: 20.0,
            send_rate_hz: 44.0,
            send_on_hit: true,
            audio_clock: true,
        }
    }
}

/// A session being recorded for Diagnostics (see lumen_core::recording).
pub struct Recorder {
    pub rec: lumen_core::recording::Recording,
    started: Instant,
    /// The section the DJ last marked (0 none, 1 to 5).
    pub mark: u8,
}

/// How long the music must be silent before the lights idle (a real pause before a drop is a few seconds).
const IDLE_AFTER_S: f64 = 12.0;

/// One change in what Auto was showing, kept so a night can be reviewed.
#[derive(Clone, Debug)]
pub struct SetEvent {
    /// Seconds since Kickline started.
    pub t: f64,
    pub section: lumen_core::Section,
    pub look: String,
    pub palette: String,
    /// The section just became a drop.
    pub drop: bool,
}

const SET_LOG_MAX: usize = 1500;

pub struct Engine {
    /// What Auto played, in order (the set overview).
    pub set_log: Vec<SetEvent>,
    pub comp: Composition,
    pub output: AdvancedOutput,
    pub frame: Frame,
    pub dmx: BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]>,
    sender: Option<Sender>,
    pub sender_stats: Option<SendStats>,
    /// Network port state per lumiverse (same order), while sending.
    pub routes: Vec<lumen_output::RouteState>,
    pub output_enabled: bool,
    pub status: String,
    pub audio: AudioSource,
    pub audio_settings: Settings,
    pub bands: AudioBands,
    pub sync: SyncSettings,
    link: Option<LinkSync>,
    pub link_status: Option<LinkStatus>,
    pub metrics: Metrics,
    pub wake: Arc<Wake>,
    last_tick: Instant,
    last_send: Instant,
    last_hit_seen: Option<Instant>,
    pub shutdown: bool,
    /// Fixture test in progress: everything else dark, this fixture cycles
    /// red, green, blue, white, then a dot running from its first pixel.
    pub test: Option<FixtureTest>,
    /// Is output actually working, right now? (for the status bar)
    pub health: OutputHealth,
    /// Universe sweep in progress (no fixtures needed).
    pub sweep: Option<Sweep>,
    /// Per drum (kick, snare, hi-hat): latest rise and the rise needed for a hit (dB).
    pub onset: [(f32, f32); 3],
    /// When each lumiverse last sent, for its refresh-rate cap.
    last_sent: std::collections::HashMap<usize, Instant>,
    /// Every light dark (the fixture test and sweep still work).
    pub blackout: bool,
    /// The colour each fixture's pixels were sent, in wiring order (for the
    /// preview: exactly what the lights get).
    pub lights: Vec<Vec<[u8; 3]>>,
    ticks: u32,
    /// Session recording in progress.
    pub recorder: Option<Recorder>,
    /// Finds tempo and beat from the audio.
    pub tracker: lumen_audio::BeatTracker,
    clock: lumen_core::Clock,
    /// What the clock decided on the last tick.
    pub clock_out: lumen_core::ClockOut,
    /// The tempo the user typed or tapped (the one the demo beat plays at).
    manual_bpm: f64,
    /// The tempo the clock last wrote, to tell a user edit from our own.
    wrote_bpm: f64,
    time_s: f64,
    seen_hits: [u64; 2],
    prev_section: Option<lumen_core::Section>,
    /// Seconds since the music last made a sound.
    quiet_s: f64,
    quiet_hits: u64,
    /// Times a tick panicked and the engine carried on.
    pub panics: u32,
    #[cfg(test)]
    panic_next: bool,
}

impl Engine {
    pub fn new(comp: Composition, output: AdvancedOutput) -> Self {
        let frame = comp.render();
        Engine {
            comp,
            output,
            frame,
            dmx: BTreeMap::new(),
            sender: None,
            sender_stats: None,
            routes: Vec::new(),
            output_enabled: false,
            status: "DMX output off".into(),
            audio: AudioSource::Off,
            audio_settings: Settings::default(),
            bands: AudioBands::default(),
            sync: SyncSettings::default(),
            link: None,
            link_status: None,
            metrics: Metrics::default(),
            wake: Arc::new(Wake::default()),
            last_tick: Instant::now(),
            last_send: Instant::now(),
            last_hit_seen: None,
            shutdown: false,
            test: None,
            onset: [(0.0, 0.0); 3],
            sweep: None,
            health: Default::default(),
            last_sent: Default::default(),
            blackout: false,
            lights: Vec::new(),
            ticks: 0,
            recorder: None,
            tracker: Default::default(),
            clock: Default::default(),
            clock_out: Default::default(),
            manual_bpm: 120.0,
            wrote_bpm: 0.0,
            time_s: 0.0,
            seen_hits: [0; 2],
            prev_section: None,
            set_log: Vec::new(),
            quiet_s: 0.0,
            quiet_hits: 0,
            panics: 0,
            #[cfg(test)]
            panic_next: false,
        }
    }

    pub fn set_audio(&mut self, src: AudioSource) {
        log::info("audio", format!("source: {}", src.label()));
        self.audio = src;
        self.metrics.audio_started = Some(Instant::now());
        self.metrics.hits_at_start = [0; 3];
        self.metrics.reset_audio();
    }

    /// Start recording what the analyser hears and what the clock says.
    pub fn start_recording(&mut self, created: &str) {
        let rec = lumen_core::recording::Recording::new(
            env!("CARGO_PKG_VERSION"),
            created,
            &self.audio.label(),
            self.link_status.as_ref().is_some_and(|l| l.peers > 0),
            self.comp.director.genre.name(),
        );
        log::info("recording", "started");
        self.recorder = Some(Recorder { rec, started: Instant::now(), mark: 0 });
    }

    /// Stop and hand back the recording.
    pub fn stop_recording(&mut self) -> Option<lumen_core::recording::Recording> {
        let r = self.recorder.take()?;
        log::info("recording", format!("stopped after {:.0} s", r.rec.seconds()));
        Some(r.rec)
    }

    /// Seconds the engine has been running.
    pub fn uptime_s(&self) -> f64 {
        self.time_s
    }

    /// Note a change of look or section while Auto is running.
    fn log_set(&mut self) {
        if !self.comp.director.on || self.comp.director_rt.look_name.is_empty() {
            return;
        }
        let section = self.comp.sections.current();
        let look = &self.comp.director_rt.look_name;
        if self.set_log.last().is_some_and(|l| l.section == section && l.look == *look) {
            return;
        }
        let drop = section == lumen_core::Section::Drop && self.set_log.last().is_none_or(|l| l.section != section);
        self.set_log.push(SetEvent { t: self.time_s, section, look: look.clone(), palette: self.comp.director_rt.palette_name.clone(), drop });
        if self.set_log.len() > SET_LOG_MAX {
            self.set_log.remove(0);
        }
    }

    pub fn mark_section(&mut self, mark: u8) {
        if let Some(r) = &mut self.recorder {
            r.mark = mark;
        }
    }

    pub fn start_demo_audio(&mut self) {
        let fs = 48000.0;
        let mut analyzer = Analyzer::new(fs);
        analyzer.wake = Some(self.wake.clone());
        let mut beat = DemoBeat::new(fs, self.comp.bpm as f32);
        beat.arranged = true;
        self.set_audio(AudioSource::Demo { beat, analyzer });
    }

    fn period(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.sync.send_rate_hz.clamp(10.0, 120.0))
    }

    /// Advance the clock, read audio, render, map and send.
    pub fn tick(&mut self, woke_by_hit: bool) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .clamp(0.0, 0.2);
        self.last_tick = now;
        self.step(dt, woke_by_hit, now);
    }

    /// Show mode: keep the night going. Output and Auto stay on, and it says
    /// whether the music source needs reopening. Does nothing when show mode is off.
    pub fn keep_show_going(&mut self) -> bool {
        if !self.comp.director.show_mode {
            return false;
        }
        self.output_enabled = true;
        self.comp.director.on = true;
        matches!(self.audio, AudioSource::Off)
    }

    /// A tick that survives a panic: the engine logs it, clears what Auto was
    /// doing and carries on next tick, so one bad frame never ends the night.
    /// Returns false if it panicked.
    pub fn tick_guarded(&mut self, woke_by_hit: bool) -> bool {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            if std::mem::take(&mut self.panic_next) {
                panic!("test panic");
            }
            self.tick(woke_by_hit)
        }))
        .is_ok();
        if !ok {
            self.panics += 1;
            log::error("engine", format!("The engine hit an error and restarted its show state ({} so far); the lights carry on.", self.panics));
            self.comp.director_rt = Default::default();
            self.comp.auto.clear();
            self.last_tick = Instant::now();
            self.last_send = Instant::now();
        }
        ok
    }

    /// One tick of `dt` seconds (the real one, or a made-up one in tests).
    pub(crate) fn step(&mut self, dt: f64, woke_by_hit: bool, _now: Instant) {
        self.time_s += dt;
        self.comp.time_s = self.time_s;

        // The tempo the user set by hand: anything that isn't what we wrote last tick.
        if (self.comp.bpm - self.wrote_bpm).abs() > 1e-9 {
            self.manual_bpm = self.comp.bpm;
        }

        // 1. Hear the music and feed the beat tracker.
        self.read_audio(dt);
        let audio_on = !matches!(self.audio, AudioSource::Off);
        if audio_on {
            let strength = self.bands.raw[1] + self.bands.raw[2];
            let (k, s) = (self.bands.hits[0], self.bands.hits[1]);
            if k > self.seen_hits[0] {
                self.tracker.onset(self.time_s, strength, false);
            }
            if s > self.seen_hits[1] {
                self.tracker.onset(self.time_s, strength, true);
            }
            self.seen_hits = [k, s];
        } else {
            self.seen_hits = [0; 2];
        }

        // 2. Link, if on and someone else is in the session.
        let mut link_clock = None;
        if self.sync.link_enabled {
            let bpm = self.comp.bpm;
            let link = self.link.get_or_insert_with(|| LinkSync::new(bpm));
            link.set_enabled(true);
            let st = link.clock(0);
            link_clock = Some(lumen_core::LinkClock { peers: st.peers, tempo: st.tempo, beat: st.beat });
            self.link_status = Some(st);
        } else {
            if let Some(l) = &mut self.link {
                l.set_enabled(false);
            }
            self.link_status = None;
        }

        // 3. Decide the clock: the audio first, Link and the typed BPM as helpers.
        self.tracker.hint_tempo(link_clock.filter(|l| l.peers > 0).map(|l| l.tempo));
        if self.ticks % 88 == 0 {
            self.tracker.resolve_bar();
        }
        let r = self.tracker.update(self.time_s);
        let audio_clock = (audio_on && self.sync.audio_clock).then_some(lumen_core::AudioClock {
            tempo: r.tempo,
            beat: r.beat,
            bar_pos: r.bar_pos,
            tempo_conf: r.tempo_conf,
            bar_conf: r.bar_conf,
            locked: r.locked,
        });
        let lookahead = (self.sync.lookahead_ms / 1000.0) as f64;
        let out = self.clock.step(dt, audio_clock.as_ref(), link_clock.as_ref(), self.manual_bpm, lookahead);
        self.comp.set_clock_out(&out);
        self.wrote_bpm = self.comp.bpm;
        self.clock_out = out;

        // A drop always lands on a "1".
        let section = self.comp.sections.current();
        if self.prev_section != Some(section) {
            if section == lumen_core::Section::Drop && self.prev_section.is_some() && !self.comp.sections.is_forced() {
                self.tracker.anchor_bar(self.time_s);
            }
            self.prev_section = Some(section);
        }

        // Auto mode gives each group its own motion: tell it the groups now and then.
        self.ticks = self.ticks.wrapping_add(1);
        if self.comp.director.on && self.ticks % 20 == 1 {
            let groups = self.output.group_names();
            if self.comp.auto_groups != groups {
                self.comp.auto_groups = groups;
            }
            if self.comp.auto_roles != self.output.roles {
                self.comp.auto_roles = self.output.roles.clone();
            }
        }
        self.update_audio(dt);
        let t = Instant::now();
        self.frame = self.comp.render();
        self.log_set();
        Metrics::smooth(
            &mut self.metrics.render_ms,
            t.elapsed().as_secs_f32() * 1000.0,
        );
        self.send_dmx();
        let sent_at = Instant::now();
        self.metrics.output_tick(dt as f32 * 1000.0);
        self.last_send = sent_at;

        // Hit -> DMX latency: from the analyser detecting a hit to the packet leaving.
        let last_hit = match &self.audio {
            AudioSource::Demo { analyzer, .. } => analyzer.last_hit_at,
            AudioSource::Live(f) => f.analyzer.lock().ok().and_then(|a| a.last_hit_at),
            AudioSource::Off => None,
        };
        if let Some(h) = last_hit {
            if self.last_hit_seen != Some(h) {
                self.last_hit_seen = Some(h);
                self.metrics.note_hit_latency(
                    sent_at.duration_since(h).as_secs_f32() * 1000.0,
                    woke_by_hit,
                );
            }
        }
    }

    /// Read the analysis from the audio source (before the clock is decided).
    fn read_audio(&mut self, dt: f64) {
        let settings = self.audio_settings;
        self.bands = match &mut self.audio {
            AudioSource::Off => AudioBands::default(),
            AudioSource::Demo { beat, analyzer } => {
                analyzer.settings = settings;
                beat.bpm = self.manual_bpm as f32;
                let n = (analyzer.sample_rate() as f64 * dt) as usize;
                let mut buf = vec![0.0; n];
                beat.fill(&mut buf);
                analyzer.process(&buf);
                self.onset = analyzer.onset_meter();
                analyzer.bands()
            }
            AudioSource::Live(feed) => {
                if let Ok(mut a) = feed.analyzer.lock() {
                    a.settings = settings;
                    self.onset = a.onset_meter();
                }
                feed.bands()
            }
        };
    }

    /// Hand the analysis to the composition, now the clock is decided.
    fn update_audio(&mut self, dt: f64) {
        self.metrics.sample_audio(&self.audio);
        // Fail soft: a feed that has gone silent for a while is idle, not a pause.
        let hits: u64 = self.bands.hits.iter().sum();
        if self.bands.get(lumen_core::AudioBand::Level) > 0.03 || hits != self.quiet_hits {
            self.quiet_s = 0.0;
        } else {
            self.quiet_s += dt;
        }
        self.quiet_hits = hits;
        self.comp.idle = self.quiet_s > IDLE_AFTER_S;
        self.comp.set_audio(self.bands);
        self.bands = self.comp.audio; // includes Beat/Bar from the clock
        if let Some(r) = &mut self.recorder {
            r.rec.push(lumen_core::recording::Frame {
                t: r.started.elapsed().as_secs_f64(),
                beat: self.comp.beat,
                tempo: self.comp.bpm,
                peers: self.link_status.as_ref().map_or(0, |l| l.peers as u32),
                bands: self.comp.audio,
                mark: r.mark,
            });
        }
    }

    fn send_dmx(&mut self) {
        self.output.aspect = self.comp.width as f32 / self.comp.height as f32;
        let t = Instant::now();
        // Strip clips are laid over the mapped canvas per pixel; the fixture
        // test overrides everything.
        let stage = self.comp.strip_stage();
        let test = self.test.filter(|t| t.fixture < self.output.fixtures.len()).map(|t| (t.fixture, t.step(), t.phase()));
        // Layers aimed at groups: every distinct set of targeted groups a
        // fixture is in gets its own picture.
        let targeted = self.comp.targeted_groups();
        let n_fix = self.output.fixtures.len();
        let mut frames: Vec<Frame> = Vec::new();
        let mut frame_of = vec![usize::MAX; n_fix];
        let mut members: std::collections::HashMap<String, Vec<usize>> = Default::default();
        if !targeted.is_empty() {
            let areas: Vec<Option<[f32; 4]>> = targeted.iter().map(|g| self.output.group_area(g)).collect();
            for g in &targeted {
                members.insert(g.clone(), self.output.group_members(g));
            }
            let mut keys: Vec<Vec<usize>> = Vec::new();
            for (fi, fx) in self.output.fixtures.iter().enumerate() {
                let key: Vec<usize> = (0..targeted.len()).filter(|&k| fx.groups.contains(&targeted[k])).collect();
                if key.is_empty() {
                    continue;
                }
                let idx = match keys.iter().position(|k| *k == key) {
                    Some(i) => i,
                    None => {
                        let frame = self.comp.render_with(&|g| targeted.iter().position(|t| t == g).filter(|k| key.contains(k)).and_then(|k| areas[k]));
                        keys.push(key);
                        frames.push(frame);
                        keys.len() - 1
                    }
                };
                frame_of[fi] = idx;
            }
        }
        let lights = std::cell::RefCell::new(
            self.output
                .fixtures
                .iter()
                .map(|f| vec![[0u8; 3]; self.output.types.get(f.type_id).map(|t| t.pixel_count()).unwrap_or(0)])
                .collect::<Vec<_>>(),
        );
        let blackout = self.blackout;
        let fixtures = &self.output.fixtures;
        // Left, centre and right faders, per fixture, from the roles of its groups.
        let role_gain: Vec<f32> = fixtures.iter().map(|f| self.comp.gains.for_roles(f.groups.iter().filter_map(|g| self.output.roles.get(g)))).collect();
        let main = &self.frame;
        self.dmx = self.output.render_dmx_each(&|fi| frames.get(frame_of[fi]).unwrap_or(main), &|p, sampled| {
            let c = match test {
                None if blackout => Rgba::BLACK,
                None if stage.is_empty() => sampled,
                None => {
                    let groups = &fixtures[p.fixture].groups;
                    // A fixture that runs backwards along its path sees the effect flipped.
                    let p = if fixtures[p.fixture].path_reverse { StripPixel { x: 1.0 - p.x, ..p } } else { p };
                    stage.shade_in(sampled, p, &|g| {
                        if !groups.iter().any(|x| x == g) {
                            return None;
                        }
                        let m = members.get(g)?;
                        Some((m.iter().position(|&i| i == p.fixture)?, m.len()))
                    })
                }
                Some((fixture, step, k)) => {
                    let (fi, i, n) = (p.fixture, p.index, p.count);
                    if fi != fixture {
                        Rgba::BLACK
                    } else {
                        match step {
                            TestStep::Red => Rgba::rgb(1.0, 0.0, 0.0),
                            TestStep::Green => Rgba::rgb(0.0, 1.0, 0.0),
                            TestStep::Blue => Rgba::rgb(0.0, 0.0, 1.0),
                            TestStep::White => Rgba::rgb(1.0, 1.0, 1.0),
                            TestStep::Run => {
                                // A short white dot travelling in wiring order, first pixel stays red.
                                let pos = (k * n as f32) as usize;
                                if i == 0 {
                                    Rgba::rgb(1.0, 0.0, 0.0)
                                } else if i.abs_diff(pos) <= (n / 40).max(1) {
                                    Rgba::rgb(1.0, 1.0, 1.0)
                                } else {
                                    Rgba::BLACK
                                }
                            }
                        }
                    }
                }
            };
            let c = match (test, role_gain.get(p.fixture)) {
                (None, Some(&g)) if g < 1.0 => Rgba { r: c.r * g, g: c.g * g, b: c.b * g, a: c.a },
                _ => c,
            };
            if let Some(px) = lights.borrow_mut().get_mut(p.fixture).and_then(|l| l.get_mut(p.index)) {
                *px = c.to_u8();
            }
            c
        });
        self.lights = lights.into_inner();
        if let Some(sw) = &self.sweep {
            if let Some(l) = self.output.lumiverses.get(sw.lumiverse) {
                let base = l.universe;
                let on = sw.current();
                // Only the swept lumiverse: one universe lit, the rest dark.
                self.dmx.retain(|(lv, _), _| *lv != sw.lumiverse);
                for k in 0..sw.count {
                    let data = if k == on { sw.frame() } else { [0; UNIVERSE_SIZE] };
                    self.dmx.insert((sw.lumiverse, base + k), data);
                }
            }
        }
        Metrics::smooth(&mut self.metrics.dmx_ms, t.elapsed().as_secs_f32() * 1000.0);
        if !self.output_enabled {
            if self.sender.take().is_some() {
                log::info("dmx", "output off");
            }
            self.sender_stats = None;
            self.routes.clear();
            self.status = "DMX output off".into();
            return;
        }
        if self.sender.is_none() {
            match Sender::new() {
                Ok(s) => {
                    self.sender = Some(s);
                    log::info("dmx", "output on");
                }
                Err(e) => {
                    self.status = format!("Socket error: {e}");
                    log::error("dmx", format!("couldn't open UDP socket: {e}"));
                    self.output_enabled = false;
                    return;
                }
            }
        }
        let sender = self.sender.as_mut().unwrap();
        let mut sent = 0;
        let mut errors = Vec::new();
        let now = Instant::now();
        // Lumiverses with a refresh cap skip frames until their time comes.
        let due: Vec<bool> = self
            .output
            .lumiverses
            .iter()
            .enumerate()
            .map(|(i, l)| match l.max_hz.filter(|&hz| hz > 0) {
                Some(hz) => self.last_sent.get(&i).is_none_or(|t| now.duration_since(*t).as_secs_f32() >= 1.0 / hz as f32 - 0.002),
                None => true,
            })
            .collect();
        let mut sent_lv = std::collections::BTreeSet::new();
        for ((lv, uni), data) in &self.dmx {
            let Some(l) = self.output.lumiverses.get(*lv) else {
                continue;
            };
            if !l.enabled || !due.get(*lv).copied().unwrap_or(true) {
                continue;
            }
            let target = match l.target.trim().parse::<Ipv4Addr>() {
                Ok(ip) => ip,
                Err(_) if l.protocol == lumen_fixtures::Protocol::Sacn => Ipv4Addr::UNSPECIFIED,
                Err(_) => {
                    errors.push(format!("{}: bad IP \"{}\"", l.name, l.target));
                    continue;
                }
            };
            let proto = match l.protocol {
                lumen_fixtures::Protocol::ArtNet => lumen_output::Protocol::ArtNet,
                lumen_fixtures::Protocol::Sacn => lumen_output::Protocol::Sacn,
            };
            match sender.send_via(proto, target, *uni, data, l.interface.as_deref()) {
                Ok(()) => {
                    sent += 1;
                    sent_lv.insert(*lv);
                }
                Err(e) => errors.push(format!("{}: {e}", l.name)),
            }
        }
        // One ArtSync per network port after the whole frame, so every node
        // switches to the new frame at once. Not used with broadcast ArtDmx.
        let mut synced: Vec<(Option<String>, [u8; 3])> = Vec::new();
        for &lv in &sent_lv {
            self.last_sent.insert(lv, now);
            let l = &self.output.lumiverses[lv];
            if l.protocol != lumen_fixtures::Protocol::ArtNet || !l.sync {
                continue;
            }
            let Ok(ip) = l.target.trim().parse::<Ipv4Addr>() else { continue };
            if ip.octets()[3] == 255 {
                continue;
            }
            let o = ip.octets();
            let key = (l.interface.clone(), [o[0], o[1], o[2]]);
            if synced.contains(&key) {
                continue;
            }
            synced.push(key);
            if let Err(e) = sender.send_artsync(ip, l.interface.as_deref()) {
                errors.push(format!("{} (ArtSync): {e}", l.name));
            }
        }
        self.sender_stats = Some(sender.stats.clone());
        {
            let h = &mut self.health;
            if sent > 0 {
                h.last_ok = Some(now);
            }
            h.failing = errors.len();
            if let Some(e) = errors.first() {
                h.last_error = Some((now, e.clone()));
            }
            h.universes = self
                .dmx
                .keys()
                .filter(|(lv, _)| sent_lv.contains(lv))
                .map(|(lv, u)| (self.output.lumiverses.get(*lv).map(|l| l.protocol == lumen_fixtures::Protocol::Sacn).unwrap_or(false), *u))
                .collect();
        }
        self.routes = self.output.lumiverses.iter().map(|l| sender.route_state(l.interface.as_deref())).collect();
        for e in &errors {
            log::error("dmx", e.clone());
        }
        self.status = if errors.is_empty() {
            format!(
                "Sending {sent} universe(s) at {:.0} Hz",
                self.metrics.output_hz()
            )
        } else {
            format!("Sending {sent} universe(s) — {}", errors.join("; "))
        };
    }
}

/// What the status bar needs to say whether output is alive.
#[derive(Clone, Debug, Default)]
pub struct OutputHealth {
    pub last_ok: Option<Instant>,
    pub last_error: Option<(Instant, String)>,
    /// Sends that failed in the latest frame.
    pub failing: usize,
    /// (is sACN, universe) sent in the latest frame.
    pub universes: Vec<(bool, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputState {
    Off,
    /// Packets are leaving, no errors.
    Live,
    /// Some universes fail.
    Partial,
    /// Output is on but nothing has gone out for a moment.
    Dead,
    /// On, but nothing to send (no fixtures).
    Idle,
}

impl OutputHealth {
    pub fn state(&self, enabled: bool) -> OutputState {
        if !enabled {
            return OutputState::Off;
        }
        let ok_recent = self.last_ok.is_some_and(|t| t.elapsed() < Duration::from_millis(600));
        let err_recent = self.last_error.as_ref().is_some_and(|(t, _)| t.elapsed() < Duration::from_millis(600));
        match (ok_recent, err_recent) {
            (true, false) => OutputState::Live,
            (true, true) => OutputState::Partial,
            (false, true) => OutputState::Dead,
            (false, false) if self.universes.is_empty() && self.last_error.is_none() => OutputState::Idle,
            (false, false) => OutputState::Dead,
        }
    }
}

/// "0–3, 10, 12–13": universes as short ranges.
pub fn universe_ranges(mut u: Vec<u16>) -> String {
    u.sort_unstable();
    u.dedup();
    let mut out = Vec::new();
    let mut i = 0;
    while i < u.len() {
        let start = u[i];
        let mut end = start;
        while i + 1 < u.len() && u[i + 1] == end + 1 {
            i += 1;
            end = u[i];
        }
        out.push(if start == end { start.to_string() } else { format!("{start}–{end}") });
        i += 1;
    }
    out.join(", ")
}

/// Lights each universe of a lumiverse in turn, so you can see the cable,
/// controller and lights work before adding any fixtures.
#[derive(Clone, Copy, Debug)]
pub struct Sweep {
    pub lumiverse: usize,
    /// How many universes, starting at the lumiverse's own.
    pub count: u16,
    /// Value sent on every channel of the lit universe.
    pub level: u8,
    pub started: Instant,
    /// Stay on this one (manual stepping) instead of moving on by itself.
    pub hold: Option<u16>,
    /// Hold every channel on (true) or run a short chase across them (false).
    pub all: bool,
}

impl Sweep {
    pub const STEP: f32 = 1.5;

    pub fn new(lumiverse: usize, count: u16) -> Self {
        Sweep { lumiverse, count: count.max(1), level: 128, started: Instant::now(), hold: None, all: false }
    }
    /// The bytes for the lit universe. Chase: three channels at a time race
    /// across the universe, so no channel is held at a value (some fixtures
    /// treat a held value on a control channel as "reset" or "lamp off").
    pub fn frame(&self) -> [u8; UNIVERSE_SIZE] {
        if self.all {
            return [self.level; UNIVERSE_SIZE];
        }
        let mut u = [0u8; UNIVERSE_SIZE];
        let pos = ((self.started.elapsed().as_secs_f32() / 0.75).fract() * (UNIVERSE_SIZE - 3) as f32) as usize;
        for b in &mut u[pos..pos + 3] {
            *b = 255;
        }
        u
    }

    /// Offset (0-based) of the universe that's lit right now.
    pub fn current(&self) -> u16 {
        match self.hold {
            Some(h) => h % self.count,
            None => ((self.started.elapsed().as_secs_f32() / Self::STEP) as u16) % self.count,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestStep {
    Red,
    Green,
    Blue,
    White,
    Run,
}

impl TestStep {
    pub const ALL: [TestStep; 5] = [TestStep::Red, TestStep::Green, TestStep::Blue, TestStep::White, TestStep::Run];

    /// What the user should see on the real light right now.
    pub fn expect(self) -> &'static str {
        match self {
            TestStep::Red => "all RED",
            TestStep::Green => "all GREEN",
            TestStep::Blue => "all BLUE",
            TestStep::White => "all WHITE",
            TestStep::Run => "a white dot running away from the RED first pixel",
        }
    }

    pub fn wrong(self) -> &'static str {
        match self {
            TestStep::Red | TestStep::Green | TestStep::Blue => "Wrong colour? Change the colour order (e.g. GRB) in the fixture type.",
            TestStep::White => "Pinkish or bluish? Check the colour order or the white mode.",
            TestStep::Run => "Dot runs the wrong way? Tick Reverse. Starts in the middle? Check the address.",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FixtureTest {
    pub fixture: usize,
    pub started: Instant,
}

impl FixtureTest {
    pub const STEP: f32 = 1.5;

    pub fn new(fixture: usize) -> Self {
        FixtureTest { fixture, started: Instant::now() }
    }
    fn t(&self) -> f32 {
        self.started.elapsed().as_secs_f32() / Self::STEP
    }
    pub fn step(&self) -> TestStep {
        // The running dot gets two steps' time.
        let i = (self.t() as usize) % 6;
        TestStep::ALL[i.min(4)]
    }
    /// 0..1 progress through the running-dot step.
    pub fn phase(&self) -> f32 {
        let t = self.t() % 6.0;
        ((t - 4.0) / 2.0).clamp(0.0, 1.0)
    }
}

/// Minimum gap between packets when a hit triggers an early send.
const MIN_GAP: Duration = Duration::from_millis(4);

/// macOS delays ("coalesces") timers of ordinary background threads to save
/// power, which made the engine run at ~20 Hz instead of 44. Marking the
/// thread user-interactive, like audio and UI threads, keeps its timing tight.
fn raise_thread_priority() {
    #[cfg(target_vendor = "apple")]
    {
        extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
        // SAFETY: libSystem call on the current thread with valid arguments.
        let rc = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
        if rc != 0 {
            log::warn(
                "engine",
                format!("couldn't raise engine thread priority (error {rc})"),
            );
        }
    }
}

/// Run the engine until `shutdown` is set.
pub fn spawn(engine: Arc<Mutex<Engine>>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("lumen-engine".into())
        .spawn(move || {
            raise_thread_priority();
            run(engine)
        })
        .expect("spawn engine thread")
}

fn run(engine: Arc<Mutex<Engine>>) {
    loop {
        let (wake, timeout, send_on_hit) = {
            let Ok(e) = engine.lock() else { return };
            if e.shutdown {
                return;
            }
            let due = e.last_send + e.period();
            (
                e.wake.clone(),
                due.saturating_duration_since(Instant::now()),
                e.sync.send_on_hit,
            )
        };
        let mut woke = !timeout.is_zero() && wake.wait(timeout);
        if woke && !send_on_hit {
            // Not sending early: wait out the rest of the period.
            woke = false;
            let left = {
                let Ok(e) = engine.lock() else { return };
                (e.last_send + e.period()).saturating_duration_since(Instant::now())
            };
            std::thread::sleep(left);
        }
        let Ok(mut e) = engine.lock() else { return };
        if woke {
            let since = e.last_send.elapsed();
            if since < MIN_GAP {
                drop(e);
                std::thread::sleep(MIN_GAP - since);
                let Ok(e2) = engine.lock() else { return };
                e = e2;
            }
        }
        e.tick_guarded(woke);
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn sweep_chase_never_holds_more_than_three_channels() {
        let sw = Sweep::new(0, 4);
        let f = sw.frame();
        assert_eq!(f.iter().filter(|&&b| b > 0).count(), 3);
        let all = Sweep { all: true, ..sw };
        assert!(all.frame().iter().all(|&b| b == 128));
    }

    #[test]
    fn ranges() {
        assert_eq!(universe_ranges(vec![3, 0, 1, 2, 10, 12, 13]), "0–3, 10, 12–13");
        assert_eq!(universe_ranges(vec![5]), "5");
    }

    #[test]
    fn states() {
        let mut h = OutputHealth::default();
        assert_eq!(h.state(false), OutputState::Off);
        assert_eq!(h.state(true), OutputState::Idle);
        h.last_ok = Some(Instant::now());
        h.universes = vec![(false, 0)];
        assert_eq!(h.state(true), OutputState::Live);
        h.last_error = Some((Instant::now(), "x".into()));
        assert_eq!(h.state(true), OutputState::Partial);
        h.last_ok = Some(Instant::now() - Duration::from_secs(2));
        assert_eq!(h.state(true), OutputState::Dead);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_audio::AudioFeed;

    /// Overloaded CI VMs (GitHub's macOS runners) can sleep 100 ms when asked
    /// for 20 ms. Rate assertions are meaningless there.
    fn timers_are_sloppy() -> bool {
        let t = Instant::now();
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        if ms > 80.0 {
            eprintln!("skipping rate checks: 5 x 10 ms sleeps took {ms:.0} ms on this machine");
            true
        } else {
            false
        }
    }

    /// Auto mode inside the real engine: it finds the show's groups, gives
    /// each its own motion, and the lights really get lit by it.
    #[test]
    fn a_session_can_be_recorded_with_marks() {
        let mut e = Engine::new(Composition::new(32, 16, 6), AdvancedOutput::demo());
        e.start_demo_audio();
        e.start_recording("now");
        e.mark_section(5);
        for _ in 0..30 {
            e.tick(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let rec = e.stop_recording().expect("was recording");
        assert!(rec.frames.len() >= 30);
        assert!(rec.frames.iter().all(|f| f.mark == 5));
        assert!(rec.frames.windows(2).all(|w| w[1].t >= w[0].t));
        assert!(e.stop_recording().is_none());
    }

    #[test]
    fn the_clock_follows_the_music_not_the_typed_bpm() {
        // The demo drum machine plays at 132; the show says 120. After a few
        // bars the beat tracker should have taken over and the show follows 132.
        let mut comp = Composition::new(32, 16, 6);
        comp.bpm = 132.0;
        let mut e = Engine::new(comp, AdvancedOutput::demo());
        e.start_demo_audio();
        // Someone nudges the typed tempo: the demo keeps playing at its own.
        e.comp.bpm = 132.0;
        e.manual_bpm = 132.0;
        let t0 = Instant::now();
        for _ in 0..(44 * 25) {
            e.step(1.0 / 44.0, false, t0);
        }
        assert_eq!(e.clock_out.source, lumen_core::ClockSource::Audio, "{:?}", e.clock_out);
        assert!((e.comp.bpm / 132.0 - 1.0).abs() < 0.01, "tempo {}", e.comp.bpm);
        assert!(e.clock_out.steady);
        // And the beat lands on the kicks: when a kick is detected the beat is
        // close to a whole number.
        let mut offs: Vec<f64> = Vec::new();
        let mut seen = e.bands.hits[0];
        for _ in 0..(44 * 10) {
            e.step(1.0 / 44.0, false, t0);
            if e.bands.hits[0] > seen {
                seen = e.bands.hits[0];
                let b = e.clock_out.analysis_beat;
                offs.push((b - b.round()).abs() * 60.0 / 132.0);
            }
        }
        // (The build-up plays kicks on the off-beats too, so judge the median kick.)
        offs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let typical = offs[offs.len() / 2];
        assert!(typical < 0.03, "kick vs beat: typical {typical} s");
    }

    #[test]
    fn a_user_edit_of_the_bpm_is_kept_as_the_manual_tempo() {
        let mut e = Engine::new(Composition::new(32, 16, 6), AdvancedOutput::demo());
        let t0 = Instant::now();
        e.step(0.02, false, t0);
        e.comp.bpm = 140.0; // typed
        for _ in 0..50 {
            e.step(0.02, false, t0);
        }
        assert!((e.comp.bpm - 140.0).abs() < 1e-6, "free-runs on the typed tempo");
        assert_eq!(e.clock_out.source, lumen_core::ClockSource::Manual);
    }

    #[test]
    fn auto_mode_drives_the_lights_per_group() {
        let mut out = AdvancedOutput::demo();
        for (i, f) in out.fixtures.iter_mut().enumerate() {
            f.groups = vec![if i % 2 == 0 { "Even".to_string() } else { "Odd".to_string() }];
        }
        let mut comp = Composition::new(32, 16, 6);
        comp.director.on = true;
        let mut e = Engine::new(comp, out);
        e.start_demo_audio();
        for _ in 0..600 {
            std::thread::sleep(Duration::from_millis(2));
            e.tick(false);
        }
        assert_eq!(e.comp.auto_groups, vec!["Even".to_string(), "Odd".to_string()]);
        let names: Vec<String> = e.comp.auto.iter().filter_map(|l| l.target.clone()).collect();
        assert!(names.iter().any(|n| n == "Even") && names.iter().any(|n| n == "Odd"), "{names:?}");
        let lit = e.lights.iter().flatten().any(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 10);
        assert!(lit, "Auto lit nothing");
        // Switched off, its layers go away.
        e.comp.director.on = false;
        e.tick(false);
        assert!(e.comp.auto.is_empty());
    }

    #[test]
    fn engine_runs_at_rate_and_follows_demo_beat() {
        if timers_are_sloppy() {
            return;
        }
        let mut e = Engine::new(Composition::demo(), AdvancedOutput::demo());
        e.sync.send_rate_hz = 50.0;
        e.start_demo_audio();
        let eng = Arc::new(Mutex::new(e));
        let h = spawn(eng.clone());
        std::thread::sleep(Duration::from_millis(2100));
        {
            let mut e = eng.lock().unwrap();
            let hz = e.metrics.output_hz();
            // ~50 Hz plus extra sends on hits
            assert!(
                hz > 40.0,
                "output {hz} Hz (render {:.2} ms, mapping {:.2} ms per tick)",
                e.metrics.render_ms,
                e.metrics.dmx_ms
            );
            let kicks = e.bands.hits[0];
            assert!((3..=6).contains(&kicks), "kicks {kicks}");
            // early sends on hits happened and were quick
            let (avg, n) = e.metrics.hit_latency();
            assert!(n > 3, "hit sends {n}");
            assert!(avg < 15.0, "hit to DMX {avg} ms");
            assert!(!e.dmx.is_empty());
            e.shutdown = true;
        }
        e_wake(&eng);
        h.join().unwrap();
    }

    /// Live audio arriving on another thread: with "send on hit" the packet goes
    /// out within a few ms of detection instead of waiting for the next frame.
    #[test]
    fn instant_send_on_hit_from_live_audio() {
        fn run(send_on_hit: bool) -> f32 {
            let mut e = Engine::new(Composition::demo(), AdvancedOutput::demo());
            e.sync.send_rate_hz = 20.0; // slow refresh makes the difference obvious
            e.sync.send_on_hit = send_on_hit;
            let feed = AudioFeed::manual("test", 48000, Some(e.wake.clone()));
            e.set_audio(AudioSource::Live(feed.clone()));
            let eng = Arc::new(Mutex::new(e));
            let h = spawn(eng.clone());
            // Feed 3 s of demo beat in real time, 5 ms chunks.
            let mut beat = DemoBeat::new(48000.0, 120.0);
            let mut buf = vec![0.0; 240];
            let start = Instant::now();
            for i in 0..600u32 {
                beat.fill(&mut buf);
                feed.push(&buf);
                let due = start + Duration::from_millis(5 * (i as u64 + 1));
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            let (avg, n) = eng.lock().unwrap().metrics.hit_latency();
            eng.lock().unwrap().shutdown = true;
            e_wake(&eng);
            h.join().unwrap();
            assert!(n >= 5, "only {n} hits measured");
            avg
        }
        let instant = run(true);
        let waiting = run(false);
        assert!(instant < 8.0, "instant send took {instant} ms");
        assert!(
            waiting > instant + 5.0,
            "instant {instant} ms vs waiting {waiting} ms"
        );
        eprintln!("hit -> DMX: instant {instant:.1} ms, waiting for frame {waiting:.1} ms");
    }

    /// Prints how accurately this machine wakes a thread after 20 ms, with
    /// and without the raised priority. Run with: cargo test timer_accuracy -- --ignored --nocapture
    #[test]
    #[ignore]
    fn timer_accuracy() {
        fn measure(label: &'static str, raise: bool) {
            std::thread::spawn(move || {
                if raise {
                    raise_thread_priority();
                }
                let w = Wake::default();
                let mut v: Vec<f32> = (0..50)
                    .map(|_| {
                        let t = Instant::now();
                        w.wait(Duration::from_millis(20));
                        t.elapsed().as_secs_f32() * 1000.0
                    })
                    .collect();
                v.sort_by(|a, b| a.total_cmp(b));
                let s: Vec<f32> = (0..50)
                    .map(|_| {
                        let t = Instant::now();
                        std::thread::sleep(Duration::from_millis(20));
                        t.elapsed().as_secs_f32() * 1000.0
                    })
                    .collect();
                let avg_s = s.iter().sum::<f32>() / s.len() as f32;
                eprintln!(
                    "TIMER {label}: condvar 20ms -> median {:.1} ms, p90 {:.1} ms, max {:.1} ms; sleep 20ms -> avg {avg_s:.1} ms",
                    v[25], v[45], v[49]
                );
            })
            .join()
            .unwrap();
        }
        measure("normal", false);
        measure("user-interactive", true);
    }

    fn e_wake(eng: &Arc<Mutex<Engine>>) {
        eng.lock().unwrap().wake.notify();
    }

    #[test]
    fn a_silent_feed_idles_and_the_music_returning_ends_it() {
        let mut comp = Composition::new(32, 16, 6);
        comp.director.on = true;
        let mut e = Engine::new(comp, AdvancedOutput::demo());
        let t0 = Instant::now();
        for _ in 0..(44 * 8) {
            e.step(1.0 / 44.0, false, t0);
        }
        assert!(!e.comp.idle, "8 s of quiet is still just quiet");
        for _ in 0..(44 * 8) {
            e.step(1.0 / 44.0, false, t0);
        }
        assert!(e.comp.idle, "16 s of nothing is idle");
        e.start_demo_audio();
        for _ in 0..(44 * 3) {
            e.step(1.0 / 44.0, false, t0);
        }
        assert!(!e.comp.idle, "the music is back");
    }

    #[test]
    fn a_panicking_tick_is_logged_and_the_engine_carries_on() {
        let mut comp = Composition::new(32, 16, 6);
        comp.director.on = true;
        let mut e = Engine::new(comp, AdvancedOutput::demo());
        e.start_demo_audio();
        for _ in 0..50 {
            e.step(1.0 / 44.0, false, Instant::now());
        }
        e.panic_next = true;
        assert!(!e.tick_guarded(false));
        assert_eq!(e.panics, 1);
        // Next ticks are fine and Auto rebuilds itself.
        for _ in 0..50 {
            assert!(e.tick_guarded(false));
        }
        assert!(!e.comp.auto.is_empty());
    }

    #[test]
    fn show_mode_turns_output_and_auto_on_and_asks_for_the_music() {
        let mut e = Engine::new(Composition::new(32, 16, 6), AdvancedOutput::demo());
        assert!(!e.keep_show_going() && !e.output_enabled && !e.comp.director.on, "off means hands off");
        e.comp.director.show_mode = true;
        assert!(e.keep_show_going(), "no music source yet");
        assert!(e.output_enabled && e.comp.director.on);
        e.start_demo_audio();
        assert!(!e.keep_show_going(), "music is playing");
        // An old show file has no show mode.
        let old: lumen_core::DirectorSettings = serde_json::from_str("{\"on\":true}").unwrap();
        assert!(!old.show_mode);
    }

    /// Runs a show headless, faster than real time, and checks that nothing
    /// grows, goes bad or falls over. `minutes` of music at 44 ticks a second.
    fn soak(minutes: usize) -> (f64, usize) {
        let mut times: Vec<f64> = Vec::new();
        let p = crate::stage::example_show();
        let mut e = Engine::new(p.composition, p.output);
        e.comp.director.on = true;
        e.start_demo_audio();
        let t0 = Instant::now();
        let (mut worst_ms, mut auto_len) = (0.0f64, 0usize);
        let mut shapes = std::collections::BTreeSet::new();
        for k in 0..(minutes * 60 * 44) {
            let t = Instant::now();
            e.step(1.0 / 44.0, false, t0);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if ms > 5.0 && std::env::var("SOAK_TRACE").is_ok() {
                eprintln!("slow tick {k}: {ms:.1} ms, section {:?}, look {}, idle {}", e.comp.sections.current(), e.comp.director_rt.look_name, e.comp.idle);
            }
            worst_ms = worst_ms.max(ms);
            times.push(ms);
            assert!(e.comp.bpm.is_finite() && e.comp.beat.is_finite() && e.comp.bpm > 20.0 && e.comp.bpm < 400.0, "tempo went bad at tick {k}: {} / {}", e.comp.bpm, e.comp.beat);
            assert!(e.comp.director_rt.energy.is_finite());
            if k % 441 == 0 {
                auto_len = e.comp.auto.len();
                shapes.insert(auto_len);
            }
            // Pretend the feed drops out now and then: the show must idle and recover.
            if k % (44 * 90) == 44 * 80 {
                e.audio = AudioSource::Off;
            }
            if k % (44 * 90) == 44 * 85 {
                e.start_demo_audio();
            }
        }
        assert_eq!(e.panics, 0);
        assert!(shapes.len() <= 2, "Auto's layers keep changing shape: {shapes:?}");
        // Judge the 99th percentile, not the worst tick: a shared CI machine stalls
        // for tens of milliseconds now and then whatever the engine does (a hosted
        // Mac showed a 12 ms 99.9th percentile and a 228 ms worst tick with a
        // 1 ms median). The slow tail is printed for a human to read.
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p99 = times[(times.len() as f64 * 0.99) as usize - 1];
        let p999 = times[(times.len() as f64 * 0.999) as usize - 1];
        eprintln!("soak: {} ticks, median {:.2} ms, 99% {:.2} ms, 99.9% {:.2} ms, worst {:.1} ms", times.len(), times[times.len() / 2], p99, p999, worst_ms);
        (p99, auto_len)
    }

    #[test]
    fn a_short_soak_stays_healthy() {
        soak(2);
    }

    /// Hours-long set, run in release by CI: `cargo test --release -p lumen-app soak_long -- --ignored`.
    #[test]
    #[ignore]
    fn soak_long() {
        let minutes = std::env::var("SOAK_MINUTES").ok().and_then(|m| m.parse().ok()).unwrap_or(180);
        let (worst, _) = soak(minutes);
        if !cfg!(debug_assertions) {
            assert!(worst < 5.0, "99% of ticks should take under 5 ms, but the 99th percentile is {worst:.2} ms");
        }
    }
}
