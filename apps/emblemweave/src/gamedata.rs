//! Detection and validation of the user's game data.
//!
//! The user points the launcher at a `.nup` dump or at one folder. A `.nup`
//! is used as it is: its small ExeFS is copied once into the launcher's work
//! folder, its RomFS is read in place by the runtime, and its title and
//! version come from its own metadata. A folder may be a plain dump (with
//! `exefs/` and `romfs/`), an NSP extraction (`exefs/`, `romfs/`,
//! `data/control/control.nacp`), or the ExeFS folder itself. Each location can
//! also be overridden individually. Nothing here writes to the user's files.

use crate::config::{Config, Paths};
use std::path::{Path, PathBuf};

/// The module the game's code lives in.
pub const MAIN_MODULE: &str = "main";
/// The title this launcher is built for.
pub const TITLE_NAME: &str = "Fire Emblem: Fortune's Weave";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub severity: Severity,
    pub label: String,
    pub detail: String,
}

impl Check {
    fn new(severity: Severity, label: &str, detail: impl Into<String>) -> Check {
        Check { severity, label: label.to_string(), detail: detail.into() }
    }
}

/// One decompressed segment compared with the size its module header declares.
#[derive(Debug, Clone)]
pub struct SegmentCheck {
    pub name: &'static str,
    pub actual: Option<u64>,
    pub declared: Option<u64>,
}

impl SegmentCheck {
    pub fn ok(&self) -> bool {
        matches!((self.actual, self.declared), (Some(a), Some(d)) if a == d)
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameData {
    /// The `.nup` dump, when the game data is one.
    pub nup: Option<PathBuf>,
    /// The program id, from the `.nup` or `control.nacp`; it names the save
    /// folder in an emulator.
    pub title_id: Option<u64>,
    pub exefs: Option<PathBuf>,
    pub romfs: Option<PathBuf>,
    pub nacp: Option<PathBuf>,
    pub decoded: Option<PathBuf>,
    pub modules: Option<ew_format::ExeFs>,
    pub npdm: Option<ew_format::exefs::NpdmInfo>,
    /// Title name and display version from `control.nacp`.
    pub title: Option<(String, String)>,
    pub romfs_entries: usize,
    pub segments: Vec<SegmentCheck>,
    pub checks: Vec<Check>,
}

impl GameData {
    /// Everything the runtime needs to attempt a boot is present and valid.
    pub fn bootable_inputs(&self) -> bool {
        self.decoded.is_some() && self.segments.iter().all(|s| s.ok()) && self.romfs.is_some()
    }

    pub fn worst(&self) -> Severity {
        let mut worst = Severity::Ok;
        for c in &self.checks {
            worst = match (worst, c.severity) {
                (_, Severity::Error) | (Severity::Error, _) => Severity::Error,
                (_, Severity::Warn) | (Severity::Warn, _) => Severity::Warn,
                (w, _) => w,
            };
        }
        worst
    }
}

fn non_empty(s: &str) -> Option<PathBuf> {
    let t = s.trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

/// Breadth-first search for a directory entry named `name` under `root`.
fn find_named(root: &Path, name: &str, want_dir: bool, depth: usize) -> Option<PathBuf> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..=depth {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for entry in rd.flatten() {
                let path = entry.path();
                let is_dir = path.is_dir();
                if entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) && is_dir == want_dir {
                    return Some(path);
                }
                if is_dir {
                    next.push(path);
                }
            }
        }
        level = next;
        if level.len() > 4000 {
            break;
        }
    }
    None
}

fn file_len(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().map(|m| m.len())
}

