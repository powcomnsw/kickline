//! A tiny drum machine, used as a test signal when no audio device is
//! connected (and by the analyser's tests).

use std::f32::consts::PI;

/// Uniform noise in -1..1 from a xorshift state.
pub fn noise(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Four-on-the-floor: kick every beat, snare on 2 and 4, open-ish hat on
/// the off-beats, and a bassline.
pub struct DemoBeat {
    fs: f32,
    pub bpm: f32,
    t: u64,
    seed: u32,
    hat_hp: crate::filter::Chain,
    snare_lp: crate::filter::Chain,
    /// Play a little track that goes through its sections (drop, groove,
    /// breakdown, build-up, pause) instead of the same loop forever.
    pub arranged: bool,
}

/// One part of the demo arrangement.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Part {
    Drop,
    Groove,
    Breakdown,
    Build,
    Pause,
}

impl DemoBeat {
    /// Bars per arrangement cycle.
    pub const CYCLE_BARS: i64 = 20;

    /// Which part of the arrangement a beat falls in.
    pub fn part(beat: i64) -> Part {
        let bar = beat.div_euclid(4).rem_euclid(Self::CYCLE_BARS);
        let in_bar = beat.rem_euclid(4);
        match bar {
            0..=7 => Part::Drop,
            8..=11 => Part::Groove,
            12..=15 => Part::Breakdown,
            19 if in_bar == 3 => Part::Pause,
            _ => Part::Build,
        }
    }
}

impl DemoBeat {
    pub fn new(sample_rate: f32, bpm: f32) -> Self {
        DemoBeat { fs: sample_rate, bpm, t: 0, seed: 0x1234_5678, hat_hp: crate::filter::Chain::hp4(sample_rate, 7000.0), snare_lp: crate::filter::Chain::lp4(sample_rate, 5000.0), arranged: false }
    }

    pub fn fill(&mut self, out: &mut [f32]) {
        let spb = 60.0 / self.bpm; // seconds per beat
        for o in out.iter_mut() {
            let time = self.t as f32 / self.fs;
            let beat_pos = time / spb;
            let beat_idx = beat_pos.floor() as i64;
            let tb = (beat_pos - beat_pos.floor()) * spb; // seconds since beat
            let n = noise(&mut self.seed);
            let n_snare = self.snare_lp.process(n) * 1.6;

            let part = if self.arranged { DemoBeat::part(beat_idx) } else { Part::Drop };
            if part == Part::Pause {
                *o = 0.0;
                self.t += 1;
                continue;
            }

            // Kick: pitch-dropping sine
            let f = 50.0 + 90.0 * (-tb * 30.0).exp();
            let kick = (2.0 * PI * f * tb).sin() * (-tb * 9.0).exp();
            let kick = match part {
                Part::Drop => kick,
                Part::Groove => kick * 0.5,
                _ => 0.0,
            };

            // Snare on beats 1 and 3 (0-based), i.e. 2 and 4 of the bar
            let snare = if beat_idx % 2 == 1 {
                let body = (2.0 * PI * 190.0 * tb).sin() * (-tb * 25.0).exp() * 0.4;
                body + n_snare * (-tb * 18.0).exp() * 0.6
            } else {
                0.0
            };

            // Hi-hat on the off-beat: high-passed noise burst
            let toff = tb - spb / 2.0;
            let hp = self.hat_hp.process(n);
            let hat = if toff >= 0.0 { hp * (-toff * 60.0).exp() * 0.7 } else { 0.0 };

            // Bass: off-beat 55 Hz note
            let bass = if toff >= 0.0 { (2.0 * PI * 55.0 * time).sin() * (-toff * 6.0).exp() * 0.25 } else { 0.0 };

            let (snare, hat, bass, extra) = match part {
                Part::Drop => (snare, hat, bass, 0.0),
                Part::Groove => (snare * 0.7, hat * 0.8, 0.0, 0.0),
                Part::Breakdown => {
                    // A soft pad chord.
                    let pad = [220.0, 277.2, 329.6].iter().map(|f| (2.0 * PI * f * time).sin()).sum::<f32>() * 0.09;
                    (0.0, hat * 0.25, 0.0, pad)
                }
                _ => {
                    // Build: a snare roll that speeds up and a rising noise sweep.
                    let bar_in = beat_idx.div_euclid(4).rem_euclid(DemoBeat::CYCLE_BARS) - 16;
                    let per_beat = [1.0, 2.0, 4.0, 4.0][bar_in.clamp(0, 3) as usize];
                    let step = spb / per_beat;
                    let ts = tb % step;
                    let roll = n_snare * (-ts * 30.0).exp() * (0.35 + 0.15 * bar_in as f32);
                    let k = ((beat_idx.rem_euclid(16)) as f32 + tb / spb) / 16.0;
                    (roll, hp * k * 0.5, 0.0, 0.0)
                }
            };
            *o = (kick * 0.8 + snare + hat + bass + extra).clamp(-1.0, 1.0);
            self.t += 1;
        }
    }
}
