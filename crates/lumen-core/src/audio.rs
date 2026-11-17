//! Audio analysis values as seen by the engine. The analysis itself lives in
//! `lumen-audio`; core only consumes these numbers, so it stays I/O free.

use serde::{Deserialize, Serialize};

/// One thing that can be pulled out of the music.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioBand {
    /// Overall loudness.
    Level,
    /// Sub bass, below ~60 Hz.
    Sub,
    /// Bass, ~60-250 Hz.
    Bass,
    /// Low mids, ~250 Hz-2 kHz (vocals, synths, snare body).
    LowMid,
    /// High mids, ~2-6 kHz (presence, snare crack, leads).
    HighMid,
    /// Highs, above ~6 kHz (hats, cymbals, air).
    High,
    /// Kick drum hits (decaying pulse after each hit).
    Kick,
    /// Snare / clap hits.
    Snare,
    /// Hi-hat / cymbal hits.
    HiHat,
    /// Every beat of the clock (from Ableton Link when synced, otherwise the BPM).
    /// Fires exactly on time, with no audio delay.
    Beat,
    /// First beat of every 4-beat bar.
    Bar,
}

impl AudioBand {
    pub const ALL: [AudioBand; 11] = [
        AudioBand::Level,
        AudioBand::Sub,
        AudioBand::Bass,
        AudioBand::LowMid,
        AudioBand::HighMid,
        AudioBand::High,
        AudioBand::Kick,
        AudioBand::Snare,
        AudioBand::HiHat,
        AudioBand::Beat,
        AudioBand::Bar,
    ];

    /// Bands that fire discrete hits (usable for "advance clip on hit").
    pub const HITS: [AudioBand; 5] = [AudioBand::Kick, AudioBand::Snare, AudioBand::HiHat, AudioBand::Beat, AudioBand::Bar];

    /// Hits detected from the audio (as opposed to the clock).
    pub const DRUMS: [AudioBand; 3] = [AudioBand::Kick, AudioBand::Snare, AudioBand::HiHat];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            AudioBand::Level => "Level",
            AudioBand::Sub => "Sub",
            AudioBand::Bass => "Bass",
            AudioBand::LowMid => "Low mid",
            AudioBand::HighMid => "High mid",
            AudioBand::High => "Highs",
            AudioBand::Kick => "Kick",
            AudioBand::Snare => "Snare",
            AudioBand::HiHat => "Hi-hat",
            AudioBand::Beat => "Beat",
            AudioBand::Bar => "Bar",
        }
    }

    pub fn hit_slot(self) -> Option<usize> {
        match self {
            AudioBand::Kick => Some(0),
            AudioBand::Snare => Some(1),
            AudioBand::HiHat => Some(2),
            AudioBand::Beat => Some(3),
            AudioBand::Bar => Some(4),
            _ => None,
        }
    }

    /// Display colour, used by meters and the Bands generator.
    pub fn color(self) -> [f32; 3] {
        match self {
            AudioBand::Level => [0.9, 0.9, 0.9],
            AudioBand::Sub => [0.45, 0.0, 0.9],
            AudioBand::Bass => [0.0, 0.35, 1.0],
            AudioBand::LowMid => [0.0, 0.85, 0.5],
            AudioBand::HighMid => [1.0, 0.6, 0.0],
            AudioBand::High => [1.0, 1.0, 1.0],
            AudioBand::Kick => [1.0, 0.1, 0.1],
            AudioBand::Snare => [1.0, 0.85, 0.2],
            AudioBand::HiHat => [0.5, 0.9, 1.0],
            AudioBand::Beat => [1.0, 0.4, 0.8],
            AudioBand::Bar => [0.8, 0.5, 1.0],
        }
    }
}

/// A snapshot of the analysis. All values 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioBands {
    pub values: [f32; 11],
    /// Running hit counters for Kick, Snare, HiHat, Beat, Bar. Consumers compare
    /// against the last value they saw to detect new hits.
    pub hits: [u64; 5],
}

impl AudioBands {
    pub fn get(&self, band: AudioBand) -> f32 {
        self.values[band.index()]
    }

    pub fn set(&mut self, band: AudioBand, v: f32) {
        self.values[band.index()] = v.clamp(0.0, 1.0);
    }
}

/// "Make this parameter follow that band": value = 1 - amount + amount * band.
/// amount 0 = ignore audio, 1 = fully driven by audio.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioLink {
    pub band: AudioBand,
    pub amount: f32,
}

impl AudioLink {
    pub fn factor(&self, a: &AudioBands) -> f32 {
        let k = self.amount.clamp(0.0, 1.0);
        1.0 - k + k * a.get(self.band)
    }
}
