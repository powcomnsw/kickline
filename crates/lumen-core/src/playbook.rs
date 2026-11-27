//! The playbook: what Auto plays in each part of a song.
//!
//! A **move** is one effect with its settings. A **look** is up to three
//! moves that play together (a wash under everything, motion, and accents on
//! the drums), written for one song section. The playbook holds a list of
//! looks per section, and `Bag` plays them in a seeded shuffled order: every
//! look is used once before any is used again, and a new round never starts
//! with the look just played. So the second drop doesn't look like the first.
//!
//! Families, by rule:
//! * Groove: steady motion on the beat: sweeps, meteors, dashes, alternates.
//! * Melodic: slow and soft: breathing washes, slow motion, twinkle.
//! * Build-up: chases and fills that speed up and brighten as it goes.
//! * Pause: dark; one small element at most.
//! * Drop: strobes, flashes and bursts on the kick, and effects driven by
//!   the bass. The whole rig, high contrast.

use crate::generator::Generator;
use crate::palette::Paint;
use crate::scenes;
use crate::sections::Section;
use crate::strip::StripEffect;

/// A setting that moves with how far through the section the music is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    /// Brightness multiplier at the start and at the end.
    pub level: (f32, f32),
    /// Speed multiplier at the start and at the end (2 = twice as fast).
    pub speed: (f32, f32),
}

impl Ramp {
    pub const BUILD: Ramp = Ramp { level: (0.35, 1.0), speed: (0.6, 2.4) };
    pub const NONE: Ramp = Ramp { level: (1.0, 1.0), speed: (1.0, 1.0) };

    pub fn level_at(&self, p: f32) -> f32 {
        self.level.0 + (self.level.1 - self.level.0) * p.clamp(0.0, 1.0)
    }
    pub fn speed_at(&self, p: f32) -> f32 {
        self.speed.0 + (self.speed.1 - self.speed.0) * p.clamp(0.0, 1.0)
    }
    pub fn is_none(&self) -> bool {
        *self == Ramp::NONE
    }
}

/// One effect aimed at one part of the show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    /// The effect's name in the effect browser.
    pub effect: &'static str,
    /// Which of the palette's four colours it paints with.
    pub slot: u8,
    /// How bright, compared with its role's usual level (0..1).
    pub level: f32,
    pub ramp: Ramp,
}

const fn mv(effect: &'static str, slot: u8, level: f32) -> Move {
    Move { effect, slot, level, ramp: Ramp::NONE }
}

const fn ramped(effect: &'static str, slot: u8, level: f32, ramp: Ramp) -> Move {
    Move { effect, slot, level, ramp }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Stable name, saved when a look is switched off.
    pub id: &'static str,
    pub section: Section,
    pub wash: Option<Move>,
    pub motion: Option<Move>,
    pub accent: Option<Move>,
}

impl Look {
    pub fn moves(&self) -> impl Iterator<Item = &Move> {
        [&self.wash, &self.motion, &self.accent].into_iter().flatten()
    }

    /// Flashes hard (a strobe).
    pub fn strobe(&self) -> bool {
        self.moves().any(|m| scenes::tags_for(m.effect).strobe)
    }

    /// Needs a steady beat to make sense.
    pub fn needs_beat(&self) -> bool {
        self.moves().any(|m| effect_needs_beat(m.effect))
    }

    pub fn uses(&self, effect: &str) -> bool {
        self.moves().any(|m| m.effect == effect)
    }
}

/// Does this effect only make sense on a steady beat? Washes, twinkles and
/// the drum-driven ones follow the music itself, not the beat grid.
pub fn effect_needs_beat(effect: &str) -> bool {
    !matches!(effect, "Solid" | "Palette wash" | "Rainbow" | "Plasma" | "Twinkle" | "Bands" | "Bass meter" | "Hi-hat sparkle" | "Kick flash" | "Kick burst" | "Kick alternate" | "Snare segments")
}

const fn look(id: &'static str, section: Section, wash: Option<Move>, motion: Option<Move>, accent: Option<Move>) -> Look {
    Look { id, section, wash, motion, accent }
}

