//! Performance controls on the Play screen: the state line, side faders,
//! crossfader and the hold-to-drop button.

use crate::{engine, theme, View};
use eframe::egui::{self, Color32, RichText, Vec2};
use lumen_core::{playbook, Section};

/// How serious the state line is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Ok,
    Warn,
    Bad,
}

/// What is wrong (or right) with the show, in words.
pub fn state(v: &View) -> (Level, String) {
    let e = &*v.e;
    if e.blackout {
        return (Level::Warn, "Lights are blacked out. Press B or the Blackout button to bring them back.".into());
    }
    if e.output.fixtures.is_empty() {
        return (Level::Bad, "No lights set up yet. Add fixtures in Set up → Stage.".into());
    }
    match e.health.state(e.output_enabled) {
        engine::OutputState::Off => return (Level::Warn, "DMX output is off. Press DMX ON in the top bar to send to the lights.".into()),
        engine::OutputState::Dead => return (Level::Bad, "DMX is on but nothing is reaching the lights. Check the interface and cable (Set up → Output).".into()),
        engine::OutputState::Partial => return (Level::Warn, "Some universes are not getting through. Check the cables and addresses (Set up → Output).".into()),
        _ => {}
    }
    if matches!(e.audio, engine::AudioSource::Off) && e.sync.link_enabled && e.link_status.as_ref().is_none_or(|s| s.peers == 0) {
        return (Level::Warn, "No music source and Link is waiting for rekordbox. Start a track, or pick an input in Set up → Music.".into());
    }
    if matches!(e.audio, engine::AudioSource::Off) {
        return (Level::Warn, "No music source. Pick one in Set up → Music, or switch Link on.".into());
    }
    if e.comp.idle {
        return (Level::Ok, "Waiting for music. The idle look is on.".into());
    }
    if e.comp.master < 0.05 {
        return (Level::Warn, "Master is down, so the lights are dark.".into());
    }
    (Level::Ok, if e.comp.director.on { "Show is running on Auto." } else { "Show is running by hand." }.into())
}

/// One line at the top of the Play side.
pub fn state_line(v: &View, ui: &mut egui::Ui) {
    let t = theme::current();
    let (lvl, text) = state(v);
    let col = match lvl {
        Level::Ok => t.ok,
        Level::Warn => t.warn,
        Level::Bad => t.err,
    };
    egui::Frame::new().fill(t.panel).corner_radius(t.radius + 2).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("●").color(col));
            ui.label(RichText::new(text).color(if lvl == Level::Ok { t.weak } else { t.text }).size(t.text_size - 1.0));
        });
    });
    ui.add_space(8.0);
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What Auto played, as a coloured timeline: one block per look, coloured by
/// the part of the song, with a mark at each drop. Hover for the details.
pub fn set_strip(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let now = v.e.uptime_s();
    let log = &v.e.set_log;
    if log.is_empty() {
        ui.label(RichText::new("Nothing yet. Switch Auto on and this fills in as the music goes.").color(t.weak).size(t.text_size - 1.0));
        return;
    }
    let from = log[0].t.max(now - 1800.0);
    let span = (now - from).max(1.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, t.raised);
    let x_of = |s: f64| rect.left() + ((s - from) / span) as f32 * rect.width();
    for (i, e) in log.iter().enumerate() {
        let end = log.get(i + 1).map_or(now, |n| n.t);
        if end < from {
            continue;
        }
        let r = egui::Rect::from_min_max(egui::pos2(x_of(e.t.max(from)), rect.top()), egui::pos2(x_of(end).max(x_of(e.t.max(from)) + 1.0), rect.bottom()));
        p.rect_filled(r.shrink2(Vec2::new(0.0, 0.0)), 0.0, crate::stage::section_color(e.section));
        p.line_segment([r.left_top(), r.left_bottom()], egui::Stroke::new(1.0, t.panel));
        if e.drop {
            p.line_segment([egui::pos2(r.left(), rect.top()), egui::pos2(r.left(), rect.top() + 8.0)], egui::Stroke::new(2.5, Color32::WHITE));
        }
    }
    if let Some(pos) = resp.hover_pos() {
        let at = from + ((pos.x - rect.left()) / rect.width()) as f64 * span;
        if let Some(e) = log.iter().rev().find(|e| e.t <= at) {
            resp.on_hover_text(format!("{}  {:?}\n{}\n{}", clock(e.t), e.section, e.look, e.palette));
        }
    }
    let drops = log.iter().filter(|e| e.drop).count();
    ui.label(RichText::new(format!("{} looks · {} drops · last {}", log.len(), drops, clock(span))).color(t.weak).size(t.text_size - 2.0));
    if ui.small_button("Clear").clicked() {
        v.e.set_log.clear();
    }
}

/// "groove-meteors" as "Groove meteors".
pub fn look_title(id: &str) -> String {
    let mut s = id.replace(['-', '_'], " ");
    if let Some(c) = s.get(..1) {
        let up = c.to_uppercase();
        s.replace_range(..1, &up);
    }
    s
}

/// One line on what a look does: its effects, in the order they are layered.
pub fn look_line(l: &playbook::Look) -> String {
    let names: Vec<&str> = l.moves().map(|m| m.effect).collect();
    if names.is_empty() { "Dark".into() } else { names.join(" + ") }
}

