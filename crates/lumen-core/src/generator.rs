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
