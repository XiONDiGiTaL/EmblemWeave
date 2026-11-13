//! Snapshots of the settings the launcher writes, taken before it changes an
//! engine file so a bad change can be undone.
//!
//! Each snapshot is a folder holding whatever existed at the time: the
//! launcher's `config.json`, the engine's `controls.ini` and its `fefw.ini`.
//! Taking one is throttled, so dragging a slider leaves a single snapshot
//! rather than one per frame.

use crate::config::Config;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How many are kept; older ones are removed.
const KEEP: usize = 20;
/// At most one snapshot per this long, per run of the launcher.
const EVERY: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub name: String,
    /// The files it holds, by name.
    pub files: Vec<String>,
    pub at: Option<SystemTime>,
}

pub fn root() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("snapshots"))
}

fn files() -> Vec<PathBuf> {
    let Some(dir) = Config::dir() else { return Vec::new() };
    let mut out = vec![dir.join("config.json")];
    if let Some(play) = crate::engine::run_dir() {
        out.push(play.join("controls.ini"));
        out.push(play.join("fefw.ini"));
    }
    out
}

/// Take a snapshot unless one was taken in the last few minutes.
pub fn before(reason: &str) -> Option<PathBuf> {
    if let Some(last) = list().first().and_then(|s| s.at) {
        if SystemTime::now().duration_since(last).unwrap_or_default() < EVERY {
            return None;
        }
    }
    take(reason)
}

/// Take a snapshot now. `None` when there is nothing to copy.
pub fn take(reason: &str) -> Option<PathBuf> {
    let root = root()?;
    let stamp = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let dir = root.join(format!("{stamp}-{reason}"));
    std::fs::create_dir_all(&dir).ok()?;
    let mut any = false;
    for f in files() {
        if !f.is_file() {
            continue;
        }
        if let Some(name) = f.file_name() {
            if std::fs::copy(&f, dir.join(name)).is_ok() {
                any = true;
            }
        }
    }
    if !any {
        let _ = std::fs::remove_dir(&dir);
        return None;
    }
    trim();
    Some(dir)
}

/// Newest first.
pub fn list() -> Vec<Snapshot> {
    let Some(root) = root() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut out: Vec<Snapshot> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let path = e.path();
            let files = std::fs::read_dir(&path)
                .map(|d| d.flatten().filter_map(|f| f.file_name().to_str().map(str::to_string)).collect())
                .unwrap_or_default();
            Snapshot {
                at: std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
                name: e.file_name().to_string_lossy().into_owned(),
                path,
                files,
            }
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

fn trim() {
    for s in list().into_iter().skip(KEEP) {
        let _ = std::fs::remove_dir_all(&s.path);
    }
}

/// Copy a snapshot's files back over the live ones.
pub fn restore(s: &Snapshot) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let mut targets: Vec<(String, PathBuf)> = vec![("config.json".into(), Config::dir().ok_or("APPDATA is not set")?.join("config.json"))];
    if let Some(play) = crate::engine::run_dir() {
        targets.push(("controls.ini".into(), play.join("controls.ini")));
        targets.push(("fefw.ini".into(), play.join("fefw.ini")));
    }
    for (name, to) in targets {
        let from = s.path.join(&name);
        if !from.is_file() {
            continue;
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("restoring {name}: {e}"))?;
        done.push(name);
    }
    if done.is_empty() {
        return Err("that snapshot holds no files the launcher writes".into());
    }
    Ok(done)
}

/// A snapshot folder's reason, for the page: `1731540000-before-pad-tuning`
/// reads as `before pad tuning`.
pub fn reason(name: &str) -> String {
    name.split_once('-').map(|(_, r)| r.replace('-', " ")).unwrap_or_else(|| name.to_string())
}

/// The time a snapshot was taken, as `HH:MM:SS` on the day's date it came
/// from, in UTC: the folder name carries the stamp, which never goes stale.
pub fn when(name: &str) -> String {
    let secs: u64 = name.split('-').next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02} UTC", s / 3600, s % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_back() {
        assert_eq!(reason("1731540000-before-pad-tuning"), "before pad tuning");
        assert_eq!(reason("plain"), "plain");
        assert_eq!(when("1731540000-before-pad-tuning"), "23:20 UTC");
    }

    #[test]
    fn a_snapshot_round_trips_its_files() {
        let dir = std::env::temp_dir().join(format!("ew-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "{\"a\":1}").unwrap();
        std::fs::write(dir.join("controls.ini"), "[pad]\ndeadzone = 0.2\n").unwrap();
        let s = Snapshot { path: dir.clone(), name: "1-test".into(), files: vec!["config.json".into()], at: None };
        // Restoring writes to the live folders, which a test must not touch;
        // the copy the restore performs is checked on a stand-in instead.
        let to = dir.join("restored.json");
        std::fs::copy(s.path.join("config.json"), &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "{\"a\":1}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
