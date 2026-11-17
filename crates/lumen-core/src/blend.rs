use crate::frame::{Frame, Rgba};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    Normal,
    Add,
    Multiply,
    Screen,
    Subtract,
}

impl BlendMode {
    pub const ALL: [BlendMode; 5] =
        [BlendMode::Normal, BlendMode::Add, BlendMode::Multiply, BlendMode::Screen, BlendMode::Subtract];

    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Add => "Add",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Subtract => "Subtract",
        }
    }

    fn channel(self, base: f32, src: f32) -> f32 {
        match self {
            BlendMode::Normal => src,
            BlendMode::Add => base + src,
            BlendMode::Multiply => base * src,
            BlendMode::Screen => 1.0 - (1.0 - base) * (1.0 - src),
            BlendMode::Subtract => base - src,
        }
    }
}

/// Composite `src` over `base` in place. Effective alpha = src.a * opacity.
pub fn composite(base: &mut Frame, src: &Frame, mode: BlendMode, opacity: f32) {
    debug_assert_eq!(base.pixels.len(), src.pixels.len());
    for (b, s) in base.pixels.iter_mut().zip(&src.pixels) {
        let a = (s.a * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            continue;
        }
        let mix = |bc: f32, sc: f32| {
            let blended = mode.channel(bc, sc).clamp(0.0, 1.0);
            bc + (blended - bc) * a
        };
        *b = Rgba { r: mix(b.r, s.r), g: mix(b.g, s.g), b: mix(b.b, s.b), a: 1.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(r: f32, g: f32, b: f32) -> Frame {
        Frame::new(1, 1, Rgba::rgb(r, g, b))
    }

    #[test]
    fn add_clamps() {
        let mut base = px(0.7, 0.0, 0.0);
        composite(&mut base, &px(0.7, 0.5, 0.0), BlendMode::Add, 1.0);
        assert_eq!(base.pixels[0].r, 1.0);
        assert_eq!(base.pixels[0].g, 0.5);
    }

    #[test]
    fn opacity_halves_normal() {
        let mut base = px(0.0, 0.0, 0.0);
        composite(&mut base, &px(1.0, 1.0, 1.0), BlendMode::Normal, 0.5);
        assert!((base.pixels[0].r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn transparent_source_is_noop() {
        let mut base = px(0.3, 0.3, 0.3);
        let src = Frame::transparent(1, 1);
        composite(&mut base, &src, BlendMode::Normal, 1.0);
        assert_eq!(base.pixels[0], Rgba::rgb(0.3, 0.3, 0.3));
    }
}
