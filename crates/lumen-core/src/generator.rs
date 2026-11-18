use crate::audio::{AudioBand, AudioBands};
use crate::color::hsv_to_rgb;
use crate::frame::{Frame, Rgba};
use crate::palette::{Paint, Rgb};
use serde::{Deserialize, Serialize};

/// Procedural sources. `beat` is the (fractional) beat count since the
/// clip was triggered, so everything is BPM-synced by construction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Generator {
    Solid { color: Paint },
    /// Horizontal rainbow that scrolls once every `beats_per_cycle` beats.
    Rainbow { scale: f32, beats_per_cycle: f32 },
    /// Flashes `color` `per_beat` times per beat; transparent when off.
    Strobe { color: Paint, per_beat: f32, duty: f32 },
    /// A bar sweeping left to right; transparent background.
    Chase { color: Paint, width: f32, beats_per_cycle: f32 },
    /// Sine plasma.
    Plasma { scale: f32, speed: f32 },
    /// Solid colour whose alpha follows an audio band (e.g. white on every kick).
    AudioFlash { band: AudioBand, color: Paint },
    /// The four palette colours side by side, drifting slowly sideways.
    PaletteWash { beats_per_cycle: f32 },
    /// An effect that runs along each strip, worked out at output time.
    /// Draws nothing on the canvas.
    Strip(crate::strip::StripClip),
    /// Five frequency bands as vertical bars, each in its band colour
    /// (sub, bass, low mid, high mid, highs from left to right).
    Bands,
    /// A beam turning around the middle like a radar, with a fading tail.
    Radar { color: Paint, beats_per_turn: f32, tail: f32 },
    /// A bar bouncing from left to right and back.
    LeftRight { color: Paint, beats: f32, width: f32 },
    /// A line moving up and down.
    Scanner { color: Paint, beats: f32, width: f32 },
    /// Two bars meeting in the middle (inward) or leaving it (outward).
    Sweep { color: Paint, beats: f32, width: f32, inward: bool },
    /// Colour fills from the left, then clears from the left.
    Wipe { color: Paint, beats: f32 },
    /// Everything flashes on the beat and fades out (from the clock, so it
    /// lands exactly on time even without audio).
    BeatPulse { color: Paint, per_beat: f32, fade: f32 },
    /// Random spots flash and fade.
    Twinkle { color: Paint, density: f32, per_beat: f32 },
}

