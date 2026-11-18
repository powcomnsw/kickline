//! Fixture library: QLC+'s fixture definitions bundled with Kickline (Apache-2.0,
//! see `assets/QLCPLUS-LICENSE.txt`), plus any folders of `.qxf` files, such
//! as ~/Documents/Kickline/Fixtures or an installed QLC+'s user fixtures.
//!
//! Download fixtures that aren't included from open-fixture-library.org
//! ("Download as QLC+ fixture") and drop them on the Kickline window.

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
    /// Type and modes, filled in when the library is indexed.
    pub meta: Option<Meta>,
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

    /// Search text, lower-case: names, type and the everyday words for it.
    fn haystack(&self) -> String {
        let kind = self.meta.as_ref().map(|m| format!("{} {}", m.kind, kind_aliases(&m.kind))).unwrap_or_default();
        format!("{} {} {}", self.manufacturer, self.model, kind).to_lowercase()
    }

    /// Distinct channel counts of the modes, smallest first.
    pub fn channel_counts(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.meta.iter().flat_map(|m| m.modes.iter().map(|x| x.pins.len())).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// What a channel does, coarsely, for the pinout strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pin {
    Dimmer,
    Red,
    Green,
    Blue,
    White,
    Amber,
    Uv,
    /// Cyan / magenta / yellow filters.
    Cmy,
    /// Colour wheel or colour macros.
    Colour,
    Pan,
    Tilt,
    Gobo,
    Prism,
    /// Shutter / strobe.
    Strobe,
    /// Focus, zoom, iris, frost.
    Beam,
    Speed,
    Effect,
    Other,
}

impl Pin {
    pub fn name(self) -> &'static str {
        match self {
            Pin::Dimmer => "Dimmer",
            Pin::Red => "Red",
            Pin::Green => "Green",
            Pin::Blue => "Blue",
            Pin::White => "White",
            Pin::Amber => "Amber",
            Pin::Uv => "UV",
            Pin::Cmy => "CMY",
            Pin::Colour => "Colour wheel / macro",
            Pin::Pan => "Pan",
            Pin::Tilt => "Tilt",
            Pin::Gobo => "Gobo",
            Pin::Prism => "Prism",
            Pin::Strobe => "Shutter / strobe",
            Pin::Beam => "Focus / zoom / iris",
            Pin::Speed => "Speed",
            Pin::Effect => "Effects / programs",
            Pin::Other => "Other",
        }
    }

    /// Light, colour or dimmer: what Kickline drives from the picture today.
    pub fn driven(self) -> bool {
        matches!(self, Pin::Dimmer | Pin::Red | Pin::Green | Pin::Blue | Pin::White | Pin::Amber | Pin::Cmy)
    }
}

