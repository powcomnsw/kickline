//! The show file and its history: undo/redo, autosave, New / Open / Save as.
//!
//! Undo works on snapshots. A few times a second, when the mouse isn't held
//! down and no text box is being typed in, the show is compared with the last
//! snapshot; if it changed, the old snapshot goes on the undo stack. So a drag
//! or a typed name is one step, however many frames it took.
//!
//! What's live is left out of the comparison and kept when undoing: which
//! clips are playing, the current palette, master and tempo. Launching clips
//! during a set never fills the undo list, and undo never stops the music.

use crate::{log, paths, View};
use eframe::egui;
use lumen_core::Composition;
use lumen_fixtures::AdvancedOutput;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    pub composition: Composition,
    pub output: AdvancedOutput,
}

const MAX_UNDO: usize = 200;
const CHECK_EVERY: Duration = Duration::from_millis(250);
const AUTOSAVE_AFTER: Duration = Duration::from_secs(2);

/// The show as undo sees it: everything saved, minus what's live.
pub fn key(p: &Project) -> String {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    if let Some(c) = v.get_mut("composition").and_then(Value::as_object_mut) {
        c.remove("master");
        c.remove("bpm");
        if let Some(p) = c.get_mut("palettes").and_then(Value::as_object_mut) {
            p.remove("active");
        }
        if let Some(ls) = c.get_mut("layers").and_then(Value::as_array_mut) {
            for l in ls.iter_mut().filter_map(Value::as_object_mut) {
                l.remove("active");
            }
        }
    }
    v.to_string()
}

pub struct Session {
    /// Where autosave writes. Starts as the default project file.
    pub path: PathBuf,
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// The last committed state, and its key.
    committed: Option<(Project, String)>,
    last_check: Option<Instant>,
    /// Unsaved changes since this moment.
    dirty_since: Option<Instant>,
    pub last_saved: Option<Instant>,
    pub save_error: Option<String>,
    /// True when no project existed at launch (first run): show the guide.
    pub first_run: bool,
}

impl Session {
    pub fn new(path: PathBuf, first_run: bool) -> Self {
        Session { path, undo: Vec::new(), redo: Vec::new(), committed: None, last_check: None, dirty_since: None, last_saved: None, save_error: None, first_run }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub fn name(&self) -> String {
        match self.path.file_stem().map(|s| s.to_string_lossy().to_string()) {
            Some(s) if s == "kickline_project" => "My show".into(),
            Some(s) => s.replace('_', " "),
            None => "Untitled".into(),
        }
    }

    /// Forget history (after New / Open).
    fn reset(&mut self, now: &Project) {
        self.undo.clear();
        self.redo.clear();
        self.committed = Some((now.clone(), key(now)));
        self.dirty_since = None;
    }

    /// Record a change if the show differs from the last snapshot.
    /// Returns true if something was committed.
    fn commit(&mut self, now: Project) -> bool {
        let k = key(&now);
        match &self.committed {
            None => {
                self.committed = Some((now, k));
                false
            }
            Some((_, old)) if *old == k => false,
            Some(_) => {
                let (prev, _) = self.committed.replace((now, k)).expect("checked above");
                self.undo.push(prev);
                if self.undo.len() > MAX_UNDO {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.dirty_since.get_or_insert_with(Instant::now);
                true
            }
        }
    }
}

fn snapshot(v: &View) -> Project {
    Project { composition: v.e.comp.clone(), output: v.e.output.clone() }
}

fn apply(v: &mut View, p: Project) {
    v.e.comp.replace_show(p.composition);
    v.e.output = p.output;
    // Selections may point past the end now.
    let n = v.e.output.fixtures.len();
    if v.s.ao.selected.is_some_and(|i| i >= n) {
        v.s.ao.selected = None;
    }
    let lv = v.e.output.lumiverses.len();
    if v.s.ao.lumiverse >= lv {
        v.s.ao.lumiverse = lv.saturating_sub(1);
    }
}

/// Every frame: keyboard shortcuts, change tracking, autosave.
pub fn tick(v: &mut View, ctx: &egui::Context) {
    let typing = ctx.egui_wants_keyboard_input();
    if !typing {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        // Check the longer shortcut first: Cmd+Shift+Z also matches Cmd+Z.
        if ctx.input_mut(|i| i.consume_shortcut(&redo) || i.consume_shortcut(&redo_y)) {
            redo_step(v);
        } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
            undo_step(v);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save)) {
            save_now(v);
        }
    }
    let busy = ctx.input(|i| i.pointer.any_down()) || typing;
    let due = v.s.session.last_check.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
    if !busy && due {
        v.s.session.last_check = Some(Instant::now());
        let now = snapshot(v);
        v.s.session.commit(now);
    }
    if v.s.session.dirty_since.is_some_and(|t| t.elapsed() >= AUTOSAVE_AFTER) && !busy {
        save_now(v);
    }
}

