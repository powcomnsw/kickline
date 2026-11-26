//! Song sections, worked out from the music as it plays.
//!
//! Kickline can't see the track ahead (rekordbox doesn't share its analysis
//! with other apps), so this listens: it keeps a few bars of history of
//! kick count, bass, highs, snares and loudness, and decides on every beat
//! whether the track is in its groove, a melodic breakdown, a build-up, the
//! pause before the drop, or the drop. It reacts within about a bar, except
//! for the pause and the drop, which it catches on the beat.
//!
//! Each section can have a "look": which clip every layer plays, which
//! palette, and how bright. With auto looks on, Kickline switches looks on
//! the beat when the section changes.

use crate::audio::{AudioBand, AudioBands};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Section {
    /// Kick running, not at full power: the body of the track.
    Groove,
    /// No kick, music still playing: breakdown, vocals, pads.
    Melodic,
    /// Energy and highs rising, snare rolls, bass pulled out.
    BuildUp,
    /// The moment of near-silence right before the drop.
    Pause,
    /// Kick and full bass back at full power.
    Drop,
}

impl Section {
    pub const ALL: [Section; 5] = [Section::Groove, Section::Melodic, Section::BuildUp, Section::Pause, Section::Drop];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Section::Groove => "Groove",
            Section::Melodic => "Melodic",
            Section::BuildUp => "Build-up",
            Section::Pause => "Pause",
            Section::Drop => "Drop",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            Section::Groove => "Kick running, not at full power",
            Section::Melodic => "No kick: breakdown, vocals, pads",
            Section::BuildUp => "Rising energy, snare rolls, bass pulled out",
            Section::Pause => "The silence just before the drop",
            Section::Drop => "Kick and full bass back",
        }
    }

    /// Display colour.
    pub fn color(self) -> [f32; 3] {
        match self {
            Section::Groove => [0.35, 0.65, 1.0],
            Section::Melodic => [0.6, 0.45, 1.0],
            Section::BuildUp => [1.0, 0.75, 0.2],
            Section::Pause => [0.75, 0.75, 0.75],
            Section::Drop => [1.0, 0.3, 0.25],
        }
    }
}

/// What was heard during one beat.
#[derive(Clone, Copy, Debug, Default)]
struct BeatSample {
    kicks: u32,
    snares: u32,
    low: f32,
    high: f32,
    level: f32,
}

/// The numbers behind the decision, for the UI.
#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    /// Kicks in the last bar.
    pub kicks: u32,
    /// Snares / claps in the last bar.
    pub snares: u32,
    /// Bass now compared with the loudest recent bass (0..1+).
    pub bass: f32,
    /// Loudness now compared with the loudest recent part (0..1+).
    pub level: f32,
    /// How much the highs rose over the last two bars (1 = no change).
    pub rise: f32,
    /// How sure Kickline is of the current section (0..1): low right after a
    /// change or while a different section is being weighed up.
    pub confidence: f32,
}

/// What counts as "kick running" in a genre. Four-on-the-floor music has 4
/// kicks a bar; drum & bass and hip-hop have 2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionTuning {
    /// Kicks in the last bar for the track to count as in its groove.
    pub groove_kicks: u32,
    /// At most this many kicks in the last bar counts as "no kick" (melodic).
    pub melodic_kicks: u32,
    /// Beats a new section has to hold before Kickline believes it.
    pub settle_beats: u32,
}

impl Default for SectionTuning {
    fn default() -> Self {
        SectionTuning { groove_kicks: 3, melodic_kicks: 1, settle_beats: 4 }
    }
}

#[derive(Clone, Debug)]
pub struct SectionDetector {
    current: Section,
    candidate: Option<(Section, u32)>,
    history: VecDeque<BeatSample>,
    acc: BeatSample,
    acc_n: u32,
    beat_index: Option<i64>,
    seen: Option<(u64, u64)>,
    ref_low: f32,
    ref_level: f32,
    /// Beats since the section started.
    pub beats_in: u32,
    pub reading: Reading,
    pub tuning: SectionTuning,
    /// Sections heard so far, oldest first, with their length in beats.
    pub past: VecDeque<(Section, u32)>,
}

impl Default for SectionDetector {
    fn default() -> Self {
        SectionDetector {
            current: Section::Groove,
            candidate: None,
            history: VecDeque::new(),
            acc: BeatSample::default(),
            acc_n: 0,
            beat_index: None,
            seen: None,
            ref_low: 0.0,
            ref_level: 0.0,
            beats_in: 0,
            reading: Reading::default(),
            tuning: SectionTuning::default(),
            past: VecDeque::new(),
        }
    }
}

