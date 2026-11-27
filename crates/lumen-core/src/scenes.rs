//! The effects Kickline knows about, grouped for the browser, and what each
//! one is good for so Auto mode can choose between them.
//!
//! The list itself is the one the effect browser shows; the tags (role, how
//! energetic, which song sections, whether it strobes) sit next to it so the
//! two never drift apart.

use crate::audio::AudioBand;
use crate::generator::Generator;
use crate::palette::Paint;
use crate::sections::Section;
use crate::strip::StripClip;
use std::sync::OnceLock;

/// Effects by category: (category, what it is, effects).
pub fn catalog() -> Vec<(&'static str, &'static str, Vec<Generator>)> {
    let c = Paint::Slot(0);
    let mut strips = Vec::new();
    for (_, sc) in StripClip::presets() {
        strips.push(Generator::Strip(sc));
    }
    vec![
        ("Movement", "Shapes that travel across the lights", vec![
            Generator::LeftRight { color: c, beats: 4.0, width: 0.2 },
            Generator::Scanner { color: c, beats: 4.0, width: 0.2 },
            Generator::Sweep { color: c, beats: 2.0, width: 0.15, inward: true },
            Generator::Sweep { color: c, beats: 2.0, width: 0.15, inward: false },
            Generator::Wipe { color: c, beats: 4.0 },
            Generator::Radar { color: c, beats_per_turn: 4.0, tail: 0.3 },
            Generator::Chase { color: c, width: 0.25, beats_per_cycle: 2.0 },
            Generator::Burst { color: c, per_beat: 1.0 },
            Generator::Checker { color: c, beats: 1.0 },
            Generator::Grow { color: c },
        ]),
        ("On the beat", "Flashes timed to the beat or the drums", vec![
            Generator::BeatPulse { color: c, per_beat: 1.0, fade: 0.6 },
            Generator::AudioFlash { band: AudioBand::Kick, color: c },
            Generator::Strobe { color: c, per_beat: 2.0, duty: 0.3 },
            Generator::Twinkle { color: c, density: 0.4, per_beat: 2.0 },
            Generator::Bands,
        ]),
        ("Along strips", "Run along every strip, whatever angle it's at", strips),
        ("Washes", "Fill everything with colour", vec![
            Generator::Solid { color: c },
            Generator::PaletteWash { beats_per_cycle: 16.0 },
            Generator::Rainbow { scale: 1.0, beats_per_cycle: 8.0 },
            Generator::Plasma { scale: 1.0, speed: 1.0 },
            Generator::Breathe { color: c, beats: 8.0 },
            Generator::Throb { color: c },
        ]),
    ]
}

/// The name an effect goes by in the browser and on clips.
pub fn name(g: &Generator) -> String {
    match g {
        Generator::Strip(sc) => StripClip::presets().into_iter().find(|(_, p)| p == sc).map(|(n, _)| n.to_string()).unwrap_or_else(|| g.kind_name().to_string()),
        Generator::AudioFlash { band, .. } => format!("{} flash", band.name()),
        _ => g.kind_name().to_string(),
    }
}

/// What part an effect plays in a show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Fills everything with colour.
    Wash,
    /// Shapes that travel.
    Motion,
    /// Hits on the beat and the drums.
    Accent,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Wash => "Wash",
            Role::Motion => "Motion",
            Role::Accent => "Accent",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tags {
    pub role: Role,
    /// Energy this effect suits (0 = calm, 1 = full power).
    pub energy: (f32, f32),
    /// Song sections it suits, one bit per `Section::index()`.
    pub sections: u8,
    /// Flashes hard: left out when flash-safe is on.
    pub strobe: bool,
}

impl Tags {
    pub fn suits(&self, s: Section) -> bool {
        self.sections & (1 << s.index()) != 0
    }
}

/// "GMBPD": the sections an effect suits (Groove, Melodic, Build-up, Pause, Drop).
fn mask(s: &str) -> u8 {
    s.chars().fold(0, |m, c| {
        m | match c {
            'G' => 1 << Section::Groove.index(),
            'M' => 1 << Section::Melodic.index(),
            'B' => 1 << Section::BuildUp.index(),
            'P' => 1 << Section::Pause.index(),
            'D' => 1 << Section::Drop.index(),
            _ => 0,
        }
    })
}

