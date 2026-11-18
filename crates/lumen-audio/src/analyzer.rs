use crate::filter::Chain;
use lumen_core::{AudioBand, AudioBands};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Lets the audio thread wake the light engine the moment a hit is detected,
/// instead of the engine finding out on its next scheduled frame.
#[derive(Default)]
pub struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Wake {
    pub fn notify(&self) {
        if let Ok(mut f) = self.flag.lock() {
            *f = true;
            self.cv.notify_one();
        }
    }

    /// Wait until notified or the timeout passes. Returns true if notified.
    pub fn wait(&self, timeout: Duration) -> bool {
        let Ok(mut f) = self.flag.lock() else { return false };
        if !*f {
            f = match self.cv.wait_timeout(f, timeout) {
                Ok((g, _)) => g,
                Err(_) => return false,
            };
        }
        std::mem::replace(&mut *f, false)
    }
}

/// Speed vs. reliability of kick detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KickResponse {
    /// ~15 ms, ignores almost anything that isn't a kick.
    Stable,
    /// ~10 ms. Default.
    Balanced,
    /// ~8 ms, may fire on busy basslines or noise.
    Fast,
}

impl KickResponse {
    pub const ALL: [KickResponse; 3] = [KickResponse::Stable, KickResponse::Balanced, KickResponse::Fast];

    pub fn name(self) -> &'static str {
        match self {
            KickResponse::Stable => "Stable (~15 ms)",
            KickResponse::Balanced => "Balanced (~10 ms)",
            KickResponse::Fast => "Fast (~8 ms)",
        }
    }

    /// (smoothing time constant, deviations above the usual rise)
    fn params(self) -> (f32, f32) {
        match self {
            KickResponse::Stable => (0.010, 6.0),
            KickResponse::Balanced => (0.006, 5.0),
            KickResponse::Fast => (0.004, 4.0),
        }
    }
}

/// User-tweakable analysis settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Input gain applied before everything else.
    pub gain: f32,
    /// Per-band output gain for the continuous bands (Level..High), multiplied in after auto-levelling.
    pub band_gain: [f32; 6],
    /// Onset sensitivity for Kick, Snare, HiHat. Higher = fires more easily.
    pub sensitivity: [f32; 3],
    /// How long the hit pulse takes to fade, seconds (Kick, Snare, HiHat).
    pub pulse_decay: [f32; 3],
    pub kick_response: KickResponse,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            gain: 1.0,
            band_gain: [1.0; 6],
            sensitivity: [1.0; 3],
            pulse_decay: [0.18, 0.15, 0.07],
            kick_response: KickResponse::Balanced,
        }
    }
}

/// Signals quieter than this (RMS) are treated as silence.
const GATE: f32 = 0.0015;

struct BandState {
    filter: Option<Chain>,
    acc: f32,
    smooth: f32,
    peak: f32,
}

/// Onset detector for one drum. Works in decibels, so the input level
/// doesn't matter: a hit is a fast *rise* in the band's loudness that stands
/// out from how jumpy the band has been over the last second or so.
struct Detector {
    filter: Chain,
    acc: f32,
    /// Short smoothing so a single hop's phase doesn't matter.
    fast_tau: f32,
    fast: f32,
    /// Recent loudness (dB), newest last, to measure the rise over `lag` hops.
    hist: [f32; 8],
    lag: usize,
    /// Running mean and mean deviation of the rise (dB), for the adaptive threshold.
    mean: f32,
    dev: f32,
    /// Loudest level (dB) lately, decaying slowly.
    peak_db: f32,
    /// Biggest rise (dB) lately, decaying slowly: a hit must be a fair share of it.
    max_rise: f32,
    /// That share, at sensitivity 1.
    share: f32,
    /// How fast `max_rise` is forgotten (seconds).
    share_tau: f32,
    /// A hit must be within this many dB of `peak_db` (ignores quiet stuff).
    near_peak: f32,
    /// Smallest rise (dB) that can count, at sensitivity 1.
    min_rise: f32,
    armed: bool,
    since_hit: f32,
    refractory: f32,
    /// How many deviations above the mean the rise must be, at sensitivity 1.
    base_ratio: f32,
    pulse: f32,
    hits: u64,
    /// Last rise and threshold (dB), for the tuning display.
    pub last_rise: f32,
    pub last_threshold: f32,
}

