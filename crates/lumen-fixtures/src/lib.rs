//! Advanced Output: lumiverses, fixture types (the Fixture Editor) and
//! fixtures placed on the composition, mapped to DMX.
//!
//! Mirrors Resolume Arena's DMX output model:
//! - a **Lumiverse** is a DMX universe with a protocol and a target IP;
//! - a **Fixture type** defines a grid of pixels and the channels per pixel;
//! - a **Fixture** is a type placed on the canvas (position, size, rotation)
//!   and patched to a lumiverse at a start address.

use lumen_core::{Frame, Rgba};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const UNIVERSE_SIZE: usize = 512;

/// What one DMX channel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelFn {
    Red,
    Green,
    Blue,
    White,
    Amber,
    /// Intensity. When a pixel has a dimmer, the colour channels are
    /// normalised so the dimmer carries the brightness.
    Dimmer,
    /// Constant value (strobe off, mode select, macro 0...).
    Fixed(u8),
}

impl ChannelFn {
    pub const CHOICES: [ChannelFn; 7] = [
        ChannelFn::Red,
        ChannelFn::Green,
        ChannelFn::Blue,
        ChannelFn::White,
        ChannelFn::Amber,
        ChannelFn::Dimmer,
        ChannelFn::Fixed(0),
    ];

    pub fn name(self) -> &'static str {
        match self {
            ChannelFn::Red => "Red",
            ChannelFn::Green => "Green",
            ChannelFn::Blue => "Blue",
            ChannelFn::White => "White",
            ChannelFn::Amber => "Amber",
            ChannelFn::Dimmer => "Dimmer",
            ChannelFn::Fixed(_) => "Fixed",
        }
    }

    pub fn same_kind(self, other: ChannelFn) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// Parse a colour order like "GRB" or "RGBW" into per-pixel channels.
pub fn color_order(s: &str) -> Vec<ChannelFn> {
    s.chars()
        .filter_map(|c| match c.to_ascii_uppercase() {
            'R' => Some(ChannelFn::Red),
            'G' => Some(ChannelFn::Green),
            'B' => Some(ChannelFn::Blue),
            'W' => Some(ChannelFn::White),
            'A' => Some(ChannelFn::Amber),
            'D' => Some(ChannelFn::Dimmer),
            _ => None,
        })
        .collect()
}

pub const COLOR_ORDERS: [&str; 10] = ["RGB", "RBG", "GRB", "GBR", "BRG", "BGR", "RGBW", "GRBW", "DRGB", "RGBWA"];

/// A fixture definition, as made in the Fixture Editor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureType {
    pub name: String,
    pub columns: usize,
    pub rows: usize,
    /// Channels repeated for every pixel.
    pub pixel: Vec<ChannelFn>,
    /// Channels sent once before the pixels (e.g. strobe, mode).
    #[serde(default)]
    pub header: Vec<ChannelFn>,
}

impl FixtureType {
    pub fn new(name: &str, columns: usize, rows: usize, order: &str) -> Self {
        FixtureType { name: name.into(), columns, rows, pixel: color_order(order), header: Vec::new() }
    }

    pub fn pixel_count(&self) -> usize {
        self.columns.max(1) * self.rows.max(1)
    }

    pub fn footprint(&self) -> usize {
        self.header.len() + self.pixel_count() * self.pixel.len()
    }

    /// The colour order string if the pixel is pure colour/dimmer channels.
    pub fn order_string(&self) -> String {
        self.pixel
            .iter()
            .map(|c| match c {
                ChannelFn::Red => 'R',
                ChannelFn::Green => 'G',
                ChannelFn::Blue => 'B',
                ChannelFn::White => 'W',
                ChannelFn::Amber => 'A',
                ChannelFn::Dimmer => 'D',
                ChannelFn::Fixed(_) => 'F',
            })
            .collect()
    }