/// Tags for an effect, by its browser name.
pub fn tags_for(name: &str) -> Tags {
    let t = |role, lo, hi, secs: &str, strobe| Tags { role, energy: (lo, hi), sections: mask(secs), strobe };
    use Role::*;
    match name {
        "Left ↔ right" => t(Motion, 0.25, 0.8, "GMD", false),
        "Up ↕ down" => t(Motion, 0.25, 0.8, "GMD", false),
        "Sweep in" => t(Motion, 0.4, 1.0, "BG", false),
        "Sweep out" => t(Motion, 0.5, 1.0, "DG", false),
        "Wipe" => t(Motion, 0.2, 0.7, "GMB", false),
        "Radar" => t(Motion, 0.2, 0.8, "GMBD", false),
        "Chase" => t(Motion, 0.4, 0.9, "GBD", false),
        "Beat pulse" => t(Accent, 0.2, 0.8, "GMBD", false),
        "Kick flash" => t(Accent, 0.4, 1.0, "GD", false),
        "Strobe" => t(Accent, 0.8, 1.0, "BD", true),
        "Twinkle" => t(Accent, 0.15, 0.7, "MG", false),
        "Bands" => t(Accent, 0.0, 0.6, "M", false),
        "Meteor" => t(Motion, 0.3, 0.8, "GMD", false),
        "Meteor bounce" => t(Motion, 0.4, 0.9, "GBD", false),
        "Fill to the bar" => t(Motion, 0.4, 0.9, "B", false),
        "Kick burst" => t(Accent, 0.5, 1.0, "GD", false),
        "Snare segments" => t(Accent, 0.4, 0.9, "GB", false),
        "Hi-hat sparkle" => t(Accent, 0.3, 0.8, "GMD", false),
        "Bass meter" => t(Motion, 0.2, 0.8, "GMD", false),
        "Dashes" => t(Motion, 0.4, 0.9, "GBD", false),
        "Kick alternate" => t(Accent, 0.5, 1.0, "GD", false),
        "Wave across strips" => t(Motion, 0.3, 0.9, "GMBD", false),
        "Burst" => t(Motion, 0.4, 1.0, "GBD", false),
        "Checker swap" => t(Motion, 0.3, 0.9, "GBD", false),
        "Bass bloom" => t(Motion, 0.3, 1.0, "GMD", false),
        "Breathe" => t(Wash, 0.0, 0.6, "MPG", false),
        "Bass throb" => t(Wash, 0.3, 1.0, "GD", false),
        "Solid" => t(Wash, 0.0, 0.5, "MG", false),
        "Palette wash" => t(Wash, 0.0, 0.8, "GMB", false),
        "Rainbow" => t(Wash, 0.3, 0.9, "GD", false),
        "Plasma" => t(Wash, 0.2, 0.8, "GMBD", false),
        _ => t(Motion, 0.0, 1.0, "GMBPD", false),
    }
}

/// How energetic a palette feels (by name): (calm end, wild end).
pub fn palette_energy(name: &str) -> (f32, f32) {
    match name {
        "Fire" => (0.5, 1.0),
        "Ice" => (0.0, 0.7),
        "Acid" => (0.4, 1.0),
        "Sunset" => (0.2, 0.8),
        "Police" => (0.75, 1.0),
        "UV" => (0.3, 0.9),
        "Ocean" => (0.0, 0.6),
        "Warm white" => (0.0, 0.4),
        _ => (0.0, 1.0),
    }
}

/// One effect Auto mode can choose.
#[derive(Clone, Debug)]
pub struct Scene {
    pub name: String,
    pub generator: Generator,
    pub tags: Tags,
}

/// Every effect with its tags (built once).
pub fn bank() -> &'static [Scene] {
    static BANK: OnceLock<Vec<Scene>> = OnceLock::new();
    BANK.get_or_init(|| {
        catalog()
            .into_iter()
            .flat_map(|(_, _, list)| list)
            .map(|g| {
                let n = name(&g);
                Scene { tags: tags_for(&n), name: n, generator: g }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_is_tagged() {
        let known = |n: &str| !matches!(tags_for(n).sections, s if s == mask("GMBPD") && tags_for(n).energy == (0.0, 1.0));
        for s in bank() {
            assert!(known(&s.name), "{} has no tags", s.name);
            assert!(s.tags.energy.0 <= s.tags.energy.1);
        }
        assert_eq!(bank().len(), 31);
    }

    #[test]
    fn each_role_has_choices_in_every_section() {
        for role in [Role::Wash, Role::Motion, Role::Accent] {
            for sec in Section::ALL {
                if sec == Section::Pause {
                    continue; // nothing new is picked in the pause
                }
                let n = bank().iter().filter(|s| s.tags.role == role && s.tags.suits(sec) && !s.tags.strobe).count();
                assert!(n >= 1, "{role:?} has no safe effect for {sec:?}");
            }
        }
    }
}