/// A small, fast hash for random-looking but repeatable patterns.
fn hash(a: u32, b: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Ping-pong 0..1..0 over one cycle, eased at the ends.
fn bounce(phase: f32) -> f32 {
    let x = 1.0 - (phase * 2.0 - 1.0).abs();
    x * x * (3.0 - 2.0 * x)
}

/// Brightness of a soft bar centred on `pos` at distance `d`.
fn bar(d: f32, half: f32) -> f32 {
    if d < half {
        1.0 - d / half
    } else {
        0.0
    }
}

fn lit(color: [f32; 3], k: f32) -> Rgba {
    if k <= 0.0 {
        Rgba::TRANSPARENT
    } else {
        let k = k.min(1.0);
        Rgba { r: color[0], g: color[1], b: color[2], a: k }
    }
}

impl Generator {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Generator::Solid { .. } => "Solid",
            Generator::Rainbow { .. } => "Rainbow",
            Generator::Strobe { .. } => "Strobe",
            Generator::Chase { .. } => "Chase",
            Generator::Plasma { .. } => "Plasma",
            Generator::AudioFlash { .. } => "Audio flash",
            Generator::Bands => "Bands",
            Generator::PaletteWash { .. } => "Palette wash",
            Generator::Strip(c) => c.name(),
            Generator::Radar { .. } => "Radar",
            Generator::LeftRight { .. } => "Left ↔ right",
            Generator::Scanner { .. } => "Up ↕ down",
            Generator::Sweep { inward: true, .. } => "Sweep in",
            Generator::Sweep { inward: false, .. } => "Sweep out",
            Generator::Wipe { .. } => "Wipe",
            Generator::BeatPulse { .. } => "Beat pulse",
            Generator::Twinkle { .. } => "Twinkle",
        }
    }

    /// The colour this effect paints with, if it has one (the effect browser
    /// keeps colour separate from the effect).
    pub fn paint_mut(&mut self) -> Option<&mut Paint> {
        match self {
            Generator::Solid { color }
            | Generator::Strobe { color, .. }
            | Generator::Chase { color, .. }
            | Generator::AudioFlash { color, .. }
            | Generator::Radar { color, .. }
            | Generator::LeftRight { color, .. }
            | Generator::Scanner { color, .. }
            | Generator::Sweep { color, .. }
            | Generator::Wipe { color, .. }
            | Generator::BeatPulse { color, .. }
            | Generator::Twinkle { color, .. } => Some(color),
            Generator::Strip(sc) => Some(sc.effect.paint_mut()),
            Generator::Rainbow { .. } | Generator::Plasma { .. } | Generator::PaletteWash { .. } | Generator::Bands => None,
        }
    }

    pub fn is_strip(&self) -> bool {
        matches!(self, Generator::Strip(_))
    }

    pub fn render(&self, frame: &mut Frame, beat: f64, audio: &AudioBands, palette: &[Rgb; 4]) {
        match *self {
            Generator::Strip(_) => {}
            Generator::PaletteWash { beats_per_cycle } => {
                let shift = (beat / beats_per_cycle.max(0.001) as f64).fract() as f32;
                frame.fill_with(|u, _| {
                    // smooth blend between neighbouring palette colours
                    let x = ((u + shift).fract()) * 4.0;
                    let i = x as usize % 4;
                    let j = (i + 1) % 4;
                    let k = x.fract();
                    let k = k * k * (3.0 - 2.0 * k);
                    let (a, b) = (palette[i], palette[j]);
                    Rgba::rgb(a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k)
                });
            }
            Generator::Solid { color } => {
                let color = color.resolve(palette);
                let c = Rgba::rgb(color[0], color[1], color[2]);
                frame.fill_with(|_, _| c);
            }
            Generator::Rainbow { scale, beats_per_cycle } => {
                let phase = (beat / beats_per_cycle.max(0.001) as f64).fract() as f32;
                frame.fill_with(|u, _| {
                    let [r, g, b] = hsv_to_rgb(u * scale - phase, 1.0, 1.0);
                    Rgba::rgb(r, g, b)
                });
            }
            Generator::Strobe { color, per_beat, duty } => {
                let color = color.resolve(palette);
                let on = ((beat * per_beat as f64).fract() as f32) < duty;
                let c = if on { Rgba::rgb(color[0], color[1], color[2]) } else { Rgba::TRANSPARENT };
                frame.fill_with(|_, _| c);
            }
            Generator::Chase { color, width, beats_per_cycle } => {
                let color = color.resolve(palette);
                let pos = (beat / beats_per_cycle.max(0.001) as f64).fract() as f32;
                let half = width.max(0.001) * 0.5;
                frame.fill_with(|u, _| {
                    // distance with wraparound
                    let d = (u - pos).abs();
                    let d = d.min(1.0 - d);
                    if d < half {
                        let k = 1.0 - d / half;
                        Rgba::rgb(color[0] * k, color[1] * k, color[2] * k)
                    } else {
                        Rgba::TRANSPARENT
                    }
                });
            }
            Generator::Plasma { scale, speed } => {
                let t = beat as f32 * speed;
                frame.fill_with(|u, v| {
                    let (x, y) = (u * scale, v * scale);
                    let s = (x * 6.0 + t).sin() + (y * 6.0 - t * 0.7).sin() + ((x + y) * 4.0 + t * 0.5).sin();
                    let [r, g, b] = hsv_to_rgb(s / 6.0 + 0.5, 0.9, 0.9);
                    Rgba::rgb(r, g, b)
                });
            }
            Generator::AudioFlash { band, color } => {
                let color = color.resolve(palette);
                let a = audio.get(band);
                let c = Rgba { r: color[0], g: color[1], b: color[2], a };
                frame.fill_with(|_, _| c);
            }
            Generator::Radar { color, beats_per_turn, tail } => {
                let c = color.resolve(palette);
                let turn = (beat / beats_per_turn.max(0.01) as f64).fract() as f32;
                let tail = tail.clamp(0.02, 1.0);
                frame.fill_with(|u, v| {
                    let a = ((v - 0.5).atan2(u - 0.5) / std::f32::consts::TAU + 1.0).fract();
                    // How far behind the beam this angle is (0 = on the beam).
                    let behind = (turn - a + 1.0).fract();
                    lit(c, 1.0 - behind / tail)
                });
            }
            Generator::LeftRight { color, beats, width } => {
                let c = color.resolve(palette);
                let pos = bounce((beat / beats.max(0.01) as f64).fract() as f32);
                let half = width.max(0.01) * 0.5;
                // Travel from just off one edge to just off the other.
                let pos = -half + pos * (1.0 + 2.0 * half);
                frame.fill_with(|u, _| lit(c, bar((u - pos).abs(), half)));
            }
            Generator::Scanner { color, beats, width } => {
                let c = color.resolve(palette);
                let pos = bounce((beat / beats.max(0.01) as f64).fract() as f32);
                let half = width.max(0.01) * 0.5;
                let pos = -half + pos * (1.0 + 2.0 * half);
                frame.fill_with(|_, v| lit(c, bar((v - pos).abs(), half)));
            }
            Generator::Sweep { color, beats, width, inward } => {
                let c = color.resolve(palette);
                let k = (beat / beats.max(0.01) as f64).fract() as f32;
                let half = width.max(0.01) * 0.5;
                // Distance from the middle: 0.5 + a bit at the edges, 0 in the middle.
                let travel = 0.5 + half;
                let pos = if inward { travel * (1.0 - k) } else { travel * k };
                frame.fill_with(|u, _| lit(c, bar(((u - 0.5).abs() - pos).abs(), half)));
            }
            Generator::Wipe { color, beats } => {
                let c = color.resolve(palette);
                let k = (beat / beats.max(0.01) as f64).fract() as f32 * 2.0;
                frame.fill_with(|u, _| {
                    let on = if k < 1.0 { u < k } else { u >= k - 1.0 };
                    lit(c, if on { 1.0 } else { 0.0 })
                });
            }
            Generator::BeatPulse { color, per_beat, fade } => {
                let c = color.resolve(palette);
                let since = (beat * per_beat.max(0.01) as f64).fract() as f32;
                let k = (-since / fade.max(0.02) * 3.0).exp();
                frame.fill_with(|_, _| lit(c, k));
            }
            Generator::Twinkle { color, density, per_beat } => {
                let c = color.resolve(palette);
                let t = beat * per_beat.max(0.01) as f64;
                let step = t.floor() as u32;
                let k = (t.fract() as f32 * 3.0).min(1.0);
                let fade = 1.0 - k;
                frame.fill_with(|u, v| {
                    // Spots on a 24 x 24 grid over the area.
                    let cell = ((u.clamp(0.0, 0.999) * 24.0) as u32) + ((v.clamp(0.0, 0.999) * 24.0) as u32) * 24;
                    if hash(cell, step) < density.clamp(0.0, 1.0) * 0.5 {
                        lit(c, fade)
                    } else {
                        Rgba::TRANSPARENT
                    }
                });
            }
            Generator::Bands => {
                const B: [AudioBand; 5] =
                    [AudioBand::Sub, AudioBand::Bass, AudioBand::LowMid, AudioBand::HighMid, AudioBand::High];
                frame.fill_with(|u, v| {
                    let i = ((u * 5.0) as usize).min(4);
                    let level = audio.get(B[i]);
                    if 1.0 - v <= level {
                        let [r, g, b] = B[i].color();
                        Rgba::rgb(r, g, b)
                    } else {
                        Rgba::TRANSPARENT
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const P: [Rgb; 4] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]];

    #[test]
    fn slot_colour_comes_from_palette() {
        let mut f = Frame::transparent(1, 1);
        Generator::Solid { color: Paint::Slot(2) }.render(&mut f, 0.0, &AudioBands::default(), &P);
        assert_eq!((f.pixels[0].r, f.pixels[0].b), (0.0, 1.0));
    }

    #[test]
    fn strobe_on_and_off() {
        let g = Generator::Strobe { color: Paint::Rgb([1.0, 1.0, 1.0]), per_beat: 1.0, duty: 0.5 };
        let mut f = Frame::transparent(2, 2);
        g.render(&mut f, 0.25, &AudioBands::default(), &P);
        assert_eq!(f.pixels[0].a, 1.0);
        g.render(&mut f, 0.75, &AudioBands::default(), &P);
        assert_eq!(f.pixels[0].a, 0.0);
    }

    #[test]
    fn chase_peaks_at_position() {
        let g = Generator::Chase { color: Paint::Rgb([1.0, 0.0, 0.0]), width: 0.2, beats_per_cycle: 4.0 };
        let mut f = Frame::transparent(100, 1);
        g.render(&mut f, 2.0, &AudioBands::default(), &P); // halfway across
        assert!(f.pixels[50].r > 0.9);
        assert_eq!(f.pixels[0].a, 0.0);
    }

    #[test]
    fn audio_flash_follows_band() {
        let g = Generator::AudioFlash { band: AudioBand::Kick, color: Paint::Rgb([1.0, 1.0, 1.0]) };
        let mut a = AudioBands::default();
        let mut f = Frame::transparent(1, 1);
        g.render(&mut f, 0.0, &a, &P);
        assert_eq!(f.pixels[0].a, 0.0);
        a.set(AudioBand::Kick, 0.8);
        g.render(&mut f, 0.0, &a, &P);
        assert!((f.pixels[0].a - 0.8).abs() < 1e-6);
    }

    fn lit_at(g: &Generator, beat: f64, w: usize, h: usize) -> Vec<f32> {
        let mut f = Frame::transparent(w, h);
        g.render(&mut f, beat, &AudioBands::default(), &P);
        f.pixels.iter().map(|p| p.a).collect()
    }

    fn brightest(a: &[f32]) -> usize {
        a.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).map(|(i, _)| i).unwrap()
    }

    #[test]
    fn left_right_bounces() {
        let g = Generator::LeftRight { color: Paint::Slot(0), beats: 4.0, width: 0.1 };
        assert!(brightest(&lit_at(&g, 0.4, 100, 1)) < 20);
        assert!(brightest(&lit_at(&g, 2.0, 100, 1)) > 80); // far end at half time
        assert!(brightest(&lit_at(&g, 3.6, 100, 1)) < 20); // and back
    }

    #[test]
    fn scanner_moves_down() {
        let g = Generator::Scanner { color: Paint::Slot(0), beats: 4.0, width: 0.1 };
        assert!(brightest(&lit_at(&g, 2.0, 1, 100)) > 80);
    }

    #[test]
    fn sweep_in_and_out() {
        let inward = Generator::Sweep { color: Paint::Slot(0), beats: 4.0, width: 0.1, inward: true };
        let outward = Generator::Sweep { color: Paint::Slot(0), beats: 4.0, width: 0.1, inward: false };
        // Early on: inward is at the edges, outward is in the middle.
        let a = lit_at(&inward, 0.4, 100, 1);
        assert!(a[2] > 0.3 && a[97] > 0.3 && a[50] == 0.0);
        let b = lit_at(&outward, 0.4, 100, 1);
        assert!(b[45] > 0.3 && b[54] > 0.3 && b[2] == 0.0);
    }

    #[test]
    fn wipe_fills_then_clears() {
        let g = Generator::Wipe { color: Paint::Slot(0), beats: 4.0 };
        let a = lit_at(&g, 1.0, 100, 1); // a quarter: left half lit
        assert!(a[10] == 1.0 && a[90] == 0.0);
        let b = lit_at(&g, 3.0, 100, 1); // three quarters: left half cleared
        assert!(b[10] == 0.0 && b[90] == 1.0);
    }

    #[test]
    fn beat_pulse_on_the_beat() {
        let g = Generator::BeatPulse { color: Paint::Slot(0), per_beat: 1.0, fade: 0.5 };
        assert!(lit_at(&g, 4.01, 1, 1)[0] > 0.9);
        assert!(lit_at(&g, 4.6, 1, 1)[0] < 0.1);
    }

    #[test]
    fn radar_turns() {
        let g = Generator::Radar { color: Paint::Slot(0), beats_per_turn: 4.0, tail: 0.25 };
        let a = lit_at(&g, 0.0, 41, 41);
        let b = lit_at(&g, 2.0, 41, 41);
        assert_ne!(brightest(&a), brightest(&b));
    }

    #[test]
    fn twinkle_lights_some_spots() {
        let g = Generator::Twinkle { color: Paint::Slot(0), density: 0.5, per_beat: 1.0 };
        let a = lit_at(&g, 3.05, 48, 48);
        let n = a.iter().filter(|&&x| x > 0.5).count();
        assert!(n > 48 * 48 / 10 && n < 48 * 48 / 2, "{n}");
    }

    #[test]
    fn area_aims_an_effect_at_part_of_the_canvas() {
        // Left-right bar at the far end of its run, inside the right half only.
        let g = Generator::LeftRight { color: Paint::Slot(0), beats: 4.0, width: 0.1 };
        let mut f = Frame::transparent(100, 1);
        f.area = [0.5, 0.0, 1.0, 1.0];
        g.render(&mut f, 0.3, &AudioBands::default(), &P);
        let i = brightest(&f.pixels.iter().map(|p| p.a).collect::<Vec<_>>());
        assert!((48..60).contains(&i), "{i}");
    }

    #[test]
    fn bands_bar_height() {
        let mut a = AudioBands::default();
        a.set(AudioBand::Bass, 0.5);
        let mut f = Frame::transparent(5, 10);
        Generator::Bands.render(&mut f, 0.0, &a, &P);
        // bass column = x 1; lower half lit, upper half empty; sub column empty
        assert_eq!(f.get(1, 9).a, 1.0);
        assert_eq!(f.get(1, 2).a, 0.0);
        assert_eq!(f.get(0, 9).a, 0.0);
    }
}