    fn encode_pixel(&self, c: Rgba, out: &mut Vec<u8>) {
        let (mut r, mut g, mut b) = (c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
        let has_dimmer = self.pixel.contains(&ChannelFn::Dimmer);
        let max = r.max(g).max(b);
        if has_dimmer && max > 0.0 {
            r /= max;
            g /= max;
            b /= max;
        }
        let has_white = self.pixel.contains(&ChannelFn::White);
        let white = if has_white { r.min(g).min(b) } else { 0.0 };
        if has_white {
            r -= white;
            g -= white;
            b -= white;
        }
        let has_amber = self.pixel.contains(&ChannelFn::Amber);
        // Amber takes what red and green share after white is removed.
        let amber = if has_amber { r.min(g * 2.0) * 0.5 } else { 0.0 };
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        for ch in &self.pixel {
            out.push(match *ch {
                ChannelFn::Red => q(r),
                ChannelFn::Green => q(g),
                ChannelFn::Blue => q(b),
                ChannelFn::White => q(white),
                ChannelFn::Amber => q(amber),
                ChannelFn::Dimmer => q(max),
                ChannelFn::Fixed(v) => v,
            });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    ArtNet,
    Sacn,
}

impl Protocol {
    pub fn name(self) -> &'static str {
        match self {
            Protocol::ArtNet => "Art-Net",
            Protocol::Sacn => "sACN",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lumiverse {
    pub name: String,
    pub protocol: Protocol,
    /// Art-Net destination IP (unicast or broadcast). Unused for sACN multicast.
    pub target: String,
    /// Art-Net port-address, 0-based. sACN universe = this + 1.
    pub universe: u16,
    pub enabled: bool,
}

impl Lumiverse {
    pub fn new(name: &str, universe: u16) -> Self {
        Lumiverse { name: name.into(), protocol: Protocol::ArtNet, target: "255.255.255.255".into(), universe, enabled: true }
    }
}

/// A fixture placed on the canvas. Coordinates are normalised (0..1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub type_id: usize,
    pub lumiverse: usize,
    /// 1-based start address.
    pub address: u16,
    /// Centre.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Degrees, clockwise.
    pub rotation: f32,
    /// Reverse every other row (zig-zag wired matrices).
    pub snake: bool,
    /// Reverse pixel order.
    #[serde(default)]
    pub reverse: bool,
}

impl Fixture {
    fn rot(&self) -> (f32, f32) {
        let a = self.rotation.to_radians();
        (a.cos(), a.sin())
    }

    /// Local (-0.5..0.5) -> canvas. `aspect` = canvas width / height; rotation
    /// happens in square (pixel) space so shapes don't skew on wide canvases.
    pub fn to_canvas(&self, aspect: f32, lx: f32, ly: f32) -> (f32, f32) {
        let (c, s) = self.rot();
        let (dx, dy) = (lx * self.w * aspect, ly * self.h);
        let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
        (self.x + rx / aspect, self.y + ry)
    }

    /// Canvas -> local (-0.5..0.5 inside).
    pub fn to_local(&self, aspect: f32, u: f32, v: f32) -> (f32, f32) {
        let (c, s) = self.rot();
        let (dx, dy) = ((u - self.x) * aspect, v - self.y);
        let (rx, ry) = (dx * c + dy * s, -dx * s + dy * c);
        (rx / (self.w * aspect).max(1e-6), ry / self.h.max(1e-6))
    }

    pub fn contains(&self, aspect: f32, u: f32, v: f32) -> bool {
        let (lx, ly) = self.to_local(aspect, u, v);
        lx.abs() <= 0.5 && ly.abs() <= 0.5
    }

    pub fn corners(&self, aspect: f32) -> [(f32, f32); 4] {
        [
            self.to_canvas(aspect, -0.5, -0.5),
            self.to_canvas(aspect, 0.5, -0.5),
            self.to_canvas(aspect, 0.5, 0.5),
            self.to_canvas(aspect, -0.5, 0.5),
        ]
    }

    /// Pixel centres in DMX order, in local coordinates.
    pub fn pixel_local(&self, t: &FixtureType) -> Vec<(f32, f32)> {
        let (cols, rows) = (t.columns.max(1), t.rows.max(1));
        let mut out = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                let c = if self.snake && r % 2 == 1 { cols - 1 - c } else { c };
                out.push(((c as f32 + 0.5) / cols as f32 - 0.5, (r as f32 + 0.5) / rows as f32 - 0.5));
            }
        }
        if self.reverse {
            out.reverse();
        }
        out
    }

    pub fn pixel_positions(&self, aspect: f32, t: &FixtureType) -> Vec<(f32, f32)> {
        self.pixel_local(t).into_iter().map(|(x, y)| self.to_canvas(aspect, x, y)).collect()
    }

    /// Average the frame over this pixel's cell (3x3 supersampling).
    pub fn sample_cell(&self, aspect: f32, t: &FixtureType, frame: &Frame, lx: f32, ly: f32) -> Rgba {
        let (cw, ch) = (1.0 / t.columns.max(1) as f32, 1.0 / t.rows.max(1) as f32);
        let mut acc = Rgba::TRANSPARENT;
        for j in 0..3 {
            for i in 0..3 {
                let ox = (i as f32 - 1.0) / 3.0 * cw;
                let oy = (j as f32 - 1.0) / 3.0 * ch;
                let (u, v) = self.to_canvas(aspect, lx + ox, ly + oy);
                let p = frame.sample(u, v);
                acc.r += p.r;
                acc.g += p.g;
                acc.b += p.b;
            }
        }
        Rgba::rgb(acc.r / 9.0, acc.g / 9.0, acc.b / 9.0)
    }
}

/// Where a run of channels lands. `universe` is an offset from the
/// lumiverse's own universe (spill-over goes to +1, +2, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub universe_offset: u16,
    /// 0-based.
    pub start: usize,
    pub len: usize,
}

fn default_aspect() -> f32 {
    16.0 / 9.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdvancedOutput {
    /// Composition width / height. Keep in sync with the composition.
    #[serde(default = "default_aspect")]
    pub aspect: f32,
    pub lumiverses: Vec<Lumiverse>,
    pub types: Vec<FixtureType>,
    pub fixtures: Vec<Fixture>,
}

impl Default for AdvancedOutput {
    fn default() -> Self {
        AdvancedOutput { aspect: default_aspect(), lumiverses: Vec::new(), types: Vec::new(), fixtures: Vec::new() }
    }
}

impl AdvancedOutput {
    pub fn demo() -> Self {
        let mut o = AdvancedOutput::default();
        o.lumiverses.push(Lumiverse::new("Strips", 0));
        o.lumiverses.push(Lumiverse::new("Pars", 2));
        o.types.push(FixtureType::new("Pixel strip 60 RGB", 60, 1, "RGB"));
        o.types.push(FixtureType::new("RGBW par", 1, 1, "RGBW"));
        o.types.push(FixtureType::new("Matrix 8x8 GRB", 8, 8, "GRB"));
        let strip = |name: &str, y: f32, address: u16| Fixture {
            name: name.into(),
            type_id: 0,
            lumiverse: 0,
            address,
            x: 0.5,
            y,
            w: 0.9,
            h: 0.04,
            rotation: 0.0,
            snake: false,
            reverse: false,
        };
        o.fixtures.push(strip("Strip top", 0.62, 1));
        o.fixtures.push(strip("Strip bottom", 0.78, 181));
        for i in 0..4 {
            o.fixtures.push(Fixture {
                name: format!("Par {}", i + 1),
                type_id: 1,
                lumiverse: 1,
                address: 1 + i as u16 * 4,
                x: 0.14 + 0.24 * i as f32,
                y: 0.15,
                w: 0.1,
                h: 0.16,
                rotation: 0.0,
                snake: false,
                reverse: false,
            });
        }
        o.fixtures.push(Fixture {
            name: "Matrix".into(),
            type_id: 2,
            lumiverse: 1,
            address: 17,
            x: 0.5,
            y: 0.38,
            w: 0.22,
            h: 0.3,
            rotation: 0.0,
            snake: true,
            reverse: false,
        });
        o
    }

    /// Where each pixel (and the header) of a fixture lands. Header and each
    /// pixel are never split across universes: if one doesn't fit, it moves
    /// to channel 1 of the next universe.
    pub fn layout(&self, fx: &Fixture) -> Vec<Segment> {
        let Some(t) = self.types.get(fx.type_id) else { return Vec::new() };
        let mut segs = Vec::new();
        let mut uni = 0u16;
        let mut addr = (fx.address.max(1) - 1) as usize;
        let mut place = |len: usize, segs: &mut Vec<Segment>| {
            if len == 0 || len > UNIVERSE_SIZE {
                return;
            }
            if addr + len > UNIVERSE_SIZE {
                uni += 1;
                addr = 0;
            }
            segs.push(Segment { universe_offset: uni, start: addr, len });
            addr += len;
        };
        place(t.header.len(), &mut segs);
        for _ in 0..t.pixel_count() {
            place(t.pixel.len(), &mut segs);
        }
        segs
    }

    /// Render DMX for every lumiverse in use. Key = (lumiverse index, absolute universe).
    pub fn render_dmx(&self, frame: &Frame) -> BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]> {
        let mut out: BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]> = BTreeMap::new();
        let mut bytes = Vec::new();
        for fx in &self.fixtures {
            let (Some(t), Some(lv)) = (self.types.get(fx.type_id), self.lumiverses.get(fx.lumiverse)) else { continue };
            let segs = self.layout(fx);
            let mut seg_iter = segs.iter();
            bytes.clear();
            if !t.header.is_empty() {
                let Some(seg) = seg_iter.next() else { continue };
                let header: Vec<u8> = t
                    .header
                    .iter()
                    .map(|c| match c {
                        ChannelFn::Fixed(v) => *v,
                        ChannelFn::Dimmer => 255,
                        _ => 0,
                    })
                    .collect();
                write(&mut out, fx.lumiverse, lv.universe + seg.universe_offset, seg.start, &header);
            }
            for (lx, ly) in fx.pixel_local(t) {
                let Some(seg) = seg_iter.next() else { break };
                bytes.clear();
                t.encode_pixel(fx.sample_cell(self.aspect, t, frame, lx, ly), &mut bytes);
                write(&mut out, fx.lumiverse, lv.universe + seg.universe_offset, seg.start, &bytes);
            }
        }
        out
    }

