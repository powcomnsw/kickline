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
use crate::palette::Palettes;
use crate::scenes::{self, Role};
use crate::playbook::{self, Bag, Look, Ramp};
use crate::sections::{GenreProfile, Section};
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

    /// How sections are judged for this genre: bar length, half-time feel,
    /// kick counts, settle time and build/drop thresholds. House is the
    /// original behaviour.
    pub fn profile(self) -> GenreProfile {
        let base = GenreProfile::default();
        match self {
            Genre::House => base,
            Genre::Techno => GenreProfile { settle_beats: 4, drop_bass: 0.75, ..base },
            Genre::Edm => GenreProfile { build_rise: 1.15, ..base },
            // Long builds and breakdowns: wait two bars before believing a change.
            Genre::Trance => GenreProfile { settle_beats: 8, ..base },
            // Two kicks a bar, snares on 2 and 4.
            Genre::DrumAndBass => GenreProfile { groove_kicks: 2, melodic_kicks: 0, ..base },
            // Felt at half the tempo: one snare a bar of eight beats, sparse kicks.
            Genre::HipHop => GenreProfile {
                half_time: true,
                groove_kicks: 3,
                melodic_kicks: 1,
                settle_beats: 8,
                snare_roll: 12,
                ..base
            },
        }
    }

    /// The old name for `profile()`.
    pub fn tuning(self) -> GenreProfile {
        self.profile()
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
    /// Looks Auto must not play (by id).
    pub off_looks: Vec<String>,
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
            off_looks: Vec::new(),
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
    /// Where we are in the phrase.
    pub phrase: crate::set::Phrase,
    /// The beat can be trusted: looks that need it may play.
    pub steady: bool,
    /// Drops heard so far in this track (the second drop looks different).
    pub drops: u32,
    /// What each group is for.
    pub roles: &'a crate::roles::RoleMap,
}

#[derive(Clone, Debug)]
struct Slot {
    role: Role,
    group: Option<String>,
    /// Which layer of the pair is showing the current effect.
    cur: usize,
    fade: f32,
    fade_beats: f32,
    last_pick: f64,
    name: String,
    /// The unscaled effect, and the ramp that speeds it up or brightens it.
    base: Option<crate::generator::Generator>,
    ramp: Ramp,
    move_level: f32,
    speed: f32,
}

impl Slot {
    fn new(role: Role, group: Option<String>) -> Self {
        Slot { role, group, cur: 0, fade: 1.0, fade_beats: 1.0, last_pick: -1e9, name: String::new(), base: None, ramp: Ramp::NONE, move_level: 1.0, speed: 1.0 }
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
    bags: [Bag; 5],
    look: Option<&'static Look>,
    /// Bars since the look was picked.
    bars_in_look: u32,
    impact_until: f64,
    /// The beat the current section started on, for ramps.
    pub look_name: String,
    /// Play this look now (a button in Set up asks for it).
    pub preview: Option<&'static str>,
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
            bags: Default::default(),
            look: None,
            bars_in_look: 0,
            impact_until: -1.0,
            look_name: String::new(),
            preview: None,
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

    /// The next look for a section from its shuffled bag.
    fn pick_look(&mut self, s: &DirectorSettings, section: Section, steady: bool) -> Option<&'static Look> {
        let ok = |l: &Look| {
            !s.off_looks.iter().any(|o| o == l.id)
                && !(l.strobe() && s.flash_safe)
                && !(l.needs_beat() && !steady)
                && !l.moves().any(|m| s.off_effects.iter().any(|o| o == m.effect))
        };
        let mut bag = std::mem::take(&mut self.bags[section.index()]);
        let mut look = bag.next(section, ok, || self.rand());
        if look.is_none() && !steady {
            // Never go dark for want of a beat-free look: play a beat look instead.
            look = bag.next(
                section,
                |l| !s.off_looks.iter().any(|o| o == l.id) && !(l.strobe() && s.flash_safe) && !l.moves().any(|m| s.off_effects.iter().any(|o| o == m.effect)),
                || self.rand(),
            );
        }
        self.bags[section.index()] = bag;
        look
    }

    fn look_ok(&self, s: &DirectorSettings, l: &Look, steady: bool) -> bool {
        !s.off_looks.iter().any(|o| o == l.id) && !(l.strobe() && s.flash_safe) && !(l.needs_beat() && !steady) && !l.moves().any(|m| s.off_effects.iter().any(|o| o == m.effect))
    }

