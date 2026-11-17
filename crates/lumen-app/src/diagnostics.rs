//! Probe + report. "Run probe" checks the system, network, audio, project and
//! performance; "Export report" writes everything to one JSON file on the
//! Desktop that can be sent for troubleshooting.

use crate::engine::AudioSource;
use crate::log::{self, Level};
use crate::{paths, View};
use eframe::egui::{self, Color32, RichText};
use lumen_fixtures::{AdvancedOutput, Protocol};
use lumen_output::{discover, explain_send_error, ArtNode};
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Status {
    Pass,
    Info,
    Warn,
    Fail,
}

impl Status {
    fn icon(self) -> (&'static str, Color32) {
        match self {
            Status::Pass => ("✔", Color32::from_rgb(90, 210, 120)),
            Status::Info => ("ℹ", Color32::from_rgb(120, 170, 230)),
            Status::Warn => ("⚠", Color32::from_rgb(240, 190, 60)),
            Status::Fail => ("✖", Color32::from_rgb(240, 90, 80)),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub category: &'static str,
    pub name: String,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

fn check(
    category: &'static str,
    name: impl Into<String>,
    status: Status,
    detail: impl Into<String>,
) -> Check {
    Check {
        category,
        name: name.into(),
        status,
        detail: detail.into(),
        hint: None,
    }
}

impl Check {
    fn hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into());
        self
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct NodeInfo {
    pub ip: String,
    pub short_name: String,
    pub long_name: String,
    pub universes: Vec<u16>,
}

impl From<&ArtNode> for NodeInfo {
    fn from(n: &ArtNode) -> Self {
        NodeInfo {
            ip: n.ip.to_string(),
            short_name: n.short_name.clone(),
            long_name: n.long_name.clone(),
            universes: n.universes.clone(),
        }
    }
}

/// Rolling performance and audio-health numbers, updated every frame.
#[derive(Default)]
pub struct Metrics {
    pub frame_ms: std::collections::VecDeque<f32>,
    pub render_ms: f32,
    pub dmx_ms: f32,
    pub audio_cb_per_sec: f64,
    pub audio_peak: f32,
    pub audio_secs_since_sound: Option<f64>,
    pub audio_errors_seen: u64,
    last_audio_sample: Option<(std::time::Instant, u64)>,
    pub hits_at_start: [u64; 3],
    pub audio_started: Option<std::time::Instant>,
    /// Engine send intervals (ms).
    pub output_ms: std::collections::VecDeque<f32>,
    /// Detected hit -> DMX packet sent (ms), smoothed.
    hit_ms: f32,
    hit_count: u64,
    pub input_latency_ms: f32,
}

impl Metrics {
    pub fn frame(&mut self, dt_ms: f32) {
        if self.frame_ms.len() >= 600 {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(dt_ms);
    }

    pub fn output_tick(&mut self, dt_ms: f32) {
        if self.output_ms.len() >= 400 {
            self.output_ms.pop_front();
        }
        self.output_ms.push_back(dt_ms);
    }

    /// Packets per second actually achieved.
    pub fn output_hz(&self) -> f32 {
        if self.output_ms.is_empty() {
            return 0.0;
        }
        let avg = self.output_ms.iter().sum::<f32>() / self.output_ms.len() as f32;
        if avg > 0.0 {
            1000.0 / avg
        } else {
            0.0
        }
    }

    pub fn note_hit_latency(&mut self, ms: f32, _early_send: bool) {
        self.hit_count += 1;
        if self.hit_count == 1 {
            self.hit_ms = ms;
        } else {
            self.hit_ms += (ms - self.hit_ms) * 0.2;
        }
    }

    /// (smoothed ms from hit detection to DMX sent, number of hits measured)
    pub fn hit_latency(&self) -> (f32, u64) {
        (self.hit_ms, self.hit_count)
    }

    pub fn reset_audio(&mut self) {
        self.hit_ms = 0.0;
        self.hit_count = 0;
        self.input_latency_ms = 0.0;
        self.last_audio_sample = None;
        self.audio_errors_seen = 0;
    }

    pub fn smooth(v: &mut f32, x: f32) {
        *v += (x - *v) * 0.05;
    }

    fn frame_stats(&self) -> (f32, f32, f32) {
        Self::stats(&self.frame_ms)
    }

    fn stats(src: &std::collections::VecDeque<f32>) -> (f32, f32, f32) {
        if src.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut v: Vec<f32> = src.iter().copied().collect();
        v.sort_by(|a, b| a.total_cmp(b));
        let avg = v.iter().sum::<f32>() / v.len() as f32;
        let p95 = v[(v.len() as f32 * 0.95) as usize - usize::from(v.len() > 1)];
        (avg, p95, *v.last().unwrap())
    }

    /// Once per second: callback rate and peak of the live input.
    pub fn sample_audio(&mut self, audio: &AudioSource) {
        let AudioSource::Live(input) = audio else {
            self.last_audio_sample = None;
            self.audio_cb_per_sec = 0.0;
            self.audio_secs_since_sound = None;
            return;
        };
        let now = std::time::Instant::now();
        let cbs = input
            .health
            .callbacks
            .load(std::sync::atomic::Ordering::Relaxed);
        match self.last_audio_sample {
            Some((t, prev)) if now.duration_since(t) >= Duration::from_secs(1) => {
                self.audio_cb_per_sec = (cbs - prev) as f64 / now.duration_since(t).as_secs_f64();
                self.audio_peak = input.health.take_peak();
                let last = input
                    .health
                    .last_sound_ms
                    .load(std::sync::atomic::Ordering::Relaxed);
                let age = input.started.elapsed().as_secs_f64() - last as f64 / 1000.0;
                self.audio_secs_since_sound = Some(if last == 0 {
                    input.started.elapsed().as_secs_f64()
                } else {
                    age
                });
                self.last_audio_sample = Some((now, cbs));
                self.input_latency_ms = input
                    .health
                    .input_latency_us
                    .load(std::sync::atomic::Ordering::Relaxed)
                    as f32
                    / 1000.0;
                let errs = input
                    .health
                    .errors
                    .load(std::sync::atomic::Ordering::Relaxed);
                if errs > self.audio_errors_seen {
                    let msg = input
                        .health
                        .last_error
                        .lock()
                        .ok()
                        .and_then(|e| e.clone())
                        .unwrap_or_default();
                    log::warn("audio", format!("stream error ({} total): {msg}", errs));
                    self.audio_errors_seen = errs;
                }
            }
            None => self.last_audio_sample = Some((now, cbs)),
            _ => {}
        }
    }
}

struct NetResult {
    checks: Vec<Check>,
    nodes: Vec<ArtNode>,
}

#[derive(Default)]
pub struct State {
    pub checks: Vec<Check>,
    pub nodes: Vec<NodeInfo>,
    rx: Option<Receiver<NetResult>>,
    pub last_export: Option<PathBuf>,
    pub note: String,
    pub include_project: bool,
    pub gpu: String,
    pub ran_once: bool,
    pub copied: bool,
}

impl State {
    pub fn new(gpu: String) -> Self {
        State {
            include_project: true,
            gpu,
            ..Default::default()
        }
    }

    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    pub fn summary_counts(&self) -> (usize, usize, usize) {
        let c = |s| self.checks.iter().filter(|c| c.status == s).count();
        (c(Status::Pass), c(Status::Warn), c(Status::Fail))
    }
}

// ---------------------------------------------------------------- checks

fn system_checks(app: &View) -> Vec<Check> {
    let mut v = vec![
        check(
            "System",
            "Lumen",
            Status::Info,
            format!("version {}", env!("CARGO_PKG_VERSION")),
        ),
        check(
            "System",
            "Platform",
            Status::Info,
            format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
        ),
        check(
            "System",
            "Graphics",
            Status::Info,
            if app.diag.gpu.is_empty() {
                "unknown".to_string()
            } else {
                app.diag.gpu.clone()
            },
        ),
        check(
            "System",
            "Uptime",
            Status::Info,
            format!("{:.0} s", log::uptime()),
        ),
    ];
    let pf = paths::project_file();
    let writable = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::projects_dir().join(".write-test"))
        .is_ok();
    let _ = std::fs::remove_file(paths::projects_dir().join(".write-test"));
    v.push(if writable {
        check(
            "System",
            "Project folder",
            Status::Pass,
            pf.display().to_string(),
        )
    } else {
        check(
            "System",
            "Project folder",
            Status::Fail,
            format!("can't write to {}", paths::projects_dir().display()),
        )
        .hint("Check permissions on ~/Documents/Lumen.")
    });
    let crashes = log::previous_crashes();
    if crashes.is_empty() {
        v.push(check(
            "System",
            "Crashes",
            Status::Pass,
            "no crash reports from earlier sessions",
        ));
    } else {
        let first = crashes
            .last()
            .map(|(_, t)| t.lines().nth(3).unwrap_or("").to_string())
            .unwrap_or_default();
        v.push(
            check(
                "System",
                "Crashes",
                Status::Fail,
                format!(
                    "{} crash report(s) from earlier sessions. Latest: {first}",
                    crashes.len()
                ),
            )
            .hint("They're included in the exported report."),
        );
    }
    v
}

fn audio_checks(app: &View) -> Vec<Check> {
    let mut v = Vec::new();
    let devs = &app.audio_devices;
    v.push(check(
        "Audio",
        "Audio system",
        Status::Info,
        format!(
            "{}; default input: {}",
            lumen_audio::host_name(),
            lumen_audio::default_input_name().unwrap_or_else(|| "none".into())
        ),
    ));
    v.push(if devs.is_empty() {
        check("Audio", "Devices", Status::Warn, "no audio devices found (press Refresh in the Audio window)")
            .hint("On macOS install BlackHole to capture rekordbox's output, or connect an audio interface.")
    } else {
        check("Audio", "Devices", Status::Pass, devs.iter().map(|d| d.label()).collect::<Vec<_>>().join(", "))
    });
    let m = &app.e.metrics;
    match &app.e.audio {
        AudioSource::Off => v.push(check(
            "Audio",
            "Source",
            Status::Info,
            "off — lights won't react to music",
        )),
        AudioSource::Demo { .. } => {
            v.push(check("Audio", "Source", Status::Info, "built-in demo beat"))
        }
        AudioSource::Live(input) => {
            let direct = input.kind == lumen_audio::FeedKind::App;
            v.push(check(
                "Audio",
                "Source",
                Status::Info,
                format!(
                    "{} — {} Hz, {} ch, {}",
                    input.label(),
                    input.sample_rate,
                    input.channels,
                    input.sample_format
                ),
            ));
            let silence_hint = if direct {
                "macOS: System Settings > Privacy & Security > Screen & System Audio Recording, allow Lumen (System Audio Recording), then pick rekordbox again. Also check a track is playing."
            } else {
                "Is music playing? macOS: a denied microphone permission gives silence. With BlackHole, set rekordbox's output (Preferences > Audio) to BlackHole."
            };
            let age = input.started.elapsed().as_secs_f64();
            if age < 1.5 {
                v.push(check(
                    "Audio",
                    "Stream",
                    Status::Info,
                    "just started — run the probe again in a few seconds",
                ));
            } else if m.audio_cb_per_sec < 1.0 {
                v.push(
                    check("Audio", "Stream", Status::Fail, "capture opened but no audio is arriving").hint(if direct {
                        "rekordbox may have restarted or switched output device: pick it again under Audio > Source."
                    } else {
                        "macOS: System Settings > Privacy & Security > Microphone, allow Lumen, then restart it. Also try another device."
                    }),
                );
            } else {
                v.push(check(
                    "Audio",
                    "Stream",
                    Status::Pass,
                    format!("{:.0} buffers/s", m.audio_cb_per_sec),
                ));
                match m.audio_secs_since_sound {
                    Some(s) if s > 3.0 => v.push(
                        check(
                            "Audio",
                            "Signal",
                            Status::Warn,
                            format!("only silence for {s:.0} s"),
                        )
                        .hint(silence_hint),
                    ),
                    _ if m.audio_peak >= 0.99 => v.push(
                        check(
                            "Audio",
                            "Signal",
                            Status::Warn,
                            "clipping (peak at full scale)",
                        )
                        .hint("Turn the source down or lower Input gain."),
                    ),
                    _ if m.audio_peak < 0.01 => v.push(
                        check(
                            "Audio",
                            "Signal",
                            Status::Warn,
                            format!("very quiet (peak {:.3})", m.audio_peak),
                        )
                        .hint("Raise Input gain in the Audio window or the source volume."),
                    ),
                    _ => v.push(check(
                        "Audio",
                        "Signal",
                        Status::Pass,
                        format!("peak {:.2}", m.audio_peak),
                    )),
                }
            }
            let errs = input
                .health
                .errors
                .load(std::sync::atomic::Ordering::Relaxed);
            if errs > 0 {
                let last = input
                    .health
                    .last_error
                    .lock()
                    .ok()
                    .and_then(|e| e.clone())
                    .unwrap_or_default();
                v.push(check(
                    "Audio",
                    "Stream errors",
                    Status::Warn,
                    format!("{errs} error(s), last: {last}"),
                ));
            }
        }
    }
    if !matches!(app.e.audio, AudioSource::Off) {
        let b = &app.e.bands;
        let secs = app
            .e
            .metrics
            .audio_started
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let hits: Vec<u64> = (0..3)
            .map(|i| b.hits[i] - app.e.metrics.hits_at_start[i])
            .collect();
        let per_min = |h: u64| {
            if secs > 1.0 {
                h as f64 * 60.0 / secs
            } else {
                0.0
            }
        };
        v.push(check(
            "Audio",
            "Detection",
            Status::Info,
            format!(
                "kick {:.0}/min, snare {:.0}/min, hi-hat {:.0}/min over {:.0} s",
                per_min(hits[0]),
                per_min(hits[1]),
                per_min(hits[2]),
                secs
            ),
        ));
        if secs > 10.0
            && hits[0] == 0
            && app.e.metrics.audio_secs_since_sound.is_none_or(|s| s < 3.0)
        {
            v.push(check("Audio", "Kick detection", Status::Warn, "no kicks detected while audio is playing").hint(
                "Raise Kick sensitivity under Audio > Tuning, or check the level isn't too low.",
            ));
        }
    }
    v
}

fn project_checks(o: &AdvancedOutput, app: &View) -> Vec<Check> {
    let mut v = Vec::new();
    let pixels: usize = o
        .fixtures
        .iter()
        .filter_map(|f| o.types.get(f.type_id))
        .map(|t| t.pixel_count())
        .sum();
    v.push(check(
        "Project",
        "Size",
        Status::Info,
        format!(
            "{} layers, {} fixtures ({} pixels), {} types, {} lumiverses, {} universes in use",
            app.e.comp.layers.len(),
            o.fixtures.len(),
            pixels,
            o.types.len(),
            o.lumiverses.len(),
            app.e.dmx.len()
        ),
    ));
    let conflicts = o.conflicts();
    v.push(if conflicts.is_empty() {
        check("Project", "Address overlaps", Status::Pass, "none")
    } else {
        let pairs: Vec<String> = conflicts
            .iter()
            .map(|&(a, b)| format!("{} ↔ {}", o.fixtures[a].name, o.fixtures[b].name))
            .collect();
        check(
            "Project",
            "Address overlaps",
            Status::Fail,
            pairs.join(", "),
        )
        .hint("Change the start address in Advanced Output.")
    });
    let broken: Vec<&str> = o
        .fixtures
        .iter()
        .filter(|f| f.type_id >= o.types.len() || f.lumiverse >= o.lumiverses.len())
        .map(|f| f.name.as_str())
        .collect();
    if !broken.is_empty() {
        v.push(
            check(
                "Project",
                "Broken fixtures",
                Status::Fail,
                broken.join(", "),
            )
            .hint("Pick a type and lumiverse for them in Advanced Output."),
        );
    }
    let off: Vec<&str> = o
        .fixtures
        .iter()
        .filter(|f| !(0.0..=1.0).contains(&f.x) || !(0.0..=1.0).contains(&f.y))
        .map(|f| f.name.as_str())
        .collect();
    if !off.is_empty() {
        v.push(
            check("Project", "Off canvas", Status::Warn, off.join(", "))
                .hint("These fixtures sample outside the composition and stay black."),
        );
    }
    for (i, l) in o.lumiverses.iter().enumerate() {
        if l.protocol == Protocol::ArtNet && l.target.trim().parse::<Ipv4Addr>().is_err() {
            v.push(check(
                "Project",
                format!("Lumiverse \"{}\"", l.name),
                Status::Fail,
                format!("target \"{}\" isn't an IP address", l.target),
            ));
        }
        if !l.enabled {
            v.push(check(
                "Project",
                format!("Lumiverse \"{}\"", l.name),
                Status::Info,
                "disabled",
            ));
        }
        if !o.fixtures.iter().any(|f| f.lumiverse == i) {
            v.push(check(
                "Project",
                format!("Lumiverse \"{}\"", l.name),
                Status::Info,
                "no fixtures on it",
            ));
        }
        for (j, m) in o.lumiverses.iter().enumerate().skip(i + 1) {
            if m.universe == l.universe
                && m.protocol == l.protocol
                && m.target.trim() == l.target.trim()
            {
                v.push(
                    check(
                        "Project",
                        "Duplicate universe",
                        Status::Warn,
                        format!(
                            "\"{}\" and \"{}\" both send universe {} to {}",
                            l.name, m.name, l.universe, l.target
                        ),
                    )
                    .hint(format!(
                        "They overwrite each other. Change one universe (lumiverse {} or {}).",
                        i + 1,
                        j + 1
                    )),
                );
            }
        }
    }
    let silent = app
        .e
        .comp
        .layers
        .iter()
        .filter(|l| l.active.is_none())
        .count();
    if silent == app.e.comp.layers.len() {
        v.push(check(
            "Project",
            "Clips",
            Status::Warn,
            "no clip is playing — output is black",
        ));
    }
    v
}

fn perf_checks(app: &View) -> Vec<Check> {
    let mut v = Vec::new();
    let (avg, p95, worst) = app.e.metrics.frame_stats();
    let fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
    let st = if p95 > 34.0 {
        Status::Warn
    } else {
        Status::Pass
    };
    let mut c = check(
        "Performance",
        "Frame rate",
        st,
        format!("{fps:.0} fps (avg {avg:.1} ms, 95% {p95:.1} ms, worst {worst:.0} ms)"),
    );
    if st == Status::Warn {
        c = c.hint("DMX is sent once per frame, so slow frames make the lights stutter. Close other windows or reduce fixtures.");
    }
    v.push(c);
    v.push(check(
        "Performance",
        "Engine time",
        Status::Info,
        format!(
            "composition {:.2} ms, pixel mapping {:.2} ms per frame",
            app.e.metrics.render_ms, app.e.metrics.dmx_ms
        ),
    ));
    let m = &app.e.metrics;
    let hz = m.output_hz();
    let want = app.e.sync.send_rate_hz;
    v.push(if hz > 0.0 && hz < want * 0.8 {
        check(
            "Performance",
            "DMX rate",
            Status::Warn,
            format!("{hz:.0} Hz (set to {want:.0} Hz)"),
        )
        .hint("The engine is falling behind. Reduce fixtures/pixels or close other heavy apps.")
    } else {
        check(
            "Performance",
            "DMX rate",
            Status::Pass,
            format!("{hz:.0} Hz, independent of the window"),
        )
    });
    let (hit, n) = m.hit_latency();
    if n > 0 {
        let st = if hit > 15.0 {
            Status::Warn
        } else {
            Status::Pass
        };
        v.push(check(
            "Performance",
            "Hit → DMX",
            st,
            format!(
                "{hit:.1} ms from detecting a hit to sending DMX ({}); detection itself ~{}",
                if app.e.sync.send_on_hit {
                    "instant send on"
                } else {
                    "instant send off"
                },
                match app.e.audio_settings.kick_response {
                    lumen_audio::KickResponse::Stable => "18 ms",
                    lumen_audio::KickResponse::Balanced => "13 ms",
                    lumen_audio::KickResponse::Fast => "11 ms",
                }
            ),
        ));
    }
    if m.input_latency_ms > 0.0 {
        v.push(check(
            "Performance",
            "Audio input delay",
            if m.input_latency_ms > 30.0 {
                Status::Warn
            } else {
                Status::Pass
            },
            format!("{:.1} ms reported by the driver", m.input_latency_ms),
        ));
    }
    match &app.e.link_status {
        None => v.push(check("Sync", "Ableton Link", Status::Info, if crate::link::LinkSync::AVAILABLE { "off" } else { "not in this build" })),
        Some(st) if st.peers == 0 => v.push(
            check("Sync", "Ableton Link", Status::Warn, "on, but no other Link app found")
                .hint("rekordbox: Performance mode, press LINK (global section). Both apps must be on the same network; allow Lumen under Local Network on macOS."),
        ),
        Some(st) => v.push(check(
            "Sync",
            "Ableton Link",
            Status::Pass,
            format!("{} peer(s), {:.2} BPM, lights fire {:.0} ms early", st.peers, st.tempo, app.e.sync.lookahead_ms),
        )),
    }
    match lumen_audio::mac_tap::supported() {
        Ok(()) => {
            let rb = lumen_audio::mac_tap::find_rekordbox(&app.audio_apps);
            v.push(match rb {
                Some(r) => check(
                    "Sync",
                    "rekordbox direct capture",
                    Status::Pass,
                    format!("available ({} pid {})", r.name, r.pid),
                ),
                None => check(
                    "Sync",
                    "rekordbox direct capture",
                    Status::Info,
                    "supported, but rekordbox isn't running",
                ),
            });
        }
        Err(why) => v.push(check("Sync", "rekordbox direct capture", Status::Info, why)),
    }
    match &app.e.sender_stats {
        None => v.push(check(
            "Output",
            "DMX out",
            Status::Info,
            "off — tick DMX out to send",
        )),
        Some(s) => {
            if s.errors == 0 {
                v.push(check(
                    "Output",
                    "DMX out",
                    Status::Pass,
                    format!("{} packets sent, no errors", s.packets),
                ));
            } else {
                v.push(
                    check(
                        "Output",
                        "DMX out",
                        Status::Fail,
                        format!(
                            "{} packets sent, {} errors, last: {}",
                            s.packets,
                            s.errors,
                            s.last_error.clone().unwrap_or_default()
                        ),
                    )
                    .hint(s.last_error_hint.unwrap_or("")),
                );
            }
        }
    }
    let (w, e) = log::counts();
    v.push(check(
        "Log",
        "Events",
        if e > 0 { Status::Warn } else { Status::Pass },
        format!("{e} error(s), {w} warning(s) this session"),
    ));
    v
}

/// Network checks run on a background thread (discovery waits 2 s).
fn network_checks(
    lumiverses: Vec<lumen_fixtures::Lumiverse>,
    used: Vec<(usize, u16)>,
) -> NetResult {
    let mut v = Vec::new();
    let ifaces: Vec<(String, Ipv4Addr, Ipv4Addr, Option<Ipv4Addr>)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if !a.ip.is_loopback() => {
                Some((i.name, a.ip, a.netmask, a.broadcast))
            }
            _ => None,
        })
        .collect();
    if ifaces.is_empty() {
        v.push(
            check("Network", "Interfaces", Status::Fail, "no network interface has an IPv4 address")
                .hint("Plug in the Ethernet cable to your node/switch, or join the network. Art-Net nodes often use 2.x.x.x or 10.x.x.x."),
        );
    } else {
        let list: Vec<String> = ifaces
            .iter()
            .map(|(n, ip, mask, _)| format!("{n} {ip}/{}", u32::from(*mask).count_ones()))
            .collect();
        v.push(check(
            "Network",
            "Interfaces",
            Status::Pass,
            list.join(", "),
        ));
    }
    let on_subnet = |t: Ipv4Addr| {
        ifaces.iter().any(|(_, ip, mask, _)| {
            u32::from(*ip) & u32::from(*mask) == u32::from(t) & u32::from(*mask)
        })
    };
    let bcasts: Vec<Ipv4Addr> = ifaces.iter().filter_map(|(_, _, _, b)| *b).collect();

    let probe_sock = UdpSocket::bind("0.0.0.0:0").and_then(|s| s.set_broadcast(true).map(|_| s));
    if let Err(e) = &probe_sock {
        v.push(check("Network", "UDP socket", Status::Fail, e.to_string()));
    }

    for (i, l) in lumiverses.iter().enumerate() {
        if !l.enabled || !used.iter().any(|(lv, _)| *lv == i) {
            continue;
        }
        let name = format!("\"{}\" (U{} {})", l.name, l.universe, l.protocol.name());
        match l.protocol {
            Protocol::ArtNet => {
                let Ok(t) = l.target.trim().parse::<Ipv4Addr>() else {
                    continue;
                };
                if t == Ipv4Addr::BROADCAST {
                    let mut c = check(
                        "Network",
                        format!("{name} target"),
                        Status::Info,
                        "255.255.255.255 (broadcast to everything)",
                    );
                    if cfg!(target_os = "macos") || ifaces.len() > 1 {
                        c.status = Status::Warn;
                        let suggest: Vec<String> = bcasts.iter().map(|b| b.to_string()).collect();
                        c = c.hint(format!(
                            "macOS sends 255.255.255.255 out of one interface only. Use your node's IP, or the subnet broadcast: {}",
                            if suggest.is_empty() { "n/a".into() } else { suggest.join(" or ") }
                        ));
                    }
                    v.push(c);
                } else if !on_subnet(t) && !ifaces.is_empty() {
                    v.push(
                        check("Network", format!("{name} target"), Status::Warn, format!("{t} isn't on any of this computer's networks"))
                            .hint("Set a manual IP on your Ethernet port in the same range as the node, e.g. 2.0.0.100 / 255.0.0.0 for 2.x.x.x nodes."),
                    );
                } else {
                    v.push(check(
                        "Network",
                        format!("{name} target"),
                        Status::Pass,
                        format!("{t} is on a local network"),
                    ));
                }
                // Send a harmless ArtPoll to the target (never DMX: that could change live lights).
                if let Ok(s) = &probe_sock {
                    let pkt = lumen_output::discovery::artpoll_packet();
                    match s.send_to(&pkt, SocketAddr::from((t, lumen_output::ARTNET_PORT))) {
                        Ok(_) => v.push(check(
                            "Network",
                            format!("{name} send test"),
                            Status::Pass,
                            "packet accepted by the OS",
                        )),
                        Err(e) => v.push(
                            check(
                                "Network",
                                format!("{name} send test"),
                                Status::Fail,
                                e.to_string(),
                            )
                            .hint(explain_send_error(&e)),
                        ),
                    }
                }
            }
            Protocol::Sacn => {
                let u = l.universe + 1;
                let dest = Ipv4Addr::new(239, 255, (u >> 8) as u8, (u & 0xff) as u8);
                if let Ok(s) = &probe_sock {
                    // Empty datagram: receivers drop it, but the route is tested.
                    match s.send_to(&[], SocketAddr::from((dest, lumen_output::SACN_PORT))) {
                        Ok(_) => v.push(check(
                            "Network",
                            format!("{name} multicast"),
                            Status::Pass,
                            format!("route to {dest} OK"),
                        )),
                        Err(e) => v.push(
                            check(
                                "Network",
                                format!("{name} multicast"),
                                Status::Fail,
                                e.to_string(),
                            )
                            .hint(explain_send_error(&e)),
                        ),
                    }
                }
            }
        }
    }

    let mut targets = vec![Ipv4Addr::BROADCAST];
    targets.extend(bcasts.iter().copied());
    let nodes = match discover(&targets, Duration::from_secs(2)) {
        Ok(nodes) => {
            if nodes.is_empty() {
                v.push(check("Network", "Art-Net nodes", Status::Warn, "no node answered an ArtPoll").hint(
                    "Normal if you're only using sACN or the node doesn't support ArtPoll. Otherwise check power, cabling and that the node is on this computer's subnet. Close Resolume or other Art-Net apps while probing: they can take the replies.",
                ));
            } else {
                let list: Vec<String> = nodes
                    .iter()
                    .map(|n| {
                        format!(
                            "{} {} (universes {:?})",
                            n.ip,
                            if n.long_name.is_empty() {
                                &n.short_name
                            } else {
                                &n.long_name
                            },
                            n.universes
                        )
                    })
                    .collect();
                v.push(check(
                    "Network",
                    "Art-Net nodes",
                    Status::Pass,
                    list.join("; "),
                ));
                for (lv, uni) in &used {
                    let Some(l) = lumiverses.get(*lv) else {
                        continue;
                    };
                    if l.protocol == Protocol::ArtNet
                        && !nodes.iter().any(|n| n.universes.contains(uni))
                    {
                        v.push(
                            check("Network", format!("Universe {uni}"), Status::Warn, format!("used by \"{}\" but no node found listens to it", l.name))
                                .hint("Check the universe numbering: some nodes count from 1, Lumen's Art-Net universes count from 0."),
                        );
                    }
                }
            }
            nodes
        }
        Err(e) => {
            v.push(
                check(
                    "Network",
                    "Art-Net nodes",
                    Status::Warn,
                    format!("discovery failed: {e}"),
                )
                .hint(explain_send_error(&e)),
            );
            Vec::new()
        }
    };
    NetResult { checks: v, nodes }
}

// ---------------------------------------------------------------- run / export

pub fn run_probe(app: &mut View) {
    if app.diag.running() {
        return;
    }
    log::info("diagnostics", "probe started");
    app.audio_devices = lumen_audio::list_devices();
    app.audio_apps = lumen_audio::mac_tap::list_audio_apps();
    let mut checks = system_checks(app);
    checks.extend(audio_checks(app));
    checks.extend(project_checks(&app.e.output, app));
    checks.extend(perf_checks(app));
    app.diag.checks = checks;
    app.diag.ran_once = true;
    let (tx, rx) = channel();
    let lv = app.e.output.lumiverses.clone();
    let used: Vec<(usize, u16)> = app.e.dmx.keys().copied().collect();
    std::thread::spawn(move || {
        let _ = tx.send(network_checks(lv, used));
    });
    app.diag.rx = Some(rx);
}

/// Merge background results when ready.
pub fn poll(app: &mut View) {
    let Some(rx) = &app.diag.rx else { return };
    if let Ok(r) = rx.try_recv() {
        app.diag.checks.extend(r.checks);
        app.diag.nodes = r.nodes.iter().map(NodeInfo::from).collect();
        app.diag.rx = None;
        let (p, w, f) = app.diag.summary_counts();
        log::info(
            "diagnostics",
            format!("probe finished: {p} pass, {w} warn, {f} fail"),
        );
    }
}

#[derive(Serialize)]
struct Report<'a> {
    lumen_report: u32,
    created: String,
    app_version: &'static str,
    os: &'static str,
    arch: &'static str,
    note: &'a str,
    summary: Summary,
    checks: &'a [Check],
    art_net_nodes: &'a [NodeInfo],
    audio_bands_now: Vec<(String, f32)>,
    frame_ms_recent: Vec<f32>,
    log: Vec<log::Entry>,
    crashes: Vec<Crash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct Summary {
    pass: usize,
    warn: usize,
    fail: usize,
}

#[derive(Serialize)]
struct Crash {
    file: String,
    text: String,
}

pub fn summary_text(app: &View) -> String {
    let (p, w, f) = app.diag.summary_counts();
    let mut s = format!(
        "Lumen {} on {}/{} — {p} pass, {w} warn, {f} fail\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if !app.diag.note.trim().is_empty() {
        s.push_str(&format!("Note: {}\n", app.diag.note.trim()));
    }
    for c in app
        .diag
        .checks
        .iter()
        .filter(|c| matches!(c.status, Status::Warn | Status::Fail))
    {
        s.push_str(&format!(
            "{:?} [{}] {}: {}\n",
            c.status, c.category, c.name, c.detail
        ));
    }
    s
}

pub fn export(app: &mut View) -> Result<PathBuf, String> {
    let (iso, stamp) = paths::timestamp();
    let (pass, warn, fail) = app.diag.summary_counts();
    let project = if app.diag.include_project {
        let p = crate::Project {
            composition: app.e.comp.clone(),
            output: app.e.output.clone(),
        };
        serde_json::to_value(&p).ok()
    } else {
        None
    };
    let report = Report {
        lumen_report: 1,
        created: iso,
        app_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        note: &app.diag.note,
        summary: Summary { pass, warn, fail },
        checks: &app.diag.checks,
        art_net_nodes: &app.diag.nodes,
        audio_bands_now: lumen_core::AudioBand::ALL
            .iter()
            .map(|b| (b.name().to_string(), app.e.bands.get(*b)))
            .collect(),
        frame_ms_recent: app
            .e
            .metrics
            .frame_ms
            .iter()
            .rev()
            .take(120)
            .copied()
            .collect(),
        log: log::entries(),
        crashes: log::previous_crashes()
            .into_iter()
            .map(|(file, text)| Crash { file, text })
            .collect(),
        project,
    };
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    let path = paths::reports_dir().join(format!("lumen-report-{stamp}.json"));
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    log::info(
        "diagnostics",
        format!("report exported to {}", path.display()),
    );
    Ok(path)
}

// ---------------------------------------------------------------- UI

pub fn ui(app: &mut View, ui: &mut egui::Ui) {
    // Wait a few seconds after launch so frame-rate and audio numbers mean something.
    if !app.diag.ran_once {
        if log::uptime() > 4.0 {
            run_probe(app);
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Collecting a few seconds of data before probing…");
            });
            ui.ctx().request_repaint();
            return;
        }
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!app.diag.running(), egui::Button::new("▶ Run probe"))
            .clicked()
        {
            run_probe(app);
        }
        if app.diag.running() {
            ui.spinner();
            ui.weak("looking for Art-Net nodes…");
        }
        ui.separator();
        if ui
            .add_enabled(!app.diag.running(), egui::Button::new("💾 Export report"))
            .clicked()
        {
            match export(app) {
                Ok(p) => app.diag.last_export = Some(p),
                Err(e) => log::error("diagnostics", format!("export failed: {e}")),
            }
        }
        if ui.button("📋 Copy summary").clicked() {
            ui.ctx().copy_text(summary_text(app));
            app.diag.copied = true;
        }
        if app.diag.copied {
            ui.weak("copied");
        }
    });
    if let Some(p) = app.diag.last_export.clone() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Saved {}", p.display()))
                    .color(Color32::from_rgb(90, 210, 120)),
            );
            if ui.small_button("Show in Finder").clicked() {
                paths::reveal(&p);
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label("What happened?");
        ui.add(
            egui::TextEdit::singleline(&mut app.diag.note)
                .hint_text("e.g. pars don't light up, kick doesn't trigger…")
                .desired_width(f32::INFINITY),
        );
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.diag.include_project, "Include project");
        ui.weak("The report contains IP addresses, device names and your project. Nothing is sent anywhere.");
    });
    let (p, w, f) = app.diag.summary_counts();
    ui.horizontal(|ui| {
        ui.colored_label(Status::Pass.icon().1, format!("✔ {p} pass"));
        ui.colored_label(Status::Warn.icon().1, format!("⚠ {w} warning"));
        ui.colored_label(Status::Fail.icon().1, format!("✖ {f} problem"));
        if !log::previous_crashes().is_empty()
            && ui.small_button("Clear old crash reports").clicked()
        {
            log::clear_crashes();
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("checks")
        .show(ui, |ui| {
            let mut cat = "";
            for c in &app.diag.checks {
                if c.category != cat {
                    cat = c.category;
                    ui.add_space(4.0);
                    ui.strong(cat);
                }
                let (icon, col) = c.status.icon();
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(col, icon);
                    ui.label(RichText::new(&c.name).strong());
                    ui.label(&c.detail);
                });
                if let Some(h) = &c.hint {
                    if !h.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(18.0);
                            ui.label(
                                RichText::new(format!("→ {h}"))
                                    .italics()
                                    .color(Color32::from_gray(170)),
                            );
                        });
                    }
                }
            }
            ui.add_space(8.0);
            ui.collapsing("Event log", |ui| {
                for e in log::entries().iter().rev().take(200) {
                    let col = match e.level {
                        Level::Info => Color32::from_gray(170),
                        Level::Warn => Status::Warn.icon().1,
                        Level::Error => Status::Fail.icon().1,
                    };
                    let rep = if e.count > 1 {
                        format!(" ×{}", e.count)
                    } else {
                        String::new()
                    };
                    ui.colored_label(
                        col,
                        format!("{:>7.1}s  {:<12} {}{rep}", e.t, e.source, e.message),
                    );
                }
            });
        });
}
