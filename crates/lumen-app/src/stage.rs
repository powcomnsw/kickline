//! Set up → Stage: groups of fixtures ("slices"), the looks for each part
//! of a song, and stage profiles (layout + groups + looks, saved to a file
//! and reused in other shows). Also the live song-section card.

use crate::{project, theme, util, View};
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Vec2};
use lumen_core::{Clip, Composition, Generator, LayerLook, Looks, Paint, Palettes, Section, StripClip};
use lumen_fixtures::{AdvancedOutput, Fixture, FixtureType, Lumiverse, WhiteMode};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct State {
    pub group: Option<String>,
    pub new_group: String,
    pub renaming: Option<(String, String)>,
    pub message: Option<String>,
    /// A guess at the roles, waiting for the user to accept or dismiss it.
    pub suggested: Option<lumen_core::RoleMap>,
}

/// A stage profile: the rig and how it looks in each part of a song.
#[derive(Clone, Serialize, Deserialize)]
pub struct StageProfile {
    pub kind: String,
    pub output: AdvancedOutput,
    pub palettes: Palettes,
    pub looks: Looks,
}

const PROFILE_KIND: &str = "kickline-stage-profile";

pub fn group_color(i: usize) -> Color32 {
    const C: [u32; 8] = [0x4FA3F7, 0xF0A030, 0x7CC47A, 0xE0607E, 0xB48CF0, 0x40C8C0, 0xF0D050, 0xA0A0A0];
    let h = C[i % C.len()];
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

pub fn section_color(s: Section) -> Color32 {
    let [r, g, b] = s.color();
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Draw what every fixture is being sent, pixel by pixel. Returns the
/// canvas rect.
pub fn paint_lights(ui: &mut egui::Ui, v: &View, size: Vec2, outline: &dyn Fn(usize) -> Option<Color32>) -> egui::Response {
    let t = theme::current();
    let aspect = v.e.output.aspect.max(0.1);
    let w = size.x.min(size.y * aspect);
    let (resp, painter) = ui.allocate_painter(Vec2::new(w, w / aspect), Sense::click());
    let r = resp.rect;
    painter.rect_filled(r, t.radius, Color32::from_rgb(8, 7, 6));
    let dot = (r.width() / 300.0).clamp(1.3, 3.2);
    for (fi, fx) in v.e.output.fixtures.iter().enumerate() {
        let Some(ty) = v.e.output.types.get(fx.type_id) else { continue };
        let lit = v.e.lights.get(fi);
        for (i, (u, vv)) in fx.pixel_positions(aspect, ty).into_iter().enumerate() {
            let c = lit.and_then(|l| l.get(i)).copied().unwrap_or([0, 0, 0]);
            let col = Color32::from_rgb(c[0], c[1], c[2]);
            let p = util::to_screen(r, u, vv);
            if c != [0, 0, 0] {
                painter.circle_filled(p, dot * 1.9, col.gamma_multiply(0.25));
            }
            painter.circle_filled(p, dot, if c == [0, 0, 0] { Color32::from_gray(28) } else { col });
        }
        if let Some(c) = outline(fi) {
            let pts: Vec<_> = fx.corners(aspect).iter().map(|&(u, vv)| util::to_screen(r, u, vv)).collect();
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, c)));
        }
    }
    resp
}