/// Open a `.nup` dump: copy its ExeFS out once (it is small) and use its
/// RomFS in place. Returns the ExeFS folder and the title metadata.
fn use_nup(g: &mut GameData, path: &Path) -> Option<PathBuf> {
    let nup = match ew_format::nup::Nup::open(path) {
        Ok(n) => n,
        Err(e) => {
            g.checks.push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return None;
        }
    };
    let Some(pkg) = nup.program_package().cloned() else {
        g.checks.push(Check::new(Severity::Error, "Game file", "the .nup holds no base game or update"));
        return None;
    };
    g.checks.push(Check::new(
        Severity::Ok,
        "Game file",
        format!(".nup, title {:016X} v{}, {} files, checked against its SHA-256", pkg.title_id, pkg.version, nup.files.len()),
    ));
    if let Ok(meta) = nup.metadata(&pkg) {
        if let (Some(name), Some(version)) = (meta.title(), meta.display_version()) {
            g.title = Some((name, version));
        }
    }
    let exefs = nup.walk(pkg.exefs);
    let dir = Config::work_dir()?.join("nup").join(format!("{:016X}-v{}", pkg.title_id, pkg.version)).join("exefs");
    let current = exefs.iter().all(|(name, i)| file_len(&dir.join(name)) == Some(nup.files[*i as usize].size));
    if !current {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            g.checks.push(Check::new(Severity::Error, "ExeFS", format!("creating {}: {e}", dir.display())));
            return None;
        }
        for (name, i) in &exefs {
            match nup.read(*i).map_err(|e| format!("{e:#}")).and_then(|b| std::fs::write(dir.join(name), b).map_err(|e| e.to_string())) {
                Ok(()) => {}
                Err(e) => {
                    g.checks.push(Check::new(Severity::Error, "ExeFS", format!("copying {name} out of the .nup: {e}")));
                    return None;
                }
            }
        }
    }
    // The game reads the package in place: a file stored as a zstd frame has no
    // offset to read into, so the runtime refuses the whole package and the game
    // breaks before its first frame. Say that here, where the fix is readable.
    let listed: Vec<u32> = nup.walk(pkg.exefs).into_iter().chain(nup.walk(pkg.romfs)).map(|(_, i)| i).collect();
    let packed = listed.iter().filter(|i| nup.files[**i as usize].compressed).count();
    if packed > 0 {
        g.checks.push(Check::new(
            Severity::Error,
            "Game file",
            format!(
                "{packed} of {} files in this .nup are zstd-compressed; the game reads the package in place and needs stored files. Repack it without compression (`nup pack` with the zstandard module unavailable writes stored files), or point the launcher at an extracted exefs and romfs folder instead",
                listed.len()
            ),
        ));
    }
    g.romfs_entries = nup.dirs[pkg.romfs as usize].children.len();
    g.nup = Some(path.to_path_buf());
    g.title_id = Some(pkg.title_id);
    g.romfs = Some(path.to_path_buf());
    Some(dir)
}

/// Locate and validate everything. Explicit paths win over detection.
pub fn detect(paths: &Paths) -> GameData {
    let mut g = GameData::default();
    let root = non_empty(&paths.game_root);

    // A .nup dump supplies everything itself.
    if let Some(r) = root.as_ref().filter(|r| r.is_file() && ew_format::nup::Nup::is_nup(r)) {
        let r = r.clone();
        if let Some(exefs) = use_nup(&mut g, &r) {
            g.exefs = non_empty(&paths.exefs).or(Some(exefs));
        }
        return finish(g, paths);
    }

    if let Some(r) = &root {
        if !r.is_dir() {
            g.checks.push(Check::new(Severity::Error, "Game folder", format!("{} does not exist", r.display())));
        }
    }

    // ExeFS: explicit, root/exefs, the root itself, or a nested exefs.
    g.exefs = non_empty(&paths.exefs).or_else(|| {
        let r = root.as_ref()?;
        if r.join("exefs").is_dir() {
            Some(r.join("exefs"))
        } else if r.join(MAIN_MODULE).is_file() {
            Some(r.clone())
        } else {
            find_named(r, "exefs", true, 3)
        }
    });
    // RomFS: explicit, next to the ExeFS, or under the root.
    g.romfs = non_empty(&paths.romfs).or_else(|| {
        let near = g.exefs.as_ref().and_then(|e| e.parent()).map(|p| p.join("romfs"));
        near.filter(|p| p.is_dir()).or_else(|| {
            let r = root.as_ref()?;
            if r.join("romfs").is_dir() {
                Some(r.join("romfs"))
            } else {
                find_named(r, "romfs", true, 3)
            }
        })
    });
    // control.nacp: explicit, the NSP layout, or a search.
    g.nacp = non_empty(&paths.nacp).or_else(|| {
        let base = g.exefs.as_ref().and_then(|e| e.parent()).map(Path::to_path_buf).or(root.clone())?;
        let nsp = base.join("data").join("control").join("control.nacp");
        if nsp.is_file() {
            return Some(nsp);
        }
        find_named(&base, "control.nacp", false, 3).or_else(|| root.as_ref().and_then(|r| find_named(r, "control.nacp", false, 3)))
    });
    finish(g, paths)
}

