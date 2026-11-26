//! The strip across the top of Play: the last bars of the music as a
//! blue / orange / white waveform (bass, mids, highs), with the beat and bar
//! lines, the song section underneath, and the tempo and section as big
//! numbers you can read from across a booth.

use crate::{stage, theme, View};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use lumen_core::{AudioBand, Section};
use std::collections::VecDeque;

/// One look at the music.
#[derive(Clone, Copy)]
struct Sample {
    t: f64,
    low: f32,
    mid: f32,
    high: f32,
    section: Section,
    beat: f64,
}

#[derive(Default)]
pub struct History {
    samples: VecDeque<Sample>,
}

/// Seconds of music kept.
const KEEP: f64 = 60.0;
/// Bars shown across the strip.
const BARS: f64 = 16.0;

impl History {
    fn push(&mut self, s: Sample) {
        // A frame or two a tick is plenty; skip repeats from busy repaints.
        if self.samples.back().is_some_and(|l| s.t - l.t < 0.02) {
            return;
        }
        self.samples.push_back(s);
        while self.samples.front().is_some_and(|f| s.t - f.t > KEEP) {
            self.samples.pop_front();
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.samples.len()
    }
}

const BLUE: Color32 = Color32::from_rgb(40, 90, 255);
const ORANGE: Color32 = Color32::from_rgb(255, 150, 40);
const WHITE: Color32 = Color32::from_rgb(245, 240, 230);

/// Draw the strip at the top of the Play screen.
pub fn strip(v: &mut View, ui: &mut egui::Ui) {
    let t = theme::current();
    let now = ui.input(|i| i.time);
    let bands = v.e.bands;
    let sample = Sample {
        t: now,
        low: bands.get(AudioBand::Sub).max(bands.get(AudioBand::Bass)),
        mid: bands.get(AudioBand::LowMid).max(bands.get(AudioBand::HighMid)),
        high: bands.get(AudioBand::High),
        section: v.e.comp.sections.current(),
        beat: v.e.comp.beat,
    };
    v.s.wave.push(sample);

    let bpm = v.e.comp.bpm.max(20.0);
    let span = (BARS * 4.0 * 60.0 / bpm).min(KEEP - 1.0);
    let height = 118.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, t.radius + 2, t.panel);

    // The picture area leaves room for the section line at the bottom.
    let area = Rect::from_min_max(rect.min + Vec2::new(0.0, 4.0), rect.max - Vec2::new(0.0, 12.0));
    let mid_y = area.center().y;
    let half = area.height() / 2.0;
    let x_of = |ts: f64| area.right() - ((now - ts) / span) as f32 * area.width();
    let step = (area.width() / (span / 0.033) as f32).max(1.5);