    /// Pairs of fixtures whose channels overlap.
    pub fn conflicts(&self) -> Vec<(usize, usize)> {
        let mut used: BTreeMap<(usize, u16, usize), usize> = BTreeMap::new();
        let mut out = Vec::new();
        for (i, fx) in self.fixtures.iter().enumerate() {
            let Some(lv) = self.lumiverses.get(fx.lumiverse) else { continue };
            for s in self.layout(fx) {
                for ch in s.start..s.start + s.len {
                    if let Some(&j) = used.get(&(fx.lumiverse, lv.universe + s.universe_offset, ch)) {
                        if j != i && !out.contains(&(j, i)) {
                            out.push((j, i));
                        }
                    } else {
                        used.insert((fx.lumiverse, lv.universe + s.universe_offset, ch), i);
                    }
                }
            }
        }
        out
    }

    /// Human-readable patch range, e.g. "U0 1-180" or "U0 1-510, U1 1-30".
    pub fn range_label(&self, fx: &Fixture) -> String {
        let Some(lv) = self.lumiverses.get(fx.lumiverse) else { return "no lumiverse".into() };
        let mut by_uni: BTreeMap<u16, (usize, usize)> = BTreeMap::new();
        for s in self.layout(fx) {
            let e = by_uni.entry(lv.universe + s.universe_offset).or_insert((s.start, s.start + s.len));
            e.0 = e.0.min(s.start);
            e.1 = e.1.max(s.start + s.len);
        }
        by_uni.iter().map(|(u, (a, b))| format!("U{u} {}-{}", a + 1, b)).collect::<Vec<_>>().join(", ")
    }

