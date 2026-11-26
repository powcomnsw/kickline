//! Auto mode: the lighting director.
//!
//! It listens to the music (the section, how energetic it is) and runs a few
//! of its own layers: a wash, some motion (one layer per group, if the show
//! has groups) and accents on the drums. It picks effects and palettes by
//! what suits the moment, changes on the bar, and crossfades. Your own
//! layers stay on top and always win: while one of them plays, Auto steps
//! back, and Hold freezes what Auto is showing.
//!
//! Each of Auto's layers is really a pair, A and B, so a change is a
//! crossfade: the new effect starts on the idle layer and fades in while the
//! old one fades out.

use crate::audio::{AudioBand, AudioBands};
use crate::composition::{Clip, Layer};
use crate::blend::BlendMode;
use crate::palette::{Paint, Palettes};
use crate::scenes::{self, Role, Scene};
use crate::sections::{Section, SectionTuning};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Genre {
    House,
    Techno,
    Edm,
    Trance,
    DrumAndBass,
    HipHop,
}

impl Genre {
    pub const ALL: [Genre; 6] = [Genre::House, Genre::Techno, Genre::Edm, Genre::Trance, Genre::DrumAndBass, Genre::HipHop];

    pub fn name(self) -> &'static str {
        match self {
            Genre::House => "House",
            Genre::Techno => "Techno",
            Genre::Edm => "EDM / Festival",
            Genre::Trance => "Trance",
            Genre::DrumAndBass => "Drum & Bass",
            Genre::HipHop => "Hip-hop / R&B",
        }
    }

    pub fn note(self) -> &'static str {
        match self {
            Genre::House => "Steady four-on-the-floor, 118-130 BPM",
            Genre::Techno => "Driving and dark, 125-150 BPM",
            Genre::Edm => "Big builds and drops, 126-150 BPM",
            Genre::Trance => "Long builds, slow colour changes, 128-140 BPM",
            Genre::DrumAndBass => "Two kicks a bar, fast, 160-180 BPM",
            Genre::HipHop => "Sparse kicks, laid back, 70-100 BPM",
        }
    }

    /// What counts as "kick running" for this genre.
    pub fn tuning(self) -> SectionTuning {
        match self {
            Genre::DrumAndBass | Genre::HipHop => SectionTuning { groove_kicks: 2, melodic_kicks: 0, settle_beats: 4 },
            _ => SectionTuning::default(),
        }
    }

    /// Multiplies how long effects stay (1 = the "change every" setting).
    pub fn pace(self) -> f64 {
        match self {
            Genre::House => 1.0,
            Genre::Techno => 1.0,
            Genre::Edm => 0.75,
            Genre::Trance => 1.5,
            Genre::DrumAndBass => 1.0,
            Genre::HipHop => 1.5,
        }
    }

    /// Pushes the overall energy up or down.
    pub fn energy_shift(self) -> f32 {
        match self {
            Genre::Edm => 0.08,
            Genre::Techno => 0.03,
            Genre::HipHop => -0.1,
            _ => 0.0,
        }
    }
}

/// Auto mode's settings, saved with the show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectorSettings {
    pub on: bool,
    pub genre: Genre,
    /// Calm (0) to wild (1).
    pub energy: f32,
    /// Bars an effect stays before Auto looks for another (accents change faster, washes slower).
    pub change_bars: u32,
    /// Never pick strobing effects.
    pub flash_safe: bool,
    /// Let Auto change the palette too.
    pub palettes: bool,
    /// How much Auto lets down while one of your own layers plays (1 = not at all).
    pub under_manual: f32,
    /// Effects Auto must not use (by name).
    pub off_effects: Vec<String>,
    /// Palettes Auto must not use.
    pub off_palettes: Vec<String>,
    /// Groups Auto leaves alone.
    pub off_groups: Vec<String>,
}

impl Default for DirectorSettings {
    fn default() -> Self {
        DirectorSettings {
            on: false,
            genre: Genre::House,
            energy: 0.5,
            change_bars: 4,
            flash_safe: true,
            palettes: true,
            under_manual: 0.4,
            off_effects: Vec::new(),
            off_palettes: Vec::new(),
            off_groups: Vec::new(),
        }
    }
}

/// What the director is told each frame.
pub struct Input<'a> {
    pub audio: &'a AudioBands,
    pub beat: f64,
    pub section: Section,
    pub beats_in: u32,
    pub groups: &'a [String],
    /// One of the user's own layers is playing.
    pub user_playing: bool,
}