fn pin_of(ch: &crate::qxf::QxfChannel) -> Pin {
    use crate::ChannelFn as F;
    match ch.func {
        F::Red => return Pin::Red,
        F::Green => return Pin::Green,
        F::Blue => return Pin::Blue,
        F::White => return Pin::White,
        F::Amber => return Pin::Amber,
        F::Uv => return Pin::Uv,
        F::Cyan | F::Magenta | F::Yellow => return Pin::Cmy,
        F::Dimmer => return Pin::Dimmer,
        F::Fixed(_) => {}
    }
    // The QLC+ group/preset says what it is; the channel name is a fallback.
    let classify = |t: &str| -> Option<Pin> {
        let has = |w: &str| t.contains(w);
        Some(if has("speed") {
            Pin::Speed
        } else if has("pan") {
            Pin::Pan
        } else if has("tilt") {
            Pin::Tilt
        } else if has("gobo") {
            Pin::Gobo
        } else if has("prism") {
            Pin::Prism
        } else if has("shutter") || has("strobe") {
            Pin::Strobe
        } else if has("colo") {
            Pin::Colour
        } else if has("beam") || has("focus") || has("zoom") || has("iris") || has("frost") {
            Pin::Beam
        } else if has("effect") || has("program") || has("macro") || has("auto") || has("sound") {
            Pin::Effect
        } else if has("intensity") || has("dimmer") {
            Pin::Dimmer
        } else {
            return None;
        })
    };
    classify(&ch.kind.to_ascii_lowercase()).or_else(|| classify(&ch.name.to_ascii_lowercase())).unwrap_or(Pin::Other)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeMeta {
    pub name: String,
    /// One per channel, in DMX order.
    pub pins: Vec<Pin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// QLC+ type: "Moving Head", "Color Changer", "LED Bar (Pixels)"…
    pub kind: String,
    pub modes: Vec<ModeMeta>,
}

impl Meta {
    fn from(f: &crate::qxf::QxfFixture) -> Meta {
        let modes = f
            .modes
            .iter()
            .map(|m| ModeMeta {
                name: m.name.clone(),
                pins: m.channels.iter().map(|c| f.channel(c).map(pin_of).unwrap_or(Pin::Other)).collect(),
            })
            .collect();
        Meta { kind: f.kind.clone(), modes }
    }

    /// Some mode has colour or dimmer channels Kickline can drive.
    pub fn drivable(&self) -> bool {
        self.modes.iter().any(|m| m.pins.iter().any(|p| p.driven()))
    }

    /// The everyday name for the type.
    pub fn kind_label(&self) -> &str {
        match self.kind.as_str() {
            "Color Changer" => "Par / wash",
            "LED Bar (Pixels)" => "Pixel bar",
            "LED Bar (Beams)" => "LED bar",
            k => k,
        }
    }
}

/// Words people use for each QLC+ type, so "par" or "mover" finds them.
fn kind_aliases(kind: &str) -> &'static str {
    match kind {
        "Color Changer" => "par wash colour color changer led",
        "Moving Head" => "mover moving head spot beam wash",
        "LED Bar (Pixels)" => "bar batten pixel pixels",
        "LED Bar (Beams)" => "bar batten beams",
        "Scanner" => "scanner mirror",
        "Strobe" => "strobe blinder",
        "Dimmer" => "dimmer pack",
        "Laser" => "laser",
        "Smoke" | "Hazer" => "fog smoke haze hazer",
        "Flower" => "flower derby effect",
        _ => "",
    }
}

/// A parsed search: words to match, channel counts to require, and bare
/// numbers that may be either (a model number or a channel count).
#[derive(Debug, PartialEq)]
struct Query {
    words: Vec<String>,
    counts: Vec<usize>,
    either: Vec<(String, usize)>,
}

