use crate::director::{Director, DirectorSettings, Input as DirectorInput};
use crate::sections::{LayerLook, Looks, Section, SectionDetector};
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
    /// How fast the clip runs against the clock (0.25 to 4 is usual).
    #[serde(default = "clip_speed")]
    pub speed: f32,
    /// Loop for ever, or play once and go dark.
    #[serde(default)]
    pub transport: Transport,
    /// How long a one-shot plays, in clip beats.
    #[serde(default = "eight")]
    pub length_beats: f32,
    /// Clip beats to fade in after a trigger.
    #[serde(default)]
    pub fade_in: f32,
    /// Clip beats to fade out: at the end of a one-shot, and when the clip is stopped.
    #[serde(default)]
    pub fade_out: f32,
}

fn clip_speed() -> f32 {
    1.0
}

fn half() -> f32 {
    0.5
}

fn eight() -> f32 {
    8.0
}

/// Which side of the crossfader a layer sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Deck {
    /// Not on a deck: the crossfader leaves it alone.
    #[default]
    Both,
    A,
    B,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    #[default]
    Loop,
    OneShot,
}

impl Clip {
    pub fn new(name: impl Into<String>, generator: Generator) -> Self {
        Clip { name: name.into(), generator, speed: 1.0, transport: Transport::Loop, length_beats: 8.0, fade_in: 0.0, fade_out: 0.0 }
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
    /// Only light this group of fixtures (a "slice"); None = every fixture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// What this layer does in each song section's look.
    #[serde(default)]
    pub looks: [LayerLook; 5],
    /// Beat position at which the active clip was triggered.
    #[serde(skip)]
    pub trigger_beat: f64,
    /// A or B side of the crossfader (Both = not on a deck).
    #[serde(default)]
    pub deck: Deck,
    #[serde(skip)]
    seen_hits: Option<u64>,
    /// Beat at which the clip was told to stop and is fading out.
    #[serde(skip)]
    pub released_at: Option<f64>,
}

impl Layer {
    /// How much of the layer the crossfader lets through.
    pub fn deck_gain(&self, crossfade: f32) -> f32 {
        let x = crossfade.clamp(0.0, 1.0);
        match self.deck {
            Deck::Both => 1.0,
            Deck::A => (2.0 * (1.0 - x)).min(1.0),
            Deck::B => (2.0 * x).min(1.0),
        }
    }

    /// The active clip.
    pub fn clip(&self) -> Option<&Clip> {
        self.active.and_then(|i| self.clips.get(i)).and_then(|c| c.as_ref())
    }

    /// The active clip's own beat at clock `beat`: time since the trigger, scaled by its speed.
    pub fn local_beat(&self, beat: f64) -> f64 {
        let speed = self.clip().map_or(1.0, |c| c.speed.clamp(0.0625, 16.0)) as f64;
        (beat - self.trigger_beat).max(0.0) * speed
    }

