//! Crash reports: when the launcher panics, it writes a bundle with what it
//! was doing, its settings and its version, so a report can be attached to an
//! issue without hunting for the pieces.

use crate::config::Config;
use std::path::PathBuf;
use std::sync::Mutex;

/// The last few things the launcher noted, kept for a crash report.
static RECENT: Mutex<Vec<String>> = Mutex::new(Vec::new());
const KEEP: usize = 40;

pub fn note(line: &str) {
    if let Ok(mut r) = RECENT.lock() {
        if r.len() >= KEEP {
            r.remove(0);
        }
        r.push(line.to_string());
    }
}

pub fn reports_dir() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("crashes"))
}

pub fn reports() -> Vec<PathBuf> {
    let Some(dir) = reports_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    out.sort();
    out.reverse();
    out
}

/// Write a bundle beside the settings. Returns its folder.
pub fn write_bundle(summary: &str, location: &str) -> Option<PathBuf> {
    let dir = reports_dir()?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let bundle = dir.join(format!("crash-{stamp}"));
    std::fs::create_dir_all(&bundle).ok()?;

    let recent = RECENT.lock().map(|r| r.join("\n")).unwrap_or_default();
    let text = format!(
        "EmblemWeave crash report\n\
         ========================\n\
         version: {} ({})\n\
         built: {} {}\n\
         profile: {}\n\
         location: {location}\n\
         \n\
         {summary}\n\
         \n\
         Before it stopped:\n{recent}\n",
        env!("CARGO_PKG_VERSION"),
        env!("EW_GIT_REV"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "debug" } else { "release" },
    );
    std::fs::write(bundle.join("summary.txt"), text).ok()?;

    if let Some(cfg) = Config::path() {
        let _ = std::fs::copy(&cfg, bundle.join("config.json"));
    }
    if let Some(play) = crate::engine::run_dir() {
        for name in ["controls.ini", "fefw.ini", "nxrt.log"] {
            let from = play.join(name);
            if from.is_file() {
                let _ = std::fs::copy(&from, bundle.join(name));
            }
        }
    }
    Some(bundle)
}

/// Turn panics into bundles. The previous hook still runs, so the message
/// reaches the console as before.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown place".into());
        if let Some(dir) = write_bundle(&info.to_string(), &location) {
            eprintln!("EmblemWeave stopped unexpectedly. A report is in {}", dir.display());
        }
        previous(info);
    }));
}

/// A plain-text report the launcher can hand to the user on demand, so an
/// unexpected exit that is not a panic still has something to attach.
pub fn diagnostics_text() -> String {
    let mut s = format!(
        "EmblemWeave {} ({}) on {} {}\n",
        env!("CARGO_PKG_VERSION"),
        env!("EW_GIT_REV"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some(root) = crate::engine::root() {
        s.push_str(&format!("engine: {}\n", root.display()));
    }
    if let Some(play) = crate::engine::run_dir() {
        s.push_str(&format!("play folder: {}\n", play.display()));
    }
    if let Ok(r) = RECENT.lock() {
        s.push_str("\nBefore now:\n");
        s.push_str(&r.join("\n"));
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_keep_the_latest() {
        for i in 0..(KEEP + 10) {
            note(&format!("line {i}"));
        }
        let r = RECENT.lock().unwrap();
        assert_eq!(r.len(), KEEP);
        assert!(r.last().unwrap().ends_with(&format!("line {}", KEEP + 9)));
    }

    #[test]
    fn the_diagnostics_text_names_the_build() {
        let t = diagnostics_text();
        assert!(t.starts_with("EmblemWeave "));
        assert!(t.contains(env!("CARGO_PKG_VERSION")));
    }
}