    /// First free address after the last fixture on a lumiverse.
    pub fn next_free_address(&self, lumiverse: usize) -> u16 {
        let mut end = 0usize;
        for fx in self.fixtures.iter().filter(|f| f.lumiverse == lumiverse) {
            for s in self.layout(fx) {
                end = end.max(s.universe_offset as usize * UNIVERSE_SIZE + s.start + s.len);
            }
        }
        (end % UNIVERSE_SIZE + 1) as u16
    }

    pub fn type_in_use(&self, type_id: usize) -> bool {
        self.fixtures.iter().any(|f| f.type_id == type_id)
    }

    /// Remove a type; returns false if a fixture still uses it.
    pub fn remove_type(&mut self, type_id: usize) -> bool {
        if self.type_in_use(type_id) || type_id >= self.types.len() {
            return false;
        }
        self.types.remove(type_id);
        for f in &mut self.fixtures {
            if f.type_id > type_id {
                f.type_id -= 1;
            }
        }
        true
    }

    /// Remove a lumiverse; fixtures on it move to lumiverse 0. Keeps at least one.
    pub fn remove_lumiverse(&mut self, idx: usize) -> bool {
        if self.lumiverses.len() <= 1 || idx >= self.lumiverses.len() {
            return false;
        }
        self.lumiverses.remove(idx);
        for f in &mut self.fixtures {
            if f.lumiverse == idx {
                f.lumiverse = 0;
            } else if f.lumiverse > idx {
                f.lumiverse -= 1;
            }
        }
        true
    }
}

