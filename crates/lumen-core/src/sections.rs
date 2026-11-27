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

/// How a genre's sections are judged. Everything that used to be a fixed
/// number in the detector lives here, so each style can be tuned on its own.
///
/// Kick and snare counts are per *felt* bar. Normally that is `beats_per_bar`
/// beats. With `half_time` the groove is felt at half the tempo (trap and
/// half-time hip-hop: the snare lands on beat 3), so the felt bar is twice as
/// many beats and the counts are taken over that longer window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenreProfile {
    /// Beats in a bar (4 for most dance music, 3 for waltz-time).
    pub beats_per_bar: u32,
    /// The groove is felt at half the tempo: judge over two bars of beats.
    pub half_time: bool,
    /// Kicks in the last felt bar for the track to count as in its groove.
    pub groove_kicks: u32,
    /// At most this many kicks in the last felt bar counts as "no kick" (melodic).
    pub melodic_kicks: u32,
    /// Beats a new section has to hold before Kickline believes it.
    pub settle_beats: u32,
    /// Snares in a felt bar that make a roll (a sign of a build-up).
    pub snare_roll: u32,
    /// Bass compared with the loudest recent bass needed for a drop (0..1).
    pub drop_bass: f32,
    /// Loudness compared with the loudest recent part needed for a drop (0..1).
    pub drop_level: f32,
    /// Rise of the highs over a felt bar that marks a build-up (1 = no change).
    pub build_rise: f32,
}

/// The old name, from before profiles carried more than kick counts.
pub type SectionTuning = GenreProfile;

impl Default for GenreProfile {
    /// The original behaviour: four beats, 3 kicks a bar is a groove.
    fn default() -> Self {
        GenreProfile {
            beats_per_bar: 4,
            half_time: false,
            groove_kicks: 3,
            melodic_kicks: 1,
            settle_beats: 4,
            snare_roll: 6,
            drop_bass: 0.72,
            drop_level: 0.6,
            build_rise: 1.2,
        }
    }
}

impl GenreProfile {
    /// Beats in one felt bar: the window every count is taken over.
    pub fn bar_beats(&self) -> u32 {
        self.beats_per_bar.clamp(2, 12) * if self.half_time { 2 } else { 1 }
    }

    /// The same profile with every number put in a sane range.
    pub fn sanitized(mut self) -> Self {
        self.beats_per_bar = self.beats_per_bar.clamp(2, 12);
        self.settle_beats = self.settle_beats.clamp(1, 64);
        self.groove_kicks = self.groove_kicks.max(1);
        self.melodic_kicks = self.melodic_kicks.min(self.groove_kicks - 1);
        self.snare_roll = self.snare_roll.max(1);
        self.drop_bass = self.drop_bass.clamp(0.1, 1.5);
        self.drop_level = self.drop_level.clamp(0.1, 1.5);
        self.build_rise = self.build_rise.clamp(1.02, 3.0);
        self
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
    pub profile: GenreProfile,
    /// Sections heard so far, oldest first, with their length in beats.
    pub past: VecDeque<(Section, u32)>,
    /// A section the DJ is calling by hand (the hold-to-drop button): what it
    /// is and how many beats it still lasts.
    forced: Option<(Section, u32)>,
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
            profile: GenreProfile::default(),
            past: VecDeque::new(),
            forced: None,
        }
    }
}

impl SectionDetector {
    /// Judge music the way this genre profile says (deterministic: the same
    /// audio always gives the same sections).
    pub fn set_profile(&mut self, profile: GenreProfile) {
        self.profile = profile.sanitized();
    }

    pub fn current(&self) -> Section {
        self.forced.map_or(self.current, |(s, _)| s)
    }

    /// True while the DJ's own call (build or drop) overrides the listening.
    pub fn is_forced(&self) -> bool {
        self.forced.is_some()
    }

    /// Call a section by hand for `beats` beats (`u32::MAX` until released).
    pub fn force(&mut self, section: Section, beats: u32) {
        self.forced = Some((section, beats));
    }