pub fn undo_step(v: &mut View) {
    // Catch a change that hasn't been committed yet, so it can be undone too.
    let now = snapshot(v);
    v.s.session.commit(now);
    let Some(prev) = v.s.session.undo.pop() else { return };
    if let Some((cur, _)) = v.s.session.committed.take() {
        v.s.session.redo.push(cur);
    }
    v.s.session.committed = Some((prev.clone(), key(&prev)));
    apply(v, prev);
    v.s.session.dirty_since.get_or_insert_with(Instant::now);
    v.e.status = "Undone".into();
}

pub fn redo_step(v: &mut View) {
    let Some(next) = v.s.session.redo.pop() else { return };
    if let Some((cur, _)) = v.s.session.committed.take() {
        v.s.session.undo.push(cur);
    }
    v.s.session.committed = Some((next.clone(), key(&next)));
    apply(v, next);
    v.s.session.dirty_since.get_or_insert_with(Instant::now);
    v.e.status = "Redone".into();
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.saving");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

fn write(path: &Path, p: &Project) -> Result<(), String> {
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    write_atomic(path, &text).map_err(|e| e.to_string())
}

pub fn read(path: &Path) -> Result<Project, String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str::<Project>(&s).map_err(|e| e.to_string())
}

/// Save to the current file now.
pub fn save_now(v: &mut View) {
    let p = snapshot(v);
    let path = v.s.session.path.clone();
    match write(&path, &p) {
        Ok(()) => {
            v.s.session.dirty_since = None;
            v.s.session.last_saved = Some(Instant::now());
            v.s.session.save_error = None;
            paths::set_last_project(&path);
        }
        Err(e) => {
            if v.s.session.save_error.as_deref() != Some(e.as_str()) {
                log::error("project", format!("save to {} failed: {e}", path.display()));
            }
            v.s.session.save_error = Some(e);
            // Try again later rather than every frame.
            v.s.session.dirty_since = Some(Instant::now());
        }
    }
}

fn switch_to(v: &mut View, path: PathBuf, p: Project) {
    v.e.comp = p.composition;
    v.e.output = p.output;
    v.s.ao = Default::default();
    v.s.fe = Default::default();
    v.s.session.path = path;
    let now = snapshot(v);
    v.s.session.reset(&now);
    paths::set_last_project(&v.s.session.path);
}

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Kickline show", &["json"]).set_directory(paths::projects_dir())
}

/// Pick a name for a new show file that doesn't exist yet.
fn fresh_path(stem: &str) -> PathBuf {
    let dir = paths::projects_dir();
    let mut p = dir.join(format!("{stem}.json"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem} {n}.json"));
        n += 1;
    }
    p
}

/// New show: demo clips, an empty patch, saved under a new name.
pub fn new_show(v: &mut View) {
    save_now(v);
    // Keep the demo's fixture types so "Add" works, but start with no fixtures.
    let mut output = AdvancedOutput::demo();
    output.fixtures.clear();
    output.lumiverses = vec![lumen_fixtures::Lumiverse::new("Lumiverse 1", 0)];
    let p = Project { composition: Composition::demo(), output };
    let path = fresh_path("Untitled show");
    switch_to(v, path, p);
    save_now(v);
    v.e.status = format!("New show: {}", v.s.session.name());
    log::info("project", v.e.status.clone());
}