/// The live section, how sure Kickline is, and the auto-looks switch.
pub fn section_card(v: &mut View, ui: &mut egui::Ui, wide: bool) {
    let t = theme::current();
    let cur = v.e.comp.sections.current();
    let listening = v.e.comp.sections.listening() && !matches!(v.e.audio, crate::engine::AudioSource::Off);
    let (reading, beats_in) = (v.e.comp.sections.reading, v.e.comp.sections.beats_in);
    ui.horizontal(|ui| {
        for s in Section::ALL {
            let on = listening && s == cur;
            let fill = if on { section_color(s) } else { t.raised };
            let text = RichText::new(s.name()).size(t.text_size - 2.0).color(if on { Color32::BLACK } else { t.weak });
            let b = egui::Button::new(text).fill(fill).corner_radius(CornerRadius::same(t.radius)).min_size(Vec2::new(if wide { 78.0 } else { 0.0 }, 22.0));
            if ui.add(b).on_hover_text(format!("{}\nClick to switch to this look now.", s.note())).clicked() {
                v.e.comp.apply_look(s);
            }
        }
    });
    if !listening {
        ui.label(RichText::new("Waiting for music…").color(t.weak).size(t.text_size - 2.0));
    } else {
        let r = reading;
        ui.label(
            RichText::new(format!("{} for {} beats · {} kicks/bar · bass {:.0}% · level {:.0}%", cur.name(), beats_in, r.kicks, r.bass * 100.0, r.level * 100.0))
                .color(t.weak)
                .size(t.text_size - 2.0),
        )
        .on_hover_text("Kickline listens to the music: kicks in the last bar, bass and loudness compared with the loudest part so far, and whether the highs are rising. It can't see the track ahead, so it reacts within about a bar; the pause and the drop are caught on the beat.");
    }
    ui.horizontal(|ui| {
        let auto = &mut v.e.comp.looks.auto;
        let label = if *auto { "Auto looks ON" } else { "Auto looks off" };
        let b = egui::Button::new(RichText::new(label).font(theme::bold(t.text_size - 1.0)).color(if *auto { t.on_accent } else { t.weak }))
            .fill(if *auto { t.accent } else { t.raised });
        if ui.add(b).on_hover_text("Switch each layer's clip, the colours and the brightness when the song moves to a new part. Set what happens in each part in each layer's settings and in Set up → Stage.").clicked() {
            *auto = !*auto;
            if *auto {
                let s = v.e.comp.sections.current();
                v.e.comp.apply_look(s);
            }
        }
    });
}

/// The song so far: one coloured block per section, as wide as it was long.
pub fn timeline(v: &View, ui: &mut egui::Ui) {
    let t = theme::current();
    let d = &v.e.comp.sections;
    let mut parts: Vec<(Section, u32)> = d.past.iter().copied().filter(|p| p.1 > 0).collect();
    parts.push((d.current(), d.beats_in.max(1)));
    let total: u32 = parts.iter().map(|p| p.1).sum::<u32>().max(1);
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, t.radius, t.raised);
    let mut x = r.min.x;
    for (i, (s, n)) in parts.iter().enumerate() {
        let w = r.width() * *n as f32 / total as f32;
        let block = egui::Rect::from_min_size(egui::pos2(x, r.min.y), Vec2::new(w, r.height())).shrink2(Vec2::new(0.5, 0.0));
        let last = i + 1 == parts.len();
        p.rect_filled(block, 2.0, section_color(*s).gamma_multiply(if last { 1.0 } else { 0.6 }));
        if w > 46.0 {
            p.text(block.left_center() + Vec2::new(5.0, 0.0), egui::Align2::LEFT_CENTER, s.name(), egui::FontId::proportional(t.text_size - 4.0), Color32::BLACK);
        }
        x += w;
    }
}

/// Top of Set up → Music: what Kickline hears in the song right now.
pub fn music_sections(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    egui::Frame::new().fill(t.panel).corner_radius(t.radius + 2).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Song sections").font(theme::bold(t.text_size + 1.0)));
        ui.label(RichText::new("Kickline listens for the groove, melodic parts, build-ups, the pause before the drop and the drop. Click one to try its look.").color(t.weak).size(t.text_size - 2.0));
        ui.add_space(4.0);
        section_card(v, ui, true);
        ui.add_space(4.0);
        timeline(v, ui);
    });
    ui.add_space(8.0);
}