fn parse_query(query: &str) -> Query {
    let lower = query.to_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
    let says_channels = tokens.iter().any(|w| matches!(*w, "ch" | "chan" | "channel" | "channels"));
    let mut q = Query { words: Vec::new(), counts: Vec::new(), either: Vec::new() };
    for w in tokens {
        if matches!(w, "ch" | "chan" | "channel" | "channels" | "-ch" | "dmx") {
            continue;
        }
        let digits: String = w.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = w[digits.len()..].trim_start_matches('-');
        if let (false, Ok(n)) = (digits.is_empty(), digits.parse::<usize>()) {
            if matches!(rest, "ch" | "chan" | "channel" | "channels") || (rest.is_empty() && says_channels) {
                q.counts.push(n);
                continue;
            }
            if rest.is_empty() {
                q.either.push((w.to_string(), n));
                continue;
            }
        }
        q.words.push(w.to_string());
    }
    q
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
                    Entry { manufacturer, model, meta: None, path, source: Source::Bundled { offset, len } }
                }));
                lib.blob = blob;
            }
            Err(e) => lib.errors.push(format!("built-in library: {e}")),
        }
        for f in folders {
            lib.scan(f, 0);
        }
        lib.entries.sort_by(|a, b| a.manufacturer.to_lowercase().cmp(&b.manufacturer.to_lowercase()).then(a.model.cmp(&b.model)));
        lib.index();
        lib
    }

    /// Read every fixture's type and modes (about a tenth of a second for
    /// the whole built-in library).
    pub fn index(&mut self) {
        for i in 0..self.entries.len() {
            if self.entries[i].meta.is_none() {
                self.entries[i].meta = self.open(i).ok().map(|f| Meta::from(&f));
            }
        }
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
        let (manufacturer, model, meta) = match crate::qxf::parse(&xml) {
            Ok(f) => {
                let m = Meta::from(&f);
                (f.manufacturer, f.model, Some(m))
            }
            Err(e) => {
                self.errors.push(format!("{}: {e}", p.display()));
                return None;
            }
        };
        let path = p.display().to_string();
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            return Some(i);
        }
        self.entries.push(Entry { manufacturer, model, meta, path, source: Source::File(p.to_path_buf()) });
        Some(self.entries.len() - 1)
    }

    /// Indices matching `query`, at most `limit`. Words match the
    /// manufacturer, model or type ("par", "moving head"); numbers match a
    /// mode's channel count ("16", "16ch", "16 channel").
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        self.search_filtered(query, limit, false)
    }

    /// Like `search`; `drivable_only` keeps fixtures with colour or dimmer channels.
    pub fn search_filtered(&self, query: &str, limit: usize, drivable_only: bool) -> Vec<usize> {
        let q = parse_query(query);
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if drivable_only && !e.meta.as_ref().is_some_and(|m| m.drivable()) {
                    return false;
                }
                let have = e.channel_counts();
                if !q.counts.iter().all(|c| have.contains(c)) {
                    return false;
                }
                let h = e.haystack();
                q.words.iter().all(|w| h.contains(w.as_str())) && q.either.iter().all(|(w, n)| h.contains(w.as_str()) || have.contains(n))
            })
            .map(|(i, _)| i)
            .take(limit)
            .collect()
    }

    /// The mode a search points at: one with the channel count asked for,
    /// otherwise None.
    pub fn preferred_mode(&self, i: usize, query: &str) -> Option<usize> {
        let q = parse_query(query);
        let modes = &self.entries.get(i)?.meta.as_ref()?.modes;
        let want: Vec<usize> = q.counts.iter().copied().chain(q.either.iter().map(|(_, n)| *n)).collect();
        modes.iter().position(|m| want.contains(&m.pins.len()))
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

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn query_parsing() {
        let q = parse_query("16 channel moving head");
        assert_eq!((q.words, q.counts), (vec!["moving".to_string(), "head".to_string()], vec![16]));
        assert_eq!(parse_query("chauvet 7ch").counts, vec![7]);
        assert_eq!(parse_query("par 8-ch").counts, vec![8]);
        // A bare number could be a model number or a channel count.
        assert_eq!(parse_query("slimpar 56").either, vec![("56".to_string(), 56)]);
    }

    #[test]
    fn finds_by_type_and_channels() {
        let lib = Library::load(&[]);
        let hits = lib.search("16 channel moving head", 5000);
        assert!(hits.len() > 5, "found {}", hits.len());
        for &i in &hits {
            let e = &lib.entries[i];
            let m = e.meta.as_ref().unwrap();
            assert_eq!(m.kind, "Moving Head", "{} {}", e.manufacturer, e.model);
            assert!(e.channel_counts().contains(&16));
        }
        // Everyday words work.
        assert!(lib.search("par", 5000).iter().any(|&i| lib.entries[i].meta.as_ref().unwrap().kind == "Color Changer"));
        // Driveable filter drops things like smoke machines.
        let smoke = lib.search_filtered("smoke", 5000, true);
        assert!(smoke.len() < lib.search("smoke", 5000).len());
    }

    #[test]
    fn pinout_knows_moving_head_channels() {
        let lib = Library::load(&[]);
        let heads = lib.search("moving head", 5000);
        let with_pan_tilt = heads
            .iter()
            .filter(|&&i| lib.entries[i].meta.as_ref().unwrap().modes.iter().any(|m| m.pins.contains(&Pin::Pan) && m.pins.contains(&Pin::Tilt)))
            .count();
        assert!(with_pan_tilt * 10 > heads.len() * 8, "{with_pan_tilt} of {} heads have pan+tilt", heads.len());
        // "Tilt speed" is a speed channel, not tilt.
        let ch = crate::qxf::QxfChannel { name: "Tilt speed".into(), func: crate::ChannelFn::Fixed(0), fine: false, kind: "Speed".into() };
        assert_eq!(pin_of(&ch), Pin::Speed);
    }
}

#[cfg(test)]
mod timing {
    #[test]
    #[ignore]
    fn parse_all_time() {
        let lib = super::Library::load(&[]);
        let t = std::time::Instant::now();
        let mut kinds = std::collections::BTreeMap::new();
        for i in 0..lib.entries.len() {
            if let Ok(f) = lib.open(i) {
                *kinds.entry(f.kind.clone()).or_insert(0) += 1;
            }
        }
        println!("{} fixtures in {:?}", lib.entries.len(), t.elapsed());
        println!("{kinds:?}");
    }
}