pub fn open_dialog(v: &mut View) {
    let Some(path) = dialog().set_title("Open a show").pick_file() else { return };
    open_path(v, path);
}

pub fn open_path(v: &mut View, path: PathBuf) {
    save_now(v);
    match read(&path) {
        Ok(p) => {
            switch_to(v, path, p);
            v.e.status = format!("Opened {}", v.s.session.name());
            log::info("project", format!("opened {}", v.s.session.path.display()));
        }
        Err(e) => {
            v.e.status = format!("Couldn't open {}: {e}", path.display());
            log::error("project", v.e.status.clone());
        }
    }
}

/// Save a copy under a new name and carry on working in the copy.
pub fn save_as_dialog(v: &mut View) {
    let name = format!("{}.json", v.s.session.name());
    let Some(mut path) = dialog().set_title("Save show as").set_file_name(name).save_file() else { return };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    v.s.session.path = path;
    save_now(v);
    v.e.status = format!("Saved as {}", v.s.session.name());
}

/// What to open at launch: the last show, else the default file, else the demo.
pub fn startup() -> (PathBuf, Project, bool) {
    let mut candidates = Vec::new();
    if let Some(p) = paths::last_project() {
        candidates.push(p);
    }
    candidates.push(paths::project_file());
    for path in candidates {
        if path.exists() {
            match read(&path) {
                Ok(p) => {
                    log::info("project", format!("opened {}", path.display()));
                    return (path, p, false);
                }
                Err(e) => log::error("project", format!("couldn't open {}: {e}", path.display())),
            }
        }
    }
    (paths::project_file(), Project { composition: Composition::demo(), output: AdvancedOutput::demo() }, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        Project { composition: Composition::demo(), output: AdvancedOutput::demo() }
    }

    #[test]
    fn live_changes_are_not_undo_steps() {
        let mut s = Session::new(PathBuf::from("x.json"), false);
        let mut p = demo();
        s.commit(p.clone());
        p.composition.layers[0].active = Some(2);
        p.composition.master = 0.3;
        p.composition.bpm = 140.0;
        p.composition.palettes.select(3, 0.0);
        assert!(!s.commit(p.clone()));
        p.composition.layers[0].name = "Edited".into();
        assert!(s.commit(p.clone()));
        assert!(s.can_undo() && s.dirty());
    }

    #[test]
    fn undo_redo_order() {
        let mut s = Session::new(PathBuf::from("x.json"), false);
        let mut p = demo();
        s.commit(p.clone());
        for n in ["A", "B", "C"] {
            p.composition.layers[0].name = n.into();
            s.commit(p.clone());
        }
        assert_eq!(s.undo.len(), 3);
        // Undo twice by hand (what undo_step does without a View).
        for _ in 0..2 {
            let prev = s.undo.pop().unwrap();
            let (cur, _) = s.committed.take().unwrap();
            s.redo.push(cur);
            s.committed = Some((prev.clone(), key(&prev)));
        }
        assert_eq!(s.committed.as_ref().unwrap().0.composition.layers[0].name, "A");
        // A new edit clears redo.
        let mut q = s.committed.as_ref().unwrap().0.clone();
        q.composition.layers[0].name = "Z".into();
        s.commit(q);
        assert!(!s.can_redo());
    }

    #[test]
    fn save_and_read_back() {
        let dir = std::env::temp_dir().join(format!("kickline-test-{}", std::process::id()));
        let path = dir.join("show.json");
        let p = demo();
        write(&path, &p).unwrap();
        let q = read(&path).unwrap();
        assert_eq!(key(&p), key(&q));
        assert!(!path.with_extension("json.saving").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
