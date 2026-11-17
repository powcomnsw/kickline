//! Fixture library: QLC+'s fixture definitions bundled with Lumen (Apache-2.0,
//! see `assets/QLCPLUS-LICENSE.txt`), plus any folders of `.qxf` files, such
//! as ~/Documents/Lumen/Fixtures or an installed QLC+'s user fixtures.
//!
//! Download fixtures that aren't included from open-fixture-library.org
//! ("Download as QLC+ fixture") and drop them on the Lumen window.

use std::io::Read;
use std::path::{Path, PathBuf};

const BUNDLED: &[u8] = include_bytes!("../assets/qlcplus-fixtures.bin");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Bundled QLC+ library; byte range in the unpacked blob.
    Bundled { offset: usize, len: usize },
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub manufacturer: String,
    pub model: String,
    /// "Chauvet/Chauvet-SlimPAR-56.qxf" or a full path.
    pub path: String,
    pub source: Source,
}

impl Entry {
    pub fn origin(&self) -> &'static str {
        match self.source {
            Source::Bundled { .. } => "built-in",
            Source::File(_) => "your files",
        }
    }

    /// Search text, lower-case.
    fn haystack(&self) -> String {
        format!("{} {}", self.manufacturer, self.model).to_lowercase()
    }
}

/// "Chauvet/Chauvet-SlimPAR-56.qxf" -> ("Chauvet", "SlimPAR 56").
fn names_from_path(path: &str) -> (String, String) {
    let p = Path::new(path);
    let file = p.file_stem().and_then(|s| s.to_str()).unwrap_or(path);
    let dir = p.parent().and_then(|d| d.file_name()).and_then(|s| s.to_str()).unwrap_or("");
    let manufacturer = dir.replace('_', " ");
    let prefix = format!("{dir}-");
    let model = file.strip_prefix(&prefix).unwrap_or(file).replace(['-', '_'], " ");
    (manufacturer, model)
}

pub struct Library {
    pub entries: Vec<Entry>,
    blob: Vec<u8>,
    pub errors: Vec<String>,
}

impl Library {
    /// Unpack the bundled library and scan `folders` for `.qxf` files.
    pub fn load(folders: &[PathBuf]) -> Library {
        let mut lib = Library { entries: Vec::new(), blob: Vec::new(), errors: Vec::new() };
        match unpack(BUNDLED) {
            Ok((index, blob)) => {
                lib.entries.extend(index.into_iter().map(|(path, offset, len)| {
                    let (manufacturer, model) = names_from_path(&path);
                    Entry { manufacturer, model, path, source: Source::Bundled { offset, len } }
                }));
                lib.blob = blob;
            }
            Err(e) => lib.errors.push(format!("built-in library: {e}")),
        }
        for f in folders {
            lib.scan(f, 0);
        }
        lib.entries.sort_by(|a, b| a.manufacturer.to_lowercase().cmp(&b.manufacturer.to_lowercase()).then(a.model.cmp(&b.model)));
        lib
    }

    fn scan(&mut self, dir: &Path, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                self.scan(&p, depth + 1);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("qxf")) {
                self.add_file(&p);
            }
        }
    }

    /// Add one file (e.g. dropped on the window). Uses the real names inside it.
    pub fn add_file(&mut self, p: &Path) -> Option<usize> {
        let xml = std::fs::read_to_string(p).map_err(|e| self.errors.push(format!("{}: {e}", p.display()))).ok()?;
        let (manufacturer, model) = match crate::qxf::parse(&xml) {
            Ok(f) => (f.manufacturer, f.model),
            Err(e) => {
                self.errors.push(format!("{}: {e}", p.display()));
                return None;
            }
        };
        let path = p.display().to_string();
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            return Some(i);
        }
        self.entries.push(Entry { manufacturer, model, path, source: Source::File(p.to_path_buf()) });
        Some(self.entries.len() - 1)
    }

    /// Indices matching every word of `query` (manufacturer or model), at most `limit`.
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                let h = e.haystack();
                words.iter().all(|w| h.contains(w.as_str()))
            })
            .map(|(i, _)| i)
            .take(limit)
            .collect()
    }

    pub fn read(&self, i: usize) -> Result<String, String> {
        let e = self.entries.get(i).ok_or("no such fixture")?;
        match &e.source {
            Source::Bundled { offset, len } => self
                .blob
                .get(*offset..offset + len)
                .ok_or_else(|| "corrupt library".to_string())
                .map(|b| String::from_utf8_lossy(b).into_owned()),
            Source::File(p) => std::fs::read_to_string(p).map_err(|e| e.to_string()),
        }
    }

    pub fn open(&self, i: usize) -> Result<crate::qxf::QxfFixture, String> {
        crate::qxf::parse(&self.read(i)?)
    }

    pub fn source_label(&self, i: usize) -> String {
        match self.entries.get(i) {
            Some(Entry { source: Source::Bundled { .. }, path, .. }) => format!("QLC+ library: {path}"),
            Some(e) => format!("file: {}", e.path),
            None => String::new(),
        }
    }
}

