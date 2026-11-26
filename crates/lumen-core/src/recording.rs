//! A recorded session: what the analyser heard and what the clock said, tick
//! by tick, with the DJ's own marks of which part of the song was playing.
//!
//! It exists so a misread can be reproduced without the rig: the same frames
//! replayed through the section detector must give the same sections, and can
//! be scored against the marks.

use crate::audio::AudioBands;
use crate::sections::{GenreProfile, Section, SectionDetector};
use serde::{Deserialize, Serialize};

pub const FORMAT: u32 = 1;

/// One engine tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// Seconds since the recording started.
    pub t: f64,
    /// The beat the engine used for this tick (Link's or its own).
    pub beat: f64,
    /// The tempo the engine used.
    pub tempo: f64,
    /// Ableton Link peers at this moment (0 when Link is off or alone).
    pub peers: u32,
    /// The analysis, as the lights saw it.
    pub bands: AudioBands,
    /// The section the DJ marked as playing: 0 none, 1 to 5 as `Section::ALL`.
    pub mark: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub kickline_recording: u32,
    pub app_version: String,
    pub created: String,
    /// The audio source's name, as shown in the app.
    pub source: String,
    pub link_on: bool,
    /// Genre set in Auto while recording.
    pub genre: String,
    pub frames: Vec<Frame>,
}

/// Longest recording kept: 30 minutes at 44 ticks a second.
pub const MAX_FRAMES: usize = 44 * 60 * 30;

pub fn section_for_mark(mark: u8) -> Option<Section> {
    (mark >= 1).then(|| Section::ALL.get(mark as usize - 1).copied()).flatten()
}

impl Recording {
    pub fn new(app_version: &str, created: &str, source: &str, link_on: bool, genre: &str) -> Self {
        Recording {
            kickline_recording: FORMAT,
            app_version: app_version.into(),
            created: created.into(),
            source: source.into(),
            link_on,
            genre: genre.into(),
            frames: Vec::new(),
        }
    }

    pub fn push(&mut self, f: Frame) {
        if self.frames.len() < MAX_FRAMES {
            self.frames.push(f);
        }
    }

    pub fn seconds(&self) -> f64 {
        self.frames.last().map_or(0.0, |f| f.t)
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        let r: Recording = serde_json::from_str(s).map_err(|e| e.to_string())?;
        if r.kickline_recording == 0 || r.kickline_recording > FORMAT {
            return Err(format!("not a recording this version can read (format {})", r.kickline_recording));
        }
        Ok(r)
    }

    /// Replay the frames through a fresh detector and return the section it
    /// believed at each frame. `clock` can swap in a different beat (for
    /// example the tracker's, or a deliberately wrong one).
    pub fn replay_with(&self, profile: GenreProfile, clock: impl Fn(&Frame) -> f64) -> Vec<Section> {
        let mut d = SectionDetector::default();
        d.set_profile(profile);
        let mut out = Vec::with_capacity(self.frames.len());
        for f in &self.frames {
            d.update(&f.bands, clock(f));
            out.push(d.current());
        }
        out
    }

    /// Replay with the recorded clock.
    pub fn replay(&self, profile: GenreProfile) -> Vec<Section> {
        self.replay_with(profile, |f| f.beat)
    }

