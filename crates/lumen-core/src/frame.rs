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
    /// The part of the canvas that generators treat as their whole 0..1
    /// space (x0, y0, x1, y1). Normally the full canvas; a layer aimed at a
    /// group of fixtures uses the group's bounding box, so a sweep runs
    /// across that group instead of the whole stage.
    pub area: [f32; 4],
}

impl Frame {
    pub fn new(width: usize, height: usize, fill: Rgba) -> Self {
        Frame { width, height, pixels: vec![fill; width * height], area: [0.0, 0.0, 1.0, 1.0] }
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

    /// Fill every pixel from `f(u, v)` where u,v are pixel-centre coordinates
    /// in 0..1 across `area` (outside it they run past 0..1).
    pub fn fill_with(&mut self, mut f: impl FnMut(f32, f32) -> Rgba) {
        let (w, h) = (self.width, self.height);
        let [x0, y0, x1, y1] = self.area;
        let (aw, ah) = ((x1 - x0).max(1e-4), (y1 - y0).max(1e-4));
        for y in 0..h {
            for x in 0..w {
                let u = ((x as f32 + 0.5) / w as f32 - x0) / aw;
                let v = ((y as f32 + 0.5) / h as f32 - y0) / ah;
                self.pixels[y * w + x] = f(u, v);
            }
        }
    }
}