    /// What the layer plays at clock `beat`: the clip's own beat and its fade
    /// (0 to 1), or None when nothing is playing (no clip, a finished
    /// one-shot, or a fade-out that has run out).
    pub fn playing(&self, beat: f64) -> Option<(f64, f32)> {
        let clip = self.clip()?;
        let local = self.local_beat(beat);
        let mut fade = 1.0f32;
        if clip.fade_in > 0.0 {
            fade = fade.min((local / clip.fade_in as f64) as f32);
        }
        if clip.transport == Transport::OneShot {
            if local >= clip.length_beats as f64 {
                return None;
            }
            if clip.fade_out > 0.0 {
                fade = fade.min(((clip.length_beats as f64 - local) / clip.fade_out as f64) as f32);
            }
        }
        if let Some(r) = self.released_at {
            let rel = (beat - r).max(0.0) * clip.speed.clamp(0.0625, 16.0) as f64;
            let gone = if clip.fade_out > 0.0 { (rel / clip.fade_out as f64) as f32 } else { 1.0 };
            if gone >= 1.0 {
                return None;
            }
            fade *= 1.0 - gone;
        }
        Some((local, fade.clamp(0.0, 1.0)))
    }

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
            target: None,
            looks: [LayerLook::Keep; 5],
            trigger_beat: 0.0,
            deck: Deck::Both,
            seen_hits: None,
            released_at: None,
        }
    }

    fn next_clip(&self) -> Option<usize> {
        let n = self.clips.len();
        let start = self.active.map(|a| a + 1).unwrap_or(0);
        (0..n).map(|i| (start + i) % n).find(|&i| self.clips[i].is_some())
    }
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Composition {
    pub width: usize,
    pub height: usize,
    pub columns: usize,
    pub layers: Vec<Layer>,
    /// Master brightness 0..1.
    pub master: f32,
    /// Crossfader: 0 = all deck A, 1 = all deck B, the middle = both at full.
    #[serde(default = "half")]
    pub crossfade: f32,
    /// Left, centre and right faders for the stage roles.
    #[serde(default)]
    pub gains: crate::roles::RoleGains,
    /// MIDI and OSC controls (saved with the show).
    #[serde(default)]
    pub controls: crate::controls::ControlMap,
    pub bpm: f64,
    #[serde(default)]
    pub palettes: Palettes,
    /// A look for each song section, switched automatically if `looks.auto`.
    #[serde(default)]
    pub looks: Looks,
    /// Auto mode's settings.
    #[serde(default)]
    pub director: DirectorSettings,
    /// Listens for song sections.
    #[serde(skip)]
    pub sections: SectionDetector,
    /// Auto mode's own layers (pairs, for crossfades). Under the user's layers.
    #[serde(skip)]
    pub auto: Vec<Layer>,
    /// What Auto is doing right now.
    #[serde(skip)]
    pub director_rt: Director,
    /// Groups in the show, for Auto's per-group motion (set by the engine).
    #[serde(skip)]
    pub auto_groups: Vec<String>,
    /// What each group is for (set by the engine from the stage).
    #[serde(skip)]
    pub auto_roles: crate::roles::RoleMap,
    /// The music has stopped for good (set by the engine).
    #[serde(skip)]
    pub idle: bool,
    /// Brightness from the current section's look (1 when looks are off).
    #[serde(skip, default = "one")]
    pub look_level: f32,
    /// The beat the lights are drawn at (the analysis beat moved earlier by the
    /// output lookahead).
    #[serde(skip)]
    pub beat: f64,
    /// The beat aligned to the audio as it arrives: sections and Auto use it.
    #[serde(skip)]
    pub analysis_beat: f64,
    /// Beats since the "1" of the bar, when the clock knows.
    #[serde(skip)]
    pub bar_pos: Option<f64>,
    /// The clock can be trusted (a locked tracker or Link).
    #[serde(skip)]
    pub steady: bool,
    /// Seconds since the engine started (advanced by the engine).
    #[serde(skip)]
    pub time_s: f64,
    #[serde(skip)]
    pub phrase: crate::set::Phrase,
    #[serde(skip)]
    pub memory: crate::set::SetMemory,
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
            crossfade: 0.5,
            gains: Default::default(),
            controls: Default::default(),
            bpm: 120.0,
            palettes: Palettes::default(),
            looks: Looks::default(),
            director: DirectorSettings::default(),
            sections: SectionDetector::default(),
            auto: Vec::new(),
            director_rt: Director::default(),
            auto_groups: Vec::new(),
            auto_roles: Default::default(),
            idle: false,
            look_level: 1.0,
            beat: 0.0,
            analysis_beat: 0.0,
            bar_pos: None,
            steady: false,
            time_s: 0.0,
            phrase: Default::default(),
            memory: Default::default(),
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
        new.sections = std::mem::take(&mut self.sections);
        new.auto = std::mem::take(&mut self.auto);
        new.director_rt = std::mem::take(&mut self.director_rt);
        new.auto_groups = std::mem::take(&mut self.auto_groups);
        new.auto_roles = std::mem::take(&mut self.auto_roles);
        new.look_level = self.look_level;
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
        self.analysis_beat = self.beat;
        self.bar_pos = None;
        self.steady = false;
    }

    /// Take the decided clock: the render beat the lights use, the analysis
    /// beat the detectors use, and the tempo.
    pub fn set_clock_out(&mut self, o: &crate::clock::ClockOut) {
        self.set_clock(o.render_beat, o.tempo);
        self.analysis_beat = o.analysis_beat;
        self.bar_pos = o.bar_pos;
        self.steady = o.steady;
    }

    /// Follow an external clock (Ableton Link): jump straight to its beat and tempo.
    pub fn set_clock(&mut self, beat: f64, bpm: f64) {
        // If the session's beat jumps (a peer re-synced), keep clip-relative time sane.
        let jump = beat - self.beat;
        if jump.abs() > 8.0 {
            for l in &mut self.layers {
                l.trigger_beat += jump;
                if let Some(r) = &mut l.released_at {
                    *r += jump;
                }
            }
        }
        self.beat = beat;
        self.analysis_beat = beat;
        self.bpm = bpm;
        self.settle_releases();
    }

    /// Beat and Bar pulses from the clock: 1.0 on the beat, fading over ~150 ms.
    fn clock_pulses(&self, a: &mut AudioBands) {
        let spb = 60.0 / self.bpm.max(1.0);
        let beat = self.beat.max(0.0);
        let since_beat = beat.fract() * spb;
        let bar = self.sections.profile.beats_per_bar.clamp(2, 12) as f64;
        let since_bar = (beat / bar).fract() * bar * spb;
        a.set(AudioBand::Beat, (-(since_beat / 0.15)).exp() as f32);
        a.set(AudioBand::Bar, (-(since_bar / 0.25)).exp() as f32);
        a.hits[3] = beat.floor() as u64;
        a.hits[4] = (beat / bar).floor() as u64;
    }

    /// Feed in the latest audio analysis. Handles "advance on hit" layers.
    pub fn set_audio(&mut self, mut audio: AudioBands) {
        self.clock_pulses(&mut audio);
        self.audio = audio;
        self.palettes.update(&audio, self.beat);
        self.sections.set_profile(self.director.genre.profile());
        let bpb = self.sections.profile.beats_per_bar;
        self.phrase.update(self.analysis_beat, self.bar_pos, bpb);
        self.memory.tempo(self.time_s, self.bpm, self.steady);
        if let Some(s) = self.sections.update(&audio, self.analysis_beat) {
            self.memory.section(s);
            // A drop, or the groove returning after a pause, starts a phrase.
            if matches!(s, Section::Drop) || (s == Section::Groove && matches!(self.sections.past.back(), Some((Section::Pause, _)))) {
                self.phrase.anchor();
            }
            if self.looks.auto {
                self.apply_look(s);
            }
        }
        self.run_director();
        if !self.looks.auto {
            self.look_level = 1.0;
        }
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

    /// Layers from the top: yours, then Auto's.
    fn stack(&self) -> impl DoubleEndedIterator<Item = &Layer> + '_ {
        self.layers.iter().chain(self.auto.iter())
    }

    /// One frame of Auto mode.
    fn run_director(&mut self) {
        let user_playing = self.layers.iter().any(|l| !l.bypass && l.active.is_some() && l.opacity > 0.0);
        let input = DirectorInput {
            audio: &self.audio,
            beat: self.beat,
            section: self.sections.current(),
            beats_in: self.sections.beats_in,
            groups: &self.auto_groups,
            user_playing,
            phrase: self.phrase,
            steady: self.steady,
            drops: self.memory.drops,
            roles: &self.auto_roles,
            idle: self.idle,
        };
        self.director_rt.step(&self.director, &input, &mut self.auto, &mut self.palettes);
    }

    /// Switch to a section's look: each layer's clip, the palette, brightness.
    pub fn apply_look(&mut self, s: Section) {
        let i = s.index();
        for li in 0..self.layers.len() {
            match self.layers[li].looks[i] {
                LayerLook::Keep => {}
                LayerLook::Stop => self.stop(li),
                LayerLook::Play(c) => {
                    if self.layers[li].active != Some(c) {
                        self.trigger(li, c);
                    }
                }
            }
        }
        if let Some(p) = self.looks.palette[i] {
            if p != self.palettes.active && p < self.palettes.list.len() {
                self.palettes.select(p, self.beat);
            }
        }
        self.look_level = self.looks.level[i].clamp(0.0, 1.0);
    }

    /// Start (or restart) the clip at (layer, column). Empty slots do nothing.
    pub fn trigger(&mut self, layer: usize, column: usize) {
        let beat = self.beat;
        if let Some(l) = self.layers.get_mut(layer) {
            if matches!(l.clips.get(column), Some(Some(_))) {
                l.active = Some(column);
                l.trigger_beat = beat;
                l.released_at = None;
            }
        }
    }

    /// Trigger a whole column; layers with an empty slot there are left alone.
    pub fn trigger_column(&mut self, column: usize) {
        for i in 0..self.layers.len() {
            self.trigger(i, column);
        }
    }

    /// Stop a layer's clip: at once, or by fading out if the clip has a fade-out time.
    pub fn stop(&mut self, layer: usize) {
        let beat = self.beat;
        if let Some(l) = self.layers.get_mut(layer) {
            if l.clip().is_some_and(|c| c.fade_out > 0.0) && l.released_at.is_none() {
                l.released_at = Some(beat);
            } else {
                l.active = None;
                l.released_at = None;
            }
        }
    }

    /// Finish stops whose fade-out has run out.
    fn settle_releases(&mut self) {
        let beat = self.beat;
        for l in &mut self.layers {
            if l.released_at.is_some() && l.playing(beat).is_none() {
                l.active = None;
                l.released_at = None;
            }
        }
    }

    /// Groups that active layers are aimed at (each once).
    pub fn targeted_groups(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in self.stack() {
            if l.bypass || l.active.is_none() {
                continue;
            }
            if let Some(t) = &l.target {
                if !out.contains(t) {
                    out.push(t.clone());
                }
            }
        }
        out
    }

    /// Everything needed to lay strip clips over the mapped canvas this frame.
    pub fn strip_stage(&self) -> StripStage {
        let palette = self.palettes.current(self.beat);
        let mut passes = Vec::new();
        for layer in self.stack().rev() {
            if layer.bypass {
                continue;
            }
            let Some(Some(clip)) = layer.active.and_then(|i| layer.clips.get(i)) else { continue };
            let Generator::Strip(sc) = clip.generator else { continue };
            let Some((local, fade)) = layer.playing(self.beat) else { continue };
            let mut opacity = layer.opacity * fade * layer.deck_gain(self.crossfade);
            if let Some(link) = layer.audio_opacity {
                opacity *= link.factor(&self.audio);
            }
            if opacity <= 0.0 {
                continue;
            }
            passes.push(StripPass {
                clip: sc,
                beat: local,
                opacity,
                blend: layer.blend,
                effects: layer.effects.clone(),
                target: layer.target.clone(),
            });
        }
        StripStage { passes, palette, audio: self.audio, master: self.master * self.look_level }
    }

    /// The picture for fixtures outside every group (layers aimed at a group
    /// are left out).
    pub fn render(&self) -> Frame {
        self.render_with(&|_| None)
    }

    /// The picture for fixtures in some groups: `member(group)` gives the
    /// group's area on the canvas if these fixtures belong to it. Layers aimed
    /// at a group are drawn across that group's area only.
    pub fn render_with(&self, member: &dyn Fn(&str) -> Option<[f32; 4]>) -> Frame {
        let mut out = Frame::black(self.width, self.height);
        let mut scratch = Frame::transparent(self.width, self.height);
        let palette = self.palettes.current(self.beat);
        for layer in self.stack().rev() {
            if layer.bypass {
                continue;
            }
            let Some(Some(clip)) = layer.active.and_then(|i| layer.clips.get(i)) else { continue };
            if clip.generator.is_strip() {
                continue; // drawn per pixel at output time (strip_stage)
            }
            scratch.area = match &layer.target {
                None => [0.0, 0.0, 1.0, 1.0],
                Some(g) => match member(g) {
                    Some(a) => a,
                    None => continue,
                },
            };
            let Some((local_beat, fade)) = layer.playing(self.beat) else { continue };
            scratch.pixels.fill(Default::default());
            clip.generator.render(&mut scratch, local_beat, &self.audio, &palette);
            for e in &layer.effects {
                e.apply(&mut scratch, local_beat, &self.audio);
            }
            let mut opacity = layer.opacity * fade * layer.deck_gain(self.crossfade);
            if let Some(link) = layer.audio_opacity {
                opacity *= link.factor(&self.audio);
            }
            composite(&mut out, &scratch, layer.blend, opacity);
        }
        let master = self.master * self.look_level;
        for p in &mut out.pixels {
            p.r *= master;
            p.g *= master;
            p.b *= master;
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
    /// Only fixtures in this group.
    pub target: Option<String>,
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

    /// Lay the strip layers over a pixel's mapped colour (fixtures outside
    /// every group).
    pub fn shade(&self, base: Rgba, p: crate::strip::StripPixel) -> Rgba {
        self.shade_in(base, p, &|_| None)
    }

    /// Like `shade`, for a fixture in some groups: `member(group)` gives
    /// (this fixture's place in the group, the group's size). A layer aimed
    /// at a group ripples across that group's fixtures only.
    pub fn shade_in(&self, base: Rgba, p: crate::strip::StripPixel, member: &dyn Fn(&str) -> Option<(usize, usize)>) -> Rgba {
        if self.passes.is_empty() {
            return base;
        }
        let mut px = Frame::new(1, 1, base);
        let mut src = Frame::transparent(1, 1);
        for pass in &self.passes {
            let p = match &pass.target {
                None => p,
                Some(g) => match member(g) {
                    Some((fixture, fixtures)) => crate::strip::StripPixel { fixture, fixtures, ..p },
                    None => continue,
                },
            };
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
    fn layer_aimed_at_a_group_only_lights_that_group() {
        let mut c = Composition::new(10, 1, 1);
        let mut l = Layer::new("Left only", 1);
        l.blend = BlendMode::Normal;
        l.clips[0] = Some(Clip::new("w", Generator::Solid { color: Paint::Rgb([1.0, 1.0, 1.0]) }));
        l.target = Some("Left".into());
        c.layers.push(l);
        c.trigger(0, 0);
        assert_eq!(c.render().pixels[0].r, 0.0); // fixtures outside the group stay dark
        let left = c.render_with(&|g| (g == "Left").then_some([0.0, 0.0, 0.5, 1.0]));
        assert_eq!(left.pixels[0].r, 1.0);
        let right = c.render_with(&|g| (g == "Right").then_some([0.5, 0.0, 1.0, 1.0]));
        assert_eq!(right.pixels[0].r, 0.0);
        assert_eq!(c.targeted_groups(), vec!["Left".to_string()]);
    }

    #[test]
    fn strip_layer_ripples_across_its_group() {
        use crate::strip::{StripClip, StripEffect, StripPixel};
        let mut c = Composition::new(4, 4, 1);
        let mut l = Layer::new("Strips", 1);
        l.blend = BlendMode::Normal;
        let mut sc = StripClip::new(StripEffect::Fill { color: Paint::Rgb([0.0, 1.0, 0.0]), beats: 4.0 });
        sc.stagger = 2.0;
        l.clips[0] = Some(Clip::new("Fill", Generator::Strip(sc)));
        l.target = Some("Panels".into());
        c.layers.push(l);
        c.trigger(0, 0);
        c.beat = 2.5;
        let st = c.strip_stage();
        let base = Rgba::rgb(1.0, 0.0, 0.0);
        let p = StripPixel { fixture: 7, fixtures: 9, index: 0, count: 10, x: 0.5, y: 0.0 };
        // Not in the group: untouched.
        assert_eq!(st.shade(base, p).r, 1.0);
        // First in the group: already filling; last in the group: waits for the ripple.
        assert!(st.shade_in(base, p, &|_| Some((0, 2))).g > 0.5, "{:?}", st.shade_in(base, p, &|_| Some((0, 2))));
        assert!(st.shade_in(base, p, &|_| Some((1, 2))).g < 0.1);
    }

    #[test]
    fn looks_switch_with_the_section() {
        let mut c = Composition::demo();
        c.palettes.list.push(c.palettes.list[0].clone());
        let drop = Section::Drop.index();
        c.layers[2].looks[drop] = LayerLook::Play(1);
        c.layers[1].looks[drop] = LayerLook::Stop;
        c.looks.palette[drop] = Some(1);
        c.looks.level[drop] = 0.5;
        c.apply_look(Section::Drop);
        assert_eq!(c.layers[2].active, Some(1));
        assert_eq!(c.layers[1].active, None);
        assert_eq!(c.palettes.active, 1);
        assert_eq!(c.look_level, 0.5);
        let s = serde_json::to_string(&c).unwrap();
        let back: Composition = serde_json::from_str(&s).unwrap();
        assert_eq!(back.layers[2].looks[drop], LayerLook::Play(1));
        assert_eq!(back.look_level, 1.0);
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

    #[test]
    fn auto_mode_lights_an_empty_show_and_is_not_saved() {
        let mut c = Composition::new(16, 8, 6);
        c.director.on = true;
        let mut a = AudioBands::default();
        a.set(AudioBand::Level, 0.8);
        // Music is playing (raw loudness), so this is a groove, not a pause.
        a.raw[AudioBand::Level.index()] = 0.8;
        a.raw[AudioBand::Bass.index()] = 0.6;
        for _ in 0..400 {
            c.tick(1.0 / 44.0);
            c.set_audio(a);
        }
        assert!(!c.auto.is_empty());
        let lit = c.render().pixels.iter().any(|p| p.r + p.g + p.b > 0.05);
        assert!(lit, "Auto produced nothing");
        // The layers Auto runs are not part of the show file.
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("Auto Wash"), "auto layers leaked into the save");
        assert!(json.contains("\"director\""));
        // A show saved before Auto existed still opens, with Auto off.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("director");
        let back: Composition = serde_json::from_value(v).unwrap();
        assert!(!back.director.on);
    }

    #[test]
    fn replace_show_keeps_auto_running() {
        let mut c = Composition::new(16, 8, 6);
        c.director.on = true;
        let mut a = AudioBands::default();
        a.set(AudioBand::Level, 0.8);
        for _ in 0..100 {
            c.tick(1.0 / 44.0);
            c.set_audio(a);
        }
        let n = c.auto.len();
        let mut edited = c.clone();
        edited.master = 0.5;
        c.replace_show(edited);
        assert_eq!(c.auto.len(), n, "undo/redo must not restart Auto");
    }

    // ---- Clip speed, transport and fades

    fn white_layer(clip: Clip) -> Composition {
        let mut c = Composition::new(2, 2, 1);
        let mut l = Layer::new("L", 1);
        l.blend = BlendMode::Normal;
        l.clips[0] = Some(clip);
        c.layers.push(l);
        c
    }

    fn solid() -> Clip {
        Clip::new("w", Generator::Solid { color: Paint::Rgb([1.0, 1.0, 1.0]) })
    }

    fn level(c: &Composition) -> f32 {
        c.render().pixels[0].r
    }

    #[test]
    fn a_clip_speed_scales_its_own_beat_not_the_clock() {
        let mut clip = Clip::new("m", Generator::Strobe { color: Paint::Rgb([1.0, 1.0, 1.0]), per_beat: 1.0, duty: 0.5 });
        let mut c = white_layer(clip.clone());
        c.trigger(0, 0);
        c.set_clock(0.25, 120.0);
        let normal = level(&c);
        clip.speed = 2.0;
        let mut c2 = white_layer(clip);
        c2.trigger(0, 0);
        c2.set_clock(0.25, 120.0);
        // At 2x, a quarter beat of clock is half a beat of the clip: past the half-duty edge.
        assert!(normal > 0.0 && level(&c2) == 0.0, "{normal} vs {}", level(&c2));
        assert_eq!(c.layers[0].local_beat(c.beat), 0.25);
        assert_eq!(c2.layers[0].local_beat(c2.beat), 0.5);
    }

    #[test]
    fn a_loop_plays_on_and_a_one_shot_plays_once_then_goes_dark() {
        let mut one = solid();
        one.transport = Transport::OneShot;
        one.length_beats = 4.0;
        let mut c = white_layer(one);
        c.trigger(0, 0);
        c.set_clock(3.9, 120.0);
        assert_eq!(level(&c), 1.0);
        c.set_clock(4.1, 120.0);
        assert_eq!(level(&c), 0.0, "the one-shot is over");
        // Re-triggering plays it again.
        c.trigger(0, 0);
        c.set_clock(4.2, 120.0);
        assert_eq!(level(&c), 1.0);
        let mut c = white_layer(solid());
        c.trigger(0, 0);
        c.set_clock(500.0, 120.0);
        assert_eq!(level(&c), 1.0, "a loop never ends");
    }

    #[test]
    fn a_one_shot_length_follows_its_speed() {
        let mut one = solid();
        one.transport = Transport::OneShot;
        one.length_beats = 4.0;
        one.speed = 2.0;
        let mut c = white_layer(one);
        c.trigger(0, 0);
        c.set_clock(1.9, 120.0);
        assert_eq!(level(&c), 1.0);
        c.set_clock(2.1, 120.0);
        assert_eq!(level(&c), 0.0, "4 beats of clip at 2x is 2 beats of clock");
    }

    #[test]
    fn a_clip_fades_in_over_its_fade_time() {
        let mut clip = solid();
        clip.fade_in = 4.0;
        let mut c = white_layer(clip);
        c.set_clock(10.0, 120.0);
        c.trigger(0, 0);
        c.set_clock(12.0, 120.0);
        assert!((level(&c) - 0.5).abs() < 1e-5, "{}", level(&c));
        c.set_clock(14.0, 120.0);
        assert!((level(&c) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_one_shot_fades_out_over_its_last_beats() {
        let mut clip = solid();
        clip.transport = Transport::OneShot;
        clip.length_beats = 8.0;
        clip.fade_out = 2.0;
        let mut c = white_layer(clip);
        c.trigger(0, 0);
        c.set_clock(7.0, 120.0);
        assert!((level(&c) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn stopping_a_clip_with_a_fade_out_lets_it_go_gently_then_stops() {
        let mut clip = solid();
        clip.fade_out = 4.0;
        let mut c = white_layer(clip);
        c.trigger(0, 0);
        c.set_clock(1.0, 120.0);
        c.stop(0);
        assert!(c.layers[0].active.is_some(), "still fading");
        c.set_clock(3.0, 120.0);
        assert!((level(&c) - 0.5).abs() < 1e-5, "{}", level(&c));
        c.set_clock(5.5, 120.0);
        assert_eq!(c.layers[0].active, None, "done");
        assert_eq!(level(&c), 0.0);
        // A clip with no fade-out stops at once, as before.
        let mut c = white_layer(solid());
        c.trigger(0, 0);
        c.stop(0);
        assert_eq!(c.layers[0].active, None);
    }

    #[test]
    fn clips_saved_before_speed_and_fades_still_open() {
        let mut v = serde_json::to_value(solid()).unwrap();
        for k in ["speed", "transport", "length_beats", "fade_in", "fade_out"] {
            v.as_object_mut().unwrap().remove(k);
        }
        let c: Clip = serde_json::from_value(v).unwrap();
        assert_eq!((c.speed, c.transport, c.length_beats, c.fade_in, c.fade_out), (1.0, Transport::Loop, 8.0, 0.0, 0.0));
    }

    // ---- A/B decks and the crossfader

    fn two_decks() -> Composition {
        let mut c = Composition::new(1, 1, 1);
        let mk = |col, deck| {
            let mut l = Layer::new("L", 1);
            l.blend = BlendMode::Add;
            l.deck = deck;
            l.clips[0] = Some(Clip::new("c", Generator::Solid { color: col }));
            l
        };
        c.layers.push(mk(Paint::Rgb([1.0, 0.0, 0.0]), Deck::A));
        c.layers.push(mk(Paint::Rgb([0.0, 0.0, 1.0]), Deck::B));
        c.trigger_column(0);
        c
    }

    #[test]
    fn the_crossfader_moves_between_deck_a_and_deck_b() {
        let mut c = two_decks();
        let px = |c: &Composition| {
            let p = c.render().pixels[0];
            (p.r, p.b)
        };
        c.crossfade = 0.0;
        assert_eq!(px(&c), (1.0, 0.0), "all A");
        c.crossfade = 1.0;
        assert_eq!(px(&c), (0.0, 1.0), "all B");
        c.crossfade = 0.5;
        assert_eq!(px(&c), (1.0, 1.0), "both at full in the middle, like a DJ mixer");
        c.crossfade = 0.25;
        let (r, b) = px(&c);
        assert!((r - 1.0).abs() < 1e-6 && (b - 0.5).abs() < 1e-6, "{r} {b}");
    }

    #[test]
    fn layers_not_on_a_deck_ignore_the_crossfader() {
        let mut c = two_decks();
        c.layers[0].deck = Deck::Both;
        c.crossfade = 1.0;
        assert_eq!(c.render().pixels[0].r, 1.0);
    }

    #[test]
    fn a_crossfader_in_the_middle_is_the_default_and_old_shows_open() {
        let c = Composition::new(1, 1, 1);
        assert_eq!(c.crossfade, 0.5);
        let mut v = serde_json::to_value(two_decks()).unwrap();
        v.as_object_mut().unwrap().remove("crossfade");
        for l in v["layers"].as_array_mut().unwrap() {
            l.as_object_mut().unwrap().remove("deck");
        }
        let back: Composition = serde_json::from_value(v).unwrap();
        assert_eq!(back.crossfade, 0.5);
        assert!(back.layers.iter().all(|l| l.deck == Deck::Both));
    }
}