impl SectionDetector {
    pub fn current(&self) -> Section {
        self.current
    }

    /// True once enough music has been heard to judge.
    pub fn listening(&self) -> bool {
        self.ref_level > 1e-4 && self.history.len() >= 4
    }

    /// Feed the latest analysis. Returns the new section when it changes
    /// (only ever on a beat).
    pub fn update(&mut self, a: &AudioBands, beat: f64) -> Option<Section> {
        let (kicks, snares) = (a.hits[0], a.hits[1]);
        if let Some((k, s)) = self.seen {
            self.acc.kicks += kicks.saturating_sub(k) as u32;
            self.acc.snares += snares.saturating_sub(s) as u32;
        }
        self.seen = Some((kicks, snares));
        let raw = |b: AudioBand| a.raw.get(b.index()).copied().unwrap_or(0.0);
        self.acc.low += raw(AudioBand::Sub) + raw(AudioBand::Bass);
        self.acc.high += raw(AudioBand::High) + raw(AudioBand::HighMid) * 0.5;
        self.acc.level += raw(AudioBand::Level);
        self.acc_n += 1;

        let idx = beat.floor() as i64;
        let prev = self.beat_index.replace(idx);
        if prev.is_none_or(|p| p == idx) {
            return None;
        }
        // A beat has ended.
        let n = self.acc_n.max(1) as f32;
        let sample = BeatSample { kicks: self.acc.kicks, snares: self.acc.snares, low: self.acc.low / n, high: self.acc.high / n, level: self.acc.level / n };
        self.acc = BeatSample::default();
        self.acc_n = 0;
        self.history.push_back(sample);
        while self.history.len() > 16 {
            self.history.pop_front();
        }
        self.beats_in += 1;
        // The loudest recent part is the yardstick (forgotten over ~64 bars).
        self.ref_low = (self.ref_low * 0.996).max(sample.low);
        self.ref_level = (self.ref_level * 0.996).max(sample.level);
        let guess = self.classify();
        let changed = self.decide(guess);
        let pending = if self.candidate.is_some() { 0.6 } else { 1.0 };
        let settled = (self.beats_in as f32 / self.tuning.settle_beats.max(1) as f32).min(1.0);
        self.reading.confidence = (0.25 + 0.75 * settled) * pending;
        changed
    }

    fn classify(&mut self) -> Section {
        let h = &self.history;
        let n = h.len();
        let bar: Vec<&BeatSample> = h.iter().skip(n.saturating_sub(4)).collect();
        let last = *h.back().unwrap_or(&BeatSample::default());
        let kicks: u32 = bar.iter().map(|b| b.kicks).sum();
        let snares: u32 = bar.iter().map(|b| b.snares).sum();
        let avg = |f: fn(&BeatSample) -> f32, from: usize, to: usize| {
            let s: Vec<f32> = h.iter().skip(from).take(to - from).map(f).collect();
            if s.is_empty() { 0.0 } else { s.iter().sum::<f32>() / s.len() as f32 }
        };
        let low_bar = avg(|b| b.low, n.saturating_sub(4), n);
        let level_beat = last.level;
        let bass = low_bar / self.ref_low.max(1e-6);
        let level = level_beat / self.ref_level.max(1e-6);
        // Highs: the last bar against the bar before last.
        let high_now = avg(|b| b.high, n.saturating_sub(4), n);
        let high_before = avg(|b| b.high, n.saturating_sub(8), n.saturating_sub(4));
        let rise = if high_before > 1e-6 { high_now / high_before } else { 1.0 };
        // Snare density rising (a roll speeding up).
        let snares_before: u32 = h.iter().skip(n.saturating_sub(8)).take(4.min(n.saturating_sub(4))).map(|b| b.snares).sum();
        self.reading = Reading { kicks, snares, bass, level, rise, confidence: 0.0 };

        let prev_level = if n >= 2 { h[n - 2].level / self.ref_level.max(1e-6) } else { 0.0 };
        // A sudden drop to near-silence (a release tail can keep it above zero).
        // (A quiet start to a build after a breakdown is not a pause.)
        let loud_before = matches!(self.current, Section::BuildUp | Section::Drop | Section::Groove);
        if level < 0.25 && prev_level > 0.6 && loud_before {
            return Section::Pause;
        }
        if level < 0.06 || (self.current == Section::Pause && level < 0.15) {
            return Section::Pause;
        }
        let kick_last = last.kicks > 0;
        let bass_now = last.low / self.ref_low.max(1e-6);
        if matches!(self.current, Section::BuildUp | Section::Pause) && kick_last && bass_now > 0.55 {
            return Section::Drop;
        }
        let building = (rise > 1.2 && bass < 0.6) || (snares >= 6 && snares > snares_before) || (rise > 1.35 && kicks <= 2);
        if building {
            return Section::BuildUp;
        }
        if kicks >= self.tuning.groove_kicks {
            return if bass >= 0.72 && level >= 0.6 { Section::Drop } else { Section::Groove };
        }
        if kicks <= self.tuning.melodic_kicks {
            return Section::Melodic;
        }
        self.current
    }

