#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Rgba { r, g, b, a: 1.0 }
    }

    /// Quantise to 8-bit RGB, ignoring alpha.
    pub fn to_u8(self) -> [u8; 3] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [q(self.r), q(self.g), q(self.b)]
    }
}

/// A CPU framebuffer. Row-major, origin top-left.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgba>,
}

impl Frame {
    pub fn new(width: usize, height: usize, fill: Rgba) -> Self {
        Frame { width, height, pixels: vec![fill; width * height] }
    }

    pub fn transparent(width: usize, height: usize) -> Self {
        Self::new(width, height, Rgba::TRANSPARENT)
    }

    pub fn black(width: usize, height: usize) -> Self {
        Self::new(width, height, Rgba::BLACK)
    }

    pub fn get(&self, x: usize, y: usize) -> Rgba {
        self.pixels[y * self.width + x]
    }

    /// Nearest-neighbour sample at normalised coordinates (0..1, clamped).
    pub fn sample(&self, u: f32, v: f32) -> Rgba {
        let x = ((u.clamp(0.0, 1.0) * self.width as f32) as usize).min(self.width - 1);
        let y = ((v.clamp(0.0, 1.0) * self.height as f32) as usize).min(self.height - 1);
        self.get(x, y)
    }

    /// Fill every pixel from `f(u, v)` where u,v are pixel-centre coordinates in 0..1.
    pub fn fill_with(&mut self, mut f: impl FnMut(f32, f32) -> Rgba) {
        let (w, h) = (self.width, self.height);
        for y in 0..h {
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let v = (y as f32 + 0.5) / h as f32;
                self.pixels[y * w + x] = f(u, v);
            }
        }
    }
}
