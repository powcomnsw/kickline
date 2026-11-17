use crate::audio::{AudioBand, AudioBands};
use crate::color::{hsv_to_rgb, rgb_to_hsv};
use crate::frame::Frame;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Brightness(f32),
    /// Hue rotation, 0..1 = a full turn.
    HueShift(f32),
    Invert,
    /// Brightness pulse locked to the beat.
    Pulse { depth: f32, per_beat: f32 },
    /// Brightness follows an audio band (amount 0 = no effect, 1 = fully driven).
    AudioBrightness { band: AudioBand, amount: f32 },
    /// Hue rotates by `amount` turns at full band level.
    AudioHue { band: AudioBand, amount: f32 },
}

impl Effect {
    pub fn name(&self) -> &'static str {
        match self {
            Effect::Brightness(_) => "Brightness",
            Effect::HueShift(_) => "Hue shift",
            Effect::Invert => "Invert",
            Effect::Pulse { .. } => "Pulse",
            Effect::AudioBrightness { .. } => "Audio brightness",
            Effect::AudioHue { .. } => "Audio hue",
        }
    }

    pub fn apply(&self, frame: &mut Frame, beat: f64, audio: &AudioBands) {
        match *self {
            Effect::AudioBrightness { band, amount } => {
                let k = crate::audio::AudioLink { band, amount }.factor(audio);
                Effect::Brightness(k).apply(frame, beat, audio);
            }
            Effect::AudioHue { band, amount } => {
                Effect::HueShift(amount * audio.get(band)).apply(frame, beat, audio);
            }
            Effect::Brightness(k) => {
                for p in &mut frame.pixels {
                    p.r *= k;
                    p.g *= k;
                    p.b *= k;
                }
            }
            Effect::HueShift(shift) => {
                for p in &mut frame.pixels {
                    let [h, s, v] = rgb_to_hsv(p.r, p.g, p.b);
                    let [r, g, b] = hsv_to_rgb(h + shift, s, v);
                    p.r = r;
                    p.g = g;
                    p.b = b;
                }
            }
            Effect::Invert => {
                for p in &mut frame.pixels {
                    p.r = 1.0 - p.r;
                    p.g = 1.0 - p.g;
                    p.b = 1.0 - p.b;
                }
            }
            Effect::Pulse { depth, per_beat } => {
                let phase = (beat * per_beat as f64).fract() as f32;
                // 1 at the start of each pulse, decaying to (1 - depth)
                let k = 1.0 - depth * phase;
                for p in &mut frame.pixels {
                    p.r *= k;
                    p.g *= k;
                    p.b *= k;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Rgba;

    #[test]
    fn hue_shift_red_to_green() {
        let mut f = Frame::new(1, 1, Rgba::rgb(1.0, 0.0, 0.0));
        Effect::HueShift(1.0 / 3.0).apply(&mut f, 0.0, &AudioBands::default());
        let p = f.pixels[0];
        assert!(p.g > 0.99 && p.r < 0.01);
    }

    #[test]
    fn invert_twice_identity() {
        let mut f = Frame::new(1, 1, Rgba::rgb(0.2, 0.4, 0.6));
        Effect::Invert.apply(&mut f, 0.0, &AudioBands::default());
        Effect::Invert.apply(&mut f, 0.0, &AudioBands::default());
        assert!((f.pixels[0].g - 0.4).abs() < 1e-6);
    }

    #[test]
    fn audio_brightness_scales() {
        let mut a = AudioBands::default();
        a.set(AudioBand::Bass, 0.5);
        let mut f = Frame::new(1, 1, Rgba::rgb(1.0, 1.0, 1.0));
        Effect::AudioBrightness { band: AudioBand::Bass, amount: 1.0 }.apply(&mut f, 0.0, &a);
        assert!((f.pixels[0].r - 0.5).abs() < 1e-6);
        let mut f = Frame::new(1, 1, Rgba::rgb(1.0, 1.0, 1.0));
        Effect::AudioBrightness { band: AudioBand::Bass, amount: 0.0 }.apply(&mut f, 0.0, &a);
        assert_eq!(f.pixels[0].r, 1.0);
    }
}
