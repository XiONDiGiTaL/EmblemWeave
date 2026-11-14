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

/// The build of the game the engine was tested against. The list of module
/// hashes comes from the fefw-recomp 0.2.0 release package (`tools/build/`),
/// which Obersteinn preserved after its author stepped away; see
/// `docs/UPSTREAM.md`.
const KNOWN_DUMP: &str = include_str!("../data/dump_hashes.json");

/// The supported-dump manifest: the ExeFS modules the engine was tested with.
#[derive(Debug, serde::Deserialize)]
struct KnownDump {
    program_id: String,
    version: String,
    main_build_id: String,
    sha256: std::collections::BTreeMap<String, String>,
}

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
    /// The `.nsp` package, when the game data is one.
    pub nsp: Option<PathBuf>,
    /// The `.nx` archive, when the game data is one.
    pub nx: Option<PathBuf>,
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

/// The Game data card holds three lines of a reason, about thirty characters each
/// at the card's width, so a message is cut at 84 characters with an ellipsis; a
/// long path then cannot push that card past its neighbours.
const CARD_LINE: usize = 84;

/// `text`, cut to `max` characters at a word boundary and marked with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    if let Some(space) = cut.rfind(' ') {
        if cut.len() - space < 12 {
            cut.truncate(space);
        }
    }
    cut.push('…');
    cut
}

impl GameData {
    /// Everything the runtime needs to attempt a boot is present and valid.
    pub fn bootable_inputs(&self) -> bool {
        self.decoded.is_some() && self.segments.iter().all(|s| s.ok()) && self.romfs.is_some()
    }

    /// Every check that is not satisfied, in the words of the check that found
    /// it. A status of "Needs attention" says nothing on its own; this is the
    /// reason behind it.
    pub fn attention(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| matches!(c.severity, Severity::Warn | Severity::Error)).collect()
    }

    /// One line naming what is not right, for a status card: the first reason,
    /// and how many more there are when there is more than one. Longer messages
    /// are clipped to the space the card has; the tooltip and the page that owns
    /// the check still show every word.
    pub fn attention_line(&self) -> Option<String> {
        let open = self.attention();
        let first = open.first()?;
        let more = open.len() - 1;
        let head = clip(&format!("{}: {}", first.label, first.detail), CARD_LINE);
        Some(match more {
            0 => head,
            more => format!("{head} (+{more} more)"),
        })
    }

    /// Every reason, one per line, for a hover tooltip.
    pub fn attention_text(&self) -> Option<String> {
        let open = self.attention();
        if open.is_empty() {
            return None;
        }
        Some(open.iter().map(|c| format!("{}: {}", c.label, c.detail)).collect::<Vec<_>>().join("\n"))
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

/// Split an ExeFS into modules that match the manifest, modules that differ
/// and modules it does not have, each by name.
fn compare_dump(dir: &Path, known: &KnownDump) -> (Vec<String>, Vec<String>, Vec<String>) {
    use sha2::Digest as _;
    let (mut same, mut differ, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for (name, want) in &known.sha256 {
        match std::fs::read(dir.join(name)) {
            Ok(bytes) => {
                let got = format!("{:x}", sha2::Sha256::digest(&bytes));
                if &got == want {
                    same.push(name.clone());
                } else {
                    differ.push(name.clone());
                }
            }
            Err(_) => missing.push(name.clone()),
        }
    }
    (same, differ, missing)
}

/// Say whether the user's ExeFS is the build the engine was tested against.
fn check_known_dump(g: &mut GameData, dir: &Path) {
    let Ok(known) = serde_json::from_str::<KnownDump>(KNOWN_DUMP) else { return };
    let (same, differ, missing) = compare_dump(dir, &known);
    let build = &known.main_build_id[..known.main_build_id.len().min(12)];
    if differ.is_empty() && missing.is_empty() && !same.is_empty() {
        g.checks.push(Check::new(
            Severity::Ok,
            "Game build",
            format!("matches the dump the engine was tested with ({} {}): all {} module hashes agree", known.version, build, same.len()),
        ));
    } else if !differ.is_empty() {
        g.checks.push(Check::new(
            Severity::Warn,
            "Game build",
            format!(
                "a different build of the game ({} differs): the engine was tested with {} {} (main {build}), so this may work but is not the tested build",
                differ.join(", "),
                known.version,
                known.program_id
            ),
        ));
    } else if !missing.is_empty() {
        g.checks.push(Check::new(
            Severity::Info,
            "Game build",
            format!("could not compare with the tested build: {} not found", missing.join(", ")),
        ));
    }
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
                    g.checks.push(Check::new(Severity::Error, "ExeFS", format!("copying {name}: {e}")));
                    return None;
                }
            }
        }
    }
    // The engine decompresses a zstd entry into its cache the first time the game
    // reads it, so the package runs as it is; say what that cache will take.
    let listed: Vec<u32> = nup.walk(pkg.exefs).into_iter().chain(nup.walk(pkg.romfs)).map(|(_, i)| i).collect();
    let packed: Vec<&ew_format::nup::FileEntry> =
        listed.iter().map(|i| &nup.files[*i as usize]).filter(|f| f.compressed).collect();
    if !packed.is_empty() {
        let bytes: u64 = packed.iter().map(|f| f.size).sum();
        g.checks.push(Check::new(
            Severity::Warn,
            "Game file",
            format!(
                "{} of {} files are zstd-compressed: each is decompressed once into %LOCALAPPDATA%\\EmblemWeave\\nup-cache (about {:.1} GB), so the first start is slower and needs that space",
                packed.len(),
                listed.len(),
                bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            ),
        ));
    }
    g.romfs_entries = nup.dirs[pkg.romfs as usize].children.len();
    g.nup = Some(path.to_path_buf());
    g.title_id = Some(pkg.title_id);
    g.romfs = Some(path.to_path_buf());
    Some(dir)
}

