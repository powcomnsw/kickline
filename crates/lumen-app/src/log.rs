//! In-memory event log for the diagnostics report, plus crash capture.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    /// Seconds since the app started.
    pub t: f64,
    pub level: Level,
    pub source: &'static str,
    pub message: String,
    /// How many times this exact message repeated in a row.
    pub count: u32,
}

const MAX: usize = 1000;

struct Log {
    start: Instant,
    entries: VecDeque<Entry>,
}

fn log() -> &'static Mutex<Log> {
    static L: OnceLock<Mutex<Log>> = OnceLock::new();
    L.get_or_init(|| {
        Mutex::new(Log {
            start: Instant::now(),
            entries: VecDeque::new(),
        })
    })
}

pub fn uptime() -> f64 {
    log()
        .lock()
        .map(|l| l.start.elapsed().as_secs_f64())
        .unwrap_or(0.0)
}

pub fn push(level: Level, source: &'static str, message: impl Into<String>) {
    let message = message.into();
    let Ok(mut l) = log().lock() else { return };
    let t = l.start.elapsed().as_secs_f64();
    if let Some(last) = l.entries.back_mut() {
        if last.source == source && last.message == message && last.level == level {
            last.count += 1;
            last.t = t;
            return;
        }
    }
    if l.entries.len() >= MAX {
        l.entries.pop_front();
    }
    l.entries.push_back(Entry {
        t,
        level,
        source,
        message,
        count: 1,
    });
}

pub fn info(source: &'static str, m: impl Into<String>) {
    push(Level::Info, source, m)
}
pub fn warn(source: &'static str, m: impl Into<String>) {
    push(Level::Warn, source, m)
}
pub fn error(source: &'static str, m: impl Into<String>) {
    push(Level::Error, source, m)
}

pub fn entries() -> Vec<Entry> {
    log()
        .lock()
        .map(|l| l.entries.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn counts() -> (usize, usize) {
    let e = entries();
    (
        e.iter().filter(|x| x.level == Level::Warn).count(),
        e.iter().filter(|x| x.level == Level::Error).count(),
    )
}

/// Write panics to a crash file so the next launch can include them in a report.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let (iso, file) = crate::paths::timestamp();
        let bt = std::backtrace::Backtrace::force_capture();
        let recent: Vec<String> = entries()
            .iter()
            .rev()
            .take(40)
            .rev()
            .map(|e| format!("[{:>8.2}s {:?} {}] {}", e.t, e.level, e.source, e.message))
            .collect();
        let text = format!(
            "Lumen {} crashed at {iso}\nOS: {} {}\n\n{info}\n\nBacktrace:\n{bt}\n\nRecent log:\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            recent.join("\n")
        );
        let _ = std::fs::write(
            crate::paths::crash_dir().join(format!("crash-{file}.txt")),
            text,
        );
        default(info);
    }));
}

/// Crash files from earlier sessions: (file name, contents).
pub fn previous_crashes() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::paths::crash_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("crash-") {
                if let Ok(s) = std::fs::read_to_string(e.path()) {
                    out.push((name, s));
                }
            }
        }
    }
    out.sort();
    out
}

pub fn clear_crashes() {
    if let Ok(rd) = std::fs::read_dir(crate::paths::crash_dir()) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedups_repeats() {
        error("test-dedup", "boom");
        error("test-dedup", "boom");
        error("test-dedup", "boom");
        let e: Vec<_> = entries()
            .into_iter()
            .filter(|e| e.source == "test-dedup")
            .collect();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].count, 3);
    }
}
