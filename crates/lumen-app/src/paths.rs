//! Where files go. Apps launched from Finder start in "/", which is
//! read-only, so nothing may be written relative to the working directory.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Settings, logs and crash reports.
pub fn data_dir() -> PathBuf {
    let d = if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Lumen")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("Lumen")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/share"))
            .join("lumen")
    };
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn crash_dir() -> PathBuf {
    let d = data_dir().join("crashes");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// User-visible project folder: ~/Documents/Lumen.
pub fn projects_dir() -> PathBuf {
    let d = home().join("Documents").join("Lumen");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn project_file() -> PathBuf {
    projects_dir().join("lumen_project.json")
}

/// Reports go on the Desktop so they're easy to find and send.
pub fn reports_dir() -> PathBuf {
    let desk = home().join("Desktop");
    if desk.is_dir() {
        desk
    } else {
        projects_dir()
    }
}

/// Show a file in Finder / Explorer / the file manager.
pub fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
}

/// UTC timestamp "2026-11-17T04:51:00Z" and a filename-safe "20261117-045100".
pub fn timestamp() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    (
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z"),
        format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}"),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let (iso, file) = super::timestamp();
        assert_eq!(iso.len(), 20);
        assert!(iso.ends_with('Z'));
        assert_eq!(file.len(), 15);
        assert!(iso.starts_with("20"));
    }
}
