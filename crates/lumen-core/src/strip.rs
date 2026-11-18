//! Effects that run along each strip (or each fixture), instead of a picture
//! mapped onto the pixels. A meteor goes from one end of every strip to the
//! other, whatever angle the strips are at on the canvas.
//!
//! Strip clips live in the normal clip grid. They're worked out per pixel at
//! output time and laid over the mapped canvas, using their layer's opacity,
//! blend mode and effects. Strip layers are drawn after all picture layers.

use crate::audio::{AudioBand, AudioBands};
use crate::frame::Rgba;
use crate::palette::{Paint, Rgb};
use serde::{Deserialize, Serialize};

/// Where a pixel is: which fixture, and where along it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StripPixel {
    /// Fixture number and how many fixtures there are.
    pub fixture: usize,
    pub fixtures: usize,
    /// Pixel number in wiring order, and pixels in this fixture.
    pub index: usize,
    pub count: usize,
    /// Position across the fixture, 0..1, in the fixture's own direction
    /// (so every strip in a zig-zag array still runs the same way).
    pub x: f32,
    /// Position down the fixture, 0..1 (rows of a panel).
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum StripEffect {
    /// A bright head with a fading tail, crossing every `beats`.
    Meteor { color: Paint, beats: f32, tail: f32, bounce: bool },
    /// The strip fills up over `beats`, then starts again.
    Fill { color: Paint, beats: f32 },
    /// Each hit bursts out from the middle and fades.
    Burst { color: Paint, band: AudioBand },
    /// Each hit lights a different random segment.
    Segments { color: Paint, count: u8, band: AudioBand },
    /// Random pixels twinkle; more of them when `band` is busy.
    Sparkle { color: Paint, density: f32, band: AudioBand },
    /// A level meter along the strip following a band.
    Meter { color: Paint, band: AudioBand },
    /// Evenly spaced dashes marching along.
    Dashes { color: Paint, beats: f32, count: u8 },
    /// Odd and even strips swap on every hit.
    Alternate { color: Paint, band: AudioBand },
}

/// A strip clip: the effect plus how it's laid across the rig.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StripClip {
    pub effect: StripEffect,
    /// Run the effect from the far end.
    #[serde(default)]
    pub reverse: bool,
    /// Run it from the middle out to both ends.
    #[serde(default)]
    pub mirror: bool,
    /// Delay each fixture a little more than the last (in beats across the
    /// whole rig), so the effect ripples from strip to strip.
    #[serde(default)]
    pub stagger: f32,
}

impl StripEffect {
    pub fn paint_mut(&mut self) -> &mut Paint {
        match self {
            StripEffect::Meteor { color, .. }
            | StripEffect::Fill { color, .. }
            | StripEffect::Burst { color, .. }
            | StripEffect::Segments { color, .. }
            | StripEffect::Sparkle { color, .. }
            | StripEffect::Meter { color, .. }
            | StripEffect::Dashes { color, .. }
            | StripEffect::Alternate { color, .. } => color,
        }
    }
}

impl StripClip {
    pub fn new(effect: StripEffect) -> Self {
        StripClip { effect, reverse: false, mirror: false, stagger: 0.0 }
    }

