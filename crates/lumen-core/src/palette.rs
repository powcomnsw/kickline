//! Colour palettes: four colours that clips can point at instead of a fixed
//! colour. Change the palette and every clip that uses it changes with it,
//! with a short fade, so a whole look can be recoloured on the beat.

use crate::audio::{AudioBand, AudioBands};
use serde::{Deserialize, Serialize};

pub type Rgb = [f32; 3];

/// A colour in a clip: either fixed, or "palette colour N" (0..4).
/// Serialised as a plain `[r,g,b]` array or a number, so older projects
/// that stored arrays still load.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Paint {
    Rgb(Rgb),
    Slot(u8),
}

impl Paint {
    pub const WHITE: Paint = Paint::Rgb([1.0, 1.0, 1.0]);

    pub fn resolve(self, palette: &[Rgb; 4]) -> Rgb {
        match self {
            Paint::Rgb(c) => c,
            Paint::Slot(i) => palette[(i as usize) % 4],
        }
    }
}

impl From<Rgb> for Paint {
    fn from(c: Rgb) -> Self {
        Paint::Rgb(c)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub name: String,
    pub colors: [Rgb; 4],
}

impl Palette {
    pub fn new(name: &str, colors: [Rgb; 4]) -> Self {
        Palette { name: name.into(), colors }
    }
}

fn hex(h: u32) -> Rgb {
    [((h >> 16) & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, (h & 255) as f32 / 255.0]
}

/// Starting palettes. Saturated, LED-friendly colours (no greys: on RGB pixels
/// they just look like dim white).
pub fn presets() -> Vec<Palette> {
    let p = |n: &str, c: [u32; 4]| Palette::new(n, c.map(hex));
    vec![
        p("Fire", [0xff2000, 0xff7a00, 0xffc400, 0xff0040]),
        p("Ice", [0x00b4ff, 0x0040ff, 0x80f0ff, 0xffffff]),
        p("Acid", [0x80ff00, 0x00ff60, 0xffee00, 0x00ffd0]),
        p("Sunset", [0xff3060, 0xff8a00, 0x8020ff, 0xff00a0]),
        p("Police", [0xff0000, 0x0030ff, 0xffffff, 0xff0000]),
        p("UV", [0x6000ff, 0xc000ff, 0x2000a0, 0xff00ff]),
        p("Ocean", [0x00ffb0, 0x0090ff, 0x0020a0, 0x00ffff]),
        p("Warm white", [0xffc070, 0xffa040, 0xffe0b0, 0xff8020]),
    ]
}

/// The palettes in a show, which one is live, and when to move on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Palettes {
    pub list: Vec<Palette>,
    pub active: usize,
    /// Step to the next palette on every hit of this band (Kick, Snare,
    /// HiHat, Beat or Bar). `None` = only when you pick one.
    #[serde(default)]
    pub advance_on: Option<AudioBand>,
    /// Bars between automatic changes when `advance_on` is Bar (1, 2, 4, 8…).
    #[serde(default = "one")]
    pub every: u32,
    /// Crossfade length in beats. 0 = cut.
    #[serde(default = "half")]
    pub fade_beats: f32,
    #[serde(skip)]
    from: Option<[Rgb; 4]>,
    #[serde(skip)]
    fade_start: f64,
    #[serde(skip)]
    seen_hits: Option<u64>,
    #[serde(skip)]
    counted: u32,
}

fn one() -> u32 {
    1
}
fn half() -> f32 {
    0.5
}

impl Default for Palettes {
    fn default() -> Self {
        Palettes {
            list: presets(),
            active: 0,
            advance_on: None,
            every: 1,
            fade_beats: 0.5,
            from: None,
            fade_start: 0.0,
            seen_hits: None,
            counted: 0,
        }
    }
}

impl Palettes {
    fn target(&self) -> [Rgb; 4] {
        self.list.get(self.active).map(|p| p.colors).unwrap_or([[1.0; 3]; 4])
    }

    /// The colours to draw with right now (mid-fade if a change just happened).
    pub fn current(&self, beat: f64) -> [Rgb; 4] {
        let to = self.target();
        let Some(from) = self.from else { return to };
        let k = if self.fade_beats <= 0.0 { 1.0 } else { ((beat - self.fade_start) / self.fade_beats as f64).clamp(0.0, 1.0) as f32 };
        let mut out = to;
        for i in 0..4 {
            for c in 0..3 {
                out[i][c] = from[i][c] + (to[i][c] - from[i][c]) * k;
            }
        }
        out
    }

    /// Switch to palette `i`, fading from whatever is showing now.
    pub fn select(&mut self, i: usize, beat: f64) {
        if self.list.is_empty() {
            return;
        }
        let i = i % self.list.len();
        self.from = Some(self.current(beat));
        self.fade_start = beat;
        self.active = i;
    }

    pub fn next(&mut self, beat: f64) {
        self.select(self.active + 1, beat);
    }

    /// Called each frame with the latest analysis; handles automatic changes.
    pub fn update(&mut self, audio: &AudioBands, beat: f64) {
        if let Some(from) = self.from {
            let _ = from;
            if self.fade_beats <= 0.0 || beat - self.fade_start >= self.fade_beats as f64 {
                self.from = None;
            }
        }
        let Some(slot) = self.advance_on.and_then(AudioBand::hit_slot) else {
            self.seen_hits = None;
            return;
        };
        let now = audio.hits[slot];
        if let Some(prev) = self.seen_hits {
            if now != prev {
                self.counted += 1;
                if self.counted >= self.every.max(1) {
                    self.counted = 0;
                    self.next(beat);
                }
            }
        }
        self.seen_hits = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_loads_old_arrays_and_slots() {
        let p: Paint = serde_json::from_str("[1.0,0.5,0.0]").unwrap();
        assert_eq!(p, Paint::Rgb([1.0, 0.5, 0.0]));
        let s: Paint = serde_json::from_str("2").unwrap();
        assert_eq!(s, Paint::Slot(2));
        assert_eq!(serde_json::to_string(&Paint::Slot(1)).unwrap(), "1");
        let pal = presets()[0].colors;
        assert_eq!(Paint::Slot(1).resolve(&pal), pal[1]);
    }

    #[test]
    fn fade_between_palettes() {
        let mut p = Palettes::default();
        p.fade_beats = 1.0;
        let a = p.current(0.0);
        p.select(1, 10.0);
        assert_eq!(p.current(10.0), a);
        let b = p.list[1].colors;
        assert_eq!(p.current(11.0), b);
        let mid = p.current(10.5)[0][0];
        assert!((mid - (a[0][0] + b[0][0]) / 2.0).abs() < 1e-5);
        p.update(&AudioBands::default(), 11.5);
        assert_eq!(p.current(11.5), b);
    }

    #[test]
    fn advances_every_n_bars() {
        let mut p = Palettes::default();
        p.advance_on = Some(AudioBand::Bar);
        p.every = 2;
        let mut a = AudioBands::default();
        let slot = AudioBand::Bar.hit_slot().unwrap();
        p.update(&a, 0.0);
        for bar in 1..=4u64 {
            a.hits[slot] = bar;
            p.update(&a, bar as f64 * 4.0);
        }
        assert_eq!(p.active, 2);
    }
}