use Section::{BuildUp, Drop, Groove, Melodic, Pause};

/// The built-in looks (compiled in; a show stores only what the user changed).
pub const LOOKS: &[Look] = &[
    // ---- Groove: steady motion on the beat.
    look("groove-sweep", Groove, Some(mv("Palette wash", 0, 0.5)), Some(mv("Left ↔ right", 1, 1.0)), Some(mv("Beat pulse", 2, 0.8))),
    look("groove-meteors", Groove, Some(mv("Solid", 3, 0.3)), Some(mv("Meteor", 0, 1.0)), Some(mv("Kick burst", 3, 0.8))),
    look("groove-dashes", Groove, Some(mv("Palette wash", 1, 0.45)), Some(mv("Dashes", 2, 1.0)), Some(mv("Kick alternate", 0, 0.9))),
    look("groove-waves", Groove, Some(mv("Plasma", 0, 0.5)), Some(mv("Wave across strips", 2, 1.0)), Some(mv("Beat pulse", 3, 0.7))),
    look("groove-radar", Groove, Some(mv("Solid", 1, 0.25)), Some(mv("Radar", 0, 1.0)), Some(mv("Hi-hat sparkle", 3, 0.8))),
    look("groove-bounce", Groove, Some(mv("Palette wash", 2, 0.45)), Some(mv("Meteor bounce", 1, 1.0)), Some(mv("Snare segments", 3, 0.9))),
    look("groove-updown", Groove, Some(mv("Solid", 2, 0.3)), Some(mv("Up ↕ down", 0, 1.0)), Some(mv("Kick flash", 3, 0.7))),
    look("groove-bass", Groove, Some(mv("Plasma", 1, 0.45)), Some(mv("Bass meter", 0, 1.0)), Some(mv("Kick alternate", 2, 0.8))),
    look("groove-bass-sparkle", Groove, Some(mv("Plasma", 2, 0.45)), Some(mv("Bass meter", 1, 1.0)), Some(mv("Hi-hat sparkle", 3, 0.8))),
    look("groove-bass-wash", Groove, Some(mv("Palette wash", 3, 0.5)), Some(mv("Bass meter", 2, 1.0)), Some(mv("Kick burst", 0, 0.8))),
    // ---- Melodic: slow and soft.
    look("melodic-breathe", Melodic, Some(mv("Palette wash", 0, 0.55)), None, Some(mv("Twinkle", 3, 0.5))),
    look("melodic-plasma", Melodic, Some(mv("Plasma", 1, 0.5)), None, Some(mv("Twinkle", 2, 0.4))),
    look("melodic-radar", Melodic, Some(mv("Solid", 0, 0.3)), Some(ramped("Radar", 1, 0.6, Ramp { level: (1.0, 1.0), speed: (0.4, 0.4) })), None),
    look("melodic-drift", Melodic, Some(mv("Palette wash", 2, 0.5)), Some(ramped("Wipe", 0, 0.5, Ramp { level: (1.0, 1.0), speed: (0.5, 0.5) })), Some(mv("Twinkle", 3, 0.4))),
    look("melodic-bands", Melodic, Some(mv("Solid", 1, 0.25)), None, Some(mv("Bands", 0, 0.6))),
    look("melodic-meteor", Melodic, Some(mv("Plasma", 2, 0.4)), Some(ramped("Meteor", 1, 0.55, Ramp { level: (1.0, 1.0), speed: (0.5, 0.5) })), None),
    look("melodic-sparkle", Melodic, Some(mv("Palette wash", 1, 0.4)), None, Some(mv("Hi-hat sparkle", 3, 0.5))),
    look("melodic-updown", Melodic, Some(mv("Solid", 2, 0.3)), Some(ramped("Up ↕ down", 0, 0.5, Ramp { level: (1.0, 1.0), speed: (0.5, 0.5) })), Some(mv("Twinkle", 1, 0.4))),
    // ---- Build-up: chases and fills that speed up and tighten.
    look("build-fill", BuildUp, Some(ramped("Palette wash", 0, 0.5, Ramp::BUILD)), Some(ramped("Fill to the bar", 2, 1.0, Ramp::BUILD)), Some(ramped("Snare segments", 3, 0.9, Ramp::BUILD))),
    look("build-chase", BuildUp, Some(ramped("Solid", 1, 0.4, Ramp::BUILD)), Some(ramped("Chase", 0, 1.0, Ramp::BUILD)), Some(ramped("Beat pulse", 3, 0.8, Ramp::BUILD))),
    look("build-sweep", BuildUp, Some(ramped("Plasma", 2, 0.5, Ramp::BUILD)), Some(ramped("Sweep in", 0, 1.0, Ramp::BUILD)), Some(ramped("Snare segments", 3, 0.9, Ramp::BUILD))),
    look("build-dashes", BuildUp, Some(ramped("Palette wash", 1, 0.5, Ramp::BUILD)), Some(ramped("Dashes", 2, 1.0, Ramp::BUILD)), Some(ramped("Beat pulse", 0, 0.8, Ramp::BUILD))),
    look("build-bounce", BuildUp, Some(ramped("Solid", 2, 0.4, Ramp::BUILD)), Some(ramped("Meteor bounce", 0, 1.0, Ramp::BUILD)), Some(ramped("Hi-hat sparkle", 3, 0.9, Ramp::BUILD))),
    look("build-radar", BuildUp, Some(ramped("Palette wash", 0, 0.45, Ramp::BUILD)), Some(ramped("Radar", 1, 1.0, Ramp::BUILD)), Some(ramped("Snare segments", 2, 0.9, Ramp::BUILD))),
    look("build-wipe", BuildUp, Some(ramped("Plasma", 1, 0.5, Ramp::BUILD)), Some(ramped("Wipe", 2, 1.0, Ramp::BUILD)), Some(ramped("Beat pulse", 3, 0.8, Ramp::BUILD))),
    look("build-strobe", BuildUp, Some(ramped("Solid", 0, 0.4, Ramp::BUILD)), Some(ramped("Chase", 2, 1.0, Ramp::BUILD)), Some(ramped("Strobe", 3, 0.9, Ramp { level: (0.2, 1.0), speed: (1.0, 1.0) }))),
    // ---- Pause: dark, one small element at most.
    look("pause-dark", Pause, None, None, None),
    look("pause-twinkle", Pause, None, None, Some(mv("Twinkle", 3, 0.5))),
    look("pause-pulse", Pause, None, None, Some(mv("Beat pulse", 0, 0.5))),
    look("pause-bands", Pause, None, None, Some(mv("Bands", 1, 0.5))),
    // ---- Drop: the whole rig, high contrast.
    look("drop-strobe", Drop, Some(mv("Solid", 3, 0.5)), Some(mv("Chase", 0, 1.0)), Some(mv("Strobe", 3, 1.0))),
    look("drop-strobe-rainbow", Drop, Some(mv("Rainbow", 0, 0.8)), Some(mv("Sweep out", 1, 1.0)), Some(mv("Strobe", 3, 1.0))),
    look("drop-flash", Drop, Some(mv("Rainbow", 0, 0.7)), Some(mv("Meteor", 2, 1.0)), Some(mv("Kick flash", 3, 1.0))),
    look("drop-burst", Drop, Some(mv("Plasma", 0, 0.7)), Some(mv("Wave across strips", 1, 1.0)), Some(mv("Kick burst", 3, 1.0))),
    look("drop-bass-meter", Drop, Some(mv("Plasma", 1, 0.8)), Some(mv("Bass meter", 0, 1.0)), Some(mv("Kick alternate", 3, 0.9))),
    look("drop-bass-sweep", Drop, Some(mv("Rainbow", 2, 0.6)), Some(mv("Sweep out", 0, 1.0)), Some(mv("Kick burst", 3, 0.9))),
    look("drop-chase", Drop, Some(mv("Solid", 1, 0.5)), Some(mv("Chase", 2, 1.0)), Some(mv("Kick flash", 3, 1.0))),
    look("drop-bounce", Drop, Some(mv("Plasma", 2, 0.7)), Some(mv("Meteor bounce", 0, 1.0)), Some(mv("Hi-hat sparkle", 3, 0.9))),
];

