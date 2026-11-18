use crate::audio::{AudioBand, AudioBands, AudioLink};
use crate::blend::{composite, BlendMode};
use crate::effect::Effect;
use crate::frame::{Frame, Rgba};
use crate::generator::Generator;
use crate::palette::{Paint, Palettes};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    pub generator: Generator,
}

impl Clip {
    pub fn new(name: impl Into<String>, generator: Generator) -> Self {
        Clip { name: name.into(), generator }
    }
}

/// A row of clip slots. Index 0 in `Composition::layers` is the TOP layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub clips: Vec<Option<Clip>>,
    pub active: Option<usize>,
    pub opacity: f32,
    pub blend: BlendMode,
    pub effects: Vec<Effect>,
    pub bypass: bool,
    /// Layer opacity follows an audio band.
    #[serde(default)]
    pub audio_opacity: Option<AudioLink>,
    /// Jump to the next clip in this layer on every hit of this band
    /// (Kick, Snare or HiHat).
    #[serde(default)]
    pub advance_on: Option<AudioBand>,
    /// Beat position at which the active clip was triggered.
    #[serde(skip)]
    pub trigger_beat: f64,
    #[serde(skip)]
    seen_hits: Option<u64>,
}

impl Layer {
    pub fn new(name: impl Into<String>, columns: usize) -> Self {
        Layer {
            name: name.into(),
            clips: vec![None; columns],
            active: None,
            opacity: 1.0,
            blend: BlendMode::Add,
            effects: Vec::new(),
            bypass: false,
            audio_opacity: None,
            advance_on: None,
            trigger_beat: 0.0,
            seen_hits: None,
        }
    }