pub fn ui(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let groups = v.e.output.group_names();
    if v.s.stage.group.as_ref().is_some_and(|g| !groups.contains(g)) {
        v.s.stage.group = None;
    }
    if v.s.stage.group.is_none() {
        v.s.stage.group = groups.first().cloned();
    }
    egui::Panel::left("stage_groups").default_size(250.0).resizable(false).show(ui, |ui| groups_list(v, ui, &groups));
    egui::Panel::right("stage_looks").default_size(360.0).resizable(false).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| looks_panel(v, ui));
    });
    egui::CentralPanel::default().show(ui, |ui| {
        let sel = v.s.stage.group.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Stage").font(theme::bold(t.text_size + 4.0)));
            match &sel {
                Some(g) => ui.label(RichText::new(format!("Click fixtures to add them to or take them out of \"{g}\"")).color(t.weak)),
                None => ui.label(RichText::new("Make a group on the left, then click fixtures to put them in it").color(t.weak)),
            };
        });
        ui.add_space(6.0);
        let gi = sel.as_ref().and_then(|g| groups.iter().position(|x| x == g));
        let fixtures = v.e.output.fixtures.clone();
        let resp = paint_lights(ui, v, ui.available_size() - Vec2::new(0.0, 60.0), &|fi| {
            let fx = &fixtures[fi];
            match (&sel, gi) {
                (Some(g), Some(i)) if fx.groups.contains(g) => Some(group_color(i)),
                _ => Some(Color32::from_white_alpha(40)),
            }
        });
        if resp.clicked() {
            if let (Some(pos), Some(g)) = (resp.interact_pointer_pos(), &sel) {
                let (u, vv) = util::to_canvas(resp.rect, pos);
                let aspect = v.e.output.aspect;
                if let Some(fi) = v.e.output.fixtures.iter().rposition(|f| f.contains(aspect, u, vv)) {
                    let gs = &mut v.e.output.fixtures[fi].groups;
                    if let Some(k) = gs.iter().position(|x| x == g) {
                        gs.remove(k);
                    } else {
                        gs.push(g.clone());
                    }
                }
            }
        }
        // Legend: every group's colour and size.
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (i, g) in groups.iter().enumerate() {
                let n = v.e.output.group_members(g).len();
                ui.label(RichText::new("■").color(group_color(i)));
                ui.label(RichText::new(format!("{g} ({n})")).color(t.weak));
                ui.add_space(8.0);
            }
        });
    });
}