    /// Put a look on the slots: each role gets its move, crossfading from what
    /// it was playing (a role the look doesn't use fades out).
    fn apply_look(&mut self, look: &'static Look, inp: &Input, auto: &mut [Layer], instant: bool, beat: f64) {
        self.look = Some(look);
        self.look_name = look.id.to_string();
        self.bars_in_look = 0;
        for si in 0..self.slots.len() {
            let role = self.slots[si].role;
            let mv = match role {
                Role::Wash => look.wash,
                Role::Motion => look.motion,
                Role::Accent => look.accent,
            };
            let new = 1 - self.slots[si].cur;
            let gen = mv.and_then(|m| playbook::generator_for(&m, inp.drops, inp.section).map(|g| (m, g)));
            let l = &mut auto[si * 2 + new];
            let sl = &mut self.slots[si];
            match gen {
                Some((m, g)) => {
                    sl.base = Some(g.clone());
                    sl.ramp = m.ramp;
                    sl.move_level = m.level;
                    sl.speed = m.ramp.speed_at(0.0);
                    let g = if m.ramp.is_none() { g } else { playbook::scale_speed(&g, sl.speed) };
                    l.clips[0] = Some(Clip::new(m.effect, g));
                    l.active = Some(0);
                    l.trigger_beat = beat;
                    // The right half of a pair answers the left, half a cycle behind.
                    if role == Role::Motion && sl.group.as_ref().is_some_and(|g| inp.roles.get(g) == Some(crate::roles::StageRole::Right)) {
                        l.trigger_beat = beat - 2.0;
                    }
                    sl.name = m.effect.to_string();
                }
                None => {
                    l.clips[0] = None;
                    l.active = None;
                    sl.base = None;
                    sl.ramp = Ramp::NONE;
                    sl.move_level = 1.0;
                    sl.name = "—".into();
                }
            }
            sl.cur = new;
            sl.fade = 0.0;
            sl.fade_beats = if instant { 0.25 } else { role_fade(role) };
            sl.last_pick = beat;
        }
    }