    /// How often the replayed sections match the DJ's marks, as
    /// (frames that match, frames marked). The first `skip_s` seconds are
    /// ignored (the detector is still learning what loud means), and so are
    /// `grace_s` seconds after each mark change (a section can't be heard
    /// before it has begun).
    pub fn score(&self, sections: &[Section], skip_s: f64, grace_s: f64) -> (usize, usize) {
        let (mut ok, mut n) = (0, 0);
        let mut since_change = f64::MAX;
        let mut last_mark = 0;
        let mut last_t = 0.0;
        for (f, s) in self.frames.iter().zip(sections) {
            since_change = if f.mark != last_mark { 0.0 } else { since_change + (f.t - last_t) };
            last_mark = f.mark;
            last_t = f.t;
            let Some(want) = section_for_mark(f.mark) else { continue };
            if f.t < skip_s || since_change < grace_s {
                continue;
            }
            n += 1;
            ok += (*s == want) as usize;
        }
        (ok, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioBand;

    /// A made-up set at 120 BPM and 44 ticks a second: loud part, groove,
    /// breakdown, with the marks a DJ would press.
    fn demo_set() -> Recording {
        let mut r = Recording::new("test", "now", "demo", true, "House");
        let mut bands = AudioBands::default();
        let (mut t, mut beat) = (0.0f64, 0.0f64);
        let parts: [(u8, u32, u32, f32, f32, f32); 3] = [
            // mark, beats, kicks per beat, low, high, level
            (5, 48, 1, 1.0, 0.5, 1.0),
            (1, 48, 1, 0.5, 0.35, 0.65),
            (2, 48, 0, 0.12, 0.25, 0.4),
        ];
        for (mark, beats, kicks, low, high, level) in parts {
            for b in 0..beats {
                for k in 0..22 {
                    if k == 0 {
                        bands.hits[0] += kicks as u64;
                        bands.hits[1] += (b % 2) as u64;
                    }
                    bands.raw[AudioBand::Sub.index()] = low * 0.4;
                    bands.raw[AudioBand::Bass.index()] = low * 0.6;
                    bands.raw[AudioBand::High.index()] = high;
                    bands.raw[AudioBand::Level.index()] = level;
                    r.push(Frame { t, beat, tempo: 120.0, peers: 1, bands, mark });
                    t += 1.0 / 44.0;
                    beat += 0.5 / 22.0;
                }
            }
        }
        r
    }

    #[test]
    fn json_round_trips_and_rejects_other_files() {
        let r = demo_set();
        let back = Recording::from_json(&r.to_json().unwrap()).unwrap();
        assert_eq!(back.frames.len(), r.frames.len());
        assert_eq!(back.frames[100].mark, r.frames[100].mark);
        assert!((back.frames[100].beat - r.frames[100].beat).abs() < 1e-9);
        assert_eq!(back.frames[100].bands.raw, r.frames[100].bands.raw);
        // And the replay of the file gives the same sections as the original.
        assert_eq!(back.replay(GenreProfile::default()), r.replay(GenreProfile::default()));
        assert!(Recording::from_json("{\"kickline_recording\":99,\"app_version\":\"\",\"created\":\"\",\"source\":\"\",\"link_on\":false,\"genre\":\"\",\"frames\":[]}").is_err());
        assert!(Recording::from_json("{}").is_err());
    }

    #[test]
    fn replay_is_deterministic_and_matches_the_marks() {
        let r = demo_set();
        let a = r.replay(GenreProfile::default());
        assert_eq!(a, r.replay(GenreProfile::default()));
        let (ok, n) = r.score(&a, 6.0, 8.0);
        assert!(n > 500, "{n}");
        assert!(ok as f64 / n as f64 > 0.9, "{ok}/{n}");
    }

    #[test]
    fn a_clock_that_jumps_is_visible_in_the_score() {
        // The harness can swap the clock: a beat that keeps restarting gives
        // the detector nonsense beats, and the score shows it.
        let r = demo_set();
        let good = r.replay(GenreProfile::default());
        let jumpy = r.replay_with(GenreProfile::default(), |f| (f.t * 1.7).sin().abs() * 6.0 + f.beat * 0.0);
        let s = |v: &Vec<Section>| {
            let (ok, n) = r.score(v, 6.0, 8.0);
            ok as f64 / n as f64
        };
        assert!(s(&good) > s(&jumpy), "{} vs {}", s(&good), s(&jumpy));
    }

    #[test]
    fn marks_map_to_sections() {
        assert_eq!(section_for_mark(0), None);
        assert_eq!(section_for_mark(1), Some(Section::Groove));
        assert_eq!(section_for_mark(5), Some(Section::Drop));
        assert_eq!(section_for_mark(6), None);
    }
}