impl Detector {
    #[allow(clippy::too_many_arguments)]
    fn new(filter: Chain, fast_tau: f32, refractory: f32, base_ratio: f32, lag: usize, near_peak: f32, min_rise: f32, share: f32) -> Self {
        Detector {
            filter,
            acc: 0.0,
            fast_tau,
            fast: 0.0,
            hist: [-100.0; 8],
            lag: lag.clamp(1, 7),
            mean: 0.0,
            dev: 1.0,
            peak_db: -100.0,
            max_rise: 0.0,
            share,
            share_tau: 2.0,
            near_peak,
            min_rise,
            armed: true,
            since_hit: 1.0,
            refractory,
            base_ratio,
            pulse: 0.0,
            hits: 0,
            last_rise: 0.0,
            last_threshold: 0.0,
        }
    }
}

/// Streaming analyser. Feed mono samples with [`Analyzer::process`], read
/// results with [`Analyzer::bands`].
pub struct Analyzer {
    pub settings: Settings,
    fs: f32,
    hop: usize,
    in_hop: usize,
    bands: Vec<BandState>, // Level, Sub, Bass, LowMid, HighMid, High
    detectors: Vec<Detector>, // Kick, Snare, HiHat
    out: AudioBands,
    /// Woken on every detected hit.
    pub wake: Option<Arc<Wake>>,
    /// When the most recent hit was detected (for latency measurement).
    pub last_hit_at: Option<Instant>,
    /// Tests: keep base_ratio as set instead of taking it from kick_response.
    kick_ratio_locked: bool,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate;
        let band = |f: Option<Chain>| BandState { filter: f, acc: 0.0, smooth: 0.0, peak: 0.02 };
        let bands = vec![
            band(None),
            band(Some(Chain::lp4(fs, 60.0))),
            band(Some(Chain::band(fs, 60.0, 250.0))),
            band(Some(Chain::band(fs, 250.0, 2000.0))),
            band(Some(Chain::band(fs, 2000.0, 6000.0))),
            band(Some(Chain::hp4(fs, 6000.0))),
        ];
        // (filter, smoothing, refractory s, deviations, rise over N hops, near-peak dB, min rise dB, share of biggest rise)
        let detectors = vec![
            // Kick: the 70-200 Hz "punch" rises sharply even when a bassline fills the sub.
            Detector::new(Chain::band(fs, 70.0, 200.0), 0.006, 0.16, 5.0, 3, 18.0, 1.5, 0.7),
            Detector::new(Chain::band(fs, 1500.0, 5000.0), 0.006, 0.12, 5.0, 2, 18.0, 3.0, 0.6),
            Detector::new(Chain::hp4(fs, 8000.0), 0.003, 0.06, 3.0, 2, 20.0, 3.0, 0.45),
        ];
        // ~5.3 ms hops at 48 kHz
        let hop = ((fs / 187.5) as usize).max(32);
        Analyzer {
            settings: Settings::default(),
            fs,
            hop,
            in_hop: 0,
            bands,
            detectors,
            out: AudioBands::default(),
            wake: None,
            last_hit_at: None,
            kick_ratio_locked: false,
        }
    }

    /// Adjust a detector (0 kick, 1 snare, 2 hi-hat): smoothing time constant and trigger ratio.
    /// The kick is reset from `settings.kick_response` on every hop.
    pub fn tune_detector(&mut self, i: usize, fast_tau: f32, base_ratio: f32) {
        if let Some(d) = self.detectors.get_mut(i) {
            d.fast_tau = fast_tau;
            d.base_ratio = base_ratio;
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.fs
    }

    pub fn bands(&self) -> AudioBands {
        self.out
    }

    /// Override the kick detector's shape (for tuning).
    #[cfg(test)]
    pub(crate) fn kick_shape(&mut self, lag: usize, min_rise: f32, share: f32, near_peak: f32, band: (f32, f32)) {
        let d = &mut self.detectors[0];
        d.lag = lag.clamp(1, 7);
        d.min_rise = min_rise;
        d.share = share;
        d.near_peak = near_peak;
        d.filter = Chain::band(self.fs, band.0, band.1);
    }

    /// For each drum (kick, snare, hi-hat): the latest rise and the rise
    /// needed to count as a hit, in dB. Drives the tuning display.
    pub fn onset_meter(&self) -> [(f32, f32); 3] {
        let m = |d: &Detector| (d.last_rise, d.last_threshold);
        [m(&self.detectors[0]), m(&self.detectors[1]), m(&self.detectors[2])]
    }

    pub fn process(&mut self, samples: &[f32]) {
        let g = self.settings.gain;
        for &s in samples {
            let x = s * g;
            for b in &mut self.bands {
                let y = match &mut b.filter {
                    Some(f) => f.process(x),
                    None => x,
                };
                b.acc += y * y;
            }
            for d in &mut self.detectors {
                let y = d.filter.process(x);
                d.acc += y * y;
            }
            self.in_hop += 1;
            if self.in_hop == self.hop {
                self.end_hop();
                self.in_hop = 0;
            }
        }
    }

    fn end_hop(&mut self) {
        let dt = self.hop as f32 / self.fs;
        let n = self.hop as f32;
        // Envelope: fast attack (~5 ms), slower release (~120 ms).
        let attack = 1.0 - (-dt / 0.005).exp();
        let release = 1.0 - (-dt / 0.12).exp();
        // Auto-level: running peak decays over ~4 s.
        let peak_decay = (-dt / 4.0).exp();

        for (i, b) in self.bands.iter_mut().enumerate() {
            let rms = (b.acc / n).sqrt();
            b.acc = 0.0;
            let k = if rms > b.smooth { attack } else { release };
            b.smooth += (rms - b.smooth) * k;
            b.peak = (b.peak * peak_decay).max(b.smooth).max(0.02);
            let v = if b.smooth < GATE { 0.0 } else { b.smooth / b.peak };
            self.out.values[i] = (v * self.settings.band_gain[i]).clamp(0.0, 1.0);
            if let Some(r) = self.out.raw.get_mut(i) {
                *r = b.smooth;
            }
        }

        // Onsets, in decibels: the rise over the last few hops, against an
        // adaptive threshold (mean + k x deviation of recent rises).
        let stat_k = 1.0 - (-dt / 1.5).exp();
        let peak_fall = dt * 3.0; // dB per hop: forgets a loud peak over a few seconds
        for d in &mut self.detectors {
            let raw = d.acc / n;
            d.acc = 0.0;
            d.fast += (raw - d.fast) * (1.0 - (-dt / d.fast_tau).exp());
        }
        // Snare vs hat: a snare's noise sits mostly in 1-4 kHz, a hat's above 8 kHz.
        let (mid, high) = (self.detectors[1].fast, self.detectors[2].fast);
        let dominant = [true, mid > high * 0.5, high > mid * 2.0];
        let (kick_tau, kick_ratio) = self.settings.kick_response.params();
        self.detectors[0].fast_tau = kick_tau;
        if !self.kick_ratio_locked {
            self.detectors[0].base_ratio = kick_ratio;
        }
        let mut any_hit = false;
        for (i, d) in self.detectors.iter_mut().enumerate() {
            d.since_hit += dt;
            let db = 10.0 * (d.fast + 1e-12).log10();
            let old = d.hist[7 - d.lag];
            d.hist.rotate_left(1);
            d.hist[7] = db;
            // Coming out of silence isn't a 70 dB "hit"; cap what one rise can be.
            let rise = (db - old).min(24.0);
            d.peak_db = (d.peak_db - peak_fall).max(db);
            d.max_rise = (d.max_rise * (-dt / d.share_tau).exp()).max(rise).max(0.0);
            let sens = self.settings.sensitivity[i].max(0.05);
            let threshold = (d.mean + d.base_ratio / sens * d.dev)
                .max(d.min_rise / sens)
                .max(d.max_rise * (d.share / sens).min(0.95));
            d.last_rise = rise;
            d.last_threshold = threshold;
            let loud_enough = d.fast.sqrt() > GATE * 2.0 && db > d.peak_db - d.near_peak * sens.sqrt();
            if rise < threshold * 0.5 {
                d.armed = true;
            }
            // A kick arriving out of real silence (the pause before a drop) is
            // a hit even if a noisy build-up taught the detector to expect
            // huge rises: that first kick of the drop matters most.
            let from_silence = i == 0 && rise >= 23.9 && old < d.peak_db - 30.0;
            if d.armed && loud_enough && dominant[i] && (rise > threshold || from_silence) && d.since_hit > d.refractory {
                d.hits += 1;
                d.since_hit = 0.0;
                d.pulse = 1.0;
                d.armed = false;
                any_hit = true;
            }
            // Learn what "normal" rises look like (only positive movement counts).
            let r = rise.max(0.0);
            d.mean += (r - d.mean) * stat_k;
            d.dev += ((r - d.mean).abs() - d.dev) * stat_k;
            d.dev = d.dev.max(0.3);
            d.pulse *= (-dt / self.settings.pulse_decay[i].max(0.01)).exp();
            self.out.set(AudioBand::DRUMS[i], d.pulse);
            self.out.hits[i] = d.hits;
        }
        if any_hit {
            self.last_hit_at = Some(Instant::now());
            if let Some(w) = &self.wake {
                w.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{noise, DemoBeat};
    use std::f32::consts::PI;

    const FS: f32 = 48000.0;

    fn run_tone(freq: f32) -> AudioBands {
        let mut a = Analyzer::new(FS);
        let s: Vec<f32> = (0..FS as usize).map(|i| 0.5 * (2.0 * PI * freq * i as f32 / FS).sin()).collect();
        a.process(&s);
        a.bands()
    }

    /// The arranged demo track, through the real analyser, into the section
    /// detector: it should name each part of the track while it plays.
    #[test]
    fn demo_track_sections_are_recognised() {
        use crate::demo::{DemoBeat, Part};
        use lumen_core::{Section, SectionDetector};
        let bpm = 128.0;
        let mut a = Analyzer::new(FS);
        let mut beat_src = DemoBeat::new(FS, bpm);
        beat_src.arranged = true;
        let mut d = SectionDetector::default();
        let block = 480; // 10 ms
        let mut buf = vec![0.0; block];
        let mut t = 0.0f64;
        let beats = DemoBeat::CYCLE_BARS as f64 * 4.0 * 3.0 + 4.0;
        let mut log: Vec<(i64, Section)> = Vec::new();
        while t * bpm as f64 / 60.0 < beats {
            beat_src.fill(&mut buf);
            a.process(&buf);
            t += block as f64 / FS as f64;
            let beat = t * bpm as f64 / 60.0;
            d.update(&a.bands(), beat);
            let b = beat.floor() as i64;
            if log.last().is_none_or(|l| l.0 != b) {
                log.push((b, d.current()));
            }
        }
        // Third time round: check what was called during each part, allowing
        // a bar to react (the pause and drop must be caught within a beat).
        let cycle = DemoBeat::CYCLE_BARS * 4;
        let start = cycle * 2;
        let at = |b: i64| log.iter().find(|l| l.0 == start + b).map(|l| l.1).unwrap();
        let summary: Vec<String> = (0..cycle).map(|b| format!("{b}:{:?}/{:?}", DemoBeat::part(b), at(b))).collect();
        let s = summary.join(" ");
        for b in 6..32 {
            assert_eq!(at(b), Section::Drop, "beat {b}: {s}");
        }
        for b in 40..48 {
            assert_eq!(at(b), Section::Groove, "beat {b}: {s}");
        }
        for b in 54..64 {
            assert_eq!(at(b), Section::Melodic, "beat {b}: {s}");
        }
        for b in 70..79 {
            assert_eq!(at(b), Section::BuildUp, "beat {b}: {s}");
        }
        // The log holds the section at the start of each beat, so the pause
        // (beat 79) shows from beat 80 and the drop (beat 80) from beat 81.
        assert_eq!(DemoBeat::part(79), Part::Pause);
        let next = |b: i64| log.iter().find(|l| l.0 == start + b).map(|l| l.1);
        assert_eq!(next(cycle), Some(Section::Pause), "{s}");
        assert_eq!(next(cycle + 1), Some(Section::Drop), "{s}");
    }

    #[test]
    fn silence_is_zero() {
        let mut a = Analyzer::new(FS);
        a.process(&vec![0.0; 48000]);
        assert!(a.bands().values.iter().all(|&v| v == 0.0));
        assert_eq!(a.bands().hits, [0; 5]);
    }

    #[test]
    fn tones_land_in_their_band() {
        let low = run_tone(100.0);
        assert!(low.get(AudioBand::Bass) > 0.8, "{low:?}");
        assert!(low.get(AudioBand::High) < 0.05, "{low:?}");
        let mid = run_tone(800.0);
        assert!(mid.get(AudioBand::LowMid) > 0.8, "{mid:?}");
        assert!(mid.get(AudioBand::Sub) < 0.05);
        let hi = run_tone(10000.0);
        assert!(hi.get(AudioBand::High) > 0.8, "{hi:?}");
        assert!(hi.get(AudioBand::Bass) < 0.05);
    }

    /// Feed 8 seconds of a synthetic 4-on-the-floor beat and count hits.
    #[test]
    fn detects_kick_snare_hat_in_demo_beat() {
        let mut beat = DemoBeat::new(FS, 120.0);
        let mut a = Analyzer::new(FS);
        let mut buf = vec![0.0; 512];
        let total = (FS * 8.0) as usize;
        let mut done = 0;
        while done < total {
            beat.fill(&mut buf);
            a.process(&buf);
            done += buf.len();
        }
        let [k, s, h, ..] = a.bands().hits;
        // 8 s at 120 BPM = 16 beats: 16 kicks, 8 snares (beats 2 & 4), 16 off-beat hats
        assert!((14..=17).contains(&k), "kicks {k}");
        assert!((7..=9).contains(&s), "snares {s}");
        assert!((14..=17).contains(&h), "hats {h}");
    }

    /// Kick hits must be reported within ~16 ms of the actual onset (Balanced).
    #[test]
    fn kick_detection_delay() {
        let mut beat = DemoBeat::new(FS, 128.0);
        let mut a = Analyzer::new(FS);
        let spb = 60.0 / 128.0f32;
        let mut buf = vec![0.0; 32];
        let (mut t, mut prev, mut delays) = (0usize, 0u64, Vec::new());
        while t < (FS * 8.0) as usize {
            beat.fill(&mut buf);
            a.process(&buf);
            t += buf.len();
            if a.bands().hits[0] != prev {
                prev = a.bands().hits[0];
                let pos = t as f32 / FS / spb;
                delays.push((pos - pos.round()) * spb * 1000.0);
            }
        }
        let avg = delays[1..].iter().sum::<f32>() / (delays.len() - 1) as f32;
        assert!(avg > 0.0 && avg < 16.0, "avg kick delay {avg} ms");
    }

    #[test]
    fn wake_fires_on_hit() {
        let w = Arc::new(Wake::default());
        let mut a = Analyzer::new(FS);
        a.wake = Some(w.clone());
        let mut beat = DemoBeat::new(FS, 120.0);
        let mut buf = vec![0.0; 4800]; // 100 ms, contains the first kick
        beat.fill(&mut buf);
        a.process(&buf);
        assert!(w.wait(Duration::from_millis(1)));
        assert!(!w.wait(Duration::from_millis(1))); // consumed
        assert!(a.last_hit_at.is_some());
    }

    /// Run `secs` of the demo beat through `f` (to add a bassline, change level…)
    /// and return (kick, snare, hat) counts.
    fn count_hits(secs: f32, bpm: f32, mut f: impl FnMut(f32, f32) -> f32) -> [u64; 3] {
        let mut beat = DemoBeat::new(FS, bpm);
        let mut a = Analyzer::new(FS);
        let mut buf = vec![0.0; 256];
        let mut t = 0usize;
        while t < (FS * secs) as usize {
            beat.fill(&mut buf);
            for (i, x) in buf.iter_mut().enumerate() {
                *x = f(*x, (t + i) as f32 / FS);
            }
            a.process(&buf);
            t += buf.len();
        }
        let h = a.bands().hits;
        [h[0], h[1], h[2]]
    }

    #[test]
    fn kick_count_ignores_input_level() {
        for gain in [0.02, 0.2, 1.0, 3.0] {
            let [k, ..] = count_hits(8.0, 120.0, |x, _| x * gain);
            assert!((14..=17).contains(&k), "gain {gain}: kicks {k}");
        }
    }

    /// A loud, always-on rolling bassline under the kicks (the case that broke
    /// the old detector): a new saw note every 16th, no gaps.
    #[test]
    fn kicks_through_a_loud_bassline() {
        let notes = [41.2f32, 41.2, 55.0, 49.0];
        let mut phase = 0.0f32;
        let [k, ..] = count_hits(8.0, 124.0, |x, t| {
            let sixteenth = 60.0 / 124.0 / 4.0;
            let n = (t / sixteenth) as usize;
            let f = notes[n % notes.len()];
            phase = (phase + f / FS).fract();
            let saw = 2.0 * phase - 1.0;
            // Each 16th has a soft attack, like a plucked bass.
            let env = 0.6 + 0.4 * (-(t % sixteenth) * 30.0).exp();
            x * 0.8 + saw * 0.45 * env
        });
        // 8 s at 124 BPM = 16.5 beats
        assert!((15..=18).contains(&k), "kicks {k}");
    }

    /// Like a finished club track: rolling sidechained bassline, then mastered
    /// loud (saturation squashes the kick's jump in level). The old detector
    /// found 1 kick here.
    #[test]
    fn kicks_in_a_loud_master() {
        let notes = [41.2f32, 41.2, 55.0, 49.0];
        let mut phase = 0.0f32;
        let drive = 3.0f32;
        let [k, ..] = count_hits(8.0, 126.0, |x, t| {
            let beat = 60.0 / 126.0;
            let sixteenth = beat / 4.0;
            let f = notes[(t / sixteenth) as usize % notes.len()];
            phase = (phase + f / FS).fract();
            let saw = 2.0 * phase - 1.0;
            // Sidechain: the bass ducks under each kick and swells back.
            let duck = 1.0 - 0.7 * (-(t % beat) * 8.0).exp();
            let mix = x * 0.9 + saw * 0.5 * duck;
            (mix * drive).tanh() / drive.tanh()
        });
        // 8 s at 126 BPM = 16.8 beats
        assert!((15..=18).contains(&k), "kicks {k}");
    }

    #[test]
    #[ignore]
    fn print_drums_in_loud_master() {
        let notes = [41.2f32, 41.2, 55.0, 49.0];
        let mut phase = 0.0f32;
        let h = count_hits(8.0, 126.0, |x, t| {
            let beat = 60.0 / 126.0;
            let f = notes[(t / (beat / 4.0)) as usize % notes.len()];
            phase = (phase + f / FS).fract();
            let duck = 1.0 - 0.7 * (-(t % beat) * 8.0).exp();
            ((x * 0.9 + (2.0 * phase - 1.0) * 0.5 * duck) * 3.0).tanh() / 3.0f32.tanh()
        });
        println!("kick/snare/hat {h:?} (expect ~17 / 8 / 17)");
    }

    #[test]
    fn snares_in_a_loud_master() {
        let notes = [41.2f32, 41.2, 55.0, 49.0];
        let mut phase = 0.0f32;
        let [_, sn, hat] = count_hits(8.0, 126.0, |x, t| {
            let beat = 60.0 / 126.0;
            let f = notes[(t / (beat / 4.0)) as usize % notes.len()];
            phase = (phase + f / FS).fract();
            let duck = 1.0 - 0.7 * (-(t % beat) * 8.0).exp();
            ((x * 0.9 + (2.0 * phase - 1.0) * 0.5 * duck) * 3.0).tanh() / 3.0f32.tanh()
        });
        // Snares on 2 and 4 (~8), hats on every off-beat (~17).
        assert!((7..=10).contains(&sn), "snares {sn}");
        assert!((14..=19).contains(&hat), "hats {hat}");
    }

    #[test]
    fn steady_noise_does_not_retrigger() {
        let mut a = Analyzer::new(FS);
        let mut seed = 1u32;
        let s: Vec<f32> = (0..(FS as usize * 4)).map(|_| 0.3 * noise(&mut seed)).collect();
        a.settings.kick_response = KickResponse::Stable;
        a.process(&s);
        // a steady signal may fire once at the very start, but not keep firing
        assert!(a.bands().hits[..3].iter().all(|&h| h <= 2), "{:?}", a.bands().hits);
    }
}

#[cfg(test)]
mod probe {
    use super::*;
    use crate::demo::DemoBeat;

    type Shape = (usize, f32, f32, f32, (f32, f32), f32, f32, f32);

    fn scenario(shape: Shape, which: usize) -> (u64, f32, f32) {
        let fs = 48000.0;
        let bpm = if which >= 5 { 120.0f32 } else { 126.0f32 };
        let mut beat = DemoBeat::new(fs, bpm);
        let mut a = Analyzer::new(fs);
        a.kick_shape(shape.0, shape.1, shape.2, shape.3, shape.4);
        a.detectors[0].base_ratio = shape.5;
        a.detectors[0].fast_tau = shape.6;
        a.kick_ratio_locked = true;
        a.detectors[0].share_tau = shape.7;
        let mut buf = vec![0.0; 32];
        let notes = [41.2f32, 41.2, 55.0, 49.0];
        let mut phase = 0.0f32;
        let spb = 60.0 / bpm;
        let (mut t, mut prev, mut delays) = (0usize, 0u64, Vec::new());
        while t < (fs * 8.0) as usize {
            beat.fill(&mut buf);
            for (i, x) in buf.iter_mut().enumerate() {
                let tt = (t + i) as f32 / fs;
                let f = notes[(tt / (spb / 4.0)) as usize % 4];
                phase = (phase + f / fs).fract();
                let saw = 2.0 * phase - 1.0;
                let duck = 1.0 - 0.7 * (-(tt % spb) * 8.0).exp();
                let pluck = 0.6 + 0.4 * (-(tt % (spb / 4.0)) * 30.0).exp();
                *x = match which {
                    0 | 5 => *x,
                    1 => *x * 0.03,
                    6 => *x * 0.8 + saw * 0.45 * pluck,
                    2 => *x * 0.8 + saw * 0.45 * pluck,
                    3 => ((*x * 0.9 + saw * 0.5 * duck) * 3.0).tanh() / 3.0f32.tanh(),
                    _ => ((*x * 0.9 + saw * 0.5 * duck * pluck) * 6.0).tanh() / 6.0f32.tanh(),
                };
            }
            a.process(&buf);
            t += buf.len();
            if a.bands().hits[0] != prev {
                prev = a.bands().hits[0];
                let pos = t as f32 / fs / spb;
                delays.push((pos - pos.round()) * spb * 1000.0);
            }
        }
        let on: Vec<f32> = delays.iter().skip(1).copied().filter(|d| d.abs() < 60.0).collect();
        let avg = if on.is_empty() { 99.0 } else { on.iter().sum::<f32>() / on.len() as f32 };
        let off = delays.iter().skip(1).filter(|d| d.abs() >= 60.0).count() as f32;
        (a.bands().hits[0], avg, off)
    }

    #[test]
    #[ignore]
    fn where_are_kicks() {
        for bpm in [120.0f32, 126.0] {
            let fs = 48000.0;
            let mut beat = DemoBeat::new(fs, bpm);
            let mut a = Analyzer::new(fs);
            let mut buf = vec![0.0; 32];
            let (mut t, mut prev) = (0usize, 0u64);
            let mut at = Vec::new();
            while t < (fs * 8.0) as usize {
                beat.fill(&mut buf);
                a.process(&buf);
                t += buf.len();
                if a.bands().hits[0] != prev {
                    prev = a.bands().hits[0];
                    at.push(format!("{:.2}", t as f32 / fs / (60.0 / bpm)));
                }
            }
            println!("{bpm}: {}", at.join(" "));
        }
    }

    fn snare_run(p: (usize, f32, f32, (f32, f32)), which: usize) -> u64 {
        let fs = 48000.0;
        let bpm = if which == 0 { 120.0f32 } else { 126.0 };
        let mut beat = DemoBeat::new(fs, bpm);
        let mut a = Analyzer::new(fs);
        {
            let d = &mut a.detectors[1];
            d.lag = p.0;
            d.base_ratio = p.1;
            d.share = p.2;
            d.filter = Chain::band(fs, p.3 .0, p.3 .1);
        }
        let mut buf = vec![0.0; 64];
        let notes = [41.2f32, 41.2, 55.0, 49.0];
        let mut phase = 0.0f32;
        let spb = 60.0 / bpm;
        let mut t = 0usize;
        while t < (fs * 8.0) as usize {
            beat.fill(&mut buf);
            for (i, x) in buf.iter_mut().enumerate() {
                let tt = (t + i) as f32 / fs;
                let f = notes[(tt / (spb / 4.0)) as usize % 4];
                phase = (phase + f / fs).fract();
                let saw = 2.0 * phase - 1.0;
                let duck = 1.0 - 0.7 * (-(tt % spb) * 8.0).exp();
                let pluck = 0.6 + 0.4 * (-(tt % (spb / 4.0)) * 30.0).exp();
                *x = match which {
                    0 => *x,
                    1 => *x * 0.8 + saw * 0.45 * pluck,
                    _ => ((*x * 0.9 + saw * 0.5 * duck) * 3.0).tanh() / 3.0f32.tanh(),
                };
            }
            a.process(&buf);
            t += buf.len();
        }
        a.bands().hits[1]
    }

    #[test]
    #[ignore]
    fn tune_snare() {
        let mut r = Vec::new();
        for lag in [2usize, 3, 4] {
            for ratio in [3.0f32, 4.0, 5.0] {
                for share in [0.5f32, 0.6, 0.7] {
                    for band in [(1000.0f32, 4000.0f32), (1500.0, 5000.0), (2000.0, 6000.0)] {
                        let p = (lag, ratio, share, band);
                        let c: Vec<u64> = (0..3).map(|w| snare_run(p, w)).collect();
                        let score = (c[0] as f32 - 8.0).abs() + (c[1] as f32 - 8.4).abs() + (c[2] as f32 - 8.4).abs();
                        r.push((score, format!("{p:?} -> {c:?}")));
                    }
                }
            }
        }
        r.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for (s, d) in r.iter().take(6) {
            println!("{s:5.1} {d}");
        }
    }

    #[test]
    #[ignore]
    fn tune_kick() {

        let mut results = Vec::new();
        // Widen these ranges to re-tune; the defaults came out of this search.
        for lag in [3usize] {
            for min_rise in [1.5f32] {
                for share in [0.7f32] {
                    for ratio in [4.0f32, 5.0] {
                        for band in [(70.0f32, 200.0f32)] {
                            for tau in [0.006f32] {
                              for stau in [2.0f32] {
                                let shape = (lag, min_rise, share, 18.0, band, ratio, tau, stau);
                                let mut score = 0.0;
                                let mut detail = Vec::new();
                                for w in 0..7 {
                                    let (k, avg, off) = scenario(shape, w);
                                    let expect = if w >= 5 { 16.0 } else { 16.8 };
                                    let weight = if w == 4 { 0.3 } else { 1.0 };
                                    score += weight * ((k as f32 - expect).abs() + off * 2.0 + (avg.max(0.0) - 12.0).max(0.0) * 0.3);
                                    detail.push(format!("{k}/{avg:.0}ms/{off}"));
                                }
                                results.push((score, format!("{shape:?} -> {}", detail.join("  "))));
                              }
                            }
                        }
                    }
                }
            }
        }
        results.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for (s, d) in results.iter().take(12) {
            println!("{s:6.1}  {d}");
        }
    }
}