    fn decide(&mut self, guess: Section) -> Option<Section> {
        if guess == self.current {
            self.candidate = None;
            return None;
        }
        // The pause and the drop land at once; anything else has to hold for a bar.
        let instant = matches!(guess, Section::Pause)
            || (guess == Section::Drop && matches!(self.current, Section::BuildUp | Section::Pause))
            || (self.current == Section::Pause && guess != Section::Pause);
        let held = match self.candidate {
            Some((s, k)) if s == guess => k + 1,
            _ => 1,
        };
        if instant || held >= self.tuning.settle_beats.max(1) {
            self.past.push_back((self.current, self.beats_in));
            while self.past.len() > 24 {
                self.past.pop_front();
            }
            self.current = guess;
            self.candidate = None;
            self.beats_in = 0;
            Some(guess)
        } else {
            self.candidate = Some((guess, held));
            None
        }
    }
}

/// What one layer does in a section's look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerLook {
    /// Leave it as it is.
    #[default]
    Keep,
    /// Play this clip (column).
    Play(usize),
    /// Stop the layer.
    Stop,
}

/// Per-section looks for the whole show (the layers keep their own part).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Looks {
    /// Switch looks automatically when the section changes.
    pub auto: bool,
    /// Palette for each section (None = keep).
    pub palette: [Option<usize>; 5],
    /// Brightness for each section, multiplied with the master.
    pub level: [f32; 5],
}