    fn next_clip(&self) -> Option<usize> {
        let n = self.clips.len();
        let start = self.active.map(|a| a + 1).unwrap_or(0);
        (0..n).map(|i| (start + i) % n).find(|&i| self.clips[i].is_some())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Composition {
    pub width: usize,
    pub height: usize,
    pub columns: usize,
    pub layers: Vec<Layer>,
    /// Master brightness 0..1.
    pub master: f32,
    pub bpm: f64,
    #[serde(default)]
    pub palettes: Palettes,
    #[serde(skip)]
    pub beat: f64,
    #[serde(skip)]
    pub audio: AudioBands,
}

impl Composition {
    pub fn new(width: usize, height: usize, columns: usize) -> Self {
        Composition {
            width,
            height,
            columns,
            layers: Vec::new(),
            master: 1.0,
            bpm: 120.0,
            palettes: Palettes::default(),
            beat: 0.0,
            audio: AudioBands::default(),
        }
    }

    /// A demo set showing how to split the music across layers:
    /// kick flashes, snare steps through chases, bass drives the background.
    /// Swap in an edited copy of the show (undo/redo) while keeping what's
    /// live: the playhead, audio, tempo, master, which clips are playing and
    /// the current palette. Nothing jumps or restarts.
    pub fn replace_show(&mut self, mut new: Composition) {
        new.beat = self.beat;
        new.audio = self.audio;
        new.bpm = self.bpm;
        new.master = self.master;
        for (i, l) in new.layers.iter_mut().enumerate() {
            if let Some(old) = self.layers.get(i) {
                l.active = old.active.filter(|&c| l.clips.get(c).is_some_and(|c| c.is_some()));
                l.trigger_beat = old.trigger_beat;
                l.seen_hits = old.seen_hits;
            }
        }
        new.palettes.keep_live(&self.palettes);
        *self = new;
    }

    pub fn demo() -> Self {
        let cols = 6;
        let mut c = Composition::new(64, 36, cols);

        let mut kick = Layer::new("Kick", cols);
        kick.clips[0] = Some(Clip::new("Kick white", Generator::AudioFlash { band: AudioBand::Kick, color: Paint::Rgb([1.0, 1.0, 1.0]) }));
        kick.clips[1] = Some(Clip::new("Kick red", Generator::AudioFlash { band: AudioBand::Kick, color: Paint::Rgb([1.0, 0.0, 0.0]) }));
        kick.clips[2] = Some(Clip::new("Hat blue", Generator::AudioFlash { band: AudioBand::HiHat, color: Paint::Rgb([0.3, 0.6, 1.0]) }));
        kick.opacity = 0.7;

        let mut fx = Layer::new("Snare chases", cols);
        fx.clips[0] = Some(Clip::new("White chase", Generator::Chase { color: Paint::Rgb([1.0, 1.0, 1.0]), width: 0.25, beats_per_cycle: 2.0 }));
        fx.clips[1] = Some(Clip::new("Strobe", Generator::Strobe { color: Paint::Rgb([1.0, 1.0, 1.0]), per_beat: 2.0, duty: 0.3 }));
        fx.clips[2] = Some(Clip::new("Palette chase", Generator::Chase { color: Paint::Slot(1), width: 0.4, beats_per_cycle: 4.0 }));
        fx.advance_on = Some(AudioBand::Snare);
        fx.audio_opacity = Some(AudioLink { band: AudioBand::HighMid, amount: 0.6 });

        let mut bg = Layer::new("Bass background", cols);
        bg.blend = BlendMode::Normal;
        bg.clips[0] = Some(Clip::new("Rainbow", Generator::Rainbow { scale: 1.0, beats_per_cycle: 8.0 }));
        bg.clips[1] = Some(Clip::new("Plasma", Generator::Plasma { scale: 1.0, speed: 1.0 }));
        bg.clips[2] = Some(Clip::new("Red", Generator::Solid { color: Paint::Rgb([1.0, 0.0, 0.1]) }));
        bg.clips[3] = Some(Clip::new("Blue", Generator::Solid { color: Paint::Rgb([0.0, 0.2, 1.0]) }));
        bg.clips[4] = Some(Clip::new("Bands", Generator::Bands));
        bg.clips[5] = Some(Clip::new("Palette wash", Generator::PaletteWash { beats_per_cycle: 16.0 }));
        bg.effects.push(Effect::AudioBrightness { band: AudioBand::Bass, amount: 0.7 });

        // Strip effects on top, none playing: click one to try it.
        let mut strips = Layer::new("Strip effects", cols);
        strips.blend = BlendMode::Add;
        let presets = crate::strip::StripClip::presets();
        for (slot, want) in ["Meteor", "Kick burst", "Hi-hat sparkle", "Bass meter", "Fill to the bar", "Wave across strips"].iter().enumerate() {
            if let Some((name, sc)) = presets.iter().find(|(n, _)| n == want) {
                strips.clips[slot] = Some(Clip::new(*name, Generator::Strip(*sc)));
            }
        }

        c.layers.push(strips);
        c.layers.push(kick);
        c.layers.push(fx);
        c.layers.push(bg);
        c.trigger(1, 0);
        c.trigger(3, 0);
        c
    }

    pub fn tick(&mut self, dt_seconds: f64) {
        self.beat += dt_seconds * self.bpm / 60.0;
    }

    /// Follow an external clock (Ableton Link): jump straight to its beat and tempo.
    pub fn set_clock(&mut self, beat: f64, bpm: f64) {
        // If the session's beat jumps (a peer re-synced), keep clip-relative time sane.
        let jump = beat - self.beat;
        if jump.abs() > 8.0 {
            for l in &mut self.layers {
                l.trigger_beat += jump;
            }
        }
        self.beat = beat;
        self.bpm = bpm;
    }

    /// Beat and Bar pulses from the clock: 1.0 on the beat, fading over ~150 ms.
    fn clock_pulses(&self, a: &mut AudioBands) {
        let spb = 60.0 / self.bpm.max(1.0);
        let beat = self.beat.max(0.0);
        let since_beat = beat.fract() * spb;
        let since_bar = (beat / 4.0).fract() * 4.0 * spb;
        a.set(AudioBand::Beat, (-(since_beat / 0.15)).exp() as f32);
        a.set(AudioBand::Bar, (-(since_bar / 0.25)).exp() as f32);
        a.hits[3] = beat.floor() as u64;
        a.hits[4] = (beat / 4.0).floor() as u64;
    }

    /// Feed in the latest audio analysis. Handles "advance on hit" layers.
    pub fn set_audio(&mut self, mut audio: AudioBands) {
        self.clock_pulses(&mut audio);
        self.audio = audio;
        self.palettes.update(&audio, self.beat);
        let mut advance = Vec::new();
        for (i, l) in self.layers.iter_mut().enumerate() {
            let Some(slot) = l.advance_on.and_then(AudioBand::hit_slot) else {
                l.seen_hits = None;
                continue;
            };
            let now = audio.hits[slot];
            if let Some(prev) = l.seen_hits {
                if now != prev {
                    advance.push(i);
                }
            }
            l.seen_hits = Some(now);
        }
        for i in advance {
            if let Some(next) = self.layers[i].next_clip() {
                self.trigger(i, next);
            }
        }
    }

    /// Start (or restart) the clip at (layer, column). Empty slots do nothing.
    pub fn trigger(&mut self, layer: usize, column: usize) {
        let beat = self.beat;
        if let Some(l) = self.layers.get_mut(layer) {
            if matches!(l.clips.get(column), Some(Some(_))) {
                l.active = Some(column);
                l.trigger_beat = beat;
            }
        }
    }

    /// Trigger a whole column; layers with an empty slot there are left alone.
    pub fn trigger_column(&mut self, column: usize) {
        for i in 0..self.layers.len() {
            self.trigger(i, column);
        }
    }

    pub fn stop(&mut self, layer: usize) {
        if let Some(l) = self.layers.get_mut(layer) {
            l.active = None;
        }
    }

    /// Everything needed to lay strip clips over the mapped canvas this frame.
    pub fn strip_stage(&self) -> StripStage {
        let palette = self.palettes.current(self.beat);
        let mut passes = Vec::new();
        for layer in self.layers.iter().rev() {
            if layer.bypass {
                continue;
            }
            let Some(Some(clip)) = layer.active.and_then(|i| layer.clips.get(i)) else { continue };
            let Generator::Strip(sc) = clip.generator else { continue };
            let mut opacity = layer.opacity;
            if let Some(link) = layer.audio_opacity {
                opacity *= link.factor(&self.audio);
            }
            if opacity <= 0.0 {
                continue;
            }
            passes.push(StripPass {
                clip: sc,
                beat: (self.beat - layer.trigger_beat).max(0.0),
                opacity,
                blend: layer.blend,
                effects: layer.effects.clone(),
            });
        }
        StripStage { passes, palette, audio: self.audio, master: self.master }
    }

    pub fn render(&self) -> Frame {
        let mut out = Frame::black(self.width, self.height);
        let mut scratch = Frame::transparent(self.width, self.height);
        let palette = self.palettes.current(self.beat);
        for layer in self.layers.iter().rev() {
            if layer.bypass {
                continue;
            }
            let Some(Some(clip)) = layer.active.and_then(|i| layer.clips.get(i)) else { continue };
            if clip.generator.is_strip() {
                continue; // drawn per pixel at output time (strip_stage)
            }
            let local_beat = (self.beat - layer.trigger_beat).max(0.0);
            scratch.pixels.fill(Default::default());
            clip.generator.render(&mut scratch, local_beat, &self.audio, &palette);
            for e in &layer.effects {
                e.apply(&mut scratch, local_beat, &self.audio);
            }
            let mut opacity = layer.opacity;
            if let Some(link) = layer.audio_opacity {
                opacity *= link.factor(&self.audio);
            }
            composite(&mut out, &scratch, layer.blend, opacity);
        }
        for p in &mut out.pixels {
            p.r *= self.master;
            p.g *= self.master;
            p.b *= self.master;
        }
        out
    }
}

/// One active strip clip with its layer's settings.
#[derive(Clone, Debug)]
pub struct StripPass {
    pub clip: crate::strip::StripClip,
    pub beat: f64,
    pub opacity: f32,
    pub blend: BlendMode,
    pub effects: Vec<crate::effect::Effect>,
}

/// The strip layers of one frame, ready to shade pixels.
#[derive(Clone, Debug, Default)]
pub struct StripStage {
    pub passes: Vec<StripPass>,
    pub palette: [crate::palette::Rgb; 4],
    pub audio: AudioBands,
    pub master: f32,
}

impl StripStage {
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    /// Lay the strip layers over a pixel's mapped colour.
    pub fn shade(&self, base: Rgba, p: crate::strip::StripPixel) -> Rgba {
        if self.passes.is_empty() {
            return base;
        }
        let mut px = Frame::new(1, 1, base);
        let mut src = Frame::transparent(1, 1);
        for pass in &self.passes {
            let mut c = pass.clip.shade(p, pass.beat, &self.audio, &self.palette);
            c.r *= self.master;
            c.g *= self.master;
            c.b *= self.master;
            src.pixels[0] = c;
            for e in &pass.effects {
                e.apply(&mut src, pass.beat, &self.audio);
            }
            composite(&mut px, &src, pass.blend, pass.opacity);
        }
        px.pixels[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_layers_skip_the_canvas_and_shade_pixels() {
        use crate::strip::{StripClip, StripEffect, StripPixel};
        let mut c = Composition::new(4, 4, 1);
        let mut l = Layer::new("Strips", 1);
        l.blend = BlendMode::Normal;
        l.clips[0] = Some(Clip::new("Fill", Generator::Strip(StripClip::new(StripEffect::Fill { color: Paint::Rgb([0.0, 1.0, 0.0]), beats: 4.0 }))));
        c.layers.push(l);
        c.trigger(0, 0);
        c.beat = 2.0;
        // Nothing on the canvas…
        assert!(c.render().pixels.iter().all(|p| p.g == 0.0));
        // …but pixels along a strip get it.
        let st = c.strip_stage();
        let base = Rgba::rgb(1.0, 0.0, 0.0);
        let at = |x: f32| st.shade(base, StripPixel { fixture: 0, fixtures: 1, index: 0, count: 10, x, y: 0.0 });
        assert_eq!((at(0.1).r, at(0.1).g), (0.0, 1.0));
        assert_eq!((at(0.9).r, at(0.9).g), (1.0, 0.0));
    }

    #[test]
    fn replace_show_keeps_playback() {
        let mut c = Composition::demo();
        c.beat = 12.5;
        c.bpm = 128.0;
        c.layers[0].active = Some(1);
        let mut edited = c.clone();
        edited.layers[0].name = "Renamed".into();
        edited.layers[0].active = None;
        edited.bpm = 90.0;
        edited.beat = 0.0;
        c.replace_show(edited);
        assert_eq!(c.layers[0].name, "Renamed");
        assert_eq!(c.layers[0].active, Some(1));
        assert_eq!((c.beat, c.bpm), (12.5, 128.0));
        // A playing clip that no longer exists stops.
        let mut gone = c.clone();
        gone.layers[0].clips[1] = None;
        c.replace_show(gone);
        assert_eq!(c.layers[0].active, None);
    }

    #[test]
    fn empty_composition_is_black() {
        let c = Composition::new(4, 4, 2);
        assert!(c.render().pixels.iter().all(|p| p.r == 0.0 && p.g == 0.0 && p.b == 0.0));
    }

    #[test]
    fn trigger_and_master() {
        let mut c = Composition::new(2, 2, 2);
        let mut l = Layer::new("L", 2);
        l.blend = BlendMode::Normal;
        l.clips[1] = Some(Clip::new("w", Generator::Solid { color: Paint::Rgb([1.0, 1.0, 1.0]) }));
        c.layers.push(l);
        c.trigger(0, 0); // empty slot: ignored
        assert_eq!(c.layers[0].active, None);
        c.trigger(0, 1);
        assert_eq!(c.render().pixels[0].r, 1.0);
        c.master = 0.5;
        assert!((c.render().pixels[0].r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn top_layer_wins_with_normal_blend() {
        let mut c = Composition::new(1, 1, 1);
        let mk = |col| {
            let mut l = Layer::new("L", 1);
            l.blend = BlendMode::Normal;
            l.clips[0] = Some(Clip::new("c", Generator::Solid { color: col }));
            l
        };
        c.layers.push(mk(Paint::Rgb([1.0, 0.0, 0.0]))); // top
        c.layers.push(mk(Paint::Rgb([0.0, 0.0, 1.0]))); // bottom
        c.trigger_column(0);
        let p = c.render().pixels[0];
        assert_eq!((p.r, p.b), (1.0, 0.0));
    }

    #[test]
    fn audio_opacity_link() {
        let mut c = Composition::new(1, 1, 1);
        let mut l = Layer::new("L", 1);
        l.blend = BlendMode::Normal;
        l.clips[0] = Some(Clip::new("w", Generator::Solid { color: Paint::Rgb([1.0, 1.0, 1.0]) }));
        l.audio_opacity = Some(AudioLink { band: AudioBand::Bass, amount: 1.0 });
        c.layers.push(l);
        c.trigger(0, 0);
        assert_eq!(c.render().pixels[0].r, 0.0); // silence
        let mut a = AudioBands::default();
        a.set(AudioBand::Bass, 1.0);
        c.set_audio(a);
        assert_eq!(c.render().pixels[0].r, 1.0);
    }

    #[test]
    fn advance_on_snare_skips_empty_and_wraps() {
        let mut c = Composition::new(1, 1, 4);
        let mut l = Layer::new("L", 4);
        l.clips[0] = Some(Clip::new("a", Generator::Solid { color: Paint::Rgb([1.0, 0.0, 0.0]) }));
        l.clips[2] = Some(Clip::new("b", Generator::Solid { color: Paint::Rgb([0.0, 1.0, 0.0]) }));
        l.advance_on = Some(AudioBand::Snare);
        c.layers.push(l);
        c.trigger(0, 0);
        let mut a = AudioBands::default();
        c.set_audio(a); // first sight: just records the counter
        assert_eq!(c.layers[0].active, Some(0));
        a.hits[1] += 1; // snare
        c.set_audio(a);
        assert_eq!(c.layers[0].active, Some(2));
        c.set_audio(a); // no new hit
        assert_eq!(c.layers[0].active, Some(2));
        a.hits[0] += 1; // kick: ignored by this layer
        c.set_audio(a);
        assert_eq!(c.layers[0].active, Some(2));
        a.hits[1] += 1;
        c.set_audio(a);
        assert_eq!(c.layers[0].active, Some(0)); // wrapped
    }

    #[test]
    fn beat_and_bar_from_clock() {
        let mut c = Composition::new(1, 1, 2);
        let mut l = Layer::new("L", 2);
        l.clips[0] = Some(Clip::new("a", Generator::Solid { color: Paint::Rgb([1.0, 0.0, 0.0]) }));
        l.clips[1] = Some(Clip::new("b", Generator::Solid { color: Paint::Rgb([0.0, 1.0, 0.0]) }));
        l.advance_on = Some(AudioBand::Bar);
        c.layers.push(l);
        c.trigger(0, 0);
        c.set_clock(0.01, 120.0);
        c.set_audio(AudioBands::default());
        assert!(c.audio.get(AudioBand::Beat) > 0.9); // right on the beat
        c.set_clock(0.5, 120.0);
        c.set_audio(AudioBands::default());
        assert!(c.audio.get(AudioBand::Beat) < 0.3); // half a beat later it has faded
        c.set_clock(3.9, 120.0);
        c.set_audio(AudioBands::default());
        assert_eq!(c.layers[0].active, Some(0)); // still in bar 0
        c.set_clock(4.02, 120.0);
        c.set_audio(AudioBands::default());
        assert_eq!(c.layers[0].active, Some(1)); // new bar -> next clip
        assert_eq!(c.audio.hits[3], 4);
    }

    #[test]
    fn clock_jump_keeps_clip_time() {
        let mut c = Composition::new(1, 1, 1);
        let mut l = Layer::new("L", 1);
        l.clips[0] = Some(Clip::new("a", Generator::Solid { color: Paint::Rgb([1.0, 0.0, 0.0]) }));
        c.layers.push(l);
        c.set_clock(10.0, 120.0);
        c.trigger(0, 0);
        c.set_clock(1000.0, 120.0); // peer joined with a very different timeline
        assert!((c.beat - c.layers[0].trigger_beat - 0.0).abs() < 1e-9);
    }

    #[test]
    fn json_roundtrip() {
        let c = Composition::demo();
        let s = serde_json::to_string(&c).unwrap();
        let back: Composition = serde_json::from_str(&s).unwrap();
        assert_eq!(back.layers.len(), c.layers.len());
        assert_eq!(back.layers[3].clips[0].as_ref().unwrap().name, "Rainbow");
        assert_eq!(back.layers[2].advance_on, Some(AudioBand::Snare));
        assert!(back.layers[0].clips[0].as_ref().unwrap().generator.is_strip());
    }
}
