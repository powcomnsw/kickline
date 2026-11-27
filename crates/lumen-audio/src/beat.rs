//! A beat tracker for dance music: tempo, the beat, and the "1" of the bar,
//! worked out from the kick and snare hits the analyser already finds.
//!
//! No models and no FFT. Every half second it looks at the last twelve
//! seconds of hits and asks, for each tempo from 90 to 180 BPM, how tightly
//! the hits cluster at one point of the beat (a circular mean: a comb filter
//! without the comb). The tightest tempo wins; where they cluster is the
//! beat. The result then steers a free-running clock gently, so one odd
//! estimate never makes the beat jump, and the clock keeps counting through
//! a breakdown with no kicks. When kicks return it takes the first confident
//! estimate straight away.
//!
//! The "1" of the bar comes from where the strongest kicks fall, from drops
//! (which always land on a bar), and from an explicit anchor (a Shift-tap).
//!
//! Everything is deterministic: the same hits at the same times give the same
//! clock. Time is in seconds from any fixed start.

use std::collections::VecDeque;

/// Slowest and fastest tempo it will report. Half and double tempos fit a
/// grid equally well, so this range (and a tempo hint) picks between them.
pub const MIN_BPM: f64 = 90.0;
pub const MAX_BPM: f64 = 180.0;

const WINDOW_S: f64 = 12.0;
const EVERY_S: f64 = 0.5;

#[derive(Clone, Copy, Debug)]
struct Onset {
    t: f64,
    w: f64,
}

/// What the tracker knows at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    pub tempo: f64,
    /// The beat now, counting up forever: whole numbers are beats.
    pub beat: f64,
    /// Beats since the last "1" (0 up to beats per bar).
    pub bar_pos: f64,
    /// How tightly the hits sit on the beat (0..1), fading in a breakdown.
    pub tempo_conf: f32,
    /// How sure the beat position is (0..1).
    pub phase_conf: f32,
    /// How sure the "1" is (0..1).
    pub bar_conf: f32,
    /// A trustworthy beat right now.
    pub locked: bool,
}

#[derive(Clone, Debug)]
pub struct BeatTracker {
    onsets: VecDeque<Onset>,
    /// Beat position at `t_ref`, and the period (seconds per beat).
    t_ref: f64,
    beat_ref: f64,
    period: f64,
    has_clock: bool,
    next_update: f64,
    /// Time of the last update that found a confident beat.
    last_good: f64,
    last_conf: f32,
    /// Estimates that disagreed with the clock in a row (a real change, or noise).
    pending: u32,
    /// Hits since the last estimate (without new hits there is nothing new to learn).
    fresh: u32,
    /// Where the strong kicks fall, by beat number mod beats per bar.
    acc: [f64; 12],
    pub beats_per_bar: u32,
    /// The beat number (mod bar length) that is the "1".
    bar_offset: i64,
    bar_conf: f32,
    anchored: bool,
    hint: Option<f64>,
}

impl Default for BeatTracker {
    fn default() -> Self {
        BeatTracker {
            onsets: VecDeque::new(),
            t_ref: 0.0,
            beat_ref: 0.0,
            period: 0.5,
            has_clock: false,
            next_update: 0.0,
            last_good: f64::NEG_INFINITY,
            last_conf: 0.0,
            pending: 0,
            fresh: 0,
            acc: [0.0; 12],
            beats_per_bar: 4,
            bar_offset: 0,
            bar_conf: 0.0,
            anchored: false,
            hint: None,
        }
    }
}