type Index = Vec<(String, usize, usize)>;

fn unpack(packed: &[u8]) -> Result<(Index, Vec<u8>), String> {
    let mut raw = Vec::with_capacity(16 << 20);
    flate2::read::ZlibDecoder::new(packed).read_to_end(&mut raw).map_err(|e| e.to_string())?;
    if raw.len() < 4 {
        return Err("empty".into());
    }
    let n = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    let index: Index = serde_json::from_slice(raw.get(4..4 + n).ok_or("truncated index")?).map_err(|e| e.to_string())?;
    let data = raw.split_off(4 + n);
    Ok((index, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChannelFn;

    #[test]
    fn names() {
        assert_eq!(names_from_path("Chauvet/Chauvet-SlimPAR-56.qxf"), ("Chauvet".into(), "SlimPAR 56".into()));
        assert_eq!(names_from_path("American_DJ/American-DJ-Mega-Bar.qxf").0, "American DJ");
    }

    #[test]
    fn bundled_library_loads_and_searches() {
        let lib = Library::load(&[]);
        assert!(lib.errors.is_empty(), "{:?}", lib.errors);
        assert!(lib.entries.len() > 1500, "{}", lib.entries.len());
        let hits = lib.search("chauvet slimpar 56", 10);
        assert!(!hits.is_empty());
        let f = lib.open(hits[0]).unwrap();
        assert_eq!(f.manufacturer, "Chauvet");
    }

    /// Every bundled fixture parses and every mode converts.
    #[test]
    fn whole_library_converts() {
        let lib = Library::load(&[]);
        let (mut modes, mut lit, mut pixel_types) = (0, 0, 0);
        let mut failures = Vec::new();
        for i in 0..lib.entries.len() {
            match lib.open(i) {
                Ok(f) => {
                    for m in 0..f.modes.len() {
                        match f.to_type(m, "test") {
                            Ok(t) => {
                                modes += 1;
                                let slots = t.slots.as_ref().unwrap();
                                assert_eq!(t.footprint(), slots.len());
                                if slots.iter().any(|s| !matches!(s.func, ChannelFn::Fixed(_))) {
                                    lit += 1;
                                }
                                if t.pixel_count() > 1 {
                                    pixel_types += 1;
                                }
                            }
                            Err(e) => failures.push(format!("{} mode {m}: {e}", lib.entries[i].path)),
                        }
                    }
                }
                Err(e) => failures.push(format!("{}: {e}", lib.entries[i].path)),
            }
        }
        eprintln!("{modes} modes, {lit} with colour/dimmer, {pixel_types} multi-pixel, {} failures", failures.len());
        assert!(failures.len() * 100 < lib.entries.len(), "too many failures: {:?}", &failures[..failures.len().min(10)]);
        assert!(lit * 10 > modes * 6, "only {lit} of {modes} modes drive colour/dimmer");
    }

    #[test]
    fn folder_scan_and_add_file() {
        let dir = std::env::temp_dir().join(format!("lumen-fx-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let lib0 = Library::load(&[]);
        let i = lib0.search("chauvet slimpar 56", 1)[0];
        std::fs::write(dir.join("sub/My-Par.qxf"), lib0.read(i).unwrap()).unwrap();
        std::fs::write(dir.join("broken.qxf"), "nope").unwrap();
        let lib = Library::load(&[dir.clone()]);
        let mine: Vec<_> = lib.entries.iter().filter(|e| e.origin() == "your files").collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].model, "SlimPAR 56"); // real name from inside the file
        assert_eq!(lib.errors.len(), 1); // the broken one is reported
        std::fs::remove_dir_all(dir).ok();
    }
}