    pub fn name(&self) -> &'static str {
        match self.effect {
            StripEffect::Meteor { .. } => "Meteor",
            StripEffect::Fill { .. } => "Fill",
            StripEffect::Burst { .. } => "Burst",
            StripEffect::Segments { .. } => "Segments",
            StripEffect::Sparkle { .. } => "Sparkle",
            StripEffect::Meter { .. } => "Meter",
            StripEffect::Dashes { .. } => "Dashes",
            StripEffect::Alternate { .. } => "Alternate",
        }
    }

    /// Colour and coverage (alpha) of one pixel.
    pub fn shade(&self, p: StripPixel, beat: f64, audio: &AudioBands, palette: &[Rgb; 4]) -> Rgba {
        let mut x = p.x.clamp(0.0, 1.0);
        if self.reverse {
            x = 1.0 - x;
        }
        if self.mirror {
            x = (2.0 * x - 1.0).abs();
        }
        let delay = if p.fixtures > 1 { self.stagger as f64 * p.fixture as f64 / (p.fixtures - 1) as f64 } else { 0.0 };
        let beat = beat - delay;
        let on = |paint: Paint, a: f32| {
            let c = paint.resolve(palette);
            Rgba { r: c[0], g: c[1], b: c[2], a: a.clamp(0.0, 1.0) }
        };
        let hits = |band: AudioBand| band.hit_slot().map(|s| audio.hits[s]).unwrap_or(0);
        match self.effect {
            StripEffect::Meteor { color, beats, tail, bounce } => {
                let mut t = (beat / beats.max(0.05) as f64).rem_euclid(if bounce { 2.0 } else { 1.0 }) as f32;
                let forward = t <= 1.0;
                if t > 1.0 {
                    t = 2.0 - t;
                }
                // Head runs a little past both ends so the tail can leave.
                let tail = tail.clamp(0.01, 1.0);
                let head = t * (1.0 + tail) - if forward { 0.0 } else { tail };
                let d = if forward { head - x } else { x - head };
                let a = if (0.0..=tail).contains(&d) { (1.0 - d / tail).powf(1.6) } else { 0.0 };
                on(color, a)
            }
            StripEffect::Fill { color, beats } => {
                let t = (beat / beats.max(0.05) as f64).rem_euclid(1.0) as f32;
                on(color, if x <= t { 1.0 } else { 0.0 })
            }
            StripEffect::Burst { color, band } => {
                let pulse = audio.get(band);
                // The ring grows outwards as the hit fades.
                let r = 1.0 - pulse;
                let d = (2.0 * x - 1.0).abs();
                let edge = (1.0 - ((d - r).abs() / 0.25)).max(0.0);
                let inside = if d < r { 0.25 } else { 0.0 };
                on(color, (edge + inside) * pulse)
            }
            StripEffect::Segments { color, count, band } => {
                let n = count.max(1) as u32;
                let seg = ((x * n as f32) as u32).min(n - 1);
                let pick = hash(hits(band) as u32 ^ (p.fixture as u32).wrapping_mul(0x9e37)) % n;
                on(color, if seg == pick { audio.get(band).max(0.15) } else { 0.0 })
            }
            StripEffect::Sparkle { color, density, band } => {
                // A new set of sparkles every 16th note, each fading out.
                let step = (beat * 4.0).floor() as i64;
                let phase = (beat * 4.0).rem_euclid(1.0) as f32;
                let busy = 0.3 + 0.7 * audio.get(band);
                let chance = (density.clamp(0.0, 1.0) * busy * 0.25 * 1000.0) as u32;
                let r = hash((p.index as u32).wrapping_mul(2654435761) ^ (p.fixture as u32) << 20 ^ step as u32) % 1000;
                on(color, if r < chance { 1.0 - phase } else { 0.0 })
            }
            StripEffect::Meter { color, band } => {
                let level = audio.get(band);
                let edge = ((level - x) / 0.04).clamp(0.0, 1.0);
                on(color, edge)
            }
            StripEffect::Dashes { color, beats, count } => {
                let n = count.max(1) as f32;
                let t = (beat / beats.max(0.05) as f64).rem_euclid(1.0) as f32;
                let u = (x * n - t * 1.0).rem_euclid(1.0);
                on(color, if u < 0.5 { 1.0 } else { 0.0 })
            }
            StripEffect::Alternate { color, band } => {
                let flip = hits(band) % 2 == 1;
                let odd = p.fixture % 2 == 1;
                on(color, if odd == flip { 1.0 } else { 0.0 })
            }
        }
    }

    /// Ready-made strip clips for the "add clip" menu.
    pub fn presets() -> Vec<(&'static str, StripClip)> {
        use StripEffect::*;
        let w = Paint::WHITE;
        vec![
            ("Meteor", StripClip::new(Meteor { color: Paint::Slot(0), beats: 2.0, tail: 0.35, bounce: false })),
            ("Meteor bounce", StripClip::new(Meteor { color: Paint::Slot(1), beats: 1.0, tail: 0.25, bounce: true })),
            ("Fill to the bar", StripClip::new(Fill { color: Paint::Slot(2), beats: 4.0 })),
            ("Kick burst", StripClip::new(Burst { color: w, band: AudioBand::Kick })),
            ("Snare segments", StripClip::new(Segments { color: Paint::Slot(3), count: 6, band: AudioBand::Snare })),
            ("Hi-hat sparkle", StripClip::new(Sparkle { color: w, density: 0.5, band: AudioBand::HiHat })),
            ("Bass meter", StripClip { mirror: true, ..StripClip::new(Meter { color: Paint::Slot(0), band: AudioBand::Bass }) }),
            ("Dashes", StripClip::new(Dashes { color: Paint::Slot(1), beats: 2.0, count: 6 })),
            ("Kick alternate", StripClip::new(Alternate { color: Paint::Slot(0), band: AudioBand::Kick })),
            ("Wave across strips", StripClip { stagger: 1.0, ..StripClip::new(Meteor { color: Paint::Slot(2), beats: 2.0, tail: 0.5, bounce: false }) }),
        ]
    }
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: [Rgb; 4] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]];

    fn px(x: f32, fixture: usize) -> StripPixel {
        StripPixel { fixture, fixtures: 4, index: (x * 99.0) as usize, count: 100, x, y: 0.0 }
    }

    #[test]
    fn meteor_travels_along_the_strip() {
        let c = StripClip::new(StripEffect::Meteor { color: Paint::Slot(0), beats: 4.0, tail: 0.2, bounce: false });
        let a = AudioBands::default();
        // A quarter of the way through, the head is near x = 0.3.
        let lit = |beat: f64| (0..100).map(|i| c.shade(px(i as f32 / 99.0, 0), beat, &a, &P).a).enumerate().max_by(|x, y| x.1.total_cmp(&y.1)).unwrap().0;
        assert!(lit(1.0) < lit(2.0) && lit(2.0) < lit(3.0));
        // Reverse runs the other way.
        let r = StripClip { reverse: true, ..c };
        assert!(r.shade(px(0.9, 0), 0.4, &a, &P).a > r.shade(px(0.1, 0), 0.4, &a, &P).a);
    }

    #[test]
    fn fill_and_mirror() {
        let c = StripClip::new(StripEffect::Fill { color: Paint::WHITE, beats: 4.0 });
        let a = AudioBands::default();
        assert_eq!(c.shade(px(0.2, 0), 2.0, &a, &P).a, 1.0);
        assert_eq!(c.shade(px(0.8, 0), 2.0, &a, &P).a, 0.0);
        let m = StripClip { mirror: true, ..c };
        // Mirrored: fills from the middle, so both ends are dark halfway through.
        assert_eq!(m.shade(px(0.5, 0), 2.0, &a, &P).a, 1.0);
        assert_eq!(m.shade(px(0.02, 0), 2.0, &a, &P).a, 0.0);
        assert_eq!(m.shade(px(0.98, 0), 2.0, &a, &P).a, 0.0);
    }

    #[test]
    fn stagger_delays_later_fixtures() {
        let c = StripClip { stagger: 1.0, ..StripClip::new(StripEffect::Fill { color: Paint::WHITE, beats: 4.0 }) };
        let a = AudioBands::default();
        // At beat 1 the first strip is a quarter full; the last (1 beat late) is empty.
        assert_eq!(c.shade(px(0.1, 0), 1.0, &a, &P).a, 1.0);
        assert_eq!(c.shade(px(0.1, 3), 1.0, &a, &P).a, 0.0);
    }

    #[test]
    fn alternate_swaps_on_each_hit() {
        let c = StripClip::new(StripEffect::Alternate { color: Paint::WHITE, band: AudioBand::Kick });
        let mut a = AudioBands::default();
        let even = |a: &AudioBands| c.shade(px(0.5, 0), 0.0, a, &P).a;
        let before = even(&a);
        a.hits[AudioBand::Kick.hit_slot().unwrap()] += 1;
        assert_ne!(before, even(&a));
    }

    #[test]
    fn meter_follows_level() {
        let c = StripClip::new(StripEffect::Meter { color: Paint::Slot(1), band: AudioBand::Bass });
        let mut a = AudioBands::default();
        a.set(AudioBand::Bass, 0.5);
        assert_eq!(c.shade(px(0.2, 0), 0.0, &a, &P).a, 1.0);
        assert_eq!(c.shade(px(0.8, 0), 0.0, &a, &P).a, 0.0);
        assert_eq!(c.shade(px(0.2, 0), 0.0, &a, &P).g, 1.0); // palette colour 2
    }

    #[test]
    fn presets_serialise() {
        for (_, c) in StripClip::presets() {
            let j = serde_json::to_string(&c).unwrap();
            let back: StripClip = serde_json::from_str(&j).unwrap();
            assert_eq!(back, c);
        }
    }
}
