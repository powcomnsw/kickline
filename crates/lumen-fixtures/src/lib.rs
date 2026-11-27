//! Advanced Output: lumiverses, fixture types (the Fixture Editor) and
//! fixtures placed on the composition, mapped to DMX.
//!
//! Mirrors Resolume Arena's DMX output model:
//! - a **Lumiverse** is a DMX universe with a protocol and a target IP;
//! - a **Fixture type** defines a grid of pixels and the channels per pixel;
//! - a **Fixture** is a type placed on the canvas (position, size, rotation)
//!   and patched to a lumiverse at a start address.
//!
//! Fixture types can be imported from QLC+ `.qxf` files; see [`qxf`] and [`library`].

pub mod library;
pub mod qxf;

use lumen_core::{Frame, Rgba, StripPixel};
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
    /// Subtractive colour (CMY fixtures): 255 = full filter.
    Cyan,
    Magenta,
    Yellow,
    /// UV emitter. Kickline has no UV in its image, so this stays off.
    Uv,
    /// Intensity. When a pixel has a dimmer, the colour channels are
    /// normalised so the dimmer carries the brightness.
    Dimmer,
    /// Constant value (strobe off, mode select, macro 0...).
    Fixed(u8),
}

impl ChannelFn {
    pub const CHOICES: [ChannelFn; 11] = [
        ChannelFn::Red,
        ChannelFn::Green,
        ChannelFn::Blue,
        ChannelFn::White,
        ChannelFn::Amber,
        ChannelFn::Cyan,
        ChannelFn::Magenta,
        ChannelFn::Yellow,
        ChannelFn::Uv,
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
            ChannelFn::Cyan => "Cyan",
            ChannelFn::Magenta => "Magenta",
            ChannelFn::Yellow => "Yellow",
            ChannelFn::Uv => "UV",
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
            'C' => Some(ChannelFn::Cyan),
            'M' => Some(ChannelFn::Magenta),
            'Y' => Some(ChannelFn::Yellow),
            'U' => Some(ChannelFn::Uv),
            'D' => Some(ChannelFn::Dimmer),
            _ => None,
        })
        .collect()
}

pub const COLOR_ORDERS: [&str; 10] = ["RGB", "RBG", "GRB", "GBR", "BRG", "BGR", "RGBW", "GRBW", "DRGB", "RGBWA"];

/// One DMX channel of an imported fixture, in DMX order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// The manufacturer's channel name ("Red", "Shutter", "Pan coarse"...).
    pub name: String,
    pub func: ChannelFn,
    /// Which pixel (head) this channel belongs to; None = whole fixture.
    #[serde(default)]
    pub pixel: Option<usize>,
    /// Low byte of a 16-bit pair.
    #[serde(default)]
    pub fine: bool,
}

/// A fixture definition, as made in the Fixture Editor or imported.
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
    /// Imported fixtures: the exact channel list in DMX order. When set, it
    /// replaces `pixel` / `header`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<Slot>>,
    /// Where it came from, e.g. "QLC+ library: Chauvet/Chauvet-SlimPAR-56.qxf".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// A pixel's colour broken down for every kind of emitter.
struct PixelColor {
    r: f32,
    g: f32,
    b: f32,
    white: f32,
    amber: f32,
    /// Brightness, for dimmer channels.
    max: f32,
}

impl PixelColor {
    /// `funcs` = the channels this pixel has; decides white/amber extraction
    /// and whether a dimmer carries the brightness.
    fn with_white(c: Rgba, funcs: &[ChannelFn], mode: WhiteMode) -> Self {
        let (mut r, mut g, mut b) = (c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
        let max = r.max(g).max(b);
        if funcs.contains(&ChannelFn::Dimmer) && max > 0.0 {
            r /= max;
            g /= max;
            b /= max;
        }
        let has_white = funcs.contains(&ChannelFn::White) && mode != WhiteMode::RgbOnly;
        let white = if has_white { r.min(g).min(b) } else { 0.0 };
        if mode != WhiteMode::RgbPlusWhite {
            r -= white;
            g -= white;
            b -= white;
        }
        // Amber takes what red and green share after white is removed.
        let amber = if funcs.contains(&ChannelFn::Amber) { r.min(g * 2.0) * 0.5 } else { 0.0 };
        PixelColor { r, g, b, white, amber, max }
    }

    /// 0..1 value for one channel function.
    fn value(&self, f: ChannelFn) -> f32 {
        match f {
            // With a white emitter, RGB carry only the colour left after white is taken out.
            ChannelFn::Red => self.r,
            ChannelFn::Green => self.g,
            ChannelFn::Blue => self.b,
            ChannelFn::White => self.white,
            ChannelFn::Amber => self.amber,
            // CMY filters subtract from white light.
            ChannelFn::Cyan => 1.0 - (self.r + self.white),
            ChannelFn::Magenta => 1.0 - (self.g + self.white),
            ChannelFn::Yellow => 1.0 - (self.b + self.white),
            ChannelFn::Uv => 0.0,
            ChannelFn::Dimmer => self.max,
            ChannelFn::Fixed(v) => v as f32 / 255.0,
        }
    }
}

fn to_byte(v: f32, fine: bool) -> u8 {
    let v16 = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
    if fine {
        (v16 & 0xff) as u8
    } else {
        (v16 >> 8) as u8
    }
}

impl FixtureType {
    pub fn new(name: &str, columns: usize, rows: usize, order: &str) -> Self {
        FixtureType { name: name.into(), columns, rows, pixel: color_order(order), header: Vec::new(), slots: None, source: None }
    }

    pub fn pixel_count(&self) -> usize {
        self.columns.max(1) * self.rows.max(1)
    }