fn groups_list(v: &mut View, ui: &mut egui::Ui, groups: &[String]) {
    let t = theme::current();
    ui.add_space(8.0);
    ui.label(RichText::new("Groups").font(theme::bold(t.text_size + 1.0)));
    ui.label(RichText::new("Slices of your rig. Aim a layer at a group and it only lights those fixtures, with its effect spread across them.").color(t.weak).size(t.text_size - 2.0));
    ui.add_space(6.0);
    let mut remove = None;
    let mut rename_done = None;
    for (i, g) in groups.iter().enumerate() {
        let n = v.e.output.group_members(g).len();
        let selected = v.s.stage.group.as_ref() == Some(g);
        ui.horizontal(|ui| {
            ui.label(RichText::new("■").color(group_color(i)));
            if let Some((from, to)) = &mut v.s.stage.renaming {
                if from == g {
                    let r = ui.add(egui::TextEdit::singleline(to).desired_width(130.0));
                    if r.lost_focus() {
                        rename_done = Some((from.clone(), to.clone()));
                    }
                    r.request_focus();
                    return;
                }
            }
            if ui.selectable_label(selected, format!("{g}  ·  {n}")).clicked() {
                v.s.stage.group = Some(g.clone());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Delete group (the fixtures stay)").clicked() {
                    remove = Some(g.clone());
                }
                if ui.small_button("✎").on_hover_text("Rename").clicked() {
                    v.s.stage.renaming = Some((g.clone(), g.clone()));
                }
            });
        });
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            ui.label(RichText::new("Role").color(t.weak).size(t.text_size - 2.0));
            role_picker(&mut v.e.output, ui, g);
        });
    }
    if let Some((from, to)) = rename_done {
        let to = to.trim().to_string();
        if !to.is_empty() && to != from {
            v.e.output.rename_group(&from, &to);
            for l in &mut v.e.comp.layers {
                if l.target.as_deref() == Some(from.as_str()) {
                    l.target = Some(to.clone());
                }
            }
            v.s.stage.group = Some(to);
        }
        v.s.stage.renaming = None;
    }
    if let Some(g) = remove {
        v.e.output.remove_group(&g);
        for l in &mut v.e.comp.layers {
            if l.target.as_deref() == Some(g.as_str()) {
                l.target = None;
            }
        }
    }
    ui.add_space(6.0);
    roles_box(v, ui, groups);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut v.s.stage.new_group).hint_text("New group").desired_width(140.0));
        let name = v.s.stage.new_group.trim().to_string();
        if ui.add_enabled(!name.is_empty() && !groups.contains(&name), egui::Button::new("+ Add")).clicked() {
            v.e.output.groups.push(name.clone());
            v.s.stage.group = Some(name);
            v.s.stage.new_group.clear();
        }
    });
    if let Some(g) = v.s.stage.group.clone() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.small_button("Add every fixture").clicked() {
                for f in &mut v.e.output.fixtures {
                    if !f.groups.contains(&g) {
                        f.groups.push(g.clone());
                    }
                }
            }
            if ui.small_button("Empty it").clicked() {
                for f in &mut v.e.output.fixtures {
                    f.groups.retain(|x| x != &g);
                }
            }
        });
    }
}

/// What a group is for, so Auto's looks know where to put what.
fn role_picker(o: &mut AdvancedOutput, ui: &mut egui::Ui, g: &str) {
    let cur = o.roles.get(g);
    let mut pick = cur;
    egui::ComboBox::from_id_salt(("role", g))
        .selected_text(cur.map(|r| r.name()).unwrap_or("No role"))
        .width(86.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut pick, None, "No role");
            for r in lumen_core::StageRole::ALL {
                ui.selectable_value(&mut pick, Some(r), r.name()).on_hover_text(r.note());
            }
        });
    if pick != cur {
        o.roles.set(g, pick);
    }
}

/// Suggest roles from the layout (never applied until accepted), and the
/// order and direction of a Path.
fn roles_box(v: &mut View, ui: &mut egui::Ui, groups: &[String]) {
    let t = theme::current();
    ui.label(RichText::new("Roles tell Auto what each group is for: Eyes get the accents, a Path carries the chases, Left and Right answer each other. With no roles every group plays everything.").color(t.weak).size(t.text_size - 2.0));
    ui.horizontal(|ui| {
        if ui.button("Suggest roles").on_hover_text("A guess from where things are. You choose whether to use it.").clicked() {
            let s = v.e.output.suggest_roles();
            v.s.stage.message = Some(if s.is_empty() { "Nothing to suggest: add some groups first.".into() } else { String::new() });
            v.s.stage.suggested = (!s.is_empty()).then_some(s);
        }
    });
    if let Some(s) = v.s.stage.suggested.clone() {
        ui.add_space(4.0);
        for (g, r) in &s.0 {
            ui.label(RichText::new(format!("{g} → {}", r.name())).color(t.weak));
        }
        ui.horizontal(|ui| {
            if ui.button("Use these").clicked() {
                for g in groups {
                    v.e.output.roles.set(g, s.get(g));
                }
                v.s.stage.suggested = None;
            }
            if ui.button("Not now").clicked() {
                v.s.stage.suggested = None;
            }
        });
    }
    // The selected group's path order, when it is a Path.
    if let Some(g) = v.s.stage.group.clone().filter(|g| v.e.output.roles.get(g) == Some(lumen_core::StageRole::Path)) {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{g}: order along the path")).font(theme::bold(t.text_size)));
        let m = v.e.output.group_members(&g);
        let mut swap = None;
        for (k, &fi) in m.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}. {}", k + 1, v.e.output.fixtures[fi].name));
                if ui.add_enabled(k > 0, egui::Button::new("▲").small()).clicked() {
                    swap = Some((k, k - 1));
                }
                if ui.add_enabled(k + 1 < m.len(), egui::Button::new("▼").small()).clicked() {
                    swap = Some((k, k + 1));
                }
                ui.checkbox(&mut v.e.output.fixtures[fi].path_reverse, "runs backwards");
            });
        }
        if let Some((a, b)) = swap {
            let mut order = m.clone();
            order.swap(a, b);
            for (k, fi) in order.into_iter().enumerate() {
                v.e.output.fixtures[fi].path_order = k as u16 + 1;
            }
        }
    }
}