impl BeatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// A tempo from outside (Link, tap, the typed BPM). It picks between a
    /// tempo and its double or half, and carries the clock through a breakdown.
    pub fn hint_tempo(&mut self, bpm: Option<f64>) {
        self.hint = bpm.filter(|b| b.is_finite() && *b > 20.0);
    }

    /// A hit. `strength` is how hard it was (the bass level at the hit works well).
    pub fn onset(&mut self, t: f64, strength: f32, snare: bool) {
        let w = if snare { 0.4 } else { 1.0 };
        self.onsets.push_back(Onset { t, w });
        self.fresh += 1;
        if self.has_clock && !snare {
            let n = self.beat_at(t).round() as i64;
            let k = self.beats_per_bar.clamp(2, 12) as i64;
            self.acc[n.rem_euclid(k) as usize] += 0.3 + strength.clamp(0.0, 2.0) as f64;
        }
    }

    /// The beat at time `t` on the current clock.
    pub fn beat_at(&self, t: f64) -> f64 {
        self.beat_ref + (t - self.t_ref) / self.period
    }

    pub fn tempo(&self) -> f64 {
        60.0 / self.period
    }

    /// This moment is a "1" (a drop just landed, or the DJ said so).
    pub fn anchor_bar(&mut self, t: f64) {
        if !self.has_clock {
            return;
        }
        let k = self.beats_per_bar.clamp(2, 12) as i64;
        self.bar_offset = (self.beat_at(t).round() as i64).rem_euclid(k);
        self.bar_conf = 0.95;
        self.anchored = true;
        self.acc = [0.0; 12];
    }

    /// Advance to time `t` and read the clock. Call this every tick.
    pub fn update(&mut self, t: f64) -> Reading {
        while self.onsets.front().is_some_and(|o| o.t < t - WINDOW_S) {
            self.onsets.pop_front();
        }
        if t >= self.next_update {
            self.next_update = t + EVERY_S;
            self.estimate(t);
        }
        // Forget old accents a little every beat.
        self.reading(t)
    }

    /// The reading at time `t` without advancing anything (for the UI).
    pub fn peek(&self, t: f64) -> Reading {
        self.reading(t)
    }

    fn reading(&self, t: f64) -> Reading {
        let beat = self.beat_at(t);
        let k = self.beats_per_bar.clamp(2, 12) as f64;
        // Confidence fades the longer it has been since a good estimate.
        let age = (t - self.last_good).max(0.0);
        let fade = (-age / 20.0).exp() as f32;
        let tempo_conf = if self.has_clock { self.last_conf * fade } else { 0.0 };
        let locked = self.has_clock && tempo_conf > 0.45 && age < 8.0;
        Reading {
            tempo: self.tempo(),
            beat,
            bar_pos: (beat - self.bar_offset as f64).rem_euclid(k),
            tempo_conf,
            phase_conf: tempo_conf,
            bar_conf: if self.has_clock { self.bar_conf.min(1.0) * fade.max(0.5) } else { 0.0 },
            locked,
        }
    }

    fn estimate(&mut self, t: f64) {
        if self.onsets.len() < 5 || self.fresh == 0 {
            return;
        }
        self.fresh = 0;
        let total_w: f64 = self.onsets.iter().map(|o| o.w).sum();
        let conc = |period: f64| -> (f64, f64) {
            let (mut re, mut im) = (0.0, 0.0);
            for o in &self.onsets {
                let a = std::f64::consts::TAU * (o.t / period);
                re += o.w * a.cos();
                im += o.w * a.sin();
            }
            ((re * re + im * im).sqrt() / total_w, im.atan2(re))
        };
        // Coarse sweep, then refine around the best.
        let mut best = (0.0f64, 0.0f64, 0.0f64); // (conc, period, angle)
        let mut scores: Vec<(f64, f64, f64)> = Vec::new();
        let mut bpm = MIN_BPM;
        while bpm <= MAX_BPM {
            let p = 60.0 / bpm;
            let (c, a) = conc(p);
            scores.push((c, p, a));
            if c > best.0 {
                best = (c, p, a);
            }
            bpm += 0.25;
        }
        if best.0 < 0.5 {
            return;
        }
        // Several periods that divide each other (every beat, every two beats)
        // score alike. Prefer the one the hint points at, else the slowest
        // that is nearly as tight.
        let near: Vec<&(f64, f64, f64)> = (0..scores.len())
            .filter(|&i| {
                let c = scores[i].0;
                c >= best.0 * 0.93 && (i == 0 || c >= scores[i - 1].0) && (i + 1 == scores.len() || c >= scores[i + 1].0)
            })
            .map(|i| &scores[i])
            .collect();
        let pick = match self.hint {
            Some(h) => near
                .iter()
                .min_by(|a, b| {
                    let da = ((60.0 / a.1) / h).ln().abs().min(((60.0 / a.1) / (2.0 * h)).ln().abs()).min(((60.0 / a.1) * 2.0 / h).ln().abs());
                    let db = ((60.0 / b.1) / h).ln().abs().min(((60.0 / b.1) / (2.0 * h)).ln().abs()).min(((60.0 / b.1) * 2.0 / h).ln().abs());
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied()
                .copied()
                .unwrap_or(best),
            None => near.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).copied().copied().unwrap_or(best),
        };
        // The nearest local peak around that period, finely.
        let mut fine = pick;
        let mut bpm = 60.0 / pick.1 - 0.4;
        while bpm <= 60.0 / pick.1 + 0.4 {
            let p = 60.0 / bpm;
            let (c, a) = conc(p);
            if c > fine.0 {
                fine = (c, p, a);
            }
            bpm += 0.02;
        }
        // Honour the hint's octave: use the hint if it is that tempo's half or double.
        let (c, period, angle) = fine;
        let enough = (self.onsets.len() as f64 / 8.0).min(1.0);
        let conf = (((c - 0.5) / 0.45).clamp(0.0, 1.0) * enough) as f32;

        // Where the beat falls: the grid time is angle/2π of a period.
        let grid0 = angle / std::f64::consts::TAU * period;
        if !self.has_clock {
            self.period = period;
            self.t_ref = t;
            // Count from zero at the first grid line before now.
            let frac = ((t - grid0) / period).rem_euclid(1.0);
            self.beat_ref = frac;
            self.has_clock = true;
            self.pending = 0;
            self.last_good = t;
            self.last_conf = conf;
            return;
        }
        // Compare with the running clock.
        let b_old = self.beat_at(t);
        let frac_new = ((t - grid0) / period).rem_euclid(1.0);
        let b_new = (b_old - frac_new).round() + frac_new;
        let err = b_new - b_old; // within half a beat
        let tempo_off = (period / self.period - 1.0).abs();
        let lost = !(t - self.last_good < 8.0 && self.last_conf > 0.45);
        let confident = conf > 0.55;
        if confident {
            self.last_good = t;
            self.last_conf = conf;
        } else if conf > 0.3 {
            self.last_conf = self.last_conf.max(conf * 0.9);
        }
        if !confident && !lost {
            return;
        }
        let big = err.abs() > 0.12 || tempo_off > 0.015;
        let (gain_phase, gain_tempo) = if lost {
            self.pending = 0;
            (1.0, 1.0)
        } else if big {
            self.pending += 1;
            if self.pending >= 3 {
                self.pending = 0;
                (1.0, 1.0)
            } else {
                return;
            }
        } else {
            self.pending = 0;
            (0.4, 0.25)
        };
        let new_period = self.period + gain_tempo * (period - self.period);
        // Re-anchor at now so the beat stays continuous apart from the correction.
        self.beat_ref = b_old + gain_phase * err;
        self.t_ref = t;
        self.period = new_period;
    }

    /// Re-learn which beat is the "1" from where the strong kicks fell. Called
    /// from `update`'s caller on bar boundaries is not needed: this is cheap.
    pub fn resolve_bar(&mut self) {
        if self.anchored {
            return;
        }
        let k = self.beats_per_bar.clamp(2, 12) as usize;
        let total: f64 = self.acc[..k].iter().sum();
        if total < 8.0 {
            return;
        }
        let (mut bi, mut bv) = (0usize, 0.0);
        for (i, v) in self.acc[..k].iter().enumerate() {
            if *v > bv {
                (bi, bv) = (i, *v);
            }
        }
        let mean = total / k as f64;
        let contrast = ((bv / mean - 1.0) / 0.6).clamp(0.0, 1.0) as f32;
        if contrast > 0.25 {
            self.bar_offset = bi as i64;
        }
        self.bar_conf = contrast * 0.8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A track at `bpm`: a kick on every beat (stronger on the one), snares on
    /// 2 and 4, for `beats` beats starting at `t0`, with `jitter` seconds of
    /// deterministic wobble.
    fn track(tr: &mut BeatTracker, t0: f64, bpm: f64, beats: u32, jitter: f64, first: u32) -> f64 {
        let spb = 60.0 / bpm;
        let mut t = t0;
        for b in 0..beats {
            let wob = ((b * 7919 % 13) as f64 / 13.0 - 0.5) * 2.0 * jitter;
            tr.onset(t + wob, if (b + first) % 4 == 0 { 1.6 } else { 0.6 }, false);
            if (b + first) % 2 == 1 {
                tr.onset(t + wob, 0.5, true);
            }
            // Tick at 44 Hz through the beat.
            let end = t + spb;
            let mut x = t;
            while x < end {
                tr.update(x);
                x += 1.0 / 44.0;
            }
            if b % 4 == 3 {
                tr.resolve_bar();
            }
            t = end;
        }
        t
    }

    #[test]
    fn finds_the_tempo_within_one_percent() {
        for bpm in [92.0, 120.0, 128.0, 140.0, 150.0, 174.0] {
            let mut tr = BeatTracker::new();
            let end = track(&mut tr, 0.0, bpm, 64, 0.008, 0);
            let r = tr.update(end);
            assert!((r.tempo / bpm - 1.0).abs() < 0.01, "{bpm}: got {}", r.tempo);
            assert!(r.locked, "{bpm}: {r:?}");
        }
    }

    #[test]
    fn the_beat_lands_on_the_kicks() {
        let mut tr = BeatTracker::new();
        let end = track(&mut tr, 0.0, 128.0, 64, 0.005, 0);
        let spb = 60.0 / 128.0;
        // The next kick would be at `end`; the beat there must be a whole number within 30 ms.
        let b = tr.beat_at(end);
        let off = (b - b.round()).abs() * spb;
        assert!(off < 0.03, "off by {off} s");
    }

    #[test]
    fn keeps_counting_through_a_sixteen_bar_breakdown_and_relocks() {
        let mut tr = BeatTracker::new();
        let spb = 60.0 / 126.0;
        let end = track(&mut tr, 0.0, 126.0, 48, 0.004, 0);
        let b_before = tr.beat_at(end);
        // 16 bars with no kicks: just keep ticking.
        let gap_end = end + 64.0 * spb;
        let mut t = end;
        while t < gap_end {
            tr.update(t);
            t += 1.0 / 44.0;
        }
        // The clock kept the beat to within a tenth of a beat.
        let expect = b_before + 64.0;
        let got = tr.beat_at(gap_end);
        assert!((got - expect).abs() < 0.1, "drifted {:.3} beats", got - expect);
        // Kicks come back on the same grid: locked again within two bars (8 beats).
        let end2 = track(&mut tr, gap_end, 126.0, 8, 0.004, 0);
        let r = tr.update(end2);
        assert!(r.locked, "{r:?}");
        assert!((r.tempo / 126.0 - 1.0).abs() < 0.01);
    }

    #[test]
    fn follows_a_new_tempo_after_a_mix() {
        let mut tr = BeatTracker::new();
        let t = track(&mut tr, 0.0, 124.0, 48, 0.004, 0);
        let end = track(&mut tr, t, 132.0, 96, 0.004, 0);
        let r = tr.update(end);
        assert!((r.tempo / 132.0 - 1.0).abs() < 0.01, "{}", r.tempo);
    }

    #[test]
    fn a_hint_chooses_between_a_tempo_and_its_double() {
        // Kicks on beats 1 and 3 only: 87 BPM and 174 BPM fit equally well.
        let two_per_bar = |hint: Option<f64>| {
            let mut tr = BeatTracker::new();
            tr.hint_tempo(hint);
            let spb = 60.0 / 174.0;
            for b in 0..140u32 {
                let t = b as f64 * spb;
                if b % 2 == 0 {
                    tr.onset(t, 1.0, false);
                }
                tr.update(t);
            }
            tr.update(140.0 * spb).tempo
        };
        assert!((two_per_bar(Some(174.0)) / 174.0 - 1.0).abs() < 0.01);
        assert!((two_per_bar(None) / 174.0 - 1.0).abs() < 0.01, "default range keeps 174, not 87");
    }

    #[test]
    fn finds_the_one_from_the_strong_kicks_and_from_an_anchor() {
        let mut tr = BeatTracker::new();
        // Kick strength is highest on every fourth beat, offset by 2 from beat 0.
        let end = track(&mut tr, 0.0, 128.0, 96, 0.003, 2);
        tr.resolve_bar();
        let r = tr.update(end);
        // The beat the strong kicks fall on must be bar position 0.
        let mut strong = None;
        for b in 0..4 {
            let probe = r.beat.floor() - 8.0 + b as f64;
            let pos = (probe - (r.beat - r.bar_pos).round()).rem_euclid(4.0);
            if pos == 0.0 {
                strong = Some(probe);
            }
        }
        assert!(strong.is_some());
        assert!(r.bar_conf > 0.2, "{r:?}");
        // An anchor says: this beat is the one.
        tr.anchor_bar(end);
        let after = tr.update(end);
        assert!(after.bar_pos < 0.2 || after.bar_pos > 3.8, "{after:?}");
        assert!(after.bar_conf > 0.9);
    }

    #[test]
    fn is_deterministic() {
        let run = || {
            let mut tr = BeatTracker::new();
            let end = track(&mut tr, 0.0, 133.0, 80, 0.01, 1);
            tr.update(end)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn silence_and_noise_never_panic_or_lock() {
        let mut tr = BeatTracker::new();
        for i in 0..2000 {
            tr.update(i as f64 / 44.0);
        }
        assert!(!tr.update(50.0).locked);
        // Random-ish hits give no confident beat.
        for i in 0..60u32 {
            tr.onset(60.0 + (i * i % 97) as f64 * 0.13, 1.0, false);
        }
        let r = tr.update(70.0);
        assert!(r.tempo_conf < 0.9, "{r:?}");
    }

    #[test]
    fn one_update_fits_the_tick_budget() {
        // The tracker re-estimates twice a second inside the engine tick (5 ms
        // budget for everything). Worst case: a busy hi-energy track, 12 s of hits.
        let mut tr = BeatTracker::new();
        for i in 0..120 {
            tr.onset(i as f64 * 0.1, 1.0, i % 2 == 1);
        }
        tr.update(12.0); // first estimate
        for i in 0..120 {
            tr.onset(12.0 + i as f64 * 0.1, 1.0, false);
        }
        let start = std::time::Instant::now();
        tr.update(24.0);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        // Debug builds (cargo test) run this code ~20x slower than release.
        let limit = if cfg!(debug_assertions) { 100.0 } else { 3.0 };
        assert!(ms < limit, "{ms} ms");
    }
}
