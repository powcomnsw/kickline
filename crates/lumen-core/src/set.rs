//! Where we are in a phrase and in the set.
//!
//! Dance music moves in 8 and 16 bar phrases that start on a "1": a drop, or
//! the groove coming back. `Phrase` counts bars from the last such line, so
//! looks can change on phrase lines and a build-up knows how far along it is.
//! `SetMemory` counts what has happened so far (drops, build-ups, tracks) so
//! the second drop doesn't have to look like the first.

use crate::sections::Section;

/// Bars in a phrase.
pub const PHRASE_BARS: u32 = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phrase {
    anchor_bar: i64,
    /// The bar now, counted from the start of the clock.
    pub bar: i64,
    /// Bars since the last phrase line (0..PHRASE_BARS).
    pub bar_in_phrase: u32,
    /// Crossed onto a new bar this tick.
    pub new_bar: bool,
    /// Crossed onto a new phrase this tick.
    pub new_phrase: bool,
    /// Where in the bar we are (0..1), when known.
    pub in_bar: f32,
}

impl Phrase {
    /// Update from the analysis beat. `bar_pos` is the beats since the "1",
    /// if known; else bars are counted from beat zero.
    pub fn update(&mut self, beat: f64, bar_pos: Option<f64>, beats_per_bar: u32) {
        let k = beats_per_bar.clamp(2, 12) as f64;
        let start = beat - bar_pos.unwrap_or(beat.rem_euclid(k));
        let bar = (start / k).round() as i64;
        let prev = self.bar;
        self.new_bar = bar != prev;
        self.bar = bar;
        let before = self.bar_in_phrase;
        self.bar_in_phrase = (bar - self.anchor_bar).rem_euclid(PHRASE_BARS as i64) as u32;
        self.new_phrase = self.new_bar && self.bar_in_phrase < before;
        self.in_bar = (bar_pos.unwrap_or(beat.rem_euclid(k)) / k).clamp(0.0, 1.0) as f32;
    }

    /// A section that starts a phrase has begun here (a drop landed, the
    /// groove came back): count from this bar.
    pub fn anchor(&mut self) {
        self.anchor_bar = self.bar;
        self.bar_in_phrase = 0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SetMemory {
    pub drops: u32,
    pub builds: u32,
    pub breakdowns: u32,
    /// Tracks heard so far (starts at 1 once there is music).
    pub tracks: u32,
    tempo: f64,
    off_since: f64,
}

impl SetMemory {
    /// A section began.
    pub fn section(&mut self, s: Section) {
        match s {
            Section::Drop => self.drops += 1,
            Section::BuildUp => self.builds += 1,
            Section::Melodic => self.breakdowns += 1,
            _ => {}
        }
    }

    /// Watch the tempo: a different tempo (not half or double) held for a
    /// while is a new track in the mix.
    pub fn tempo(&mut self, t_now: f64, bpm: f64, confident: bool) {
        if !confident || bpm <= 0.0 {
            return;
        }
        if self.tracks == 0 || self.tempo <= 0.0 {
            self.tracks = 1;
            self.tempo = bpm;
            return;
        }
        if crate::clock::same_tempo(bpm, self.tempo, 0.03) {
            self.tempo += 0.05 * (crate::clock::fold_to(bpm, self.tempo) - self.tempo);
            self.off_since = 0.0;
        } else {
            if self.off_since == 0.0 {
                self.off_since = t_now.max(1e-6);
            }
            if t_now - self.off_since > 12.0 {
                self.tracks += 1;
                self.tempo = bpm;
                self.off_since = 0.0;
                self.drops = 0;
                self.builds = 0;
                self.breakdowns = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bars_in_a_phrase_and_wraps_at_sixteen() {
        let mut p = Phrase::default();
        let mut phrases = 0;
        for i in 0..(16 * 4 * 3) {
            p.update(i as f64 / 1.0, None, 4);
            phrases += p.new_phrase as u32;
        }
        assert_eq!(phrases, 2);
        assert!(p.bar_in_phrase < PHRASE_BARS);
    }

    #[test]
    fn anchoring_restarts_the_count() {
        let mut p = Phrase::default();
        for i in 0..(5 * 4) {
            p.update(i as f64, None, 4);
        }
        assert_eq!(p.bar_in_phrase, 4);
        p.anchor();
        assert_eq!(p.bar_in_phrase, 0);
        p.update(5.0 * 4.0, None, 4);
        assert_eq!(p.bar_in_phrase, 1);
    }

    #[test]
    fn uses_the_bar_position_when_known() {
        let mut p = Phrase::default();
        // Beat 10.5 with the "1" 2.5 beats ago: the bar began at beat 8.
        p.update(10.5, Some(2.5), 4);
        assert_eq!(p.bar, 2);
        assert!((p.in_bar - 0.625).abs() < 1e-6);
    }

    #[test]
    fn counts_drops_and_notices_a_new_track() {
        let mut m = SetMemory::default();
        m.tempo(1.0, 128.0, true);
        m.section(Section::Drop);
        m.section(Section::Drop);
        m.section(Section::BuildUp);
        assert_eq!((m.drops, m.builds, m.tracks), (2, 1, 1));
        // Double time of the same track is not a new track.
        for i in 0..400 {
            m.tempo(2.0 + i as f64 * 0.1, 64.0, true);
        }
        assert_eq!(m.tracks, 1);
        // A different tempo held for a while is.
        for i in 0..400 {
            m.tempo(50.0 + i as f64 * 0.1, 140.0, true);
        }
        assert_eq!(m.tracks, 2);
        assert_eq!(m.drops, 0);
    }
}