/// The look's main colour: the palette slot its leading effect paints with.
pub fn look_color(l: &playbook::Look, palette: &[[f32; 3]; 4]) -> Color32 {
    let m = l.motion.or(l.wash).or(l.accent);
    let c = palette[m.map_or(0, |m| m.slot as usize % 4)];
    Color32::from_rgb((c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8)
}

/// Scene cards: every look Auto can play for a part of the song. Click one to
/// play it now (Auto switches on if it was off).
pub fn scene_cards(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let id = ui.make_persistent_id("scene_section");
    let mut sec: Section = ui.data(|d| d.get_temp(id)).unwrap_or(Section::Groove);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Scenes").font(theme::bold(t.text_size)));
        for s in Section::ALL {
            ui.selectable_value(&mut sec, s, s.name());
        }
    });
    ui.data_mut(|d| d.insert_temp(id, sec));
    let pal = v.e.comp.palettes.current(v.e.comp.beat);
    let playing = v.e.comp.director_rt.look_name.clone();
    let mut pick = None;
    ui.horizontal_wrapped(|ui| {
        for l in playbook::looks_for(sec) {
            let active = v.e.comp.director.on && playing == l.id;
            let col = look_color(l, &pal);
            let (rect, r) = ui.allocate_exact_size(Vec2::new(150.0, 66.0), egui::Sense::click());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, t.radius, t.panel);
            p.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(5.0, rect.height())), t.radius, col);
            if active || r.hovered() {
                p.rect_stroke(rect, t.radius, egui::Stroke::new(1.5, if active { col } else { t.line }), egui::StrokeKind::Inside);
            }
            let text = rect.shrink2(Vec2::new(12.0, 6.0));
            p.text(text.left_top(), egui::Align2::LEFT_TOP, look_title(l.id), theme::bold(t.text_size - 1.0), t.text);
            let line = look_line(l);
            let galley = p.layout(line, egui::FontId::proportional(t.text_size - 3.0), t.weak, text.width());
            p.galley(text.left_top() + Vec2::new(0.0, 18.0), galley, t.weak);
            if r.on_hover_text("Play this look now").clicked() {
                pick = Some(l.id);
            }
        }
    });
    if let Some(idn) = pick {
        v.e.comp.director.on = true;
        v.e.comp.director_rt.preview = Some(idn);
    }
}

/// Left, Centre and Right faders, the crossfader, and hold-to-drop.
pub fn mix_card(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    ui.spacing_mut().slider_width = 200.0;
    for (label, which) in [("Left", 0), ("Centre", 1), ("Right", 2)] {
        ui.horizontal(|ui| {
            ui.add_sized([52.0, 18.0], egui::Label::new(RichText::new(label).color(t.weak)));
            let g = &mut v.e.comp.gains;
            let x = match which {
                0 => &mut g.left,
                1 => &mut g.centre,
                _ => &mut g.right,
            };
            ui.add(egui::Slider::new(x, 0.0..=1.0).show_value(false)).on_hover_text("Dims the lights set to this side of the stage (Set up → Stage → roles)");
        });
    }
    ui.horizontal(|ui| {
        ui.add_sized([52.0, 18.0], egui::Label::new(RichText::new("A ↔ B").color(t.weak)));
        ui.add(egui::Slider::new(&mut v.e.comp.crossfade, 0.0..=1.0).show_value(false))
            .on_hover_text("Crossfade between layers set to deck A and deck B. In the middle both play. Layers on neither deck are unaffected.");
    });
    ui.add_space(6.0);
    drop_button(v, ui);
}

/// Hold to build, let go to drop.
pub fn drop_button(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let id = ui.make_persistent_id("hold_to_drop");
    let was: bool = ui.data(|d| d.get_temp(id)).unwrap_or(false);
    let label = if was { "BUILDING… let go to drop" } else { "HOLD TO BUILD, RELEASE TO DROP" };
    let b = egui::Button::new(RichText::new(label).font(theme::bold(t.text_size)).color(if was { Color32::BLACK } else { t.text }))
        .fill(if was { t.warn } else { t.raised })
        .min_size(Vec2::new(ui.available_width(), 34.0));
    let r = ui.add(b).on_hover_text("Hold: the lights build up. Let go: the drop lands, for a few bars (Space).");
    let down = r.is_pointer_button_down_on() || (!ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_down(egui::Key::Space)));
    set_drop(&mut v.e.comp.sections, was, down);
    ui.data_mut(|d| d.insert_temp(id, down));
}

/// The change from not holding to holding, and back.
pub fn set_drop(sections: &mut lumen_core::SectionDetector, was: bool, down: bool) {
    if down && !was {
        sections.force(Section::BuildUp, 64);
    } else if !down && was {
        sections.force(Section::Drop, 16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::SectionDetector;

    #[test]
    fn holding_builds_and_letting_go_drops() {
        let mut s = SectionDetector::default();
        set_drop(&mut s, false, true);
        assert_eq!(s.current(), Section::BuildUp);
        set_drop(&mut s, true, true);
        assert_eq!(s.current(), Section::BuildUp);
        set_drop(&mut s, true, false);
        assert_eq!(s.current(), Section::Drop);
        set_drop(&mut s, false, false);
        assert_eq!(s.current(), Section::Drop, "nothing changes while not holding");
    }

    #[test]
    fn scene_cards_have_names_lines_and_colours_for_every_look() {
        let pal = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0]];
        assert_eq!(look_title("groove-meteors"), "Groove meteors");
        for s in Section::ALL {
            assert!(playbook::looks_for(s).count() >= 1);
            for l in playbook::looks_for(s) {
                assert!(!look_title(l.id).is_empty());
                assert!(look_line(l) != "Dark" || l.id.contains("dark"), "{} has effects", l.id);
                let _ = look_color(l, &pal);
            }
        }
        let l = playbook::find("groove-meteors").unwrap();
        assert_eq!(look_line(l), "Solid + Meteor + Kick burst");
        assert_eq!(look_color(l, &pal), Color32::from_rgb(255, 0, 0), "the motion effect paints slot 0");
    }
}