    pub fn footprint(&self) -> usize {
        match &self.slots {
            Some(s) => s.len(),
            None => self.header.len() + self.pixel_count() * self.pixel.len(),
        }
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
                ChannelFn::Cyan => 'C',
                ChannelFn::Magenta => 'M',
                ChannelFn::Yellow => 'Y',
                ChannelFn::Uv => 'U',
                ChannelFn::Dimmer => 'D',
                ChannelFn::Fixed(_) => 'F',
            })
            .collect()
    }

    fn encode_pixel(&self, c: Rgba, white: WhiteMode, out: &mut Vec<u8>) {
        let pc = PixelColor::with_white(c, &self.pixel, white);
        for ch in &self.pixel {
            out.push(match *ch {
                ChannelFn::Fixed(v) => v,
                f => to_byte(pc.value(f), false),
            });
        }
    }

    /// Encode an imported fixture: `colors[i]` is pixel i's sampled colour.
    fn encode_slots(&self, slots: &[Slot], colors: &[Rgba], white: WhiteMode, out: &mut Vec<u8>) {
        let n = colors.len().max(1);
        let funcs: Vec<Vec<ChannelFn>> = (0..n)
            .map(|p| slots.iter().filter(|s| s.pixel == Some(p)).map(|s| s.func).collect())
            .collect();
        let pcs: Vec<PixelColor> =
            (0..n).map(|p| PixelColor::with_white(colors.get(p).copied().unwrap_or(Rgba::BLACK), &funcs[p], white)).collect();
        // Whole-fixture colour channels get the average of all pixels.
        let avg = {
            let mut a = Rgba::TRANSPARENT;
            for c in colors {
                a.r += c.r / n as f32;
                a.g += c.g / n as f32;
                a.b += c.b / n as f32;
            }
            let global: Vec<ChannelFn> = slots.iter().filter(|s| s.pixel.is_none()).map(|s| s.func).collect();
            PixelColor::with_white(Rgba::rgb(a.r, a.g, a.b), &global, white)
        };
        let pixels_have_colour = funcs.iter().any(|f| f.iter().any(|x| !matches!(x, ChannelFn::Fixed(_) | ChannelFn::Dimmer)));
        for s in slots {
            let v = match (s.func, s.pixel) {
                (ChannelFn::Fixed(v), _) => {
                    out.push(v);
                    continue;
                }
                (f, Some(p)) => pcs.get(p).map(|pc| pc.value(f)).unwrap_or(0.0),
                // A master dimmer over pixels that carry their own colour: leave it open.
                (ChannelFn::Dimmer, None) if pixels_have_colour => 1.0,
                (f, None) => avg.value(f),
            };
            out.push(to_byte(v, s.fine));
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
    /// Network port to send through, by name ("en5"); None = let the computer choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// Send ArtSync after each frame so all universes change together
    /// (only when sending straight to a node, not by broadcast).
    #[serde(default = "yes")]
    pub sync: bool,
    /// Cap the refresh rate for controllers that glitch when fed too fast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hz: Option<u16>,
}

impl Lumiverse {
    pub fn new(name: &str, universe: u16) -> Self {
        Lumiverse {
            name: name.into(),
            protocol: Protocol::ArtNet,
            target: "255.255.255.255".into(),
            universe,
            enabled: true,
            interface: None,
            sync: true,
            max_hz: None,
        }
    }
}

fn yes() -> bool {
    true
}

/// A fixture placed on the canvas. Coordinates are normalised (0..1).
/// One DMX channel of a placed fixture.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelInfo {
    pub label: String,
    pub func: ChannelFn,
    /// Absolute universe.
    pub universe: u16,
    /// 1-based DMX address.
    pub address: u16,
}

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
    /// Universe within the lumiverse to start on (0 = the lumiverse's own universe).
    #[serde(default)]
    pub universe_offset: u16,
    /// How RGBW fixtures use their white emitter.
    #[serde(default)]
    pub white: WhiteMode,
    /// Place along a Path (1 = first). 0 means "by position on the stage".
    #[serde(default, skip_serializing_if = "is_zero")]
    pub path_order: u16,
    /// This fixture runs backwards along its Path.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub path_reverse: bool,
    /// Groups ("slices") this fixture is in, e.g. "Left panel", "Triangle".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

/// How a fixture with a white emitter makes white.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhiteMode {
    /// White comes only from the white emitter; dimming white dims W cleanly.
    #[default]
    WhiteOnly,
    /// White emitter plus full RGB: brightest, slightly cooler/mixed white.
    RgbPlusWhite,
    /// Ignore the white emitter.
    RgbOnly,
}

impl WhiteMode {
    pub const ALL: [WhiteMode; 3] = [WhiteMode::WhiteOnly, WhiteMode::RgbPlusWhite, WhiteMode::RgbOnly];

    pub fn name(self) -> &'static str {
        match self {
            WhiteMode::WhiteOnly => "White emitter only",
            WhiteMode::RgbPlusWhite => "RGB + white (brightest)",
            WhiteMode::RgbOnly => "RGB only",
        }
    }
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

    pub fn pos(&self) -> (f32, f32) {
        (self.x, self.y)
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

/// A photo of the rig shown behind the stage, to place fixtures against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Backdrop {
    /// Where the photo is on this computer.
    pub path: String,
    /// The four corners of the stage in the photo, as 0..1 of its width and
    /// height: top-left, top-right, bottom-right, bottom-left.
    #[serde(default = "full_photo")]
    pub corners: [[f32; 2]; 4],
    /// How visible the photo is (0..1).
    #[serde(default = "backdrop_opacity")]
    pub opacity: f32,
}

