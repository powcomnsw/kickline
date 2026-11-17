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
    /// ~18 ms, ignores almost anything that isn't a kick.
    Stable,
    /// ~13 ms. Default.
    Balanced,
    /// ~11 ms, may fire on busy basslines or noise.
    Fast,
}

impl KickResponse {
    pub const ALL: [KickResponse; 3] = [KickResponse::Stable, KickResponse::Balanced, KickResponse::Fast];

    pub fn name(self) -> &'static str {
        match self {
            KickResponse::Stable => "Stable (~18 ms)",
            KickResponse::Balanced => "Balanced (~13 ms)",
            KickResponse::Fast => "Fast (~11 ms)",
        }
    }

    /// (smoothing time constant, trigger ratio)
    fn params(self) -> (f32, f32) {
        match self {
            KickResponse::Stable => (0.020, 3.0),
            KickResponse::Balanced => (0.010, 3.5),
            KickResponse::Fast => (0.006, 4.0),
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

struct Detector {
    filter: Chain,
    acc: f32,
    /// Short smoothing so a single hop's phase doesn't matter.
    fast_tau: f32,
    fast: f32,
    slow: f32,
    prev: f32,
    since_hit: f32,
    refractory: f32,
    base_ratio: f32,
    pulse: f32,
    hits: u64,
}

impl Detector {
    fn new(filter: Chain, fast_tau: f32, refractory: f32, base_ratio: f32) -> Self {
        Detector { filter, acc: 0.0, fast_tau, fast: 0.0, slow: 0.0, prev: 0.0, since_hit: 1.0, refractory, base_ratio, pulse: 0.0, hits: 0 }
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
        let detectors = vec![
            Detector::new(Chain::lp4(fs, 120.0), 0.02, 0.15, 3.0),
            Detector::new(Chain::band(fs, 1000.0, 4000.0), 0.006, 0.12, 3.0),
            Detector::new(Chain::hp4(fs, 8000.0), 0.003, 0.06, 3.0),
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
        }

        // Onsets: compare short-term energy against a ~250 ms average.
        let slow_k = 1.0 - (-dt / 0.25).exp();
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
        self.detectors[0].base_ratio = kick_ratio;
        let mut any_hit = false;
        for (i, d) in self.detectors.iter_mut().enumerate() {
            let e = d.fast;
            d.since_hit += dt;
            let ratio = d.base_ratio / self.settings.sensitivity[i].max(0.05);
            let loud_enough = e.sqrt() > GATE * 2.0;
            let rising = e > d.prev;
            if loud_enough && rising && dominant[i] && e > d.slow * ratio && d.since_hit > d.refractory {
                d.hits += 1;
                d.since_hit = 0.0;
                d.pulse = 1.0;
                any_hit = true;
            }
            d.prev = e;
            d.slow += (e - d.slow) * slow_k;
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