    /// Hand the call back to the listening.
    pub fn release_force(&mut self) {
        self.forced = None;
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
        if let Some((sec, left)) = self.forced {
            if left != u32::MAX {
                self.forced = (left > 1).then_some((sec, left - 1));
            }
        }
        let n = self.acc_n.max(1) as f32;
        let sample = BeatSample { kicks: self.acc.kicks, snares: self.acc.snares, low: self.acc.low / n, high: self.acc.high / n, level: self.acc.level / n };
        self.acc = BeatSample::default();
        self.acc_n = 0;
        self.history.push_back(sample);
        let keep = (2 * self.profile.bar_beats() as usize).max(16);
        while self.history.len() > keep {
            self.history.pop_front();
        }
        self.beats_in += 1;
        // The loudest recent part is the yardstick (forgotten over ~64 bars).
        self.ref_low = (self.ref_low * 0.996).max(sample.low);
        self.ref_level = (self.ref_level * 0.996).max(sample.level);
        let guess = self.classify();
        let changed = self.decide(guess);
        let pending = if self.candidate.is_some() { 0.6 } else { 1.0 };
        let settled = (self.beats_in as f32 / self.profile.settle_beats.max(1) as f32).clamp(0.0, 1.0);
        self.reading.confidence = ((0.25 + 0.75 * settled) * pending).clamp(0.0, 1.0);
        changed
    }

