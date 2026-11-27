//! One place decides what time it is in the music.
//!
//! Three things can say where the beat is: the beat tracker (from the audio),
//! Ableton Link, and the typed or tapped BPM. This picks one by fixed rules,
//! and hands out two beats:
//!
//! * the **analysis beat**, aligned to the audio as it arrives. Section
//!   detection and the playbook use it.
//! * the **render beat**, the analysis beat moved earlier by the output
//!   lookahead, so the lights land on the sound. Generators and the Beat and
//!   Bar pulses use it.
//!
//! The rules:
//! 1. The audio decides the beat whenever the tracker is locked.
//! 2. Link's tempo is used when it agrees with the audio (allowing half and
//!    double), and carries the tempo through a breakdown.
//! 3. If Link and the audio disagree for more than a few bars, the audio
//!    wins and `note` says so in plain words.
//! 4. With no locked tracker, Link's beat is used if it's there; the tracker
//!    keeps counting for a while through a breakdown first.
//! 5. With nothing, the clock free-runs on the last tempo and `steady` is
//!    false, so the playbook sticks to looks that don't need the beat.

/// What the beat tracker says (a copy of lumen_audio's reading).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioClock {
    pub tempo: f64,
    pub beat: f64,
    /// Beats since the last "1".
    pub bar_pos: f64,
    pub tempo_conf: f32,
    pub bar_conf: f32,
    pub locked: bool,
}

/// What Ableton Link says (the raw beat, with no lookahead).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkClock {
    pub peers: u64,
    pub tempo: f64,
    pub beat: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClockSource {
    /// Beat from the audio.
    Audio,
    /// Beat from Ableton Link.
    Link,
    /// Counting on the typed or tapped BPM.
    #[default]
    Manual,
}