/// Open a `.nx` archive: copy its ExeFS out once (it is small) and hand the
/// archive itself to the engine as the RomFS, the way a `.nup` is used. An NX
/// snapshot is a complete file tree, and a tree packed from an extracted dump
/// keeps `exefs/`, `romfs/` and an optional `data/` at its root.
fn use_nx(g: &mut GameData, path: &Path) -> Option<PathBuf> {
    let nx = match ew_format::nx::Nx::open(path) {
        Ok(n) => n,
        Err(e) => {
            g.checks.push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return None;
        }
    };
    // The launcher is built for one game; take its application when the archive
    // holds several, and the only one when it holds exactly one.
    let wanted = serde_json::from_str::<KnownDump>(KNOWN_DUMP)
        .ok()
        .and_then(|k| u64::from_str_radix(k.program_id.trim_start_matches("0x"), 16).ok());
    let app_index = wanted.and_then(|id| nx.application(id)).or_else(|| (nx.applications().len() == 1).then_some(0));
    let Some(app_index) = app_index else {
        g.checks.push(Check::new(Severity::Error, "Game file", "the .nx holds no application for this game"));
        return None;
    };
    let app = &nx.applications()[app_index];
    let Some(snapshot) = nx.current(app_index) else {
        g.checks.push(Check::new(Severity::Error, "Game file", "the .nx holds no snapshot for this game"));
        return None;
    };
    let snap = &nx.snapshots()[snapshot];
    let version = if snap.display_version.is_empty() { snap.version.to_string() } else { snap.display_version.clone() };
    let tree = match nx.tree(snapshot) {
        Ok(t) => t,
        Err(e) => {
            g.checks.push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return None;
        }
    };
    let (compressed, _) = nx.compression();
    g.checks.push(Check::new(
        Severity::Ok,
        "Game file",
        format!(
            ".nx, {} {:016X} v{version}, {} files, checked against its digests",
            if app.title.is_empty() { "title" } else { app.title.as_str() },
            app.id,
            tree.len()
        ),
    ));
    if compressed > 0 {
        g.checks.push(Check::new(
            Severity::Info,
            "Game file",
            format!(
                "{compressed} of {} blocks are zstd-compressed: each is decoded when the game reads it, so a read is a little slower than raw data",
                nx.header().block_count
            ),
        ));
    }

    // The ExeFS is a folder of modules at the snapshot root.
    let modules: Vec<(String, u32, u64)> = tree
        .iter()
        .filter_map(|(p, id, size)| {
            let name = p.strip_prefix("exefs/")?;
            (!name.is_empty() && !name.contains('/')).then(|| (name.to_string(), *id, *size))
        })
        .collect();
    if modules.is_empty() {
        g.checks.push(Check::new(Severity::Error, "ExeFS", "the .nx has no exefs/ folder at the root of this version"));
        return None;
    }
    if !modules.iter().any(|(name, _, _)| name == MAIN_MODULE) {
        g.checks.push(Check::new(Severity::Error, "ExeFS", "the .nx has no exefs/main module"));
        return None;
    }
    let dir = Config::work_dir()?.join("nx").join(format!("{:016X}-v{}", app.id, snap.version)).join("exefs");
    let current = modules.iter().all(|(name, _, size)| file_len(&dir.join(name)) == Some(*size));
    if !current {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            g.checks.push(Check::new(Severity::Error, "ExeFS", format!("creating {}: {e}", dir.display())));
            return None;
        }
        for (name, id, _) in &modules {
            match nx.read(*id).map_err(|e| format!("{e:#}")).and_then(|b| std::fs::write(dir.join(name), b).map_err(|e| e.to_string())) {
                Ok(()) => {}
                Err(e) => {
                    g.checks.push(Check::new(Severity::Error, "ExeFS", format!("copying {name}: {e}")));
                    return None;
                }
            }
        }
    }

    let romfs: Vec<&(String, u32, u64)> = tree.iter().filter(|(p, _, _)| p.starts_with("romfs/")).collect();
    if romfs.is_empty() {
        g.checks.push(Check::new(Severity::Error, "RomFS", "the .nx has no romfs/ folder at the root of this version"));
        return None;
    }
    let mut tops: Vec<&str> = romfs.iter().filter_map(|(p, _, _)| p.strip_prefix("romfs/")).filter_map(|rest| rest.split('/').next()).collect();
    tops.sort_unstable();
    tops.dedup();
    g.romfs_entries = tops.len();
    g.romfs = Some(path.to_path_buf());
    g.nx = Some(path.to_path_buf());
    g.title_id = Some(app.id);
    if !app.title.is_empty() {
        g.title = Some((app.title.clone(), version));
    }
    Some(dir)
}

