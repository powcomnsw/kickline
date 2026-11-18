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
                analyzer.bands()
            }
            AudioSource::Live(feed) => {
                if let Ok(mut a) = feed.analyzer.lock() {
                    a.settings = settings;
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
        self.dmx = match &self.test {
            Some(t) if t.fixture < self.output.fixtures.len() => {
                let (fixture, step, k) = (t.fixture, t.step(), t.phase());
                self.output.render_dmx_by(&self.frame, &move |fi, i, n, _| {
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
            }
            _ => self.output.render_dmx(&self.frame),
        };
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
        for ((lv, uni), data) in &self.dmx {
            let Some(l) = self.output.lumiverses.get(*lv) else {
                continue;
            };
            if !l.enabled {
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
                Ok(()) => sent += 1,
                Err(e) => errors.push(format!("{}: {e}", l.name)),
            }
        }
        self.sender_stats = Some(sender.stats.clone());
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
