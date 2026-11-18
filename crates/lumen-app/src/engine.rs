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
use lumen_core::{AudioBands, Composition, Frame, Rgba};
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
}

impl Default for SyncSettings {
    fn default() -> Self {
        SyncSettings {
            link_enabled: false,
            lookahead_ms: 20.0,
            send_rate_hz: 44.0,
            send_on_hit: true,
        }
    }
}

pub struct Engine {
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
        }
    }

    pub fn set_audio(&mut self, src: AudioSource) {
        log::info("audio", format!("source: {}", src.label()));
        self.audio = src;
        self.metrics.audio_started = Some(Instant::now());
        self.metrics.hits_at_start = [0; 3];
        self.metrics.reset_audio();
    }

    pub fn start_demo_audio(&mut self) {
        let fs = 48000.0;
        let mut analyzer = Analyzer::new(fs);
        analyzer.wake = Some(self.wake.clone());
        self.set_audio(AudioSource::Demo {
            beat: DemoBeat::new(fs, self.comp.bpm as f32),
            analyzer,
        });
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

        // Clock: Ableton Link if on and someone else is in the session, else our BPM.
        if self.sync.link_enabled {
            let bpm = self.comp.bpm;
            let link = self.link.get_or_insert_with(|| LinkSync::new(bpm));
            link.set_enabled(true);
            let st = link.clock((self.sync.lookahead_ms * 1000.0) as i64);
            if st.peers > 0 {
                self.comp.set_clock(st.beat, st.tempo);
            } else {
                self.comp.tick(dt);
            }
            self.link_status = Some(st);
        } else {
            if let Some(l) = &mut self.link {
                l.set_enabled(false);
            }
            self.link_status = None;
            self.comp.tick(dt);
        }

        self.update_audio(dt);
        let t = Instant::now();
        self.frame = self.comp.render();
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

    fn update_audio(&mut self, dt: f64) {
        let settings = self.audio_settings;
        self.bands = match &mut self.audio {
            AudioSource::Off => AudioBands::default(),
            AudioSource::Demo { beat, analyzer } => {
                analyzer.settings = settings;
                beat.bpm = self.comp.bpm as f32;
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
        self.metrics.sample_audio(&self.audio);
        self.comp.set_audio(self.bands);
        self.bands = self.comp.audio; // includes Beat/Bar from the clock
    }

    fn send_dmx(&mut self) {
        self.output.aspect = self.comp.width as f32 / self.comp.height as f32;
        let t = Instant::now();
        // Strip clips are laid over the mapped canvas per pixel; the fixture
        // test overrides everything.
        let stage = self.comp.strip_stage();
        let test = self.test.filter(|t| t.fixture < self.output.fixtures.len()).map(|t| (t.fixture, t.step(), t.phase()));
        self.dmx = if stage.is_empty() && test.is_none() {
            self.output.render_dmx(&self.frame)
        } else {
            self.output.render_dmx_by(&self.frame, &move |p, sampled| {
                let Some((fixture, step, k)) = test else {
                    return stage.shade(sampled, p);
                };
                let (fi, i, n) = (p.fixture, p.index, p.count);
                if fi != fixture {
                    return Rgba::BLACK;
                }
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
            })
        };
        if let Some(sw) = &self.sweep {
            if let Some(l) = self.output.lumiverses.get(sw.lumiverse) {
                let base = l.universe;
                let on = sw.current();
                // Only the swept lumiverse: one universe lit, the rest dark.
                self.dmx.retain(|(lv, _), _| *lv != sw.lumiverse);
                for k in 0..sw.count {
                    let level = if k == on { sw.level } else { 0 };
                    self.dmx.insert((sw.lumiverse, base + k), [level; UNIVERSE_SIZE]);
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
}

impl Sweep {
    pub const STEP: f32 = 1.5;

    pub fn new(lumiverse: usize, count: u16) -> Self {
        Sweep { lumiverse, count: count.max(1), level: 128, started: Instant::now(), hold: None }
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
        e.tick(woke);
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

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
}
