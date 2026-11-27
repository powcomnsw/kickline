//! Two more example stages besides the DJ stage: a face and a dragon. They
//! are only examples (the user's real stages differ every time), but they
//! stress what the DJ stage doesn't: a closed outline, focal points, and a
//! long chain of strips that an effect has to run along as one line.

use crate::project;
use lumen_core::{Composition, Looks, Section, StageRole};
use lumen_fixtures::{AdvancedOutput, Fixture, FixtureType, Lumiverse};
use std::collections::BTreeMap;

/// Places fixtures and numbers their controller outputs (H807SA: output N
/// starts on universe 4 x (N - 1)).
struct Builder {
    o: AdvancedOutput,
    used: BTreeMap<u16, u16>,
}

impl Builder {
    fn new() -> Self {
        let mut o = AdvancedOutput { aspect: 16.0 / 9.0, ..Default::default() };
        o.lumiverses.push(Lumiverse::new("H807SA", 0));
        Builder { o, used: BTreeMap::new() }
    }

    fn type_of(&mut self, cols: usize, rows: usize) -> usize {
        let name = if rows == 1 { format!("Pixel strip {cols} RGB") } else { format!("LED matrix {cols}x{rows}") };
        if let Some(i) = self.o.types.iter().position(|t| t.name == name) {
            return i;
        }
        self.o.types.push(FixtureType::new(&name, cols, rows, "RGB"));
        self.o.types.len() - 1
    }