/// Open a `.nsp` package: unpack its ExeFS, RomFS and control metadata once
/// into the launcher's work folder, then use that folder like a dump. Returns
/// the ExeFS folder and keeps the dump counts for the checks.
/// What came of the package the user pointed at.
enum Package {
    Ready(PathBuf),
    /// The user stopped the unpack; there is nothing to check yet.
    Cancelled,
    Failed,
}

fn use_nsp(
    g: &mut GameData,
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Package {
    let nsp = match ew_format::nsp::Nsp::open(path) {
        Ok(n) => n,
        Err(e) => {
            g.checks.push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return Package::Failed;
        }
    };
    let program = match nsp.program() {
        Ok(p) => p,
        Err(e) => {
            g.checks.push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return Package::Failed;
        }
    };
    g.checks.push(Check::new(Severity::Ok, "Game file", nsp.summary().unwrap_or_else(|e| format!(".nsp, {e:#}"))));

    // The dump is keyed by the package's title and size, and re-done when the
    // package changes, so pointing at a new build never reuses stale files.
    let stamp = format!("{}-{}", nsp.size, modified_stamp(path));
    let Some(work) = Config::work_dir() else { return Package::Failed };
    let dir = work.join("nsp").join(format!("{:016X}-{}", program.title_id, nsp.size));
    let marker = dir.join("dump.txt");
    let cached = std::fs::read_to_string(&marker).ok().and_then(|t| {
        let mut lines = t.lines();
        let same = lines.next() == Some(stamp.as_str());
        let counts = lines.next().and_then(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.parse::<usize>().ok()?, parts.next()?.parse::<usize>().ok()?, parts.next()?.parse::<u64>().ok()?))
        });
        same.then_some(counts).flatten()
    });

    let (exefs_files, romfs_files, romfs_bytes) = match cached {
        Some(c) => c,
        None => {
            g.checks.push(Check::new(
                Severity::Info,
                "Game file",
                format!("unpacking {} into {}, which takes a while the first time", path.display(), dir.display()),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let dumped = match nsp.dump(&dir, progress, cancelled) {
                Ok(d) => d,
                Err(e) if e.downcast_ref::<ew_format::nsp::Cancelled>().is_some() => {
                    // A half-unpacked folder would be mistaken for a complete
                    // one next time, so what this run wrote goes with it.
                    let _ = std::fs::remove_dir_all(&dir);
                    g.checks.push(Check::new(
                        Severity::Info,
                        "Game file",
                        "the unpack was cancelled and the copy was removed; Detect starts it again",
                    ));
                    return Package::Cancelled;
                }
                Err(e) => {
                    g.checks.push(Check::new(Severity::Error, "Game file", format!("unpacking {}: {e:#}", path.display())));
                    return Package::Failed;
                }
            };
            let counts = (dumped.exefs, dumped.romfs, dumped.romfs_bytes);
            if std::fs::write(&marker, format!("{stamp}\n{} {} {}\n", counts.0, counts.1, counts.2)).is_err() {
                g.checks.push(Check::new(Severity::Warn, "Game file", format!("unpack not recorded in {}", marker.display())));
            }
            counts
        }
    };
    if romfs_files > 0 {
        g.checks.push(Check::new(
            Severity::Warn,
            "Game file",
            format!(
                "unpacked below {}: {exefs_files} ExeFS files and {romfs_files} RomFS files ({:.1} GB), so the package is read once and the copy is what the game and the recompiler use; delete that folder to reclaim the space",
                dir.display(),
                romfs_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            ),
        ));
    }
    if let Some(nacp) = dir.join("data").join("control").join("control.nacp").is_file().then(|| dir.join("data").join("control").join("control.nacp")) {
        g.nacp = Some(nacp);
    }
    g.title_id = Some(program.title_id);
    g.romfs = Some(dir.join("romfs"));
    g.nsp = Some(path.to_path_buf());
    g.romfs_entries = std::fs::read_dir(dir.join("romfs")).map(|rd| rd.count()).unwrap_or(0);
    Package::Ready(dir.join("exefs"))
}

/// A file's modification time as nanoseconds, for the unpack marker.
fn modified_stamp(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// [`detect`], reporting how many bytes of a package have been unpacked and
/// how many there are, while there is a package to unpack.
pub fn detect_with(paths: &Paths, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> GameData {
    let mut g = GameData::default();
    let root = non_empty(&paths.game_root);

    // A .nsp package is unpacked once, then used like a dump.
    if let Some(r) = root.as_ref().filter(|r| r.is_file() && ew_format::nsp::Nsp::is_nsp(r)) {
        let r = r.clone();
        match use_nsp(&mut g, &r, progress, cancelled) {
            Package::Ready(exefs) => g.exefs = non_empty(&paths.exefs).or(Some(exefs)),
            // Someone who stops the unpack does not want a page of complaints
            // about the ExeFS it never finished writing.
            Package::Cancelled => return g,
            Package::Failed => {}
        }
        return finish(g, paths);
    }

    // A .nup dump supplies everything itself.
    if let Some(r) = root.as_ref().filter(|r| r.is_file() && ew_format::nup::Nup::is_nup(r)) {
        let r = r.clone();
        if let Some(exefs) = use_nup(&mut g, &r) {
            g.exefs = non_empty(&paths.exefs).or(Some(exefs));
        }
        return finish(g, paths);
    }

    // An .nx archive does the same, in NXFormat's own container.
    if let Some(r) = root.as_ref().filter(|r| r.is_file() && ew_format::nx::is_nx(r)) {
        let r = r.clone();
        if let Some(exefs) = use_nx(&mut g, &r) {
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
        None => g.checks.push(Check::new(Severity::Error, "ExeFS", "not found; choose the folder with your extracted exefs")),
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

    // Is this the build the engine was tested against?
    if let Some(dir) = g.exefs.clone() {
        check_known_dump(&mut g, &dir);
    }

    // RomFS.
    match &g.romfs {
        None => g.checks.push(Check::new(Severity::Error, "RomFS", "not found; the game cannot load its assets")),
        Some(_) if g.nup.is_some() => g.checks.push(Check::new(
            Severity::Ok,
            "RomFS",
            format!("{} top-level entries, read in place from the .nup", g.romfs_entries),
        )),
        // A package is a file, not a folder: reading it with read_dir would
        // report no entries and turn a good archive into an empty RomFS.
        Some(_) if g.nx.is_some() => g.checks.push(Check::new(
            Severity::Ok,
            "RomFS",
            format!("{} top-level entries, read in place from the .nx", g.romfs_entries),
        )),
        Some(dir) => {
            g.romfs_entries = std::fs::read_dir(dir).map(|rd| rd.count()).unwrap_or(0);
            if g.romfs_entries == 0 {
                g.checks.push(Check::new(Severity::Error, "RomFS", format!("{} is empty", dir.display())));
            } else if dir.join("Fs.bin.zst").is_file() {
                g.checks.push(Check::new(Severity::Ok, "RomFS", format!("{} top-level entries, content index present", g.romfs_entries)));
            } else {
                g.checks.push(Check::new(Severity::Warn, "RomFS", "Fs.bin.zst (the content index) is missing"));
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
            Check::new(Severity::Error, "Decoded module", "segment sizes do not match its header; decode again")
        });
    } else {
        g.checks.push(Check::new(
            Severity::Warn,
            "Decoded module",
            "not decoded yet; decode the ZBIC-compressed main module once",
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
        let g = detect_with(&paths, &mut |_, _| {}, &|| false);
        assert_eq!(g.exefs.as_deref(), Some(root.join("dump").join("exefs").as_path()));
        assert_eq!(g.romfs.as_deref(), Some(root.join("dump").join("romfs").as_path()));
        assert_eq!(g.romfs_entries, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tells_a_matching_dump_from_a_different_one() {
        use sha2::Digest as _;
        let dir = std::env::temp_dir().join(format!("ew-dump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main"), b"module bytes").unwrap();
        std::fs::write(dir.join("sdk"), b"other bytes").unwrap();
        let mut sha256 = std::collections::BTreeMap::new();
        sha256.insert("main".to_string(), format!("{:x}", sha2::Sha256::digest(b"module bytes")));
        sha256.insert("sdk".to_string(), format!("{:x}", sha2::Sha256::digest(b"expected sdk")));
        sha256.insert("rtld".to_string(), "00".repeat(32));
        let known = KnownDump {
            program_id: "04007EF01F8AE000".into(),
            version: "update 131072".into(),
            main_build_id: "db93b0125f4850601d61c3edde012f8eb2ecfe77".into(),
            sha256,
        };
        let (same, differ, missing) = compare_dump(&dir, &known);
        assert_eq!(same, vec!["main".to_string()]);
        assert_eq!(differ, vec!["sdk".to_string()]);
        assert_eq!(missing, vec!["rtld".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On a real dump: `EW_NUP=<file.nup> cargo test -p emblemweave -- --ignored`.
    #[test]
    #[ignore]
    fn detects_a_nup_dump() {
        let path = std::env::var("EW_NUP").expect("EW_NUP");
        let g = detect_with(&Paths { game_root: path.clone(), ..Default::default() }, &mut |_, _| {}, &|| false);
        for c in &g.checks {
            eprintln!("{:?} {}: {}", c.severity, c.label, c.detail);
        }
        assert_eq!(g.nup.as_deref(), Some(std::path::Path::new(&path)));
        assert_eq!(g.romfs.as_deref(), Some(std::path::Path::new(&path)));
        assert!(g.exefs.as_ref().is_some_and(|e| e.join("main").is_file() && e.join("sdk").is_file()));
        assert_eq!(g.title.as_ref().map(|t| t.0.as_str()), Some(TITLE_NAME));
        assert!(g.romfs_entries > 0);
    }

    /// On a real package: `EW_NSP=<game.nsp> cargo test -p emblemweave -- --ignored`.
    #[test]
    #[ignore]
    fn detects_an_nsp_package() {
        let path = std::env::var("EW_NSP").expect("EW_NSP");
        let g = detect_with(&Paths { game_root: path.clone(), ..Default::default() }, &mut |_, _| {}, &|| false);
        for c in &g.checks {
            eprintln!("{:?} {}: {}", c.severity, c.label, c.detail);
        }
        assert_eq!(g.nsp.as_deref(), Some(std::path::Path::new(&path)));
        assert!(g.exefs.as_ref().is_some_and(|e| e.join("main").is_file() && e.join("sdk").is_file()));
        assert!(g.romfs.as_ref().is_some_and(|r| r.is_dir()));
        assert!(g.romfs_entries > 0);
        assert_eq!(g.title_id, Some(0x04007EF01F8AE000));
    }

    /// On a real package: `EW_NSP=<game.nsp> cargo test -p emblemweave -- --ignored cancels`.
    #[test]
    #[ignore]
    fn cancels_an_nsp_unpack() {
        let path = std::env::var("EW_NSP").expect("EW_NSP");
        let stop = std::cell::Cell::new(false);
        let mut seen = 0u64;
        let g = detect_with(
            &Paths { game_root: path.clone(), ..Default::default() },
            &mut |done, _total| {
                seen = done;
                if done > (64 << 20) {
                    stop.set(true);
                }
            },
            &|| stop.get(),
        );
        for c in &g.checks {
            eprintln!("{:?} {}: {}", c.severity, c.label, c.detail);
        }
        assert!(seen > 0, "the unpack never reported progress");
        assert!(g.checks.iter().any(|c| c.detail.contains("cancelled")), "no cancellation check");
        assert!(!g.checks.iter().any(|c| c.severity == Severity::Error), "a cancel reported an error: {:?}", g.checks);
        assert!(g.exefs.is_none() && g.romfs.is_none(), "a cancelled unpack was used anyway");
        let dir = Config::work_dir().expect("work dir").join("nsp");
        let left: Vec<_> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        for d in &left {
            eprintln!("left behind: {}", d.display());
        }
        assert!(left.is_empty(), "a cancelled unpack left files behind");
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let paths = Paths { game_root: "Z:/definitely/not/here".into(), ..Default::default() };
        let g = detect_with(&paths, &mut |_, _| {}, &|| false);
        assert_eq!(g.worst(), Severity::Error);
    }

    #[test]
    fn an_nx_archive_is_recognised_and_its_failure_is_explained() {
        let dir = std::env::temp_dir().join(format!("ew-nx-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The magic is enough for the picker to take this for an archive; the
        // rest is junk, so detection has to report the damage instead of panicking.
        let p = dir.join("broken.nx");
        std::fs::write(&p, b"NXAR\0\0\0\0").unwrap();
        let paths = Paths { game_root: p.to_string_lossy().into(), ..Default::default() };
        let g = detect_with(&paths, &mut |_, _| {}, &|| false);
        assert!(g.checks.iter().any(|c| c.label == "Game file" && c.severity == Severity::Error), "{:?}", g.checks);
        assert!(g.exefs.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_warning_names_the_check_that_raised_it() {
        let mut g = GameData::default();
        g.checks.push(Check::new(Severity::Warn, "Title metadata", "control.nacp not found"));
        g.checks.push(Check::new(Severity::Warn, "RomFS", "Fs.bin.zst is missing"));
        g.checks.push(Check::new(Severity::Ok, "ExeFS", "seven modules"));
        assert_eq!(g.worst(), Severity::Warn);
        assert_eq!(g.attention_line().as_deref(), Some("Title metadata: control.nacp not found (+1 more)"));
        assert_eq!(g.attention_text().as_deref(), Some("Title metadata: control.nacp not found\nRomFS: Fs.bin.zst is missing"));
        g.checks.clear();
        g.checks.push(Check::new(Severity::Ok, "ExeFS", "seven modules"));
        assert_eq!(g.attention_line(), None);
        assert_eq!(g.attention_text(), None);
    }

    #[test]
    fn a_long_reason_is_clipped_to_the_card_line() {
        let mut g = GameData::default();
        let path = format!("C:\\Users\\someone\\AppData\\Roaming\\EmblemWeave\\{}", "work\\".repeat(20));
        g.checks.push(Check::new(Severity::Warn, "Game file", format!("unpack not recorded in {path}")));
        let line = g.attention_line().unwrap();
        assert!(line.chars().count() <= CARD_LINE, "{} chars: {line}", line.chars().count());
        assert!(line.ends_with('…'), "{line}");
        // The tooltip keeps the whole message, clipping is only for the card.
        assert!(g.attention_text().unwrap().contains("work\\work\\work\\"));
        assert_eq!(clip("short", CARD_LINE), "short");
    }
}