/// The checks shared by every kind of game data: ExeFS contents, RomFS, title
/// metadata and the decoded module.
fn finish(mut g: GameData, paths: &Paths) -> GameData {
    // Decoded module: explicit, the launcher's work folder, or the repository's.
    g.decoded = non_empty(&paths.decoded).or_else(|| {
        let mut candidates = Vec::new();
        if let Some(w) = Config::work_dir() {
            candidates.push(w.join("decoded").join(MAIN_MODULE));
        }
        candidates.push(PathBuf::from("work/decoded/main"));
        candidates.into_iter().find(|p| p.join("text.bin").is_file())
    });

    // ExeFS contents.
    match &g.exefs {
        None => g.checks.push(Check::new(Severity::Error, "ExeFS", "not found; choose the folder that contains your extracted exefs")),
        Some(dir) => match ew_format::ExeFs::scan_dir(dir) {
            Ok(exefs) => {
                if let Ok(Some(npdm)) = exefs.read_npdm() {
                    g.npdm = Some(npdm);
                }
                match exefs.find(MAIN_MODULE) {
                    Some(m) => g.checks.push(Check::new(
                        Severity::Ok,
                        "ExeFS",
                        format!("{} modules; `main` is `{}` ({})", exefs.modules.len(), m.module_name, m.compressions.join(" + ")),
                    )),
                    None => g.checks.push(Check::new(Severity::Error, "ExeFS", "no `main` module in this ExeFS")),
                }
                g.modules = Some(exefs);
            }
            Err(e) => g.checks.push(Check::new(Severity::Error, "ExeFS", format!("{}: {e:#}", dir.display()))),
        },
    }

    // RomFS.
    match &g.romfs {
        None => g.checks.push(Check::new(Severity::Error, "RomFS", "not found; the game cannot load its assets without it")),
        Some(_) if g.nup.is_some() => g.checks.push(Check::new(
            Severity::Ok,
            "RomFS",
            format!("{} top-level entries, read in place from the .nup", g.romfs_entries),
        )),
        Some(dir) => {
            g.romfs_entries = std::fs::read_dir(dir).map(|rd| rd.count()).unwrap_or(0);
            if g.romfs_entries == 0 {
                g.checks.push(Check::new(Severity::Error, "RomFS", format!("{} is empty", dir.display())));
            } else if dir.join("Fs.bin.zst").is_file() {
                g.checks.push(Check::new(Severity::Ok, "RomFS", format!("{} top-level entries, content index present", g.romfs_entries)));
            } else {
                g.checks.push(Check::new(Severity::Warn, "RomFS", "Fs.bin.zst (the content index the game loads first) is missing"));
            }
        }
    }

    // Title metadata: the .nup's own, or control.nacp.
    let nacp_title = if g.nup.is_some() { g.title.clone() } else { g.nacp.as_deref().and_then(ew_boot::read_nacp) };
    match nacp_title {
        Some((name, version)) => {
            let sev = if name == TITLE_NAME { Severity::Ok } else { Severity::Warn };
            g.checks.push(Check::new(sev, "Title", format!("{name}, version {version}")));
            g.title = Some((name, version));
        }
        None => g.checks.push(Check::new(
            Severity::Warn,
            "Title metadata",
            "control.nacp not found; the game's display version cannot be reported to it",
        )),
    }

    // Decoded segments against the module header.
    let main = g.modules.as_ref().and_then(|e| e.find(MAIN_MODULE));
    if let Some(dir) = &g.decoded {
        for (name, declared) in [
            ("text", main.map(|m| m.text_size as u64)),
            ("rodata", main.map(|m| m.rodata_size as u64)),
            ("data", main.map(|m| m.data_size as u64)),
        ] {
            g.segments.push(SegmentCheck { name, actual: file_len(&dir.join(format!("{name}.bin"))), declared });
        }
        let ok = g.segments.iter().all(|s| s.ok());
        g.checks.push(if ok {
            Check::new(Severity::Ok, "Decoded module", "every segment matches the size its header declares")
        } else if main.is_none() {
            Check::new(Severity::Warn, "Decoded module", "present, but there is no ExeFS to check it against")
        } else {
            Check::new(Severity::Error, "Decoded module", "segment sizes do not match the module header; decode again")
        });
    } else {
        g.checks.push(Check::new(
            Severity::Warn,
            "Decoded module",
            "not decoded yet; the main module is ZBIC-compressed and must be decoded once",
        ));
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_nsp_layout() {
        let root = std::env::temp_dir().join(format!("ew-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dump").join("exefs")).unwrap();
        std::fs::create_dir_all(root.join("dump").join("romfs")).unwrap();
        std::fs::create_dir_all(root.join("dump").join("data").join("control")).unwrap();
        std::fs::write(root.join("dump").join("romfs").join("Fs.bin.zst"), b"x").unwrap();
        let paths = Paths { game_root: root.to_string_lossy().into(), ..Default::default() };
        let g = detect(&paths);
        assert_eq!(g.exefs.as_deref(), Some(root.join("dump").join("exefs").as_path()));
        assert_eq!(g.romfs.as_deref(), Some(root.join("dump").join("romfs").as_path()));
        assert_eq!(g.romfs_entries, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// On a real dump: `EW_NUP=<file.nup> cargo test -p emblemweave -- --ignored`.
    #[test]
    #[ignore]
    fn detects_a_nup_dump() {
        let path = std::env::var("EW_NUP").expect("EW_NUP");
        let g = detect(&Paths { game_root: path.clone(), ..Default::default() });
        for c in &g.checks {
            eprintln!("{:?} {}: {}", c.severity, c.label, c.detail);
        }
        assert_eq!(g.nup.as_deref(), Some(std::path::Path::new(&path)));
        assert_eq!(g.romfs.as_deref(), Some(std::path::Path::new(&path)));
        assert!(g.exefs.as_ref().is_some_and(|e| e.join("main").is_file() && e.join("sdk").is_file()));
        assert_eq!(g.title.as_ref().map(|t| t.0.as_str()), Some(TITLE_NAME));
        assert!(g.romfs_entries > 0);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let paths = Paths { game_root: "Z:/definitely/not/here".into(), ..Default::default() };
        let g = detect(&paths);
        assert_eq!(g.worst(), Severity::Error);
    }
}