#[derive(Clone, Debug)]
struct Slot {
    role: Role,
    group: Option<String>,
    /// Which layer of the pair is showing the current effect.
    cur: usize,
    fade: f32,
    fade_beats: f32,
    next_change: f64,
    last_pick: f64,
    name: String,
    recent: Vec<String>,
}

impl Slot {
    fn new(role: Role, group: Option<String>) -> Self {
        Slot { role, group, cur: 0, fade: 1.0, fade_beats: 1.0, next_change: 0.0, last_pick: -1e9, name: String::new(), recent: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct Director {
    rng: u64,
    last_beat: f64,
    /// How energetic the music is right now (0..1).
    pub energy: f32,
    /// Where the energy is heading (what the section calls for): effects and
    /// palettes are chosen for this, not for the energy still catching up.
    mood: f32,
    live: f32,
    duck: f32,
    last_section: Option<Section>,
    last_palette: f64,
    slots: Vec<Slot>,
    /// Freeze what Auto is showing.
    pub hold: bool,
    /// What Auto is showing: (what it is, effect name).
    pub now: Vec<(String, String)>,
    pub palette_name: String,
}

impl Default for Director {
    fn default() -> Self {
        Director {
            rng: 0x9E37_79B9_7F4A_7C15,
            last_beat: 0.0,
            energy: 0.0,
            mood: 0.0,
            live: 0.0,
            duck: 1.0,
            last_section: None,
            last_palette: -1e9,
            slots: Vec::new(),
            hold: false,
            now: Vec::new(),
            palette_name: String::new(),
        }
    }
}

fn base_energy(s: Section, beats_in: u32) -> f32 {
    match s {
        Section::Groove => 0.55,
        Section::Melodic => 0.25,
        Section::BuildUp => 0.55 + 0.025 * beats_in.min(16) as f32,
        Section::Pause => 0.04,
        Section::Drop => 1.0,
    }
}

fn role_gain(r: Role) -> f32 {
    match r {
        Role::Wash => 0.55,
        Role::Motion => 0.9,
        Role::Accent => 0.8,
    }
}

fn role_fade(r: Role) -> f32 {
    match r {
        Role::Wash => 2.0,
        Role::Motion => 1.0,
        Role::Accent => 0.5,
    }
}

/// How well `e` fits a range: 1 inside, falling off outside.
fn fit(range: (f32, f32), e: f32) -> f32 {
    let d = if e < range.0 {
        range.0 - e
    } else if e > range.1 {
        e - range.1
    } else {
        0.0
    };
    1.0 / (1.0 + 8.0 * d)
}

impl Director {
    /// Start the random choices from a known place (tests, replays).
    pub fn seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }

    fn rand(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }

    fn weighted(&mut self, weights: &[f32]) -> Option<usize> {
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut r = self.rand() * total;
        for (i, w) in weights.iter().enumerate() {
            if r < *w {
                return Some(i);
            }
            r -= w;
        }
        weights.iter().rposition(|w| *w > 0.0)
    }

    fn choose(&mut self, s: &DirectorSettings, role: Role, section: Section, recent: &[String]) -> Option<&'static Scene> {
        let bank = scenes::bank();
        let e = self.mood;
        let weights: Vec<f32> = bank
            .iter()
            .map(|sc| {
                if sc.tags.role != role || s.off_effects.contains(&sc.name) || (sc.tags.strobe && s.flash_safe) {
                    return 0.0;
                }
                let mut w = fit(sc.tags.energy, e);
                if !sc.tags.suits(section) {
                    w *= 0.3;
                }
                if recent.contains(&sc.name) {
                    w *= 0.15;
                }
                w
            })
            .collect();
        self.weighted(&weights).map(|i| &bank[i])
    }

    fn pick_palette(&mut self, s: &DirectorSettings, palettes: &mut Palettes, beat: f64) {
        let e = self.mood;
        let weights: Vec<f32> = palettes
            .list
            .iter()
            .enumerate()
            .map(|(i, p)| if s.off_palettes.contains(&p.name) { 0.0 } else { fit(scenes::palette_energy(&p.name), e) * if i == palettes.active { 0.1 } else { 1.0 } })
            .collect();
        if let Some(i) = self.weighted(&weights) {
            palettes.select(i, beat);
            self.palette_name = palettes.list[i].name.clone();
        }
    }

    /// Drop everything Auto set up (it was switched off).
    fn clear(&mut self, auto: &mut Vec<Layer>) {
        auto.clear();
        self.slots.clear();
        self.now.clear();
    }

    /// Run one frame of the director.
    pub fn step(&mut self, s: &DirectorSettings, inp: &Input, auto: &mut Vec<Layer>, palettes: &mut Palettes) {
        let beat = inp.beat;
        if !s.on {
            if !auto.is_empty() || !self.slots.is_empty() {
                self.clear(auto);
            }
            self.last_beat = beat;
            self.last_section = None;
            return;
        }
        // Which layers Auto needs: a wash and accents over everything, and
        // motion per group (or over everything when there are no groups).
        let mut wanted: Vec<(Role, Option<String>)> = vec![(Role::Wash, None), (Role::Accent, None)];
        let groups: Vec<&String> = inp.groups.iter().filter(|g| !s.off_groups.contains(g)).collect();
        if groups.is_empty() {
            wanted.push((Role::Motion, None));
        } else {
            for g in groups {
                wanted.push((Role::Motion, Some(g.clone())));
            }
        }
        let same = self.slots.len() == wanted.len() && self.slots.iter().zip(&wanted).all(|(a, b)| a.role == b.0 && a.group == b.1);
        if !same {
            auto.clear();
            self.slots = wanted.into_iter().map(|(r, g)| Slot::new(r, g)).collect();
            for sl in &self.slots {
                for ab in ["A", "B"] {
                    let label = match &sl.group {
                        Some(g) => format!("Auto {} {} {}", sl.role.name(), g, ab),
                        None => format!("Auto {} {}", sl.role.name(), ab),
                    };
                    let mut l = Layer::new(label, 1);
                    l.blend = BlendMode::Add;
                    l.opacity = 0.0;
                    l.target = sl.group.clone();
                    auto.push(l);
                }
            }
        }

        let dbeat = if beat >= self.last_beat { (beat - self.last_beat).min(1.0) } else { 0.0 };
        let bar_crossed = beat >= self.last_beat && (self.last_beat / 4.0).floor() != (beat / 4.0).floor();
        self.last_beat = beat;

        // Energy: the section's character, nudged by how loud it is now.
        let changed = self.last_section != Some(inp.section);
        if changed {
            self.last_section = Some(inp.section);
        }
        let instant = changed && matches!(inp.section, Section::Pause | Section::Drop);
        self.live += (inp.audio.get(AudioBand::Level) - self.live) * (dbeat as f32 * 0.5).min(1.0);
        let shift = (s.energy - 0.5) * 0.5 + s.genre.energy_shift();
        // The pause is dark whatever the (still fading) level says.
        let target = if inp.section == Section::Pause { 0.04 } else { (0.7 * base_energy(inp.section, inp.beats_in) + 0.3 * self.live + shift).clamp(0.0, 1.0) };
        self.mood = target;
        if instant {
            self.energy = target;
        } else {
            self.energy += (target - self.energy) * dbeat as f32;
        }
        let duck_target = if inp.user_playing { s.under_manual.clamp(0.0, 1.0) } else { 1.0 };
        self.duck += (duck_target - self.duck) * (dbeat as f32).min(1.0);

        let bars = s.change_bars.max(1) as f64 * s.genre.pace();
        for si in 0..self.slots.len() {
            let (role, first) = (self.slots[si].role, self.slots[si].name.is_empty());
            let due = !self.hold && beat >= self.slots[si].next_change && bar_crossed;
            let forced = !self.hold && changed && inp.section != Section::Pause && (instant || beat - self.slots[si].last_pick >= 4.0);
            if first || due || forced {
                let recent = self.slots[si].recent.clone();
                if let Some(sc) = self.choose(s, role, inp.section, &recent) {
                    let mut g = sc.generator.clone();
                    let slot = (self.rand() * 4.0) as u8 % 4;
                    if let Some(p) = g.paint_mut() {
                        *p = Paint::Slot(slot);
                    }
                    let new = 1 - self.slots[si].cur;
                    let l = &mut auto[si * 2 + new];
                    l.clips[0] = Some(Clip::new(sc.name.clone(), g));
                    l.active = Some(0);
                    l.trigger_beat = beat;
                    let hold_beats = 4.0
                        * match role {
                            Role::Wash => bars * 2.0,
                            Role::Motion => bars,
                            Role::Accent => (bars * 0.5).max(1.0),
                        };
                    let sl = &mut self.slots[si];
                    sl.cur = new;
                    sl.fade = 0.0;
                    sl.fade_beats = if instant { 0.25 } else { role_fade(role) };
                    sl.last_pick = beat;
                    sl.next_change = beat + hold_beats - 0.5;
                    sl.name = sc.name.clone();
                    sl.recent.push(sc.name.clone());
                    if sl.recent.len() > 3 {
                        sl.recent.remove(0);
                    }
                }
            }
        }

        if s.palettes && !self.hold && (changed && inp.section != Section::Pause || (bar_crossed && beat - self.last_palette >= 64.0)) {
            self.last_palette = beat;
            self.pick_palette(s, palettes, beat);
        }

        // Levels and the crossfade.
        for si in 0..self.slots.len() {
            let level = self.energy.powf(0.8) * role_gain(self.slots[si].role) * self.duck;
            let sl = &mut self.slots[si];
            sl.fade = (sl.fade + dbeat as f32 / sl.fade_beats.max(0.01)).min(1.0);
            let (new, old) = (si * 2 + sl.cur, si * 2 + 1 - sl.cur);
            if auto[new].active.is_some() {
                auto[new].opacity = level * sl.fade;
            }
            if auto[old].active.is_some() {
                auto[old].opacity = level * (1.0 - sl.fade);
                if sl.fade >= 1.0 {
                    auto[old].active = None;
                    auto[old].opacity = 0.0;
                }
            }
        }

        self.now = self
            .slots
            .iter()
            .map(|sl| {
                let what = match &sl.group {
                    Some(g) => format!("{} · {}", sl.role.name(), g),
                    None => sl.role.name().to_string(),
                };
                (what, sl.name.clone())
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rig {
        d: Director,
        s: DirectorSettings,
        auto: Vec<Layer>,
        pal: Palettes,
        audio: AudioBands,
        beat: f64,
        groups: Vec<String>,
        user: bool,
    }

    impl Rig {
        fn new() -> Self {
            let mut d = Director::default();
            d.seed(7);
            Rig { d, s: DirectorSettings { on: true, ..Default::default() }, auto: Vec::new(), pal: Palettes::default(), audio: AudioBands::default(), beat: 0.0, groups: Vec::new(), user: false }
        }

        /// Play `beats` beats of a section (20 steps a beat). Returns the effect names seen.
        fn play(&mut self, section: Section, beats: u32) -> Vec<String> {
            let mut seen: Vec<String> = Vec::new();
            for b in 0..beats * 20 {
                self.audio.set(AudioBand::Level, 0.7);
                let inp = Input { audio: &self.audio, beat: self.beat, section, beats_in: b / 20, groups: &self.groups, user_playing: self.user };
                self.d.step(&self.s, &inp, &mut self.auto, &mut self.pal);
                for (_, n) in &self.d.now {
                    if !n.is_empty() && !seen.contains(n) {
                        seen.push(n.clone());
                    }
                }
                self.beat += 0.05;
            }
            seen
        }
    }

    #[test]
    fn builds_a_wash_accents_and_motion_pairs() {
        let mut r = Rig::new();
        r.play(Section::Groove, 2);
        assert_eq!(r.auto.len(), 6, "wash, accent, motion x2 layers each");
        assert_eq!(r.d.now.len(), 3);
        assert!(r.d.now.iter().all(|(_, n)| !n.is_empty()));
        // With groups, motion runs per group.
        r.groups = vec!["Left".into(), "Right".into()];
        r.play(Section::Groove, 1);
        assert_eq!(r.auto.len(), 8);
        assert_eq!(r.auto.iter().filter(|l| l.target.as_deref() == Some("Left")).count(), 2);
    }

    #[test]
    fn a_change_is_a_crossfade() {
        let mut r = Rig::new();
        r.play(Section::Groove, 8);
        // Run until the motion slot swaps its effect, watching both layers.
        let first = r.d.now[2].1.clone();
        let mut both = false;
        for _ in 0..400 {
            r.play(Section::Groove, 1);
            let pair = &r.auto[4..6];
            if pair.iter().all(|l| l.active.is_some() && l.opacity > 0.01) {
                both = true;
                // The two opacities fade in opposite directions (sum stays near the level).
                let sum = pair[0].opacity + pair[1].opacity;
                assert!(sum > 0.1);
            }
            if r.d.now[2].1 != first && both {
                break;
            }
        }
        assert!(both, "never saw both layers lit during a change");
        assert_ne!(r.d.now[2].1, first);
    }

    #[test]
    fn changes_only_on_bar_lines_and_not_too_often() {
        let mut r = Rig::new();
        r.play(Section::Groove, 4);
        let mut last = r.d.now.clone();
        let mut changes = 0;
        for _ in 0..64 {
            r.play(Section::Groove, 1);
            if r.d.now != last {
                changes += 1;
                // Landed on a beat that starts a bar (we step 20 times a beat).
                assert!((r.beat / 4.0).fract() < 0.3 || (r.beat / 4.0).fract() > 0.7, "beat {}", r.beat);
                last = r.d.now.clone();
            }
        }
        // 64 beats = 16 bars, motion every 4, wash every 8, accents every 2.
        assert!((2..=40).contains(&changes), "{changes} changes");
    }

    #[test]
    fn flash_safe_never_strobes() {
        let mut r = Rig::new();
        let mut names = Vec::new();
        for _ in 0..40 {
            names.extend(r.play(Section::Drop, 8));
            names.extend(r.play(Section::BuildUp, 8));
        }
        assert!(!names.iter().any(|n| n == "Strobe"), "{names:?}");
        r.s.flash_safe = false;
        let mut names = Vec::new();
        for _ in 0..60 {
            names.extend(r.play(Section::Drop, 8));
            names.extend(r.play(Section::BuildUp, 8));
        }
        assert!(names.iter().any(|n| n == "Strobe"), "strobe never picked with flash-safe off");
    }

    #[test]
    fn disabled_effects_and_groups_are_left_alone() {
        let mut r = Rig::new();
        r.groups = vec!["Left".into(), "Right".into()];
        r.s.off_groups = vec!["Right".into()];
        r.s.off_effects = vec!["Chase".into(), "Radar".into(), "Meteor".into()];
        let mut names = Vec::new();
        for _ in 0..40 {
            names.extend(r.play(Section::Groove, 8));
            names.extend(r.play(Section::Drop, 8));
        }
        assert!(!names.iter().any(|n| n == "Chase" || n == "Radar" || n == "Meteor"), "{names:?}");
        assert!(!r.auto.iter().any(|l| l.target.as_deref() == Some("Right")));
        assert!(r.auto.iter().any(|l| l.target.as_deref() == Some("Left")));
    }

    #[test]
    fn hold_freezes_the_look() {
        let mut r = Rig::new();
        r.play(Section::Groove, 8);
        r.d.hold = true;
        let frozen = r.d.now.clone();
        let pal = r.pal.active;
        r.play(Section::Drop, 32);
        r.play(Section::Melodic, 32);
        assert_eq!(r.d.now, frozen);
        assert_eq!(r.pal.active, pal);
        r.d.hold = false;
        r.play(Section::Drop, 16);
        assert_ne!(r.d.now, frozen);
    }

    #[test]
    fn steps_back_while_the_user_plays() {
        let mut r = Rig::new();
        r.play(Section::Groove, 16);
        let level = |r: &Rig| r.auto.iter().map(|l| l.opacity).sum::<f32>();
        let before = level(&r);
        r.user = true;
        r.play(Section::Groove, 8);
        let ducked = level(&r);
        assert!(ducked < before * 0.7, "{ducked} vs {before}");
        r.user = false;
        r.play(Section::Groove, 8);
        assert!(level(&r) > ducked * 1.3);
    }

    #[test]
    fn drop_is_brighter_than_a_breakdown_and_the_pause_is_dark() {
        let mut r = Rig::new();
        r.play(Section::Melodic, 16);
        let calm = r.d.energy;
        r.play(Section::Drop, 4);
        let loud = r.d.energy;
        assert!(loud > calm + 0.3, "{calm} {loud}");
        r.play(Section::Pause, 2);
        assert!(r.d.energy < 0.25, "{}", r.d.energy);
        let lit: f32 = r.auto.iter().map(|l| l.opacity).sum();
        assert!(lit < 0.6, "{lit}");
    }

    #[test]
    fn same_seed_same_show() {
        let run = || {
            let mut r = Rig::new();
            let mut v = Vec::new();
            for sec in [Section::Groove, Section::BuildUp, Section::Drop, Section::Melodic, Section::Drop] {
                v.extend(r.play(sec, 16));
            }
            (v, r.pal.active)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn switching_off_removes_the_layers() {
        let mut r = Rig::new();
        r.play(Section::Groove, 4);
        assert!(!r.auto.is_empty());
        r.s.on = false;
        r.play(Section::Groove, 1);
        assert!(r.auto.is_empty());
        assert!(r.d.now.is_empty());
    }

    #[test]
    fn palette_follows_the_mood() {
        let mut r = Rig::new();
        let mut calm = [0u32; 8];
        let mut wild = [0u32; 8];
        for _ in 0..80 {
            r.play(Section::Melodic, 4);
            calm[r.pal.active] += 1;
            r.play(Section::Drop, 4);
            wild[r.pal.active] += 1;
        }
        let name = |i: usize| r.pal.list[i].name.clone();
        // Warm white and Ocean are calm colours, Police and Fire are not.
        let calm_hits = calm[7] + calm[6];
        let wild_hits = wild[7] + wild[6];
        assert!(calm_hits > wild_hits, "calm {calm:?} wild {wild:?} ({})", name(7));
    }
}