fn write(out: &mut BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]>, lv: usize, uni: u16, start: usize, data: &[u8]) {
    let buf = out.entry((lv, uni)).or_insert([0; UNIVERSE_SIZE]);
    buf[start..start + data.len()].copy_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(t: FixtureType, fx: Fixture) -> AdvancedOutput {
        AdvancedOutput { aspect: 1.0, lumiverses: vec![Lumiverse::new("L", 3)], types: vec![t], fixtures: vec![fx] }
    }

    fn fx(address: u16) -> Fixture {
        Fixture {
            name: "f".into(),
            type_id: 0,
            lumiverse: 0,
            address,
            x: 0.5,
            y: 0.5,
            w: 1.0,
            h: 1.0,
            rotation: 0.0,
            snake: false,
            reverse: false,
        }
    }

    #[test]
    fn rgbw_par_extracts_white() {
        let o = one(FixtureType::new("p", 1, 1, "RGBW"), fx(10));
        let d = o.render_dmx(&Frame::new(4, 4, Rgba::rgb(1.0, 1.0, 0.5)));
        let u = d[&(0, 3)];
        assert_eq!(&u[9..13], &[128, 128, 0, 128]);
    }

    #[test]
    fn grb_order() {
        let o = one(FixtureType::new("s", 1, 1, "GRB"), fx(1));
        let d = o.render_dmx(&Frame::new(1, 1, Rgba::rgb(1.0, 0.0, 0.2)));
        assert_eq!(&d[&(0, 3)][0..3], &[0, 255, 51]);
    }

    #[test]
    fn dimmer_carries_brightness() {
        let o = one(FixtureType::new("d", 1, 1, "DRGB"), fx(1));
        let d = o.render_dmx(&Frame::new(1, 1, Rgba::rgb(0.0, 0.4, 0.2)));
        assert_eq!(&d[&(0, 3)][0..4], &[102, 0, 255, 128]);
    }

    #[test]
    fn header_channels_come_first() {
        let mut t = FixtureType::new("h", 1, 1, "RGB");
        t.header = vec![ChannelFn::Fixed(42), ChannelFn::Dimmer];
        let o = one(t, fx(1));
        let d = o.render_dmx(&Frame::new(1, 1, Rgba::rgb(1.0, 0.0, 0.0)));
        assert_eq!(&d[&(0, 3)][0..5], &[42, 255, 255, 0, 0]);
    }

    #[test]
    fn strip_left_to_right_and_reverse() {
        let mut f = Frame::black(10, 1);
        for x in 0..10 {
            f.pixels[x] = Rgba::rgb(x as f32 / 9.0, 0.0, 0.0);
        }
        let mut o = one(FixtureType::new("s", 10, 1, "RGB"), fx(1));
        let u = o.render_dmx(&f)[&(0, 3)];
        assert!(u[0] < u[3] && u[3] < u[27]);
        o.fixtures[0].reverse = true;
        let u = o.render_dmx(&f)[&(0, 3)];
        assert!(u[0] > u[27]);
    }

    #[test]
    fn rotation_90_maps_strip_vertically() {
        // Top half red, bottom half blue. A horizontal strip rotated 90° clockwise
        // runs top -> bottom... its first pixel (local left) lands at the top.
        let mut f = Frame::black(1, 10);
        for y in 0..10 {
            f.pixels[y] = if y < 5 { Rgba::rgb(1.0, 0.0, 0.0) } else { Rgba::rgb(0.0, 0.0, 1.0) };
        }
        let mut s = fx(1);
        s.rotation = 90.0;
        s.h = 0.05;
        let o = one(FixtureType::new("s", 4, 1, "RGB"), s);
        let u = o.render_dmx(&f)[&(0, 3)];
        assert_eq!(&u[0..3], &[255, 0, 0]);
        assert_eq!(&u[9..12], &[0, 0, 255]);
    }

    #[test]
    fn rotation_keeps_shape_on_wide_canvas() {
        // On a 2:1 canvas a square-in-pixels fixture rotated 90 degrees stays square in pixels.
        let mut f = fx(1);
        f.w = 0.1; // 0.1 * 2 = 0.2 pixel-units wide
        f.h = 0.2;
        f.rotation = 90.0;
        let c = f.corners(2.0);
        let width_px = ((c[1].0 - c[0].0) * 2.0).hypot(c[1].1 - c[0].1);
        assert!((width_px - 0.2).abs() < 1e-5);
        assert!(f.contains(2.0, 0.5, 0.59));
    }

    #[test]
    fn snake_reverses_odd_rows() {
        let mut s = fx(1);
        s.snake = true;
        let t = FixtureType::new("m", 3, 2, "RGB");
        let p = s.pixel_local(&t);
        assert!(p[0].0 < p[2].0); // row 0 left to right
        assert!(p[3].0 > p[5].0); // row 1 right to left
    }

    #[test]
    fn long_strip_spills_into_next_universe() {
        let o = one(FixtureType::new("s", 171, 1, "RGB"), fx(1));
        let out = o.render_dmx(&Frame::new(1, 1, Rgba::rgb(1.0, 1.0, 1.0)));
        assert_eq!(out.keys().copied().collect::<Vec<_>>(), vec![(0, 3), (0, 4)]);
        assert_eq!(out[&(0, 3)][509], 255);
        assert_eq!(out[&(0, 3)][510], 0);
        assert_eq!(&out[&(0, 4)][0..3], &[255, 255, 255]);
        assert_eq!(o.range_label(&o.fixtures[0]), "U3 1-510, U4 1-3");
    }

    #[test]
    fn conflicts_and_next_address() {
        let mut o = one(FixtureType::new("p", 1, 1, "RGBW"), fx(1));
        o.fixtures.push(fx(3));
        assert_eq!(o.conflicts(), vec![(0, 1)]);
        o.fixtures[1].address = 5;
        assert!(o.conflicts().is_empty());
        assert_eq!(o.next_free_address(0), 9);
    }

    #[test]
    fn demo_has_no_conflicts_and_hit_test_works() {
        let o = AdvancedOutput::demo();
        assert!(o.conflicts().is_empty(), "{:?}", o.conflicts());
        assert!(o.fixtures[0].contains(o.aspect, 0.5, 0.62));
        assert!(!o.fixtures[0].contains(o.aspect, 0.5, 0.2));
    }

    #[test]
    fn remove_type_and_lumiverse() {
        let mut o = AdvancedOutput::demo();
        assert!(!o.remove_type(0)); // in use
        o.types.push(FixtureType::new("x", 1, 1, "RGB"));
        assert!(o.remove_type(3));
        assert!(o.remove_lumiverse(0));
        assert!(o.fixtures.iter().all(|f| f.lumiverse == 0));
        assert!(!o.remove_lumiverse(0)); // last one stays
    }

    #[test]
    fn json_roundtrip() {
        let o = AdvancedOutput::demo();
        let s = serde_json::to_string(&o).unwrap();
        let back: AdvancedOutput = serde_json::from_str(&s).unwrap();
        assert_eq!(back.fixtures, o.fixtures);
        assert_eq!(back.types, o.types);
    }
}
