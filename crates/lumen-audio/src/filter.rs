//! RBJ biquad filters, Direct Form I.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn w0_alpha(fs: f32, f: f32, q: f32) -> (f32, f32) {
        let f = f.clamp(1.0, fs * 0.49);
        let w0 = 2.0 * PI * f / fs;
        (w0, w0.sin() / (2.0 * q))
    }

    pub fn lowpass(fs: f32, f: f32, q: f32) -> Self {
        let (w0, alpha) = Self::w0_alpha(fs, f, q);
        let c = w0.cos();
        Self::from_coeffs((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn highpass(fs: f32, f: f32, q: f32) -> Self {
        let (w0, alpha) = Self::w0_alpha(fs, f, q);
        let c = w0.cos();
        Self::from_coeffs((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        // flush denormals
        self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
        self.y2 = if self.y2.abs() < 1e-20 { 0.0 } else { self.y2 };
        self.y1
    }
}

/// A chain of biquads applied in series.
#[derive(Clone, Debug, Default)]
pub struct Chain(pub Vec<Biquad>);

impl Chain {
    /// 4th-order Butterworth-ish low-pass (two cascaded 2nd-order sections).
    pub fn lp4(fs: f32, f: f32) -> Self {
        Chain(vec![Biquad::lowpass(fs, f, 0.541), Biquad::lowpass(fs, f, 1.307)])
    }

    pub fn hp4(fs: f32, f: f32) -> Self {
        Chain(vec![Biquad::highpass(fs, f, 0.541), Biquad::highpass(fs, f, 1.307)])
    }

    /// Band-pass made from a 4th-order high-pass at `lo` and low-pass at `hi`.
    pub fn band(fs: f32, lo: f32, hi: f32) -> Self {
        let mut c = Self::hp4(fs, lo);
        c.0.extend(Self::lp4(fs, hi).0);
        c
    }

    #[inline]
    pub fn process(&mut self, mut x: f32) -> f32 {
        for f in &mut self.0 {
            x = f.process(x);
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(mut c: Chain, fs: f32, freq: f32) -> f32 {
        let n = (fs * 0.5) as usize;
        let mut peak = 0.0f32;
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f32 / fs).sin();
            let y = c.process(x);
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn lowpass_passes_low_blocks_high() {
        let fs = 48000.0;
        assert!(gain(Chain::lp4(fs, 200.0), fs, 50.0) > 0.9);
        assert!(gain(Chain::lp4(fs, 200.0), fs, 2000.0) < 0.02);
    }

    #[test]
    fn band_is_centred() {
        let fs = 48000.0;
        assert!(gain(Chain::band(fs, 250.0, 2000.0), fs, 700.0) > 0.8);
        assert!(gain(Chain::band(fs, 250.0, 2000.0), fs, 50.0) < 0.05);
        assert!(gain(Chain::band(fs, 250.0, 2000.0), fs, 10000.0) < 0.05);
    }
}