/// The built-in looks for a section.
pub fn looks_for(s: Section) -> impl Iterator<Item = &'static Look> {
    LOOKS.iter().filter(move |l| l.section == s)
}

pub fn find(id: &str) -> Option<&'static Look> {
    LOOKS.iter().find(|l| l.id == id)
}

/// The effect for a move, painted with its colour slot.
pub fn generator_for(m: &Move, drops: u32, section: Section) -> Option<Generator> {
    let sc = scenes::bank().iter().find(|s| s.name == m.effect)?;
    let mut g = sc.generator.clone();
    if let Some(p) = g.paint_mut() {
        // The second drop paints with a different colour from the first.
        let shift = if section == Section::Drop { drops as u8 } else { 0 };
        *p = Paint::Slot((m.slot + shift) % 4);
    }
    Some(g)
}

/// The effect running `f` times as fast (2 = twice).
pub fn scale_speed(g: &Generator, f: f32) -> Generator {
    let f = f.max(0.05);
    let mut g = g.clone();
    match &mut g {
        Generator::Chase { beats_per_cycle, .. } | Generator::Rainbow { beats_per_cycle, .. } | Generator::PaletteWash { beats_per_cycle } => *beats_per_cycle /= f,
        Generator::Radar { beats_per_turn, .. } => *beats_per_turn /= f,
        Generator::LeftRight { beats, .. } | Generator::Scanner { beats, .. } | Generator::Sweep { beats, .. } | Generator::Wipe { beats, .. } => *beats /= f,
        Generator::Strobe { per_beat, .. } | Generator::BeatPulse { per_beat, .. } | Generator::Twinkle { per_beat, .. } => *per_beat *= f,
        Generator::Plasma { speed, .. } => *speed *= f,
        Generator::Strip(sc) => match &mut sc.effect {
            StripEffect::Meteor { beats, .. } | StripEffect::Fill { beats, .. } | StripEffect::Dashes { beats, .. } => *beats /= f,
            _ => {}
        },
        _ => {}
    }
    g
}