    fn put(&mut self, name: &str, type_id: usize, out: u16, x: f32, y: f32, w: f32, h: f32, rot: f32, groups: &[&str]) -> usize {
        let px = self.o.types[type_id].pixel_count() as u16;
        let used = self.used.entry(out).or_insert(0);
        let address = 1 + *used;
        *used += px * 3;
        self.o.fixtures.push(Fixture {
            name: name.into(),
            type_id,
            lumiverse: 0,
            address,
            x,
            y,
            w,
            h,
            rotation: rot,
            snake: rot == 0.0 && self.o.types[type_id].rows > 1,
            reverse: false,
            universe_offset: (out - 1) * 4,
            path_order: 0,
            path_reverse: false,
            white: Default::default(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
        });
        self.o.fixtures.len() - 1
    }

    /// A strip from `a` to `b` (canvas fractions).
    fn strip(&mut self, name: &str, px: usize, out: u16, a: (f32, f32), b: (f32, f32), groups: &[&str]) -> usize {
        let asp = self.o.aspect;
        let (dx, dy) = ((b.0 - a.0) * asp, b.1 - a.1);
        let t = self.type_of(px, 1);
        self.put(name, t, out, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, (dx * dx + dy * dy).sqrt() / asp, 0.018, dy.atan2(dx).to_degrees(), groups)
    }

    fn matrix(&mut self, name: &str, cols: usize, rows: usize, out: u16, x: f32, y: f32, w: f32, h: f32, groups: &[&str]) -> usize {
        let t = self.type_of(cols, rows);
        self.put(name, t, out, x, y, w, h, 0.0, groups)
    }

    fn finish(mut self, roles: &[(&str, StageRole)]) -> project::Project {
        self.o.groups = self.o.group_names();
        for (g, r) in roles {
            self.o.roles.set(g, Some(*r));
        }
        let mut c = Composition::demo();
        c.layers.clear();
        c.director.on = true;
        c.looks = Looks { auto: true, palette: [Some(0), Some(1), Some(2), None, Some(0)], level: [0.8, 0.6, 0.9, 0.1, 1.0] };
        c.apply_look(Section::Groove);
        project::Project { composition: c, output: self.o }
    }
}

/// A face: a closed head outline, two eyes, brows, a mouth and a nose.
pub fn face_show() -> project::Project {
    let mut b = Builder::new();
    let asp = b.o.aspect;
    // Head: ten strips round an oval, in one closed loop.
    let (cx, cy, rx, ry) = (0.5, 0.5, 0.26, 0.44);
    let pt = |k: usize| -> (f32, f32) {
        let a = (-90.0f32 + 36.0 * k as f32).to_radians();
        (cx + rx * a.cos() * 9.0 / 16.0 * 1.0 / (asp / (16.0 / 9.0)), cy + ry * a.sin())
    };
    for k in 0..10 {
        b.strip(&format!("Head {}", k + 1), 30, 1, pt(k), pt((k + 1) % 10), &["Head"]);
    }
    b.matrix("Eye left", 8, 8, 2, 0.43, 0.38, 0.06, 0.11, &["Eye left"]);
    b.matrix("Eye right", 8, 8, 3, 0.57, 0.38, 0.06, 0.11, &["Eye right"]);
    b.strip("Brow left", 30, 4, (0.39, 0.27), (0.47, 0.25), &["Brow left"]);
    b.strip("Brow right", 30, 4, (0.53, 0.25), (0.61, 0.27), &["Brow right"]);
    b.matrix("Nose", 8, 8, 5, 0.5, 0.5, 0.05, 0.1, &["Nose"]);
    // Mouth: five strips along a smile, in order from the left corner.
    let mouth = [(0.41, 0.62), (0.45, 0.67), (0.5, 0.69), (0.55, 0.67), (0.59, 0.62)];
    for k in 0..4 {
        let i = b.strip(&format!("Mouth {}", k + 1), 30, 6, mouth[k], mouth[k + 1], &["Mouth"]);
        b.o.fixtures[i].path_order = k as u16 + 1;
    }
    b.finish(&[("Head", StageRole::Outline), ("Eye left", StageRole::Eyes), ("Eye right", StageRole::Eyes), ("Brow left", StageRole::Left), ("Brow right", StageRole::Right), ("Nose", StageRole::Centre), ("Mouth", StageRole::Path)])
}

/// A dragon: a long spine that has to be run as one line, wings, a head,
/// two eyes and a belly.
pub fn dragon_show() -> project::Project {
    let mut b = Builder::new();
    // Spine: two rows of four strips, the second row wired back the other way.
    for k in 0..4 {
        let x0 = 0.25 + 0.125 * k as f32;
        let i = b.strip(&format!("Spine {}", k + 1), 60, 1, (x0, 0.74), (x0 + 0.125, 0.74), &["Spine"]);
        b.o.fixtures[i].path_order = k as u16 + 1;
    }
    for k in 0..4 {
        let x1 = 0.75 - 0.125 * k as f32;
        let i = b.strip(&format!("Spine {}", k + 5), 60, 1, (x1 - 0.125, 0.84), (x1, 0.84), &["Spine"]);
        b.o.fixtures[i].path_order = k as u16 + 5;
        b.o.fixtures[i].path_reverse = true;
    }
    b.matrix("Wing left", 16, 16, 2, 0.14, 0.38, 0.2, 0.62, &["Wing left"]);
    b.matrix("Wing right", 16, 16, 3, 0.86, 0.38, 0.2, 0.62, &["Wing right"]);
    b.matrix("Head", 16, 8, 4, 0.5, 0.3, 0.2, 0.3, &["Head"]);
    b.matrix("Eye left", 8, 8, 5, 0.44, 0.2, 0.05, 0.09, &["Eye left"]);
    b.matrix("Eye right", 8, 8, 6, 0.56, 0.2, 0.05, 0.09, &["Eye right"]);
    b.matrix("Belly", 16, 8, 7, 0.5, 0.56, 0.24, 0.2, &["Belly"]);
    b.finish(&[("Spine", StageRole::Path), ("Wing left", StageRole::Left), ("Wing right", StageRole::Right), ("Head", StageRole::Centre), ("Eye left", StageRole::Eyes), ("Eye right", StageRole::Eyes), ("Belly", StageRole::Fill)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stages() -> Vec<(&'static str, project::Project)> {
        vec![("face", face_show()), ("dragon", dragon_show())]
    }

    #[test]
    fn stages_are_addressed_like_an_h807sa_and_nothing_spills() {
        for (n, p) in stages() {
            for f in &p.output.fixtures {
                for s in p.output.layout(f) {
                    assert!(s.universe_offset < f.universe_offset + 4, "{n}: {} spills into the next output", f.name);
                }
            }
        }
    }

    #[test]
    fn every_group_has_a_role_that_matches_the_layout_guess() {
        for (n, p) in stages() {
            let o = &p.output;
            let guess = o.suggest_roles();
            for g in o.group_names() {
                assert!(o.roles.get(&g).is_some(), "{n}: {g} has no role");
            }
            // The guess is only a guess, but it should be right for these obvious shapes.
            let agree = o.group_names().iter().filter(|g| guess.get(g) == o.roles.get(g)).count();
            assert!(agree * 10 >= o.group_names().len() * 7, "{n}: the guess agrees on {agree} of {}", o.group_names().len());
        }
    }

    #[test]
    fn the_dragon_spine_is_one_line_in_wiring_order() {
        let p = dragon_show();
        let m = p.output.group_members("Spine");
        let orders: Vec<u16> = m.iter().map(|&i| p.output.fixtures[i].path_order).collect();
        assert_eq!(orders, (1..=8).collect::<Vec<u16>>());
        assert!(p.output.fixtures[m[7]].path_reverse && !p.output.fixtures[m[0]].path_reverse);
    }

    #[test]
    fn the_face_mouth_runs_left_to_right() {
        let p = face_show();
        let m = p.output.group_members("Mouth");
        let xs: Vec<f32> = m.iter().map(|&i| p.output.fixtures[i].x).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    }

    // ---- The same playbook on any stage

    use crate::engine::Engine;
    use std::time::Instant;

    /// Which fixtures have any light going to them right now.
    fn lit(e: &Engine) -> Vec<bool> {
        e.output
            .fixtures
            .iter()
            .map(|f| {
                let Some(lv) = e.output.lumiverses.get(f.lumiverse) else { return false };
                e.output.layout(f).iter().any(|s| e.dmx.get(&(f.lumiverse, lv.universe + s.universe_offset)).is_some_and(|u| u[s.start..(s.start + s.len).min(u.len())].iter().any(|&b| b > 0)))
            })
            .collect()
    }

    /// Plays every look of every lit section on a stage and returns, per look,
    /// the share of fixtures that lit at some point.
    fn coverage(p: &project::Project) -> Vec<(&'static str, lumen_core::Section, f32)> {
        let mut e = Engine::new(p.composition.clone(), p.output.clone());
        e.comp.director.on = true;
        e.start_demo_audio();
        let t0 = Instant::now();
        let mut out = Vec::new();
        for _ in 0..30 {
            e.step(1.0 / 44.0, false, t0);
        }
        for look in lumen_core::playbook::LOOKS.iter().filter(|l| l.section != lumen_core::Section::Pause) {
            e.comp.director_rt.preview = Some(look.id);
            let mut any = vec![false; e.output.fixtures.len()];
            for k in 0..30 {
                e.comp.director_rt.energy = 0.85;
                e.step(1.0 / 44.0, false, t0);
                if k >= 12 && k % 3 == 0 {
                    for (a, l) in any.iter_mut().zip(lit(&e)) {
                        *a |= l;
                    }
                }
            }
            out.push((look.id, look.section, any.iter().filter(|&&a| a).count() as f32 / any.len().max(1) as f32));
        }
        out
    }

    fn without_roles(mut p: project::Project) -> project::Project {
        p.output.roles = Default::default();
        p
    }

    /// A made-up stage: a few strips and panels scattered about, in a few groups.
    fn random_stage(seed: u64, with_roles: bool) -> project::Project {
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut r = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f32 / (1u64 << 53) as f32
        };
        let mut b = Builder::new();
        let names = ["Alpha", "Beta", "Gamma"];
        for k in 0..(3 + (r() * 6.0) as usize) {
            let g = if r() < 0.25 { vec![] } else { vec![names[(r() * 3.0) as usize % 3]] };
            let out = 1 + (k as u16 % 6);
            if r() < 0.6 {
                let a = (0.1 + 0.8 * r(), 0.1 + 0.8 * r());
                let c = ((a.0 + 0.2 * (r() - 0.5)).clamp(0.05, 0.95), (a.1 + 0.2 * (r() - 0.5)).clamp(0.05, 0.95));
                b.strip(&format!("S{k}"), 30, out, a, c, &g);
            } else {
                b.matrix(&format!("M{k}"), 8, 8, out, 0.1 + 0.8 * r(), 0.1 + 0.8 * r(), 0.05 + 0.15 * r(), 0.1 + 0.3 * r(), &g);
            }
        }
        let suggested = {
            let mut t = Builder { o: b.o.clone(), used: b.used.clone() };
            t.o.groups = t.o.group_names();
            t.o.suggest_roles()
        };
        let mut p = b.finish(&[]);
        if with_roles {
            p.output.roles = suggested;
        }
        p
    }

    #[test]
    fn every_look_lights_something_on_every_stage_with_or_without_roles() {
        let mut all = vec![("DJ stage", crate::stage::example_show()), ("face", face_show()), ("dragon", dragon_show())];
        let bare: Vec<_> = all.iter().map(|(n, p)| (*n, without_roles(p.clone()))).collect();
        all.extend(bare);
        for seed in 1..=1 {
            all.push(("random", random_stage(seed, true)));
            all.push(("random bare", random_stage(seed, false)));
        }
        for (name, p) in &all {
            let cov = coverage(p);
            if std::env::var("COVERAGE").is_ok() {
                let (lo, hi) = cov.iter().fold((1.0f32, 0.0f32), |(l, h), c| (l.min(c.2), h.max(c.2)));
                eprintln!("{name}: {} looks, coverage {:.0}%..{:.0}%", cov.len(), lo * 100.0, hi * 100.0);
            }
            for (id, section, share) in &cov {
                assert!(*share > 0.0, "{name}: look {id} lights nothing");
                if *section == lumen_core::Section::Drop {
                    assert!(*share >= 0.5, "{name}: drop look {id} lights only {:.0}% of the rig", share * 100.0);
                }
            }
        }
    }

    #[test]
    fn an_old_show_without_roles_still_plays() {
        let p = without_roles(crate::stage::example_show());
        assert!(p.output.roles.is_empty());
        let cov = coverage(&p);
        assert!(cov.iter().all(|(_, _, share)| *share > 0.0));
    }

    #[test]
    fn the_left_fader_darkens_only_the_left_side_of_the_stage() {
        let p = crate::stage::example_show();
        let mut e = Engine::new(p.composition, p.output);
        e.comp.director.on = true;
        e.start_demo_audio();
        let t0 = Instant::now();
        let left = e.output.group_members("Left panel");
        let right = e.output.group_members("Right panel");
        assert!(!left.is_empty() && !right.is_empty());
        let run = |e: &mut Engine| {
            let mut any = vec![false; e.output.fixtures.len()];
            for k in 0..90 {
                e.comp.director_rt.energy = 0.9;
                e.step(1.0 / 44.0, false, t0);
                if k > 30 && k % 5 == 0 {
                    for (a, l) in any.iter_mut().zip(lit(e)) {
                        *a |= l;
                    }
                }
            }
            any
        };
        let before = run(&mut e);
        assert!(left.iter().all(|&i| before[i]) && right.iter().all(|&i| before[i]));
        e.comp.gains.left = 0.0;
        let after = run(&mut e);
        assert!(left.iter().all(|&i| !after[i]), "left fader at zero: left panel dark");
        assert!(right.iter().all(|&i| after[i]), "the right side carries on");
    }

    #[test]
    fn auto_keeps_a_log_of_what_it_played_for_the_set_overview() {
        let p = crate::stage::example_show();
        let mut e = Engine::new(p.composition, p.output);
        e.start_demo_audio();
        let t0 = Instant::now();
        for _ in 0..200 {
            e.step(1.0 / 44.0, false, t0);
        }
        assert!(e.set_log.is_empty(), "nothing is logged while Auto is off");
        e.comp.director.on = true;
        for _ in 0..2500 {
            e.step(1.0 / 44.0, false, t0);
        }
        assert!(e.set_log.len() >= 3, "looks changed over the run: {}", e.set_log.len());
        assert!(e.set_log.windows(2).all(|w| w[1].t >= w[0].t));
        assert!(e.set_log.windows(2).all(|w| (w[0].section, &w[0].look) != (w[1].section, &w[1].look)), "only changes are kept");
        assert!(e.set_log.iter().all(|l| !l.look.is_empty()));
    }
}