    /// How strongly a slot plays given the stage's roles: motion leads on the
    /// role its effect was written for and supports softly elsewhere; accents
    /// gather on the eyes. Without roles every slot plays fully.
    fn support_gain(&self, sl: &Slot, inp: &Input) -> f32 {
        use crate::roles::StageRole;
        let roles = inp.roles;
        match sl.role {
            Role::Motion => {
                let (Some(g), Some(m)) = (&sl.group, self.look.and_then(|l| l.motion)) else { return 1.0 };
                if roles.is_empty() {
                    return 1.0;
                }
                let lead = playbook::lead_for(m.effect);
                let mut set = roles.groups_for(lead, inp.groups);
                if lead == StageRole::Left {
                    set.extend(roles.groups_for(StageRole::Right, inp.groups));
                }
                if set.is_empty() || set.contains(g) {
                    1.0
                } else {
                    0.4
                }
            }
            Role::Accent if sl.group.is_none() && roles.has(StageRole::Eyes, inp.groups) => 0.6,
            _ => 1.0,
        }
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
            // Focal points get an accent of their own.
            for g in inp.roles.direct(crate::roles::StageRole::Eyes, inp.groups).into_iter().filter(|g| !s.off_groups.contains(g)) {
                wanted.push((Role::Accent, Some(g)));
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
        let bar = s.genre.profile().bar_beats() as f64;
        let bar_crossed = beat >= self.last_beat && (self.last_beat / bar).floor() != (beat / bar).floor();
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

        // Which look: a new one when the section changes, and in a long section
        // on a phrase line (every 8 bars) once the look has had its time.
        let bar_beats = s.genre.profile().bar_beats();
        if inp.phrase.new_bar {
            self.bars_in_look += 1;
        }
        let min_bars = ((s.change_bars.max(1) as f64 * 2.0 * s.genre.pace()).round() as u32).max(4);
        let line = inp.phrase.new_bar && inp.phrase.bar_in_phrase % 8 == 0;
        let long_section = !matches!(inp.section, Section::Pause | Section::Drop) || inp.beats_in >= bar_beats * 8;
        let stale = self.look.is_some_and(|l| !self.look_ok(s, l, inp.steady));
        if let Some(l) = self.preview.take().and_then(playbook::find) {
            self.apply_look(l, inp, auto, true, beat);
        }
        let want_new = self.look.is_none() || stale || (changed && !self.hold) || (!self.hold && line && self.bars_in_look >= min_bars && long_section);
        if want_new {
            if let Some(look) = self.pick_look(s, inp.section, inp.steady) {
                self.apply_look(look, inp, auto, instant, beat);
                if inp.section == Section::Drop && changed {
                    // A drop lands with a one-bar impact before its look settles.
                    self.impact_until = beat + bar_beats as f64;
                }
            }
        }

        // Ramps: speed and brightness follow how far through the section we are.
        let progress = (inp.beats_in as f32 / 32.0).clamp(0.0, 1.0);
        for si in 0..self.slots.len() {
            let ramp = self.slots[si].ramp;
            if ramp.is_none() {
                continue;
            }
            let f = ramp.speed_at(progress);
            if (f - self.slots[si].speed).abs() > 0.04 {
                if let Some(base) = self.slots[si].base.clone() {
                    let (idx, old) = (si * 2 + self.slots[si].cur, self.slots[si].speed);
                    if let Some(c) = auto[idx].clips[0].as_mut() {
                        c.generator = playbook::scale_speed(&base, f);
                    }
                    // Keep the effect's phase where it was while it speeds up.
                    auto[idx].trigger_beat = beat - (beat - auto[idx].trigger_beat) * (old / f) as f64;
                    self.slots[si].speed = f;
                }
            }
        }

        if s.palettes && !self.hold && (changed && inp.section != Section::Pause || (bar_crossed && beat - self.last_palette >= 64.0)) {
            self.last_palette = beat;
            self.pick_palette(s, palettes, beat);
        }

        // Levels and the crossfade.
        for si in 0..self.slots.len() {
            let sl0 = &self.slots[si];
            let support = self.support_gain(sl0, inp);
            let mut level = self.energy.powf(0.8) * role_gain(sl0.role) * self.duck * sl0.move_level * sl0.ramp.level_at(progress) * support;
            if beat < self.impact_until {
                // The impact: everything hits hard, easing down over the bar.
                let k = ((self.impact_until - beat) / bar_beats as f64).clamp(0.0, 1.0) as f32;
                level = level + (1.0 - level) * k * if sl0.role == Role::Motion { 0.4 } else { 1.0 };
            }
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
    use crate::generator::Generator;
    use crate::palette::Paint;

    struct Rig {
        d: Director,
        s: DirectorSettings,
        auto: Vec<Layer>,
        pal: Palettes,
        audio: AudioBands,
        beat: f64,
        groups: Vec<String>,
        user: bool,
        phrase: crate::set::Phrase,
        steady: bool,
        drops: u32,
        roles: crate::roles::RoleMap,
    }

    impl Rig {
        fn new() -> Self {
            let mut d = Director::default();
            d.seed(7);
            Rig { d, s: DirectorSettings { on: true, ..Default::default() }, auto: Vec::new(), pal: Palettes::default(), audio: AudioBands::default(), beat: 0.0, groups: Vec::new(), user: false, phrase: Default::default(), steady: true, drops: 0, roles: Default::default() }
        }

        /// Play `beats` beats of a section (20 steps a beat). Returns the effect names seen.
        fn play(&mut self, section: Section, beats: u32) -> Vec<String> {
            let mut seen: Vec<String> = Vec::new();
            for b in 0..beats * 20 {
                self.audio.set(AudioBand::Level, 0.7);
                self.phrase.update(self.beat, None, 4);
                let inp = Input { audio: &self.audio, beat: self.beat, section, beats_in: b / 20, groups: &self.groups, user_playing: self.user, phrase: self.phrase, steady: self.steady, drops: self.drops, roles: &self.roles };
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

    #[test]
    fn a_section_never_plays_the_same_look_twice_running() {
        let mut r = Rig::new();
        r.s.flash_safe = false;
        let mut drops = Vec::new();
        let mut grooves = Vec::new();
        for _ in 0..24 {
            r.play(Section::Drop, 8);
            drops.push(r.d.look_name.clone());
            r.play(Section::Groove, 8);
            grooves.push(r.d.look_name.clone());
        }
        for v in [&drops, &grooves] {
            for w in v.windows(2) {
                assert_ne!(w[0], w[1], "{v:?}");
            }
        }
        // Every drop look gets used before any is used again.
        let n = playbook::looks_for(Section::Drop).count();
        for round in drops.chunks(n).take(2) {
            let mut ids = round.to_vec();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), round.len(), "{round:?}");
        }
    }

    #[test]
    fn build_ups_only_get_the_build_up_family_and_speed_up() {
        let mut r = Rig::new();
        r.s.flash_safe = false;
        for _ in 0..12 {
            r.play(Section::Groove, 4);
            r.play(Section::BuildUp, 2);
            assert!(r.d.look_name.starts_with("build-"), "{}", r.d.look_name);
        }
        // A build-up brightens and speeds up as it goes.
        r.play(Section::Groove, 4);
        r.play(Section::BuildUp, 2);
        let early: f32 = r.auto.iter().map(|l| l.opacity).sum();
        let speed_early = r.d.slots.iter().map(|s| s.speed).fold(0.0, f32::max);
        r.play(Section::BuildUp, 28);
        let late: f32 = r.auto.iter().map(|l| l.opacity).sum();
        let speed_late = r.d.slots.iter().map(|s| s.speed).fold(0.0, f32::max);
        assert!(late > early * 1.2, "{early} -> {late}");
        assert!(speed_late > speed_early * 1.5, "{speed_early} -> {speed_late}");
    }

    #[test]
    fn a_drop_lands_with_an_impact_that_eases_off() {
        let mut r = Rig::new();
        r.play(Section::Groove, 16);
        // Step the first bar of the drop and measure the brightest moment against the settled look.
        let mut peak = 0.0f32;
        for b in 0..(4 * 20) {
            self_step(&mut r, Section::Drop, b / 20);
            peak = peak.max(r.auto.iter().map(|l| l.opacity).sum());
        }
        let mut settled = 0.0f32;
        for b in 0..(8 * 20) {
            self_step(&mut r, Section::Drop, 4 + b / 20);
            settled = r.auto.iter().map(|l| l.opacity).sum();
        }
        assert!(peak > settled * 1.15, "impact {peak} vs settled {settled}");
    }

    fn self_step(r: &mut Rig, section: Section, beats_in: u32) {
        r.audio.set(AudioBand::Level, 0.7);
        r.phrase.update(r.beat, None, 4);
        let inp = Input { audio: &r.audio, beat: r.beat, section, beats_in, groups: &r.groups, user_playing: r.user, phrase: r.phrase, steady: r.steady, drops: r.drops, roles: &r.roles };
        r.d.step(&r.s, &inp, &mut r.auto, &mut r.pal);
        r.beat += 0.05;
    }

    #[test]
    fn the_pause_is_dark_with_at_most_one_small_element() {
        let mut r = Rig::new();
        for _ in 0..10 {
            r.play(Section::Drop, 8);
            r.play(Section::Pause, 3);
            let lit = r.auto.iter().filter(|l| l.active.is_some() && l.opacity > 0.01).count();
            assert!(lit <= 1, "{lit} layers lit in the pause ({})", r.d.look_name);
            assert!(r.auto.iter().map(|l| l.opacity).sum::<f32>() < 0.2);
        }
    }

    #[test]
    fn without_a_steady_beat_only_beat_free_looks_play() {
        let mut r = Rig::new();
        r.steady = false;
        r.s.flash_safe = false;
        for _ in 0..30 {
            for sec in [Section::Groove, Section::Melodic, Section::Drop] {
                r.play(sec, 8);
                let l = playbook::find(&r.d.look_name).unwrap();
                assert!(!l.needs_beat(), "{} needs the beat", l.id);
            }
            // A section with no beat-free look still plays something rather than going dark.
            r.play(Section::BuildUp, 8);
            assert!(r.auto.iter().any(|l| l.active.is_some() && l.opacity > 0.05));
        }
    }

    #[test]
    fn looks_can_be_switched_off_and_every_section_keeps_one() {
        let mut r = Rig::new();
        r.s.off_looks = vec!["drop-flash".into(), "drop-burst".into()];
        let mut seen = std::collections::HashSet::new();
        for _ in 0..40 {
            r.play(Section::Drop, 8);
            r.play(Section::Groove, 8);
            seen.insert(r.d.look_name.clone());
        }
        assert!(!seen.contains("drop-flash") && !seen.contains("drop-burst"));
        // Even with all but one off, that one plays rather than nothing.
        let keep = "groove-sweep";
        r.s.off_looks = playbook::looks_for(Section::Groove).filter(|l| l.id != keep).map(|l| l.id.to_string()).collect();
        r.play(Section::Melodic, 8);
        r.play(Section::Groove, 8);
        assert_eq!(r.d.look_name, keep);
    }

    #[test]
    fn a_long_section_moves_on_at_a_phrase_line() {
        let mut r = Rig::new();
        r.play(Section::Groove, 4);
        let first = r.d.look_name.clone();
        let mut changes = Vec::new();
        let mut last = first.clone();
        for _ in 0..(16 * 4) {
            r.play(Section::Groove, 1);
            if r.d.look_name != last {
                changes.push(r.beat);
                last = r.d.look_name.clone();
            }
        }
        assert!(!changes.is_empty(), "never moved on in 16 bars");
        for c in changes {
            let bar = (c / 4.0).round() as i64;
            assert_eq!(bar % 8, 0, "changed at beat {c}, not on an 8 bar line");
        }
    }

    #[test]
    fn the_second_drop_paints_a_different_colour() {
        let slot_of = |drops: u32| {
            let m = playbook::find("drop-chase").unwrap().motion.unwrap();
            match playbook::generator_for(&m, drops, Section::Drop).unwrap() {
                Generator::Chase { color: Paint::Slot(s), .. } => s,
                g => panic!("{g:?}"),
            }
        };
        assert_ne!(slot_of(1), slot_of(2));
    }

    fn level_of(r: &Rig, group: &str, role: Role) -> f32 {
        let i = r.d.slots.iter().position(|s| s.role == role && s.group.as_deref() == Some(group)).expect("slot");
        let sl = &r.d.slots[i];
        r.auto[i * 2 + sl.cur].opacity
    }

    #[test]
    fn motion_leads_on_the_role_it_was_written_for() {
        use crate::roles::StageRole;
        let mut r = Rig::new();
        r.groups = vec!["Edge".into(), "Panels".into()];
        r.roles.set("Edge", Some(StageRole::Path));
        r.roles.set("Panels", Some(StageRole::Fill));
        r.s.off_looks = crate::playbook::looks_for(Section::Groove).filter(|l| l.id != "groove-meteors").map(|l| l.id.to_string()).collect();
        r.play(Section::Groove, 8);
        assert_eq!(r.d.look_name, "groove-meteors");
        let (edge, panels) = (level_of(&r, "Edge", Role::Motion), level_of(&r, "Panels", Role::Motion));
        assert!(edge > panels * 1.5, "meteors run on the path: {edge} vs {panels}");
        assert!(panels > 0.0, "the panels still play, softly");
    }

    #[test]
    fn without_roles_every_group_plays_the_motion_fully() {
        let mut r = Rig::new();
        r.groups = vec!["A".into(), "B".into()];
        r.s.off_looks = crate::playbook::looks_for(Section::Groove).filter(|l| l.id != "groove-meteors").map(|l| l.id.to_string()).collect();
        r.play(Section::Groove, 8);
        assert_eq!(level_of(&r, "A", Role::Motion), level_of(&r, "B", Role::Motion));
    }

    #[test]
    fn accents_gather_on_the_eyes_when_the_stage_has_them() {
        use crate::roles::StageRole;
        let mut r = Rig::new();
        r.groups = vec!["Eye L".into(), "Eye R".into(), "Face".into()];
        r.roles.set("Eye L", Some(StageRole::Eyes));
        r.roles.set("Eye R", Some(StageRole::Eyes));
        r.play(Section::Groove, 8);
        let global = r.d.slots.iter().position(|s| s.role == Role::Accent && s.group.is_none()).unwrap();
        let global_op = r.auto[global * 2 + r.d.slots[global].cur].opacity;
        assert!(level_of(&r, "Eye L", Role::Accent) > global_op, "eyes get the full accent");
        assert!(r.d.slots.iter().filter(|s| s.role == Role::Accent && s.group.is_some()).count() == 2);
    }

    #[test]
    fn the_right_half_of_a_pair_answers_the_left() {
        use crate::roles::StageRole;
        let mut r = Rig::new();
        r.groups = vec!["L".into(), "R".into()];
        r.roles.set("L", Some(StageRole::Left));
        r.roles.set("R", Some(StageRole::Right));
        r.s.off_looks = crate::playbook::looks_for(Section::Groove).filter(|l| l.id != "groove-sweep").map(|l| l.id.to_string()).collect();
        r.play(Section::Groove, 2);
        let t = |g: &str| {
            let i = r.d.slots.iter().position(|s| s.role == Role::Motion && s.group.as_deref() == Some(g)).unwrap();
            r.auto[i * 2 + r.d.slots[i].cur].trigger_beat
        };
        assert!((t("L") - t("R")).abs() > 1.0, "the right half is half a cycle behind");
        assert!(level_of(&r, "L", Role::Motion) > 0.0 && level_of(&r, "R", Role::Motion) > 0.0);
    }
}