/// Plays a section's looks in a seeded shuffled order.
#[derive(Clone, Debug, Default)]
pub struct Bag {
    /// Looks still to come this round (next at the end).
    pending: Vec<&'static str>,
    last: Option<&'static str>,
}

impl Bag {
    /// The next look. `ok` says which looks may play now (switched off,
    /// strobing with flash-safe on, needing a beat that isn't there). `rand`
    /// gives numbers in 0..1.
    pub fn next(&mut self, section: Section, ok: impl Fn(&Look) -> bool, mut rand: impl FnMut() -> f32) -> Option<&'static Look> {
        let allowed: Vec<&'static str> = looks_for(section).filter(|l| ok(l)).map(|l| l.id).collect();
        if allowed.is_empty() {
            return None;
        }
        self.pending.retain(|id| allowed.contains(id));
        if self.pending.is_empty() {
            let mut round = allowed.clone();
            // Fisher-Yates.
            for i in (1..round.len()).rev() {
                let j = (rand() * (i + 1) as f32) as usize % (i + 1);
                round.swap(i, j);
            }
            // The next look is the last element: never the one just played.
            if round.len() > 1 && round.last().copied() == self.last {
                let n = round.len();
                round.swap(n - 1, 0);
            }
            self.pending = round;
        }
        let id = self.pending.pop()?;
        self.last = Some(id);
        find(id)
    }

    pub fn last(&self) -> Option<&'static str> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> impl FnMut() -> f32 {
        let mut x = 0x1234_5678_9ABC_DEF1u64;
        move || {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
        }
    }

    #[test]
    fn every_move_names_a_real_effect_and_ids_are_unique() {
        for l in LOOKS {
            for m in l.moves() {
                assert!(scenes::bank().iter().any(|s| s.name == m.effect), "{}: no effect {}", l.id, m.effect);
                assert!((0.0..=1.0).contains(&m.level), "{}", l.id);
            }
            assert_eq!(LOOKS.iter().filter(|o| o.id == l.id).count(), 1, "duplicate id {}", l.id);
        }
    }

    #[test]
    fn every_section_has_looks_and_the_families_hold() {
        for s in Section::ALL {
            assert!(looks_for(s).count() >= 4, "{s:?}");
        }
        // Build-ups only get the build-up family: moves that ramp.
        for l in looks_for(BuildUp) {
            assert!(l.moves().any(|m| !m.ramp.is_none()), "{} does not build", l.id);
            assert!(l.moves().all(|m| !matches!(m.effect, "Twinkle" | "Bands" | "Kick burst" | "Kick flash")), "{}", l.id);
        }
        // Pauses are dark: at most one small element.
        for l in looks_for(Pause) {
            assert!(l.moves().count() <= 1 && l.moves().all(|m| m.level <= 0.5), "{}", l.id);
        }
        // Melodic is soft: nothing hits hard.
        for l in looks_for(Melodic) {
            assert!(l.moves().all(|m| !matches!(m.effect, "Strobe" | "Kick flash" | "Kick burst" | "Kick alternate" | "Snare segments")), "{}", l.id);
        }
        // Drops have real strobes and bass-driven looks, and enough that are safe.
        assert!(looks_for(Drop).filter(|l| l.strobe()).count() >= 2);
        assert!(looks_for(Drop).filter(|l| l.uses("Bass meter") || l.id.contains("bass")).count() >= 2);
        assert!(looks_for(Drop).filter(|l| !l.strobe()).count() >= 6);
        // Bass-driven looks belong in Groove too.
        assert!(looks_for(Groove).any(|l| l.uses("Bass meter")));
    }

    #[test]
    fn every_look_is_used_before_any_repeats_and_never_twice_running() {
        for s in Section::ALL {
            let n = looks_for(s).count();
            let mut bag = Bag::default();
            let mut r = rng();
            let mut seq = Vec::new();
            for _ in 0..(n * 6) {
                seq.push(bag.next(s, |_| true, &mut r).unwrap().id);
            }
            for w in seq.windows(2) {
                assert_ne!(w[0], w[1], "{s:?}: same look twice running");
            }
            // Within every round of n, each look appears exactly once.
            for round in seq.chunks(n) {
                let mut ids = round.to_vec();
                ids.sort();
                ids.dedup();
                assert_eq!(ids.len(), n, "{s:?}: {round:?}");
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_order_and_another_seed_another() {
        let run = |seed: u64| {
            let mut x = seed | 1;
            let mut bag = Bag::default();
            (0..16)
                .map(|_| {
                    bag.next(Drop, |_| true, || {
                        x ^= x >> 12;
                        x ^= x << 25;
                        x ^= x >> 27;
                        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
                    })
                    .unwrap()
                    .id
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(99));
    }

    #[test]
    fn switched_off_looks_are_skipped_and_the_bag_copes_with_one_left() {
        let mut bag = Bag::default();
        let mut r = rng();
        for _ in 0..20 {
            let l = bag.next(Drop, |l| !l.strobe(), &mut r).unwrap();
            assert!(!l.strobe());
        }
        for _ in 0..5 {
            assert_eq!(bag.next(Drop, |l| l.id == "drop-chase", &mut r).unwrap().id, "drop-chase");
        }
        assert!(bag.next(Drop, |_| false, &mut r).is_none());
    }

    #[test]
    fn speeding_up_shortens_the_cycle() {
        let g = Generator::Chase { color: Paint::Slot(0), width: 0.2, beats_per_cycle: 4.0 };
        match scale_speed(&g, 2.0) {
            Generator::Chase { beats_per_cycle, .. } => assert_eq!(beats_per_cycle, 2.0),
            _ => panic!(),
        }
    }

    #[test]
    fn needs_beat_is_false_for_washes_and_drum_driven_effects() {
        assert!(!effect_needs_beat("Plasma"));
        assert!(!effect_needs_beat("Kick burst"));
        assert!(effect_needs_beat("Chase"));
        assert!(look("x", Groove, Some(mv("Plasma", 0, 1.0)), None, Some(mv("Kick burst", 0, 1.0))).needs_beat() == false);
    }
}