    fn classify(&mut self) -> Section {
        let p = self.profile.sanitized();
        let w = p.bar_beats() as usize;
        let h = &self.history;
        let n = h.len();
        let bar: Vec<&BeatSample> = h.iter().skip(n.saturating_sub(w)).collect();
        let last = *h.back().unwrap_or(&BeatSample::default());
        let kicks: u32 = bar.iter().map(|b| b.kicks).sum();
        let snares: u32 = bar.iter().map(|b| b.snares).sum();
        let avg = |f: fn(&BeatSample) -> f32, from: usize, to: usize| {
            let s: Vec<f32> = h.iter().skip(from).take(to - from).map(f).collect();
            if s.is_empty() { 0.0 } else { s.iter().sum::<f32>() / s.len() as f32 }
        };
        let low_bar = avg(|b| b.low, n.saturating_sub(w), n);
        let level_beat = last.level;
        let bass = low_bar / self.ref_low.max(1e-6);
        let level = level_beat / self.ref_level.max(1e-6);
        // Highs: the last bar against the bar before last.
        let high_now = avg(|b| b.high, n.saturating_sub(w), n);
        let high_before = avg(|b| b.high, n.saturating_sub(2 * w), n.saturating_sub(w));
        let rise = if high_before > 1e-6 { high_now / high_before } else { 1.0 };
        // Snare density rising (a roll speeding up).
        let snares_before: u32 = h.iter().skip(n.saturating_sub(2 * w)).take(w.min(n.saturating_sub(w))).map(|b| b.snares).sum();
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
        let building = (rise > p.build_rise && bass < 0.6)
            || (snares >= p.snare_roll && snares > snares_before)
            || (rise > p.build_rise + 0.15 && kicks < p.groove_kicks);
        if building {
            return Section::BuildUp;
        }
        if kicks >= p.groove_kicks {
            return if bass >= p.drop_bass && level >= p.drop_level { Section::Drop } else { Section::Groove };
        }
        if kicks <= p.melodic_kicks {
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
        if instant || held >= self.profile.settle_beats.max(1) {
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
        let tuned = GenreProfile { groove_kicks: 2, melodic_kicks: 0, ..GenreProfile::default() };
        let run = |profile: GenreProfile| {
            let mut d = SectionDetector { profile, ..Default::default() };
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
        assert_ne!(run(GenreProfile::default()).1, Section::Groove);
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

    /// Run a loud part, a breakdown and a groove built from `kick`/`snare`
    /// patterns (functions of the beat number) under a profile.
    fn three_parts(profile: GenreProfile, kick: fn(u32) -> u32, snare: fn(u32) -> u32) -> (Section, Section) {
        let mut d = SectionDetector::default();
        d.set_profile(profile);
        let mut a = AudioBands::default();
        let mut beat = 0.0;
        play(&mut d, &mut a, &mut beat, 32, |a, b| beat_bands(kick(b), snare(b), 1.0, 0.5, 1.0)(a));
        play(&mut d, &mut a, &mut beat, 32, |a, _| beat_bands(0, 0, 0.12, 0.25, 0.4)(a));
        let breakdown = d.current();
        play(&mut d, &mut a, &mut beat, 32, |a, b| beat_bands(kick(b), snare(b), 0.5, 0.35, 0.65)(a));
        (breakdown, d.current())
    }

    #[test]
    fn default_profile_is_the_original_behaviour() {
        let p = GenreProfile::default();
        assert_eq!((p.beats_per_bar, p.half_time, p.groove_kicks, p.melodic_kicks, p.settle_beats), (4, false, 3, 1, 4));
        assert_eq!(p.bar_beats(), 4);
        assert_eq!(SectionDetector::default().profile, p);
    }

    #[test]
    fn half_time_hip_hop_reads_a_sparse_groove() {
        // Kick on beats 1 and 3 of every 8, snare on beat 5: one felt bar is 8 beats.
        let kick: fn(u32) -> u32 = |b| (b % 8 == 0 || b % 8 == 3) as u32;
        let snare: fn(u32) -> u32 = |b| (b % 8 == 4) as u32;
        let hip_hop = GenreProfile { half_time: true, groove_kicks: 2, melodic_kicks: 0, settle_beats: 8, snare_roll: 12, ..Default::default() };
        assert_eq!(hip_hop.bar_beats(), 8);
        assert_eq!(three_parts(hip_hop, kick, snare), (Section::Melodic, Section::Groove));
    }

    #[test]
    fn three_beat_bars_are_supported() {
        let waltz = GenreProfile { beats_per_bar: 3, groove_kicks: 1, melodic_kicks: 0, ..Default::default() };
        assert_eq!(waltz.bar_beats(), 3);
        // Kick on the one only.
        let r = three_parts(waltz, |b| (b % 3 == 0) as u32, |b| (b % 3 == 2) as u32);
        assert_eq!(r, (Section::Melodic, Section::Groove));
    }

    #[test]
    fn slower_settle_waits_longer_to_change() {
        let first_change = |settle: u32| {
            let mut d = SectionDetector::default();
            d.set_profile(GenreProfile { settle_beats: settle, ..Default::default() });
            let mut a = AudioBands::default();
            let mut beat = 0.0;
            play(&mut d, &mut a, &mut beat, 24, |a, _| beat_bands(1, 0, 0.5, 0.3, 0.6)(a));
            let start = beat;
            let ch = play(&mut d, &mut a, &mut beat, 32, |a, _| beat_bands(0, 0, 0.12, 0.25, 0.4)(a));
            ch.first().map(|c| c.0 - start).expect("melodic never arrived")
        };
        assert!(first_change(8) > first_change(2) + 3.0);
    }

    #[test]
    fn every_genre_profile_is_sane_and_confidence_stays_in_range() {
        use crate::director::Genre;
        for g in Genre::ALL {
            let p = g.profile();
            assert_eq!(p, p.sanitized(), "{}", g.name());
            let mut d = SectionDetector::default();
            d.set_profile(p);
            let (mut a, mut beat) = (AudioBands::default(), 0.0);
            play(&mut d, &mut a, &mut beat, 64, |a, b| beat_bands((b % 2 == 0) as u32, (b % 4 == 2) as u32, 0.6, 0.3, 0.7)(a));
            assert!((0.0..=1.0).contains(&d.reading.confidence), "{} {}", g.name(), d.reading.confidence);
        }
        // Nonsense settings are tamed rather than crashing.
        let wild = GenreProfile { beats_per_bar: 0, settle_beats: 0, groove_kicks: 0, melodic_kicks: 9, ..Default::default() }.sanitized();
        assert!(wild.beats_per_bar >= 2 && wild.settle_beats >= 1 && wild.melodic_kicks < wild.groove_kicks);
    }

    #[test]
    fn same_audio_gives_the_same_sections() {
        let run = || {
            let mut d = SectionDetector::default();
            d.set_profile(GenreProfile { half_time: true, groove_kicks: 2, ..Default::default() });
            let (mut a, mut beat) = (AudioBands::default(), 0.0);
            play(&mut d, &mut a, &mut beat, 96, |a, b| beat_bands((b % 5 == 0) as u32, (b % 3 == 0) as u32, 0.4 + (b % 7) as f32 * 0.05, 0.3, 0.6)(a))
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_called_section_overrides_the_listening_until_released_or_timed_out() {
        let mut d = SectionDetector::default();
        let heard = d.current();
        d.force(Section::BuildUp, u32::MAX);
        assert!(d.is_forced());
        assert_eq!(d.current(), Section::BuildUp);
        let a = AudioBands::default();
        for b in 0..40 {
            d.update(&a, b as f64 + 0.5);
        }
        assert_eq!(d.current(), Section::BuildUp, "a held build never times out");
        d.force(Section::Drop, 3);
        for b in 40..43 {
            d.update(&a, b as f64 + 0.5);
        }
        assert!(!d.is_forced(), "a called drop lasts its beats, then hands back");
        assert_ne!(d.current(), Section::Drop);
        let _ = heard;
        d.force(Section::Drop, 100);
        d.release_force();
        assert!(!d.is_forced());
    }
}