    let samples = &v.s.wave.samples;
    let live = samples.back().is_some_and(|s| s.low + s.mid + s.high > 0.0) || !matches!(v.e.audio, crate::engine::AudioSource::Off);
    let mut prev_beat: Option<f64> = None;
    for s in samples {
        let x = x_of(s.t);
        if x < area.left() - 2.0 {
            continue;
        }
        // Section colour along the bottom.
        let c = stage::section_color(s.section);
        p.rect_filled(Rect::from_min_max(Pos2::new(x - step, rect.bottom() - 8.0), Pos2::new(x + 0.5, rect.bottom() - 3.0)), 0.0, c);
        // Beat and bar lines.
        if let Some(pb) = prev_beat {
            if s.beat.floor() != pb.floor() && s.beat > pb {
                let bar = (s.beat.floor() as i64).rem_euclid(4) == 0;
                let stroke = if bar { Stroke::new(1.5, t.accent.gamma_multiply(0.9)) } else { Stroke::new(1.0, t.line) };
                p.line_segment([Pos2::new(x, area.top()), Pos2::new(x, area.bottom())], stroke);
            }
        }
        prev_beat = Some(s.beat);
        // Bass behind, mids over it, highs on top, mirrored around the middle.
        for (val, col, k) in [(s.low, BLUE, 1.0f32), (s.mid, ORANGE, 0.7), (s.high, WHITE, 0.4)] {
            let h = (val.clamp(0.0, 1.0) * half * k).max(0.0);
            if h > 0.3 {
                p.rect_filled(Rect::from_min_max(Pos2::new(x - step * 0.5, mid_y - h), Pos2::new(x + step * 0.5, mid_y + h)), 0.0, col);
            }
        }
    }
    if !live {
        p.line_segment([Pos2::new(area.left(), mid_y), Pos2::new(area.right(), mid_y)], Stroke::new(1.0, t.line));
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "Pick an audio source to see the music here", egui::FontId::proportional(t.text_size - 1.0), t.weak);
    }
    // Now.
    p.line_segment([Pos2::new(area.right() - 1.0, area.top()), Pos2::new(area.right() - 1.0, rect.bottom() - 3.0)], Stroke::new(2.0, Color32::WHITE));

    // Big readouts over the left and right ends.
    let synced = v.e.link_status.as_ref().is_some_and(|s| s.peers > 0);
    let big = theme::bold(34.0);
    let small = egui::FontId::proportional(t.text_size - 3.0);
    let back = t.panel.gamma_multiply(0.82);
    let bpm_box = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(128.0, 58.0));
    p.rect_filled(bpm_box, t.radius, back);
    p.text(bpm_box.min + Vec2::new(10.0, 4.0), egui::Align2::LEFT_TOP, format!("{:.1}", v.e.comp.bpm), big.clone(), t.text);
    p.text(bpm_box.min + Vec2::new(10.0, 42.0), egui::Align2::LEFT_TOP, if synced { "BPM · LINK" } else { "BPM" }, small.clone(), if synced { t.ok } else { t.weak });

    let sec = v.e.comp.sections.current();
    let listening = v.e.comp.sections.listening() && !matches!(v.e.audio, crate::engine::AudioSource::Off);
    let sec_box = Rect::from_min_size(Pos2::new(rect.right() - 208.0, rect.top() + 8.0), Vec2::new(200.0, 58.0));
    p.rect_filled(sec_box, t.radius, back);
    let (name, col) = if listening { (sec.name().to_uppercase(), stage::section_color(sec)) } else { ("LISTENING".to_string(), t.weak) };
    p.text(sec_box.min + Vec2::new(10.0, 4.0), egui::Align2::LEFT_TOP, name, big, col);
    let sub = if listening { format!("{} beats · {:.0}% sure", v.e.comp.sections.beats_in, v.e.comp.sections.reading.confidence * 100.0) } else { "waiting for music".to_string() };
    p.text(sec_box.min + Vec2::new(10.0, 42.0), egui::Align2::LEFT_TOP, sub, small, t.weak);
    resp.on_hover_text(format!("The last {BARS:.0} bars. Blue is bass, orange the mids, white the highs; the line underneath is the song section Kickline heard."));
    ui.add_space(8.0);
}

/// Section names for a legend, if a caller wants one.
#[allow(dead_code)]
pub fn legend(ui: &mut egui::Ui) {
    let t = theme::current();
    ui.horizontal(|ui| {
        for s in Section::ALL {
            ui.label(RichText::new("●").color(stage::section_color(s)));
            ui.label(RichText::new(s.name()).color(t.weak).size(t.text_size - 3.0));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: f64) -> Sample {
        Sample { t, low: 0.5, mid: 0.3, high: 0.1, section: Section::Groove, beat: t * 2.0 }
    }

    #[test]
    fn keeps_a_minute_and_skips_repeats() {
        let mut h = History::default();
        for i in 0..4000 {
            h.push(sample(i as f64 * 0.033));
        }
        // 132 s of samples: only the last 60 s are kept.
        assert!(h.len() > 1500 && h.len() < 2000, "{}", h.len());
        let before = h.len();
        let last = h.samples.back().unwrap().t;
        h.push(sample(last + 0.001));
        assert_eq!(h.len(), before, "a repaint right after another must not add a sample");
    }
}