impl Default for Looks {
    fn default() -> Self {
        Looks { auto: false, palette: [None; 5], level: [0.8, 0.6, 0.85, 0.15, 1.0] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up track, beat by beat: what the analyser would report.
    fn beat_bands(kicks: u32, snares: u32, low: f32, high: f32, level: f32) -> impl Fn(&mut AudioBands) {
        move |a: &mut AudioBands| {
            a.hits[0] += kicks as u64;
            a.hits[1] += snares as u64;
            a.raw[AudioBand::Sub.index()] = low * 0.4;
            a.raw[AudioBand::Bass.index()] = low * 0.6;
            a.raw[AudioBand::High.index()] = high;
            a.raw[AudioBand::HighMid.index()] = 0.0;
            a.raw[AudioBand::Level.index()] = level;
        }
    }

    /// Run `beats` beats of a part through the detector (20 updates per beat)
    /// and return (beat, section) for every change.
    fn play(d: &mut SectionDetector, a: &mut AudioBands, beat: &mut f64, beats: u32, part: impl Fn(&mut AudioBands, u32)) -> Vec<(f64, Section)> {
        let mut out = Vec::new();
        for b in 0..beats {
            for k in 0..20 {
                if k == 0 {
                    part(a, b);
                }
                if let Some(s) = d.update(a, *beat) {
                    out.push((*beat, s));
                }
                *beat += 0.05;
            }
        }
        out
    }

    #[test]
    fn follows_a_track_through_its_sections() {
        let mut d = SectionDetector::default();
        let mut a = AudioBands::default();
        let mut beat = 0.0;
        // 16 bars of drop-level intro to learn what loud is, then the groove.
        let drop = |a: &mut AudioBands, _| beat_bands(1, 0, 1.0, 0.5, 1.0)(a);
        play(&mut d, &mut a, &mut beat, 32, drop);
        assert_eq!(d.current(), Section::Drop);
        let groove = |a: &mut AudioBands, _| beat_bands(1, 0, 0.5, 0.35, 0.65)(a);
        let ch = play(&mut d, &mut a, &mut beat, 32, groove);
        assert_eq!(d.current(), Section::Groove, "{ch:?}");
        // Breakdown: no kick, pads.
        let pads = |a: &mut AudioBands, b: u32| beat_bands(0, (b % 4 == 2) as u32, 0.12, 0.25, 0.4)(a);
        let ch = play(&mut d, &mut a, &mut beat, 32, pads);
        assert_eq!(d.current(), Section::Melodic, "{ch:?}");
        // Build: highs climb, snare roll speeds up.
        let build = |a: &mut AudioBands, b: u32| {
            let k = b as f32 / 32.0;
            beat_bands(0, 1 + b / 6, 0.1, 0.25 + 0.6 * k, 0.45 + 0.4 * k)(a)
        };
        let ch = play(&mut d, &mut a, &mut beat, 32, build);
        assert_eq!(d.current(), Section::BuildUp, "{ch:?}");
        // One beat of silence…
        let start = beat;
        let ch = play(&mut d, &mut a, &mut beat, 2, |a, _| beat_bands(0, 0, 0.01, 0.01, 0.01)(a));
        assert_eq!(d.current(), Section::Pause, "{ch:?}");
        assert!(ch[0].0 - start < 1.5, "pause caught late: {ch:?}");
        // …and the drop lands on its first beat.
        let start = beat;
        let ch = play(&mut d, &mut a, &mut beat, 8, drop);
        assert_eq!(d.current(), Section::Drop, "{ch:?}");
        assert!(ch[0].0 - start < 1.5, "drop caught late: {ch:?}");
    }

    #[test]
    fn steady_groove_does_not_flicker() {
        let mut d = SectionDetector::default();
        let mut a = AudioBands::default();
        let mut beat = 0.0;
        // Loud reference first.
        play(&mut d, &mut a, &mut beat, 16, |a, _| beat_bands(1, 0, 1.0, 0.5, 1.0)(a));
        // A groove with the odd missed kick and a snare on 2 and 4.
        let ch = play(&mut d, &mut a, &mut beat, 64, |a, b| beat_bands((b % 7 != 3) as u32, (b % 2 == 1) as u32, 0.5, 0.3, 0.6)(a));
        assert_eq!(ch.len(), 1, "{ch:?}");
        assert_eq!(ch[0].1, Section::Groove);
    }

    #[test]
    fn silence_alone_is_a_pause() {
        let mut d = SectionDetector::default();
        let mut a = AudioBands::default();
        let mut beat = 0.0;
        play(&mut d, &mut a, &mut beat, 16, |a, _| beat_bands(1, 0, 1.0, 0.5, 1.0)(a));
        play(&mut d, &mut a, &mut beat, 2, |a, _| beat_bands(0, 0, 0.0, 0.0, 0.0)(a));
        assert_eq!(d.current(), Section::Pause);
    }

    #[test]
    fn drum_and_bass_has_two_kicks_a_bar() {
        let tuned = SectionTuning { groove_kicks: 2, melodic_kicks: 0, settle_beats: 4 };
        let run = |tuning: SectionTuning| {
            let mut d = SectionDetector { tuning, ..Default::default() };
            let mut a = AudioBands::default();
            let mut beat = 0.0;
            // Kick on 1 and 3 only. A loud part, a breakdown, then the groove.
            play(&mut d, &mut a, &mut beat, 32, |a, b| beat_bands((b % 2 == 0) as u32, (b % 2 == 1) as u32, 1.0, 0.5, 1.0)(a));
            play(&mut d, &mut a, &mut beat, 32, |a, _| beat_bands(0, 0, 0.12, 0.25, 0.4)(a));
            let breakdown = d.current();
            play(&mut d, &mut a, &mut beat, 32, |a, b| beat_bands((b % 2 == 0) as u32, (b % 2 == 1) as u32, 0.5, 0.35, 0.65)(a));
            (breakdown, d.current())
        };
        // With the genre's own counts the kicks bring the groove back.
        assert_eq!(run(tuned), (Section::Melodic, Section::Groove));
        // With four-on-the-floor counts two kicks a bar never counts as a groove.
        assert_ne!(run(SectionTuning::default()).1, Section::Groove);
    }

    #[test]
    fn confidence_grows_while_a_section_holds() {
        let mut d = SectionDetector::default();
        let mut a = AudioBands::default();
        let mut beat = 0.0;
        play(&mut d, &mut a, &mut beat, 24, |a, _| beat_bands(1, 0, 1.0, 0.5, 1.0)(a));
        assert!(d.reading.confidence > 0.9, "{:?}", d.reading);
        // Just after a change it is low.
        play(&mut d, &mut a, &mut beat, 2, |a, _| beat_bands(0, 0, 0.0, 0.0, 0.0)(a));
        assert!(d.reading.confidence < 0.6, "{:?}", d.reading);
    }
}