fn looks_panel(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    ui.add_space(8.0);
    ui.label(RichText::new("Song sections").font(theme::bold(t.text_size + 1.0)));
    section_card(v, ui, false);
    timeline(v, ui);
    ui.add_space(10.0);
    ui.label(RichText::new("Looks").font(theme::bold(t.text_size + 1.0)));
    ui.label(RichText::new("Colours and brightness for each part of the song. Which clip each layer plays is in that layer's settings (Play → ⋯).").color(t.weak).size(t.text_size - 2.0));
    ui.add_space(4.0);
    let names: Vec<String> = v.e.comp.palettes.list.iter().map(|p| p.name.clone()).collect();
    egui::Grid::new("looks_grid").num_columns(3).spacing([8.0, 8.0]).show(ui, |ui| {
        ui.label("");
        ui.label(RichText::new("Colours").color(t.weak));
        ui.label(RichText::new("Brightness").color(t.weak));
        ui.end_row();
        for s in Section::ALL {
            let i = s.index();
            ui.label(RichText::new(s.name()).color(section_color(s)));
            let looks = &mut v.e.comp.looks;
            let text = looks.palette[i].and_then(|p| names.get(p).cloned()).unwrap_or_else(|| "Keep".into());
            egui::ComboBox::from_id_salt(("look_pal", i)).width(120.0).selected_text(text).show_ui(ui, |ui| {
                ui.selectable_value(&mut looks.palette[i], None, "Keep");
                for (k, n) in names.iter().enumerate() {
                    ui.selectable_value(&mut looks.palette[i], Some(k), n);
                }
            });
            ui.spacing_mut().slider_width = 90.0;
            ui.add(egui::Slider::new(&mut looks.level[i], 0.0..=1.0).custom_formatter(|x, _| format!("{:.0}%", x * 100.0)));
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    // What each layer does, at a glance.
    egui::CollapsingHeader::new(RichText::new("Layers in each section").color(t.weak)).default_open(true).show(ui, |ui| {
        egui::Grid::new("looks_layers").num_columns(6).spacing([6.0, 4.0]).striped(true).show(ui, |ui| {
            ui.label("");
            for s in Section::ALL {
                ui.label(RichText::new(&s.name()[..s.name().len().min(5)]).color(section_color(s)).size(t.text_size - 3.0));
            }
            ui.end_row();
            for l in &v.e.comp.layers {
                ui.label(RichText::new(&l.name).size(t.text_size - 2.0));
                for s in Section::ALL {
                    let txt = match l.looks[s.index()] {
                        LayerLook::Keep => "·".to_string(),
                        LayerLook::Stop => "off".to_string(),
                        LayerLook::Play(c) => format!("{}", c + 1),
                    };
                    ui.label(RichText::new(txt).color(t.weak).size(t.text_size - 2.0));
                }
                ui.end_row();
            }
        });
    });

    ui.add_space(12.0);
    ui.separator();
    ui.label(RichText::new("Stage profile").font(theme::bold(t.text_size + 1.0)));
    ui.label(RichText::new("Your rig (outputs, fixtures, groups, roles and path order) with its colours and looks, in one file. Load it into any show.").color(t.weak).size(t.text_size - 2.0));
    ui.horizontal_wrapped(|ui| {
        if ui.button("Save profile…").clicked() {
            let msg = save_profile(v);
            v.s.stage.message = msg;
        }
        if ui.button("Load profile…").on_hover_text("Replaces this show's rig, colours and looks; your clips stay").clicked() {
            let msg = load_profile(v);
            v.s.stage.message = msg;
        }
    });
    ui.add_space(4.0);
    if ui.button("Start a show from the example stage").on_hover_text("Two LED panels, a centre matrix, a triangle and a square on an H807SA, with groups and a look for every section").clicked() {
        project::new_show_from(v, "DJ stage", example_show());
        v.s.stage.message = Some("Opened a new show from the example stage.".into());
    }
    ui.horizontal(|ui| {
        if ui.button("Face example").on_hover_text("A head outline, two eyes, brows, a nose and a mouth: tests focal points and a closed outline").clicked() {
            project::new_show_from(v, "Face stage", crate::example_stages::face_show());
            v.s.stage.message = Some("Opened a new show from the face example.".into());
        }
        if ui.button("Dragon example").on_hover_text("A long spine wired as one chain, two wings, a head, eyes and a belly").clicked() {
            project::new_show_from(v, "Dragon stage", crate::example_stages::dragon_show());
            v.s.stage.message = Some("Opened a new show from the dragon example.".into());
        }
    });
    if let Some(m) = &v.s.stage.message {
        ui.label(RichText::new(m).color(t.weak).size(t.text_size - 2.0));
    }
}

fn profiles_dir() -> std::path::PathBuf {
    let d = crate::paths::projects_dir().join("Stage profiles");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn save_profile(v: &mut View) -> Option<String> {
    let p = StageProfile { kind: PROFILE_KIND.into(), output: v.e.output.clone(), palettes: v.e.comp.palettes.clone(), looks: v.e.comp.looks.clone() };
    let name = format!("{} stage.json", v.s.session.name());
    let path = rfd::FileDialog::new().set_directory(profiles_dir()).set_title("Save stage profile").set_file_name(name).save_file()?;
    let json = serde_json::to_string_pretty(&p).ok()?;
    Some(match std::fs::write(&path, json) {
        Ok(()) => format!("Saved {}", path.display()),
        Err(e) => format!("Couldn't save: {e}"),
    })
}

fn load_profile(v: &mut View) -> Option<String> {
    let path = rfd::FileDialog::new().set_directory(profiles_dir()).set_title("Load stage profile").add_filter("Stage profile", &["json"]).pick_file()?;
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<StageProfile>(&text) {
        Ok(p) if p.kind == PROFILE_KIND => {
            v.e.output = p.output;
            v.e.comp.palettes = p.palettes;
            v.e.comp.looks = p.looks;
            Some(format!("Loaded {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
        }
        _ => Some("That file isn't a Kickline stage profile.".into()),
    }
}

/// The example stage: left and right LED panels, a centre matrix, a
/// triangle and a square, all on one H807SA, with groups, layers aimed at
/// them and a look for every part of a song.
pub fn example_show() -> project::Project {
    let mut o = AdvancedOutput { aspect: 16.0 / 9.0, ..Default::default() };
    let aspect = o.aspect;
    o.lumiverses.push(Lumiverse::new("H807SA", 0));
    o.types.push(FixtureType::new("LED panel 16x32", 16, 32, "RGB"));
    o.types.push(FixtureType::new("LED matrix 32x16", 32, 16, "RGB"));
    o.types.push(FixtureType::new("Pixel strip 60 RGB", 60, 1, "RGB"));
    o.types.push(FixtureType::new("Pixel strip 30 RGB", 30, 1, "RGB"));
    // H807SA set to 4 universes per output: output N starts on universe 4(N-1).
    let fx = |name: &str, type_id: usize, out: u16, address: u16, x: f32, y: f32, w: f32, h: f32, rot: f32, snake: bool, groups: &[&str]| Fixture {
        name: name.into(),
        type_id,
        lumiverse: 0,
        address,
        x,
        y,
        w,
        h,
        rotation: rot,
        snake,
        reverse: false,
        universe_offset: (out - 1) * 4,
        path_order: 0,
        path_reverse: false,
        white: WhiteMode::default(),
        groups: groups.iter().map(|g| g.to_string()).collect(),
    };
    o.fixtures.push(fx("Left panel", 0, 1, 1, 0.09, 0.5, 0.11, 0.8, 0.0, true, &["Left panel", "Panels"]));
    o.fixtures.push(fx("Right panel", 0, 2, 1, 0.91, 0.5, 0.11, 0.8, 0.0, true, &["Right panel", "Panels"]));
    o.fixtures.push(fx("Centre matrix", 1, 3, 1, 0.5, 0.3, 0.36, 0.36, 0.0, true, &["Centre matrix"]));
    // Edges of a shape, in square (pixel) space, as strips.
    let mut edges = |name: &str, type_id: usize, out: u16, px: u16, cx: f32, cy: f32, r: f32, sides: usize, group: &str| {
        let pts: Vec<(f32, f32)> = (0..sides)
            .map(|k| {
                let a = (-90.0f32 + 360.0 * k as f32 / sides as f32 + if sides == 4 { 45.0 } else { 0.0 }).to_radians();
                (cx * aspect + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        for k in 0..sides {
            let (a, b) = (pts[k], pts[(k + 1) % sides]);
            let (mx, my) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let rot = (b.1 - a.1).atan2(b.0 - a.0).to_degrees();
            let address = 1 + (k as u16) * px * 3;
            o.fixtures.push(fx(&format!("{name} {}", k + 1), type_id, out, address, mx / aspect, my, len / aspect, 0.018, rot, false, &[group, "Shapes"]));
        }
    };
    edges("Triangle", 2, 4, 60, 0.33, 0.73, 0.2, 3, "Triangle");
    edges("Square", 3, 5, 30, 0.67, 0.73, 0.19, 4, "Square");
    o.groups = ["Panels", "Left panel", "Right panel", "Centre matrix", "Shapes", "Triangle", "Square"].iter().map(|s| s.to_string()).collect();
    use lumen_core::StageRole as R;
    for (g, r) in [("Panels", R::Fill), ("Left panel", R::Left), ("Right panel", R::Right), ("Centre matrix", R::Centre), ("Shapes", R::Outline), ("Triangle", R::Outline), ("Square", R::Outline)] {
        o.roles.set(g, Some(r));
    }

    let mut c = Composition::demo();
    c.layers.clear();
    let cols = c.columns;
    let strip = |name: &str| StripClip::presets().into_iter().find(|(n, _)| *n == name).map(|(_, s)| Generator::Strip(s)).unwrap();
    let mut layer = |name: &str, target: Option<&str>, clips: Vec<(&str, Generator)>, looks: [LayerLook; 5]| {
        let mut l = lumen_core::Layer::new(name, cols);
        l.target = target.map(|s| s.to_string());
        for (i, (n, g)) in clips.into_iter().enumerate() {
            l.clips[i] = Some(Clip::new(n, g));
        }
        l.looks = looks;
        c.layers.push(l);
    };
    use LayerLook::{Play, Stop};
    let s = |k: u8| Paint::Slot(k);
    // Order of looks: Groove, Melodic, Build-up, Pause, Drop.
    layer(
        "Shapes",
        Some("Shapes"),
        vec![("Meteor", strip("Meteor")), ("Fill to the bar", strip("Fill to the bar")), ("Hi-hat sparkle", strip("Hi-hat sparkle")), ("Kick burst", strip("Kick burst"))],
        [Play(0), Play(1), Play(2), Stop, Play(3)],
    );
    layer(
        "Centre",
        Some("Centre matrix"),
        vec![
            ("Radar", Generator::Radar { color: s(2), beats_per_turn: 4.0, tail: 0.3 }),
            ("Plasma", Generator::Plasma { scale: 1.0, speed: 0.5 }),
            ("Up ↕ down", Generator::Scanner { color: s(1), beats: 2.0, width: 0.2 }),
            ("Kick flash", Generator::AudioFlash { band: lumen_core::AudioBand::Kick, color: Paint::WHITE }),
        ],
        [Play(0), Play(1), Play(2), Stop, Play(3)],
    );
    layer(
        "Panels",
        Some("Panels"),
        vec![
            ("Left ↔ right", Generator::LeftRight { color: s(0), beats: 4.0, width: 0.3 }),
            ("Twinkle", Generator::Twinkle { color: s(3), density: 0.3, per_beat: 1.0 }),
            ("Sweep in", Generator::Sweep { color: s(1), beats: 1.0, width: 0.2, inward: true }),
            ("Beat pulse", Generator::BeatPulse { color: s(0), per_beat: 1.0, fade: 0.5 }),
            ("Wipe", Generator::Wipe { color: s(2), beats: 4.0 }),
        ],
        [Play(0), Play(1), Play(2), Stop, Play(3)],
    );
    layer(
        "Background",
        None,
        vec![("Palette wash", Generator::PaletteWash { beats_per_cycle: 16.0 }), ("Deep colour", Generator::Solid { color: s(3) })],
        [Play(0), Play(1), Stop, Stop, Play(1)],
    );
    for l in &mut c.layers {
        l.blend = lumen_core::BlendMode::Add;
    }
    if let Some(bg) = c.layers.last_mut() {
        bg.blend = lumen_core::BlendMode::Normal;
        bg.opacity = 0.35;
    }
    c.looks = Looks { auto: true, palette: [Some(0), Some(1), Some(2), None, Some(0)], level: [0.8, 0.6, 0.9, 0.1, 1.0] };
    c.apply_look(Section::Groove);
    project::Project { composition: c, output: o }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_stage_is_addressed_like_an_h807sa() {
        let p = example_show();
        let o = &p.output;
        // Every output's first pixel is channel 1 of universe 4 x (output - 1).
        let starts: Vec<(u16, u16)> = o.fixtures.iter().filter(|f| f.address == 1).map(|f| (f.universe_offset, f.address)).collect();
        assert_eq!(starts, vec![(0, 1), (4, 1), (8, 1), (12, 1), (16, 1)]);
        // Nothing spills into the next output's universes.
        for f in &o.fixtures {
            for s in o.layout(f) {
                assert!(s.universe_offset < f.universe_offset + 4, "{} spills", f.name);
            }
        }
        assert_eq!(o.group_members("Triangle").len(), 3);
        assert_eq!(o.group_members("Square").len(), 4);
        assert_eq!(o.group_members("Panels").len(), 2);
        assert!(p.composition.looks.auto);
        // Every layer is aimed at a group that exists.
        for l in &p.composition.layers {
            if let Some(g) = &l.target {
                assert!(o.group_names().contains(g), "{g}");
            }
        }
    }

    #[test]
    fn profile_roundtrip() {
        let p = example_show();
        let prof = StageProfile { kind: PROFILE_KIND.into(), output: p.output.clone(), palettes: p.composition.palettes.clone(), looks: p.composition.looks.clone() };
        let s = serde_json::to_string(&prof).unwrap();
        let back: StageProfile = serde_json::from_str(&s).unwrap();
        assert_eq!(back.output.fixtures.len(), p.output.fixtures.len());
        assert_eq!(back.looks, p.composition.looks);
    }
}