pub fn full_photo() -> [[f32; 2]; 4] {
    [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
}

fn backdrop_opacity() -> f32 {
    0.5
}

impl Backdrop {
    pub fn new(path: impl Into<String>) -> Self {
        Backdrop { path: path.into(), corners: full_photo(), opacity: backdrop_opacity() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdvancedOutput {
    /// Composition width / height. Keep in sync with the composition.
    #[serde(default = "default_aspect")]
    pub aspect: f32,
    pub lumiverses: Vec<Lumiverse>,
    pub types: Vec<FixtureType>,
    pub fixtures: Vec<Fixture>,
    /// Group names in display order (fixtures carry their own tags too).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// What each group is for (Auto plays looks written for these roles).
    #[serde(default, skip_serializing_if = "lumen_core::RoleMap::is_empty")]
    pub roles: lumen_core::RoleMap,
    /// A photo behind the stage (not part of what the lights show).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<Backdrop>,
}

impl Default for AdvancedOutput {
    fn default() -> Self {
        AdvancedOutput { aspect: default_aspect(), lumiverses: Vec::new(), types: Vec::new(), fixtures: Vec::new(), groups: Vec::new(), roles: Default::default(), backdrop: None }
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
            universe_offset: 0,
            path_order: 0,
            path_reverse: false,
            white: Default::default(),
            groups: Vec::new(),
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
                universe_offset: 0,
                path_order: 0,
                path_reverse: false,
                white: Default::default(),
                groups: Vec::new(),
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
            universe_offset: 0,
            path_order: 0,
            path_reverse: false,
            white: Default::default(),
            groups: Vec::new(),
        });
        o
    }

    /// Where each pixel (and the header) of a fixture lands. Header and each
    /// pixel are never split across universes: if one doesn't fit, it moves
    /// to channel 1 of the next universe.
    pub fn layout(&self, fx: &Fixture) -> Vec<Segment> {
        let Some(t) = self.types.get(fx.type_id) else { return Vec::new() };
        let mut segs = Vec::new();
        let mut uni = fx.universe_offset;
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
        if let Some(slots) = &t.slots {
            place(slots.len(), &mut segs);
            return segs;
        }
        place(t.header.len(), &mut segs);
        for _ in 0..t.pixel_count() {
            place(t.pixel.len(), &mut segs);
        }
        segs
    }

    /// Render DMX for every lumiverse in use. Key = (lumiverse index, absolute universe).
    pub fn render_dmx(&self, frame: &Frame) -> BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]> {
        self.render_dmx_by(frame, &|_, c| c)
    }

    /// Like `render_dmx`, but every pixel's colour goes through
    /// `f(where, sampled)` first: strip effects and the fixture test.
    /// Pixels are numbered in wiring order (the order the data reaches them).
    pub fn render_dmx_by(
        &self,
        frame: &Frame,
        f: &dyn Fn(StripPixel, Rgba) -> Rgba,
    ) -> BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]> {
        self.render_dmx_each(&|_| frame, f)
    }

    /// Like `render_dmx_by`, but each fixture samples its own picture
    /// (`frame_for(fixture index)`): fixtures in groups see the layers
    /// aimed at their groups.
    pub fn render_dmx_each<'a>(
        &self,
        frame_for: &dyn Fn(usize) -> &'a Frame,
        f: &dyn Fn(StripPixel, Rgba) -> Rgba,
    ) -> BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]> {
        let fixtures = self.fixtures.len();
        let at = |fi: usize, i: usize, n: usize, lx: f32, ly: f32| StripPixel {
            fixture: fi,
            fixtures,
            index: i,
            count: n,
            x: (lx + 0.5).clamp(0.0, 1.0),
            y: (ly + 0.5).clamp(0.0, 1.0),
        };
        let mut out: BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]> = BTreeMap::new();
        let mut bytes = Vec::new();
        for (fi, fx) in self.fixtures.iter().enumerate() {
            let (Some(t), Some(lv)) = (self.types.get(fx.type_id), self.lumiverses.get(fx.lumiverse)) else { continue };
            let frame = frame_for(fi);
            let segs = self.layout(fx);
            let cells = fx.pixel_local(t);
            let n = cells.len();
            if let Some(slots) = &t.slots {
                let Some(seg) = segs.first() else { continue };
                let colors: Vec<Rgba> = cells
                    .into_iter()
                    .enumerate()
                    .map(|(i, (lx, ly))| f(at(fi, i, n, lx, ly), fx.sample_cell(self.aspect, t, frame, lx, ly)))
                    .collect();
                bytes.clear();
                t.encode_slots(slots, &colors, fx.white, &mut bytes);
                write(&mut out, fx.lumiverse, lv.universe + seg.universe_offset, seg.start, &bytes);
                continue;
            }
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
            for (i, (lx, ly)) in cells.into_iter().enumerate() {
                let Some(seg) = seg_iter.next() else { break };
                bytes.clear();
                t.encode_pixel(f(at(fi, i, n, lx, ly), fx.sample_cell(self.aspect, t, frame, lx, ly)), fx.white, &mut bytes);
                write(&mut out, fx.lumiverse, lv.universe + seg.universe_offset, seg.start, &bytes);
            }
        }
        out
    }

    /// Every channel a fixture uses, in order: what it does and where it is.
    /// Mirrors how `render_dmx` lays the bytes out.
    pub fn channel_map(&self, fx: &Fixture) -> Vec<ChannelInfo> {
        let mut out = Vec::new();
        let (Some(t), Some(lv)) = (self.types.get(fx.type_id), self.lumiverses.get(fx.lumiverse)) else { return out };
        let segs = self.layout(fx);
        if let Some(slots) = &t.slots {
            let Some(seg) = segs.first() else { return out };
            for (i, sl) in slots.iter().enumerate() {
                out.push(ChannelInfo {
                    label: sl.name.clone(),
                    func: sl.func,
                    universe: lv.universe + seg.universe_offset,
                    address: (seg.start + i + 1) as u16,
                });
            }
            return out;
        }
        let mut it = segs.iter();
        if !t.header.is_empty() {
            if let Some(seg) = it.next() {
                for (i, f) in t.header.iter().enumerate() {
                    out.push(ChannelInfo { label: f.name().to_string(), func: *f, universe: lv.universe + seg.universe_offset, address: (seg.start + i + 1) as u16 });
                }
            }
        }
        for (p, seg) in it.enumerate() {
            for (i, f) in t.pixel.iter().enumerate() {
                let label = if t.pixel_count() > 1 { format!("Pixel {} {}", p + 1, f.name()) } else { f.name().to_string() };
                out.push(ChannelInfo { label, func: *f, universe: lv.universe + seg.universe_offset, address: (seg.start + i + 1) as u16 });
            }
        }
        out
    }

    /// Pairs of fixtures whose channels overlap.
    pub fn conflicts(&self) -> Vec<(usize, usize)> {
        let mut used: BTreeMap<(usize, u16, usize), usize> = BTreeMap::new();
        let mut out = Vec::new();
        // Lumiverses that reach the same devices share channel space.
        let group = |l: usize| (0..=l).find(|&j| j == l || self.same_wire(j, l)).unwrap_or(l);
        for (i, fx) in self.fixtures.iter().enumerate() {
            let Some(lv) = self.lumiverses.get(fx.lumiverse) else { continue };
            let g = group(fx.lumiverse);
            for s in self.layout(fx) {
                for ch in s.start..s.start + s.len {
                    if let Some(&j) = used.get(&(g, lv.universe + s.universe_offset, ch)) {
                        if j != i && !out.contains(&(j, i)) {
                            out.push((j, i));
                        }
                    } else {
                        used.insert((g, lv.universe + s.universe_offset, ch), i);
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

    /// Where a fixture's last channel ends: (universe offset, 0-based channel after it).
    fn end_of(&self, fx: &Fixture) -> Option<(u16, usize)> {
        self.layout(fx).last().map(|s| (s.universe_offset, s.start + s.len))
    }

    /// First universe offset on a lumiverse that nothing uses yet.
    pub fn next_free_universe(&self, lumiverse: usize) -> u16 {
        self.fixtures
            .iter()
            .filter(|f| f.lumiverse == lumiverse)
            .filter_map(|f| self.end_of(f))
            .map(|(u, _)| u + 1)
            .max()
            .unwrap_or(0)
    }

    /// Next free (universe offset, 1-based address) after everything on a lumiverse.
    pub fn next_free_slot(&self, lumiverse: usize) -> (u16, u16) {
        let end = self
            .fixtures
            .iter()
            .filter(|f| f.lumiverse == lumiverse)
            .filter_map(|f| self.end_of(f))
            .max();
        match end {
            None => (0, 1),
            Some((u, ch)) if ch >= UNIVERSE_SIZE => (u + 1, 1),
            Some((u, ch)) => (u, ch as u16 + 1),
        }
    }

    /// Absolute universes this lumiverse sends (its base universe if empty).
    pub fn universes_used(&self, lumiverse: usize) -> Vec<u16> {
        let Some(l) = self.lumiverses.get(lumiverse) else { return Vec::new() };
        let mut out: Vec<u16> = self
            .fixtures
            .iter()
            .filter(|f| f.lumiverse == lumiverse)
            .flat_map(|f| self.layout(f).into_iter().map(|s| l.universe + s.universe_offset))
            .collect();
        if out.is_empty() {
            out.push(l.universe);
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Could two lumiverses' packets land on the same device? Same protocol and
    /// either sACN (universes are network-wide), the same target, or a broadcast.
    pub fn same_wire(&self, a: usize, b: usize) -> bool {
        let (Some(x), Some(y)) = (self.lumiverses.get(a), self.lumiverses.get(b)) else { return false };
        if x.protocol != y.protocol {
            return false;
        }
        let bcast = |t: &str| t.trim().ends_with(".255");
        x.protocol == Protocol::Sacn || x.target.trim() == y.target.trim() || bcast(&x.target) || bcast(&y.target)
    }

    /// Absolute universes that other lumiverses on the same wire already use:
    /// their base universe plus everything their fixtures fill.
    pub fn universes_taken_by_others(&self, lumiverse: usize) -> std::collections::HashSet<u16> {
        let mut out = std::collections::HashSet::new();
        for (i, l) in self.lumiverses.iter().enumerate() {
            if i == lumiverse || !self.same_wire(i, lumiverse) {
                continue;
            }
            out.insert(l.universe);
            for fx in self.fixtures.iter().filter(|f| f.lumiverse == i) {
                for s in self.layout(fx) {
                    out.insert(l.universe + s.universe_offset);
                }
            }
        }
        out
    }

    /// Every group: the listed ones first, then any other fixture tags.
    pub fn group_names(&self) -> Vec<String> {
        let mut out = self.groups.clone();
        for f in &self.fixtures {
            for g in &f.groups {
                if !out.contains(g) {
                    out.push(g.clone());
                }
            }
        }
        out
    }

    /// Fixtures in a group, left to right (then top to bottom): the order
    /// strip effects ripple across it.
    pub fn group_members(&self, group: &str) -> Vec<usize> {
        let mut m: Vec<usize> = (0..self.fixtures.len()).filter(|&i| self.fixtures[i].groups.iter().any(|g| g == group)).collect();
        m.sort_by(|&a, &b| {
            let (fa, fb) = (&self.fixtures[a], &self.fixtures[b]);
            // Fixtures with a path place come first, in that order; the rest by position.
            let (oa, ob) = ((fa.path_order == 0) as u8, (fb.path_order == 0) as u8);
            oa.cmp(&ob).then(fa.path_order.cmp(&fb.path_order)).then(fa.x.total_cmp(&fb.x)).then(fa.y.total_cmp(&fb.y))
        });
        m
    }

    /// A guess at each group's role from the layout. A starting point for the
    /// user to correct; it is never applied by itself. Groups that only repeat
    /// other groups ("Panels" over "Left panel" and "Right panel") get none.
    pub fn suggest_roles(&self) -> lumen_core::RoleMap {
        use lumen_core::StageRole as R;
        let names = self.group_names();
        let sets: Vec<Vec<usize>> = names.iter().map(|g| (0..self.fixtures.len()).filter(|&i| self.fixtures[i].groups.iter().any(|x| x == g)).collect()).collect();
        let keep: Vec<bool> = (0..names.len())
            .map(|a| {
                !sets[a].is_empty()
                    && !(0..names.len()).any(|b| {
                        b != a && !sets[b].is_empty() && sets[b].iter().all(|i| sets[a].contains(i)) && (sets[b].len() < sets[a].len() || (sets[b].len() == sets[a].len() && b < a))
                    })
            })
            .collect();
        let is_line = |i: usize| self.types.get(self.fixtures[i].type_id).is_some_and(|t| t.pixel_count() >= 2 && (t.columns <= 1 || t.rows <= 1));
        let centroid = |set: &[usize]| {
            let n = set.len() as f32;
            (set.iter().map(|&i| self.fixtures[i].x).sum::<f32>() / n, set.iter().map(|&i| self.fixtures[i].y).sum::<f32>() / n)
        };
        let small = |set: &[usize]| set.iter().all(|&i| self.fixtures[i].w * self.fixtures[i].h < 0.012);
        let mirrored = |a: (f32, f32), b: (f32, f32)| (a.0 + b.0 - 1.0).abs() < 0.1 && (a.1 - b.1).abs() < 0.12 && (a.0 - b.0).abs() > 0.1;
        let mut out = lumen_core::RoleMap::default();
        for a in 0..names.len() {
            if !keep[a] {
                continue;
            }
            let set = &sets[a];
            let role = if set.iter().all(|&i| is_line(i)) {
                // Strips: a closed loop is an outline, an open chain a path.
                let ends: Vec<(f32, f32)> = set
                    .iter()
                    .flat_map(|&i| {
                        let f = &self.fixtures[i];
                        [f.to_canvas(self.aspect, -0.5, 0.0), f.to_canvas(self.aspect, 0.5, 0.0)]
                    })
                    .map(|(u, v)| (u * self.aspect, v))
                    .collect();
                let touching = |k: usize| ends.iter().enumerate().any(|(j, e)| j / 2 != k / 2 && ((e.0 - ends[k].0).powi(2) + (e.1 - ends[k].1).powi(2)).sqrt() < 0.06);
                if set.len() >= 3 && (0..ends.len()).all(touching) {
                    R::Outline
                } else {
                    R::Path
                }
            } else {
                let c = centroid(set);
                let partner = |only_small: bool| (0..names.len()).any(|b| b != a && keep[b] && (!only_small || small(&sets[b])) && !sets[b].iter().all(|&i| is_line(i)) && mirrored(c, centroid(&sets[b])));
                if small(set) && ((set.len() == 2 && mirrored(self.fixtures[set[0]].pos(), self.fixtures[set[1]].pos())) || partner(true)) {
                    R::Eyes
                } else if partner(false) && (c.0 - 0.5).abs() > 0.05 {
                    if c.0 < 0.5 { R::Left } else { R::Right }
                } else if (c.0 - 0.5).abs() < 0.15 && (c.1 - 0.5).abs() < 0.3 {
                    R::Centre
                } else {
                    R::Fill
                }
            };
            out.set(&names[a], Some(role));
        }
        out
    }

    /// The part of the canvas a group covers (x0, y0, x1, y1).
    pub fn group_area(&self, group: &str) -> Option<[f32; 4]> {
        let mut a = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut any = false;
        for f in self.fixtures.iter().filter(|f| f.groups.iter().any(|g| g == group)) {
            for (u, v) in f.corners(self.aspect) {
                a = [a[0].min(u), a[1].min(v), a[2].max(u), a[3].max(v)];
                any = true;
            }
        }
        any.then_some(a)
    }

    pub fn rename_group(&mut self, from: &str, to: &str) {
        for (g, _) in self.roles.0.iter_mut() {
            if g == from {
                *g = to.to_string();
            }
        }
        for g in self.groups.iter_mut().chain(self.fixtures.iter_mut().flat_map(|f| f.groups.iter_mut())) {
            if g == from {
                *g = to.to_string();
            }
        }
    }

    pub fn remove_group(&mut self, name: &str) {
        self.roles.set(name, None);
        self.groups.retain(|g| g != name);
        for f in &mut self.fixtures {
            f.groups.retain(|g| g != name);
        }
    }

    /// What adding a controller would do, output by output.
    pub fn controller_plan(&self, s: &ControllerSetup) -> Vec<PlanRow> {
        let ppu = s.pixels_per_universe();
        let base = self.lumiverses.get(s.lumiverse).map(|l| l.universe).unwrap_or(0);
        s.leds
            .iter()
            .enumerate()
            .map(|(i, &leds)| {
                let need = leds.div_ceil(ppu) as u16;
                let first = base + i as u16 * s.universes_per_output;
                PlanRow { output: i + 1, leds, universes: (first, first + need.max(1) - 1), needed: need, fits: need <= s.universes_per_output && need <= s.max_universes() }
            })
            .collect()
    }

    /// Add one strip per controller output, addressed the way the
    /// controller counts: output N starts on universe
    /// base + (N - 1) x universes-per-output, channel 1.
    pub fn add_controller(&mut self, s: &ControllerSetup) -> Result<std::ops::Range<usize>, String> {
        let plan = self.controller_plan(s);
        if let Some(bad) = plan.iter().find(|r| r.leds > 0 && !r.fits) {
            return Err(format!(
                "Output {} needs {} universes for {} LEDs, but each output has {} (the {} takes up to {})",
                bad.output, bad.needed, bad.leds, s.universes_per_output, s.controller().name, s.max_universes()
            ));
        }
        if self.lumiverses.get(s.lumiverse).is_none() {
            return Err("Pick an output (lumiverse) first".into());
        }
        let order = if s.rgbw { "RGBW" } else { "RGB" };
        let first = self.fixtures.len();
        let used: Vec<(usize, usize)> = s.leds.iter().copied().enumerate().filter(|(_, n)| *n > 0).collect();
        let rows = used.len().max(1);
        let (x0, y0, x1, y1) = s.area;
        for (k, (out, leds)) in used.into_iter().enumerate() {
            let tname = format!("Pixel strip {leds} {order}");
            let type_id = match self.types.iter().position(|t| t.name == tname) {
                Some(i) => i,
                None => {
                    self.types.push(FixtureType::new(&tname, leds, 1, order));
                    self.types.len() - 1
                }
            };
            let pitch = (y1 - y0) / rows as f32;
            self.fixtures.push(Fixture {
                name: format!("{} out {}", s.name, out + 1),
                type_id,
                lumiverse: s.lumiverse,
                address: 1,
                x: (x0 + x1) / 2.0,
                y: y0 + pitch * (k as f32 + 0.5),
                w: x1 - x0,
                h: (pitch * 0.6).min(0.04),
                rotation: 0.0,
                snake: false,
                reverse: false,
                universe_offset: out as u16 * s.universes_per_output,
                path_order: 0,
                path_reverse: false,
                white: WhiteMode::default(),
                groups: vec![s.name.clone()],
            });
        }
        if !self.groups.contains(&s.name) {
            self.groups.push(s.name.clone());
        }
        Ok(first..self.fixtures.len())
    }

    /// Lay out a row/column of identical fixtures in one go. Returns the new fixtures' indices.
    pub fn add_array(&mut self, spec: &ArraySpec) -> std::ops::Range<usize> {
        let first = self.fixtures.len();
        let n = spec.count.max(1);
        let (x0, y0, x1, y1) = spec.area;
        let taken = self.universes_taken_by_others(spec.lumiverse);
        let (mut uni, mut addr) = match spec.addressing {
            Addressing::Continue => self.next_free_slot(spec.lumiverse),
            Addressing::UniversePerFixture => (self.next_free_universe(spec.lumiverse), 1),
        };
        for i in 0..n {
            let k = (i as f32 + 0.5) / n as f32;
            let (x, y, w, h, rotation) = match spec.direction {
                ArrayDirection::Rows => {
                    let pitch = (y1 - y0) / n as f32;
                    ((x0 + x1) / 2.0, y0 + k * (y1 - y0), x1 - x0, spec.thickness.min(pitch * 0.8), 0.0)
                }
                ArrayDirection::Columns => {
                    // Rotated 90°: length runs down the canvas, pixel 1 at the top.
                    let pitch = (x1 - x0) / n as f32;
                    let len = (y1 - y0) / self.aspect.max(0.01);
                    (x0 + k * (x1 - x0), (y0 + y1) / 2.0, len, spec.thickness.min(pitch * 0.8 * self.aspect), 90.0)
                }
            };
            let mut fx = Fixture {
                name: format!("{} {}", spec.name, i + 1),
                type_id: spec.type_id,
                lumiverse: spec.lumiverse,
                address: addr,
                x,
                y,
                w,
                h,
                rotation,
                snake: false,
                reverse: spec.zigzag && i % 2 == 1,
                universe_offset: uni,
                path_order: 0,
                path_reverse: false,
                white: WhiteMode::default(),
                groups: Vec::new(),
            };
            // Never land on a universe another lumiverse already sends.
            let base = self.lumiverses.get(spec.lumiverse).map(|l| l.universe).unwrap_or(0);
            while self.layout(&fx).iter().any(|s| taken.contains(&(base + s.universe_offset))) && fx.universe_offset < 32_000 {
                fx.universe_offset += 1;
                fx.address = 1;
            }
            let end = self.end_of(&fx);
            self.fixtures.push(fx);
            if let Some((u, ch)) = end {
                match spec.addressing {
                    Addressing::Continue if ch >= UNIVERSE_SIZE => (uni, addr) = (u + 1, 1),
                    Addressing::Continue => (uni, addr) = (u, ch as u16 + 1),
                    Addressing::UniversePerFixture => (uni, addr) = (u + 1, 1),
                }
            }
        }
        first..self.fixtures.len()
    }

    /// Absolute universes used by fixtures in `range` (for previews).
    pub fn universes_of(&self, range: std::ops::Range<usize>) -> Option<(u16, u16)> {
        let mut lo = u16::MAX;
        let mut hi = 0;
        for fx in &self.fixtures[range] {
            let base = self.lumiverses.get(fx.lumiverse).map(|l| l.universe).unwrap_or(0);
            for s in self.layout(fx) {
                lo = lo.min(base + s.universe_offset);
                hi = hi.max(base + s.universe_offset);
            }
        }
        (lo <= hi).then_some((lo, hi))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayDirection {
    /// Horizontal strips stacked top to bottom.
    Rows,
    /// Vertical strips side by side, left to right.
    Columns,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addressing {
    /// Each fixture starts right after the previous one (spilling into the next universe).
    Continue,
    /// Each fixture starts at channel 1 of its own universe, as most pixel controllers expect.
    UniversePerFixture,
}

/// A pixel controller Kickline knows how to address.
#[derive(Clone, Copy, Debug)]
pub struct Controller {
    pub name: &'static str,
    pub outputs: usize,
    /// Most universes one output can take.
    pub max_universes: u16,
    pub note: &'static str,
}

pub const CONTROLLERS: &[Controller] = &[
    Controller {
        name: "H807SA",
        outputs: 8,
        max_universes: 6,
        note: "8 outputs, up to 6 universes each (older units 4). In LED Edit, set the start universe and universes per port to match below.",
    },
    Controller { name: "H802RA", outputs: 4, max_universes: 4, note: "4 outputs, up to 680 RGB pixels (4 universes) each." },
    Controller { name: "H803SA", outputs: 8, max_universes: 4, note: "8 outputs, up to 4 universes each." },
    Controller { name: "Generic, 1 output", outputs: 1, max_universes: 32, note: "Any controller with one data output." },
    Controller { name: "Generic, 2 outputs", outputs: 2, max_universes: 16, note: "Any controller with two data outputs." },
    Controller { name: "Generic, 4 outputs", outputs: 4, max_universes: 16, note: "Any controller with four data outputs." },
    Controller { name: "Generic, 8 outputs", outputs: 8, max_universes: 16, note: "Any controller with eight data outputs." },
    Controller { name: "Generic, 16 outputs", outputs: 16, max_universes: 16, note: "Any controller with sixteen data outputs." },
];

/// "I have this controller with this many LEDs on each output."
#[derive(Clone, Debug)]
pub struct ControllerSetup {
    /// Index into `CONTROLLERS`.
    pub controller: usize,
    pub name: String,
    pub lumiverse: usize,
    /// LEDs on each output (0 = nothing plugged in).
    pub leds: Vec<usize>,
    pub universes_per_output: u16,
    pub rgbw: bool,
    pub area: (f32, f32, f32, f32),
}

impl ControllerSetup {
    pub fn new(controller: usize, lumiverse: usize) -> Self {
        let c = CONTROLLERS.get(controller).copied().unwrap_or(CONTROLLERS[0]);
        ControllerSetup {
            controller,
            name: c.name.split(',').next().unwrap_or("Controller").to_string(),
            lumiverse,
            leds: vec![0; c.outputs],
            universes_per_output: 1,
            rgbw: false,
            area: (0.1, 0.1, 0.9, 0.9),
        }
    }

    pub fn controller(&self) -> Controller {
        CONTROLLERS.get(self.controller).copied().unwrap_or(CONTROLLERS[0])
    }

    pub fn max_universes(&self) -> u16 {
        self.controller().max_universes
    }

    pub fn pixels_per_universe(&self) -> usize {
        if self.rgbw { UNIVERSE_SIZE / 4 } else { UNIVERSE_SIZE / 3 }
    }

    /// The fewest universes per output that fit the longest output.
    pub fn suggested_universes(&self) -> u16 {
        let ppu = self.pixels_per_universe();
        self.leds.iter().map(|&n| n.div_ceil(ppu) as u16).max().unwrap_or(1).max(1)
    }
}

/// One controller output in a plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanRow {
    /// 1-based, as printed on the controller.
    pub output: usize,
    pub leds: usize,
    /// First and last absolute universe.
    pub universes: (u16, u16),
    pub needed: u16,
    pub fits: bool,
}

#[derive(Clone, Debug)]
pub struct ArraySpec {
    pub name: String,
    pub type_id: usize,
    pub lumiverse: usize,
    pub count: usize,
    pub direction: ArrayDirection,
    /// x0, y0, x1, y1 on the canvas (0..1).
    pub area: (f32, f32, f32, f32),
    /// Every other fixture runs the opposite way (snake wiring between strips).
    pub zigzag: bool,
    pub addressing: Addressing,
    /// Strip thickness on the canvas.
    pub thickness: f32,
}

fn write(out: &mut BTreeMap<(usize, u16), [u8; UNIVERSE_SIZE]>, lv: usize, uni: u16, start: usize, data: &[u8]) {
    let buf = out.entry((lv, uni)).or_insert([0; UNIVERSE_SIZE]);
    buf[start..start + data.len()].copy_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(t: FixtureType, fx: Fixture) -> AdvancedOutput {
        AdvancedOutput { aspect: 1.0, lumiverses: vec![Lumiverse::new("L", 3)], types: vec![t], fixtures: vec![fx], groups: Vec::new(), roles: Default::default(), backdrop: None }
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
            universe_offset: 0,
            path_order: 0,
            path_reverse: false,
            white: Default::default(),
            groups: Vec::new(),
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

    fn strips(n_px: usize) -> AdvancedOutput {
        AdvancedOutput {
            aspect: 1.0,
            lumiverses: vec![Lumiverse::new("Ctrl", 10)],
            types: vec![FixtureType::new("strip", n_px, 1, "RGB")],
            fixtures: vec![],
            groups: Vec::new(),
            roles: Default::default(),
            backdrop: None,
        }
    }

    #[test]
    fn h807sa_outputs_start_on_their_own_universes() {
        let mut o = strips(10);
        let mut s = ControllerSetup::new(0, 0);
        assert_eq!(s.controller().name, "H807SA");
        s.leds = vec![600, 600, 300, 0, 0, 0, 0, 0];
        assert_eq!(s.suggested_universes(), 4); // 600 RGB pixels = 3.5 universes
        s.universes_per_output = 4;
        let plan = o.controller_plan(&s);
        assert_eq!(plan[0].universes, (10, 13));
        assert_eq!(plan[1].universes, (14, 17));
        assert_eq!(plan[2].universes, (18, 19));
        let r = o.add_controller(&s).unwrap();
        assert_eq!(r.len(), 3);
        // Output 2's first pixel is channel 1 of universe 14.
        let out2 = &o.fixtures[r.start + 1];
        assert_eq!((o.lumiverses[0].universe + out2.universe_offset, out2.address), (14, 1));
        let dmx = o.render_dmx(&Frame::new(4, 4, Rgba::rgb(1.0, 0.0, 0.0)));
        assert_eq!(dmx[&(0, 14)][0], 255);
        assert!(dmx.contains_key(&(0, 13)) && dmx.contains_key(&(0, 19)) && !dmx.contains_key(&(0, 20)));
        assert_eq!(o.group_members("H807SA").len(), 3);
        // Same strip length: one type, reused.
        assert_eq!(o.types.iter().filter(|t| t.name == "Pixel strip 600 RGB").count(), 1);
    }

    #[test]
    fn controller_refuses_an_output_that_is_too_long() {
        let mut o = strips(10);
        let mut s = ControllerSetup::new(0, 0);
        s.leds[0] = 2000; // 12 universes: more than an H807SA output takes
        s.universes_per_output = s.suggested_universes();
        assert!(o.add_controller(&s).is_err());
        assert!(o.fixtures.is_empty());
    }

    #[test]
    fn groups_have_an_area_and_an_order() {
        let mut o = AdvancedOutput::demo();
        o.fixtures[0].groups = vec!["Left".into()];
        o.fixtures[1].groups = vec!["Left".into()];
        let a = o.group_area("Left").unwrap();
        assert!(a[0] < a[2] && a[1] < a[3]);
        assert_eq!(o.group_names(), vec!["Left".to_string()]);
        o.rename_group("Left", "Side");
        assert_eq!(o.group_members("Side").len(), 2);
        o.remove_group("Side");
        assert!(o.group_names().is_empty());
        assert!(o.group_area("Side").is_none());
    }

    fn spec(count: usize, addressing: Addressing, direction: ArrayDirection, zigzag: bool) -> ArraySpec {
        ArraySpec {
            name: "Strip".into(),
            type_id: 0,
            lumiverse: 0,
            count,
            direction,
            area: (0.1, 0.1, 0.9, 0.9),
            zigzag,
            addressing,
            thickness: 0.04,
        }
    }

    #[test]
    fn array_universe_per_strip() {
        let mut o = strips(150);
        let r = o.add_array(&spec(4, Addressing::UniversePerFixture, ArrayDirection::Rows, true));
        assert_eq!(r, 0..4);
        let labels: Vec<String> = o.fixtures.iter().map(|f| o.range_label(f)).collect();
        assert_eq!(labels, ["U10 1-450", "U11 1-450", "U12 1-450", "U13 1-450"]);
        assert_eq!(o.fixtures.iter().map(|f| f.reverse).collect::<Vec<_>>(), [false, true, false, true]);
        assert!(o.fixtures[0].y < o.fixtures[3].y);
        assert_eq!(o.universes_of(r), Some((10, 13)));
        assert!(o.conflicts().is_empty());
        // A second array continues on the next free universe.
        let r2 = o.add_array(&spec(1, Addressing::UniversePerFixture, ArrayDirection::Rows, false));
        assert_eq!(o.range_label(&o.fixtures[r2.start]), "U14 1-450");
    }

    #[test]
    fn array_continue_spills_without_splitting_pixels() {
        let mut o = strips(100); // 300 ch each
        o.add_array(&spec(3, Addressing::Continue, ArrayDirection::Columns, false));
        let labels: Vec<String> = o.fixtures.iter().map(|f| o.range_label(f)).collect();
        // 1-300; then 301-510 + spill 1-90; then 91-390
        assert_eq!(labels, ["U10 1-300", "U10 301-510, U11 1-90", "U11 91-390"]);
        assert!(o.conflicts().is_empty());
        assert_eq!(o.fixtures[0].rotation, 90.0);
        assert!(o.fixtures[0].x < o.fixtures[2].x);
        // vertical strip covers the area's height: top pixel near y0
        let (_, top) = o.fixtures[0].pixel_positions(1.0, &o.types[0])[0];
        assert!(top < 0.15, "{top}");
    }

    #[test]
    fn array_skips_universes_other_lumiverses_use() {
        let mut o = strips(150);
        o.lumiverses[0].target = "2.0.0.50".into();
        let mut pars = Lumiverse::new("Pars", 12);
        pars.target = "2.0.0.50".into();
        o.lumiverses.push(pars);
        let r = o.add_array(&spec(4, Addressing::UniversePerFixture, ArrayDirection::Rows, false));
        assert_eq!(o.universes_of(r.clone()), Some((10, 14)));
        let labels: Vec<String> = o.fixtures[r].iter().map(|f| o.range_label(f)).collect();
        assert_eq!(labels, ["U10 1-450", "U11 1-450", "U13 1-450", "U14 1-450"]);
        // A different node may reuse the number: no skipping, no conflict.
        let mut o2 = strips(150);
        o2.lumiverses[0].target = "2.0.0.50".into();
        let mut other = Lumiverse::new("Other node", 12);
        other.target = "2.0.0.51".into();
        o2.lumiverses.push(other);
        let r = o2.add_array(&spec(4, Addressing::UniversePerFixture, ArrayDirection::Rows, false));
        assert_eq!(o2.universes_of(r), Some((10, 13)));
        // Overlap on the same node is reported as a conflict.
        let mut o3 = o2.clone();
        o3.lumiverses[1].target = "2.0.0.50".into();
        o3.fixtures.push(Fixture { lumiverse: 1, ..o3.fixtures[0].clone() });
        assert!(!o3.conflicts().is_empty());
    }

    #[test]
    fn render_by_can_solo_one_fixture() {
        let o = AdvancedOutput::demo();
        let frame = Frame::transparent(8, 8);
        // Identity matches the normal render.
        assert_eq!(o.render_dmx_by(&frame, &|_, c| c), o.render_dmx(&frame));
        // Solo fixture 0 in full red; everything else must be zero.
        let solo = o.render_dmx_by(&frame, &|p, _| if p.fixture == 0 { Rgba::rgb(1.0, 0.0, 0.0) } else { Rgba::BLACK });
        let lit: usize = solo.values().map(|u| u.iter().filter(|&&b| b > 0).count()).sum();
        let t = &o.types[o.fixtures[0].type_id];
        assert_eq!(lit, t.pixel_count(), "one red channel per pixel of fixture 0 only");
    }

    #[test]
    fn channel_map_matches_render() {
        let o = AdvancedOutput::demo();
        let frame = Frame::new(8, 8, Rgba::rgb(1.0, 0.0, 0.0));
        let dmx = o.render_dmx(&frame);
        for fx in &o.fixtures {
            let map = o.channel_map(fx);
            assert_eq!(map.len(), o.types[fx.type_id].footprint(), "{}", fx.name);
            for ch in &map {
                // Red channels of a red frame carry light; blue ones don't.
                let lv = fx.lumiverse;
                let v = dmx[&(lv, ch.universe)][ch.address as usize - 1];
                if ch.func == ChannelFn::Blue {
                    assert_eq!(v, 0, "{} {}", fx.name, ch.label);
                }
                if ch.func == ChannelFn::Red {
                    assert!(v > 0, "{} {}", fx.name, ch.label);
                }
            }
        }
    }

    #[test]
    fn universe_offset_moves_fixture() {
        let mut o = strips(2);
        o.add_array(&spec(1, Addressing::Continue, ArrayDirection::Rows, false));
        o.fixtures[0].universe_offset = 3;
        let d = o.render_dmx(&Frame::new(1, 1, Rgba::rgb(1.0, 0.0, 0.0)));
        assert_eq!(d.keys().copied().collect::<Vec<_>>(), vec![(0, 13)]);
        assert_eq!(o.next_free_slot(0), (3, 7));
        assert_eq!(o.next_free_universe(0), 4);
    }

    #[test]
    fn white_modes() {
        let mk = |w: WhiteMode| {
            let mut o = one(FixtureType::new("p", 1, 1, "RGBW"), fx(1));
            o.fixtures[0].white = w;
            o.render_dmx(&Frame::new(1, 1, Rgba::rgb(0.5, 0.5, 0.5)))[&(0, 3)][0..4].to_vec()
        };
        // grey (e.g. white at 50% opacity) dims the white emitter, not the RGB diodes
        assert_eq!(mk(WhiteMode::WhiteOnly), vec![0, 0, 0, 128]);
        assert_eq!(mk(WhiteMode::RgbPlusWhite), vec![128, 128, 128, 128]);
        assert_eq!(mk(WhiteMode::RgbOnly), vec![128, 128, 128, 0]);
    }

    #[test]
    fn json_roundtrip() {
        let o = AdvancedOutput::demo();
        let s = serde_json::to_string(&o).unwrap();
        let back: AdvancedOutput = serde_json::from_str(&s).unwrap();
        assert_eq!(back.fixtures, o.fixtures);
        assert_eq!(back.types, o.types);
    }

    // ---- Stage roles

    /// A strip from (x0,y0) to (x1,y1) on a 16:9 canvas, one group per `groups`.
    fn bar(o: &mut AdvancedOutput, name: &str, a: (f32, f32), b: (f32, f32), groups: &[&str]) {
        if o.types.is_empty() {
            o.types.push(FixtureType::new("Pixel strip 30 RGB", 30, 1, "RGB"));
            o.lumiverses.push(Lumiverse::new("C", 0));
        }
        let asp = o.aspect;
        let (dx, dy) = ((b.0 - a.0) * asp, b.1 - a.1);
        let mut f = fx(1);
        f.name = name.into();
        f.x = (a.0 + b.0) / 2.0;
        f.y = (a.1 + b.1) / 2.0;
        f.w = (dx * dx + dy * dy).sqrt() / asp;
        f.h = 0.02;
        f.rotation = dy.atan2(dx).to_degrees();
        f.groups = groups.iter().map(|g| g.to_string()).collect();
        o.fixtures.push(f);
    }

    fn panel(o: &mut AdvancedOutput, name: &str, x: f32, y: f32, w: f32, h: f32, groups: &[&str]) {
        let t = o.types.iter().position(|t| t.name == "Panel 8x8").unwrap_or_else(|| {
            o.types.push(FixtureType::new("Panel 8x8", 8, 8, "RGB"));
            o.types.len() - 1
        });
        if o.lumiverses.is_empty() {
            o.lumiverses.push(Lumiverse::new("C", 0));
        }
        let mut f = fx(1);
        f.name = name.into();
        f.type_id = t;
        f.x = x;
        f.y = y;
        f.w = w;
        f.h = h;
        f.groups = groups.iter().map(|g| g.to_string()).collect();
        o.fixtures.push(f);
    }

    #[test]
    fn a_path_runs_in_its_own_order_not_left_to_right() {
        let mut o = AdvancedOutput { aspect: 16.0 / 9.0, ..Default::default() };
        bar(&mut o, "a", (0.1, 0.5), (0.2, 0.5), &["Spine"]);
        bar(&mut o, "b", (0.3, 0.5), (0.4, 0.5), &["Spine"]);
        bar(&mut o, "c", (0.5, 0.5), (0.6, 0.5), &["Spine"]);
        assert_eq!(o.group_members("Spine"), vec![0, 1, 2]);
        o.fixtures[0].path_order = 3;
        o.fixtures[1].path_order = 1;
        o.fixtures[2].path_order = 2;
        assert_eq!(o.group_members("Spine"), vec![1, 2, 0]);
    }

    #[test]
    fn a_show_from_before_roles_still_opens() {
        let o = AdvancedOutput::demo();
        let mut v: serde_json::Value = serde_json::to_value(&o).unwrap();
        v.as_object_mut().unwrap().remove("roles");
        for f in v["fixtures"].as_array_mut().unwrap() {
            f.as_object_mut().unwrap().remove("path_order");
            f.as_object_mut().unwrap().remove("path_reverse");
        }
        let back: AdvancedOutput = serde_json::from_value(v).unwrap();
        assert!(back.roles.is_empty());
        assert!(back.fixtures.iter().all(|f| f.path_order == 0 && !f.path_reverse));
    }

    #[test]
    fn suggestions_read_the_layout() {
        use lumen_core::StageRole::*;
        let mut o = AdvancedOutput { aspect: 16.0 / 9.0, ..Default::default() };
        // An open chain of strips: a Path.
        bar(&mut o, "s1", (0.10, 0.9), (0.20, 0.9), &["Tail", "Body"]);
        bar(&mut o, "s2", (0.20, 0.9), (0.30, 0.8), &["Tail", "Body"]);
        bar(&mut o, "s3", (0.30, 0.8), (0.40, 0.8), &["Tail", "Body"]);
        // A closed triangle: an Outline.
        bar(&mut o, "t1", (0.6, 0.2), (0.7, 0.2), &["Ring"]);
        bar(&mut o, "t2", (0.7, 0.2), (0.65, 0.3), &["Ring"]);
        bar(&mut o, "t3", (0.65, 0.3), (0.6, 0.2), &["Ring"]);
        // A small matched pair: Eyes.
        panel(&mut o, "e1", 0.42, 0.3, 0.05, 0.08, &["Eye A"]);
        panel(&mut o, "e2", 0.58, 0.3, 0.05, 0.08, &["Eye B"]);
        // Big panels either side: a left/right pair. One in the middle: Centre.
        panel(&mut o, "pl", 0.1, 0.4, 0.12, 0.5, &["Panel L"]);
        panel(&mut o, "pr", 0.9, 0.4, 0.12, 0.5, &["Panel R"]);
        panel(&mut o, "pc", 0.5, 0.5, 0.2, 0.2, &["Core"]);
        o.groups = o.group_names();
        let r = o.suggest_roles();
        assert_eq!(r.get("Tail"), Some(Path));
        assert_eq!(r.get("Ring"), Some(Outline));
        assert_eq!(r.get("Panel L"), Some(Left));
        assert_eq!(r.get("Panel R"), Some(Right));
        assert_eq!(r.get("Core"), Some(Centre));
        assert!(matches!(r.get("Eye A"), Some(Eyes)) && matches!(r.get("Eye B"), Some(Eyes)));
        // A group that only repeats others ("Body" = Tail) gets no role of its own.
        assert_eq!(r.get("Body"), None);
    }

    #[test]
    fn a_stage_without_groups_gets_no_suggestions() {
        let mut o = AdvancedOutput { aspect: 16.0 / 9.0, ..Default::default() };
        bar(&mut o, "x", (0.1, 0.5), (0.2, 0.5), &[]);
        assert!(o.suggest_roles().is_empty());
    }

    #[test]
    fn suggestions_are_repeatable() {
        let o = AdvancedOutput::demo();
        assert_eq!(o.suggest_roles(), o.suggest_roles());
    }

    #[test]
    fn roles_follow_a_renamed_group_and_go_with_a_deleted_one() {
        let mut o = AdvancedOutput::demo();
        o.fixtures[0].groups = vec!["Left".into()];
        o.roles.set("Left", Some(lumen_core::StageRole::Fill));
        o.rename_group("Left", "Side");
        assert_eq!(o.roles.get("Side"), Some(lumen_core::StageRole::Fill));
        assert_eq!(o.roles.get("Left"), None);
        o.remove_group("Side");
        assert!(o.roles.is_empty());
    }
}