impl ClockSource {
    pub fn name(self) -> &'static str {
        match self {
            ClockSource::Audio => "AUDIO",
            ClockSource::Link => "LINK",
            ClockSource::Manual => "MANUAL",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClockOut {
    pub analysis_beat: f64,
    pub render_beat: f64,
    pub tempo: f64,
    pub source: ClockSource,
    /// Beat position in the bar, when known.
    pub bar_pos: Option<f64>,
    /// The beat can be trusted: looks that need it may play.
    pub steady: bool,
    /// A sentence for the UI when something is off.
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Clock {
    free_beat: f64,
    /// Seconds Link and the audio have disagreed.
    disagree_s: f64,
    /// Link agreed with the audio the last time both were there.
    link_agreed: bool,
    last_tempo: f64,
}

/// Is `a` the same tempo as `b`, or its half or double, within `tol`?
pub fn same_tempo(a: f64, b: f64, tol: f64) -> bool {
    a > 0.0 && b > 0.0 && [1.0, 2.0, 0.5].iter().any(|k| (a * k / b - 1.0).abs() < tol)
}

/// Move `t` by doubling or halving to the octave nearest `to`.
pub fn fold_to(t: f64, to: f64) -> f64 {
    let mut x = t;
    while x > to * 1.41 {
        x /= 2.0;
    }
    while x < to / 1.41 {
        x *= 2.0;
    }
    x
}

impl Clock {
    /// Advance by `dt` seconds and decide. `lookahead_s` is how early the
    /// lights should fire; `manual_bpm` is the typed or tapped tempo.
    pub fn step(&mut self, dt: f64, audio: Option<&AudioClock>, link: Option<&LinkClock>, manual_bpm: f64, lookahead_s: f64) -> ClockOut {
        let link = link.filter(|l| l.peers > 0 && l.tempo > 0.0);
        let mut note = None;
        let mut out = ClockOut::default();

        // Does Link agree with the audio?
        if let (Some(a), Some(l)) = (audio.filter(|a| a.locked), link) {
            if same_tempo(a.tempo, l.tempo, 0.015) {
                self.disagree_s = 0.0;
                self.link_agreed = true;
            } else {
                self.disagree_s += dt;
                self.link_agreed = false;
            }
        } else if link.is_none() {
            self.disagree_s = 0.0;
        }
        // "A few bars": 8 s is four bars at 120 BPM.
        if self.disagree_s > 8.0 {
            if let (Some(a), Some(l)) = (audio, link) {
                note = Some(format!("Link says {:.0} BPM but the music is {:.0}: following the music.", l.tempo, a.tempo));
            }
        }

        let tracking = audio.filter(|a| a.locked || a.tempo_conf > 0.1);
        match (tracking, link) {
            (Some(a), l) => {
                // The audio's beat, with Link's tempo when it agrees.
                let tempo = match l {
                    Some(l) if self.link_agreed && same_tempo(a.tempo, l.tempo, 0.015) => fold_to(l.tempo, a.tempo),
                    _ => a.tempo,
                };
                out.analysis_beat = a.beat;
                out.tempo = tempo;
                out.source = ClockSource::Audio;
                out.bar_pos = (a.bar_conf > 0.2).then_some(a.bar_pos);
                out.steady = a.locked || a.tempo_conf > 0.1;
            }
            (None, Some(l)) => {
                out.analysis_beat = l.beat;
                out.tempo = l.tempo;
                out.source = ClockSource::Link;
                out.steady = true;
            }
            (None, None) => {
                self.free_beat += dt * manual_bpm / 60.0;
                out.analysis_beat = self.free_beat;
                out.tempo = manual_bpm;
                out.source = ClockSource::Manual;
                out.steady = false;
            }
        }
        // Keep the free-running beat in step so a switch to Manual doesn't jump.
        if out.source != ClockSource::Manual {
            self.free_beat = out.analysis_beat;
        }
        self.last_tempo = out.tempo;
        // The lookahead cancels output delay; a free-running clock has none to cancel.
        let la = if out.source == ClockSource::Manual { 0.0 } else { lookahead_s.max(0.0) };
        out.render_beat = out.analysis_beat + la * out.tempo / 60.0;
        out.note = note;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locked(tempo: f64, beat: f64) -> AudioClock {
        AudioClock { tempo, beat, bar_pos: 0.0, tempo_conf: 0.9, bar_conf: 0.0, locked: true }
    }

    #[test]
    fn audio_decides_the_beat_when_locked() {
        let mut c = Clock::default();
        let link = LinkClock { peers: 1, tempo: 128.0, beat: 500.3 };
        let o = c.step(0.02, Some(&locked(128.0, 10.25)), Some(&link), 120.0, 0.0);
        assert_eq!(o.source, ClockSource::Audio);
        assert_eq!(o.analysis_beat, 10.25);
        assert!(o.steady);
    }

    #[test]
    fn link_tempo_is_used_when_it_agrees_even_at_double() {
        let mut c = Clock::default();
        let link = LinkClock { peers: 1, tempo: 87.0, beat: 1.0 };
        let o = c.step(0.02, Some(&locked(174.2, 3.0)), Some(&link), 120.0, 0.0);
        assert!((o.tempo - 174.0).abs() < 0.01, "{}", o.tempo);
    }

    #[test]
    fn audio_wins_and_says_so_when_link_disagrees_for_a_few_bars() {
        let mut c = Clock::default();
        let link = LinkClock { peers: 1, tempo: 100.0, beat: 1.0 };
        let mut last = ClockOut::default();
        for _ in 0..(10 * 50) {
            last = c.step(0.02, Some(&locked(128.0, 5.0)), Some(&link), 120.0, 0.0);
        }
        assert_eq!(last.source, ClockSource::Audio);
        assert_eq!(last.tempo, 128.0);
        let note = last.note.expect("should explain");
        assert!(note.contains("100") && note.contains("128"), "{note}");
        // And not before then.
        let mut c = Clock::default();
        let o = c.step(0.02, Some(&locked(128.0, 5.0)), Some(&link), 120.0, 0.0);
        assert!(o.note.is_none());
    }

    #[test]
    fn link_is_the_beat_until_the_audio_locks() {
        let mut c = Clock::default();
        let link = LinkClock { peers: 2, tempo: 126.0, beat: 40.5 };
        let o = c.step(0.02, None, Some(&link), 120.0, 0.0);
        assert_eq!((o.source, o.analysis_beat, o.tempo), (ClockSource::Link, 40.5, 126.0));
        // Link with nobody else in the session is no clock at all.
        let alone = LinkClock { peers: 0, ..link };
        let o = c.step(0.02, None, Some(&alone), 120.0, 0.0);
        assert_eq!(o.source, ClockSource::Manual);
    }

    #[test]
    fn with_nothing_it_free_runs_and_says_it_is_not_steady() {
        let mut c = Clock::default();
        let mut o = ClockOut::default();
        for _ in 0..100 {
            o = c.step(0.01, None, None, 120.0, 0.0);
        }
        assert!((o.analysis_beat - 2.0).abs() < 1e-9);
        assert!(!o.steady);
        assert_eq!(o.source, ClockSource::Manual);
    }

    #[test]
    fn render_beat_is_the_analysis_beat_moved_earlier_by_the_lookahead() {
        let mut c = Clock::default();
        let o = c.step(0.02, Some(&locked(120.0, 8.0)), None, 120.0, 0.05);
        assert!((o.render_beat - o.analysis_beat - 0.1).abs() < 1e-9);
    }

    #[test]
    fn an_unlocked_tracker_that_still_counts_carries_a_breakdown() {
        let mut c = Clock::default();
        let a = AudioClock { tempo: 126.0, beat: 77.0, tempo_conf: 0.3, locked: false, ..Default::default() };
        let o = c.step(0.02, Some(&a), None, 120.0, 0.0);
        assert_eq!(o.source, ClockSource::Audio);
        assert_eq!(o.analysis_beat, 77.0);
    }
}
