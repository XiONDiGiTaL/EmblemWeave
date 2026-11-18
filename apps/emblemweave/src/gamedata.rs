//! Detection and validation of the user's game data.
//!
//! The user points the launcher at a `.nup` dump or at one folder. A `.nup`
//! is used as it is: its small ExeFS is copied once into the launcher's work
//! folder, its RomFS is read in place by the runtime, and its title and
//! version come from its own metadata. Add-on packages in the `.nup` are
//! unpacked once into a managed `dlc/` folder beside that ExeFS. A folder may
//! be a plain dump (with `exefs/` and `romfs/`), an NSP extraction (`exefs/`,
//! `romfs/`, `data/control/control.nacp`), or the ExeFS folder itself. Each
//! location can also be overridden individually. Nothing here writes to the
//! user's files.

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

/// The marker in a managed add-on folder. Its stamp records the package and
/// the selected packs, so a later detection can reuse the files, and a changed
/// package is unpacked again.
const ADDON_MARKER: &str = ".nup-dlc";

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
        Check {
            severity,
            label: label.to_string(),
            detail: detail.into(),
        }
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
    /// The plaintext `.xci` game card, when the game data is one.
    pub xci: Option<PathBuf>,
    /// The base program id, from the `.nup` or `control.nacp`; it names the
    /// save folder in an emulator and selects this game's add-on packs.
    pub title_id: Option<u64>,
    pub exefs: Option<PathBuf>,
    pub romfs: Option<PathBuf>,
    /// The player's add-on content folder (`[aoc] dir`), when packs were found.
    pub dlc: Option<PathBuf>,
    /// The add-on title ids in `dlc`, upper-case hex, sorted.
    pub dlc_ids: Vec<String>,
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
        self.checks
            .iter()
            .filter(|c| matches!(c.severity, Severity::Warn | Severity::Error))
            .collect()
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
        Some(
            open.iter()
                .map(|c| format!("{}: {}", c.label, c.detail))
                .collect::<Vec<_>>()
                .join("\n"),
        )
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

/// Add-ons for a base title live in the next title-id block: `...E000` has
/// `...F001`, `...F002`; `...8BF001` belongs to another game.
fn is_this_titles_addon(base: u64, title: u64) -> bool {
    (base & !0xFFF)
        .checked_add(0x1000)
        .is_some_and(|group| title & !0xFFF == group && title & 0xFFF != 0)
}

/// A pack's RomFS, directly (`<id>/romfs`) or in one numbered folder (`<id>/<n>/romfs`).
fn has_addon_romfs(addon: &Path) -> bool {
    if addon.join("romfs").is_dir() {
        return true;
    }
    std::fs::read_dir(addon).map_or(false, |rd| {
        rd.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit())
                && e.path().join("romfs").is_dir()
        })
    })
}

/// The add-on title ids in `dir` for the base title, upper-case and sorted. A
/// folder with another name, another title's id, or no RomFS is not a pack.
fn scan_dlc(dir: &Path, base: u64) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let id = if name.len() == 16 {
                        u64::from_str_radix(&name, 16).ok()
                    } else {
                        None
                    };
                    (id.is_some_and(|id| is_this_titles_addon(base, id))
                        && has_addon_romfs(&e.path()))
                    .then(|| name.to_uppercase())
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The explicit `Paths.dlc`, else a `dlc/` folder beside the ExeFS, RomFS or
/// game root that holds at least one pack for this title, and the packs found.
fn find_dlc(g: &GameData, paths: &Paths) -> Option<(PathBuf, Vec<String>)> {
    let base = g.title_id.or_else(known_program_id)?;
    if let Some(p) = non_empty(&paths.dlc) {
        let ids = scan_dlc(&p, base);
        return (!ids.is_empty()).then_some((p, ids));
    }
    let mut candidates = Vec::new();
    for side in [g.romfs.as_ref(), g.exefs.as_ref()] {
        if let Some(parent) = side.and_then(|p| p.parent()) {
            candidates.push(parent.join("dlc"));
        }
    }
    if let Some(root) = non_empty(&paths.game_root).filter(|r| r.is_dir()) {
        candidates.push(root.join("dlc"));
    }
    for p in candidates {
        let ids = scan_dlc(&p, base);
        if !ids.is_empty() {
            return Some((p, ids));
        }
    }
    None
}

/// Breadth-first search for a directory entry named `name` under `root`.
fn find_named(root: &Path, name: &str, want_dir: bool, depth: usize) -> Option<PathBuf> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..=depth {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                let is_dir = path.is_dir();
                if entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
                    && is_dir == want_dir
                {
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
    let Ok(known) = serde_json::from_str::<KnownDump>(KNOWN_DUMP) else {
        return;
    };
    let (same, differ, missing) = compare_dump(dir, &known);
    let build = &known.main_build_id[..known.main_build_id.len().min(12)];
    if differ.is_empty() && missing.is_empty() && !same.is_empty() {
        g.checks.push(Check::new(
            Severity::Ok,
            "Game build",
            format!(
                "matches the dump the engine was tested with ({} {}): all {} module hashes agree",
                known.version,
                build,
                same.len()
            ),
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
            format!(
                "could not compare with the tested build: {} not found",
                missing.join(", ")
            ),
        ));
    }
}

fn file_len(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().map(|m| m.len())
}

/// The zstd entries of a `.nup`'s RomFS cannot be read in place by the engine (one zstd frame per file, no random access), so they are
/// written out as files once, into the folder the game reads as `[fs] romfs_overlay` over the package. Returns how many files were written
/// and how many bytes; a file already at its full size is left alone, so an interrupted run resumes where it stopped. `say` gets progress.
pub fn unpack_packed_romfs(
    nup_path: &Path,
    dest: &Path,
    say: &mut dyn FnMut(String),
) -> Result<(usize, u64), String> {
    let nup = ew_format::nup::Nup::open(nup_path)
        .map_err(|e| format!("reading {}: {e}", nup_path.display()))?;
    let pkg = nup
        .program_package()
        .ok_or("the .nup holds no base game or update")?;
    let entries = nup.walk(pkg.romfs);
    std::fs::create_dir_all(dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    let (mut written, mut bytes) = (0usize, 0u64);
    for (n, (rel, index)) in entries.iter().enumerate() {
        let file = &nup.files[*index as usize];
        if !file.compressed {
            continue; // the engine serves stored files from the package itself
        }
        let out = dest.join(rel);
        if std::fs::metadata(&out).map(|m| m.len()).ok() == Some(file.size) {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        let data = nup
            .read(*index)
            .map_err(|e| format!("reading {rel} from {}: {e}", nup_path.display()))?;
        std::fs::write(&out, &data).map_err(|e| format!("writing {}: {e}", out.display()))?;
        written += 1;
        bytes += file.size;
        if n % 500 == 0 {
            say(format!("{n} of {} unpacked", entries.len()));
        }
    }
    Ok((written, bytes))
}

/// The zstd entries of a `.nup`'s RomFS: how many, and how many bytes they carry. `None` when the package has none, which means the
/// engine can serve it where it lies.
pub fn packed_romfs(nup_path: &Path) -> Option<(usize, u64)> {
    let nup = ew_format::nup::Nup::open(nup_path).ok()?;
    let pkg = nup.program_package()?;
    let packed: Vec<&ew_format::nup::FileEntry> = nup
        .walk(pkg.romfs)
        .iter()
        .map(|(_, i)| &nup.files[*i as usize])
        .filter(|f| f.compressed)
        .collect();
    (!packed.is_empty()).then(|| (packed.len(), packed.iter().map(|f| f.size).sum()))
}

/// A file name from a container is data, not a path: reject `..`, a drive
/// prefix and anything else that could leave the managed folder.
fn safe_relative(root: &Path, rel: &str) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = root.to_path_buf();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(name) => out.push(name),
            _ => return None,
        }
    }
    (out != root).then_some(out)
}

/// Remove a managed unpack without reporting cleanup failure; the unpack's
/// original error is the useful one.
fn discard_dir(dest: &Path) {
    let _ = std::fs::remove_dir_all(dest);
}

fn reset_managed_dir(dest: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::remove_dir_all(dest) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("clearing {}: {e}", dest.display()));
        }
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("creating {}: {e}", dest.display()))
}

/// What came of unpacking a `.nup`'s add-on packages.
enum AddonOutcome {
    Ready,
    Cancelled,
}

/// Write every add-on package for `base` into `<dest>/<title id>/romfs`.
/// `progress` counts the bytes of the selected packages, `cancelled` is polled
/// between files, and the marker records the source so a later detection can
/// reuse a finished unpack. A partial unpack is removed, never handed to the
/// engine. Packages for other titles are ignored.
fn unpack_nup_addons(
    nup: &ew_format::nup::Nup,
    base: u64,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<AddonOutcome, String> {
    let packs: Vec<(String, &ew_format::nup::Package)> = nup
        .packages
        .iter()
        .filter(|p| {
            p.kind == ew_format::nup::PackageKind::AddOn && is_this_titles_addon(base, p.title_id)
        })
        .map(|p| (format!("{:016X}", p.title_id), p))
        .collect();

    if packs.is_empty() {
        // A package that has lost its add-ons must not leave last run's packs
        // for the engine to serve as if they were still part of it.
        discard_dir(dest);
        return Ok(AddonOutcome::Ready);
    }

    for (id, pkg) in &packs {
        if pkg.romfs as usize >= nup.dirs.len() {
            return Err(format!("add-on {id} has no RomFS"));
        }
    }
    let total = packs
        .iter()
        .flat_map(|(_, pkg)| nup.walk(pkg.romfs))
        .map(|(_, index)| nup.files[index as usize].size)
        .sum::<u64>();
    let ids: Vec<&str> = packs.iter().map(|(id, _)| id.as_str()).collect();

    let marker = dest.join(ADDON_MARKER);
    let stamp = format!(
        "{:016X}|{}|{}|{}|{}",
        base,
        nup.path.display(),
        nup.size,
        modified_stamp(&nup.path),
        ids.join(",")
    );
    let reusable = std::fs::read_to_string(&marker)
        .map(|s| s.trim() == stamp)
        .unwrap_or(false)
        && ids.iter().all(|id| dest.join(id).join("romfs").is_dir());
    if reusable {
        return Ok(AddonOutcome::Ready);
    }

    reset_managed_dir(dest)?;
    progress(0, total);
    let mut done = 0u64;
    for (id, pkg) in &packs {
        let root = dest.join(id).join("romfs");
        if let Err(e) = std::fs::create_dir_all(&root) {
            discard_dir(dest);
            return Err(format!("creating {}: {e}", root.display()));
        }
        for (rel, index) in nup.walk(pkg.romfs) {
            if cancelled() {
                discard_dir(dest);
                return Ok(AddonOutcome::Cancelled);
            }
            let Some(out) = safe_relative(&root, &rel) else {
                discard_dir(dest);
                return Err(format!("the .nup has an unsafe add-on path {rel:?}"));
            };
            if let Some(parent) = out.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    discard_dir(dest);
                    return Err(format!("creating {}: {e}", parent.display()));
                }
            }
            let data = match nup.read(index) {
                Ok(data) => data,
                Err(e) => {
                    discard_dir(dest);
                    return Err(format!("reading {rel} from {}: {e}", nup.path.display()));
                }
            };
            if let Err(e) = std::fs::write(&out, &data) {
                discard_dir(dest);
                return Err(format!("writing {}: {e}", out.display()));
            }
            done += nup.files[index as usize].size;
            progress(done, total);
            // A progress callback may have set the stop flag; do not go on to
            // finish the marker for an unpack the user just cancelled.
            if cancelled() {
                discard_dir(dest);
                return Ok(AddonOutcome::Cancelled);
            }
        }
    }
    if let Err(e) = std::fs::write(&marker, format!("{stamp}\n")) {
        discard_dir(dest);
        return Err(format!("writing {}: {e}", marker.display()));
    }
    Ok(AddonOutcome::Ready)
}

/// Open a `.nup` dump: copy its ExeFS out once (it is small), use its RomFS in
/// place, and unpack its add-ons. Returns the ExeFS folder.
fn use_nup(
    g: &mut GameData,
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Option<PathBuf> {
    let nup = match ew_format::nup::Nup::open(path) {
        Ok(n) => n,
        Err(e) => {
            g.checks
                .push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return None;
        }
    };
    let Some(pkg) = nup.program_package() else {
        g.checks.push(Check::new(
            Severity::Error,
            "Game file",
            "the .nup holds no base game or update",
        ));
        return None;
    };
    // An update's id is in the same title-id block, so it still gives the
    // add-on group when the package has no base.
    let base = nup
        .packages
        .iter()
        .find(|p| p.kind == ew_format::nup::PackageKind::Base)
        .map_or(pkg.title_id, |p| p.title_id);
    g.checks.push(Check::new(
        Severity::Ok,
        "Game file",
        format!(
            ".nup, title {base:016X} v{}, {} files, checked against its SHA-256",
            pkg.version,
            nup.files.len()
        ),
    ));
    if let Ok(meta) = nup.metadata(pkg) {
        if let (Some(name), Some(version)) = (meta.title(), meta.display_version()) {
            g.title = Some((name, version));
        }
    }
    let exefs = nup.walk(pkg.exefs);
    let dump = Config::work_dir()?
        .join("nup")
        .join(format!("{:016X}-v{}", pkg.title_id, pkg.version));
    let dir = dump.join("exefs");
    let current = exefs
        .iter()
        .all(|(name, i)| file_len(&dir.join(name)) == Some(nup.files[*i as usize].size));
    if !current {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            g.checks.push(Check::new(
                Severity::Error,
                "ExeFS",
                format!("creating {}: {e}", dir.display()),
            ));
            return None;
        }
        for (name, i) in &exefs {
            match nup
                .read(*i)
                .map_err(|e| format!("{e:#}"))
                .and_then(|b| std::fs::write(dir.join(name), b).map_err(|e| e.to_string()))
            {
                Ok(()) => {}
                Err(e) => {
                    g.checks.push(Check::new(
                        Severity::Error,
                        "ExeFS",
                        format!("copying {name}: {e}"),
                    ));
                    return None;
                }
            }
        }
    }
    // The add-on packages in the same container become the engine's [aoc] dir.
    // The base game is already usable if this fails, so an add-on problem is a
    // warning, not a failed detection.
    match unpack_nup_addons(&nup, base, &dump.join("dlc"), progress, cancelled) {
        Ok(AddonOutcome::Ready) => {}
        Ok(AddonOutcome::Cancelled) => g.checks.push(Check::new(
            Severity::Info,
            "Add-on content",
            "the .nup add-on unpack was cancelled; those packs are not used",
        )),
        Err(e) => g.checks.push(Check::new(
            Severity::Warn,
            "Add-on content",
            format!("could not unpack the .nup add-on content ({e}); those packs are not used"),
        )),
    }

    // The engine serves stored files straight from the package and cannot read into a zstd frame, so a packed package is unpacked
    // into the work folder once, at build time, and the game reads that. Say how much disk that takes before it starts.
    let listed: Vec<u32> = nup
        .walk(pkg.exefs)
        .into_iter()
        .chain(nup.walk(pkg.romfs))
        .map(|(_, i)| i)
        .collect();
    let packed: Vec<&ew_format::nup::FileEntry> = listed
        .iter()
        .map(|i| &nup.files[*i as usize])
        .filter(|f| f.compressed)
        .collect();
    if !packed.is_empty() {
        let bytes: u64 = packed.iter().map(|f| f.size).sum();
        g.checks.push(Check::new(
            Severity::Warn,
            "Game file",
            format!(
                "{} of {} files are zstd-compressed: the engine reads packages in place and cannot read into a zstd frame, so the game's files are unpacked into your work folder once, on the next build (about {:.1} GB of disk, and the package itself is never rewritten)",
                packed.len(),
                listed.len(),
                bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            ),
        ));
    }
    g.romfs_entries = nup.dirs[pkg.romfs as usize].children.len();
    g.nup = Some(path.to_path_buf());
    g.title_id = Some(base);
    g.romfs = Some(path.to_path_buf());
    Some(dir)
}

/// Open a `.nx` archive: copy its ExeFS out (it is small), then hand the archive
/// itself to the engine for the RomFS. An NX snapshot is a complete file tree,
/// and a tree packed from an extracted dump keeps `exefs/`, `romfs/` and an
/// optional `data/` at its root. The engine reads NXFormat's blocks directly
/// (`runtime/src/loader/nx.cpp`), so the 29 GB RomFS is not materialised (issue
/// 28).
fn use_nx(g: &mut GameData, path: &Path) -> Package {
    let nx = match ew_format::nx::Nx::open(path) {
        Ok(n) => n,
        Err(e) => {
            g.checks
                .push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return Package::Failed;
        }
    };
    // The launcher is built for one game; take its application when the archive
    // holds several, and the only one when it holds exactly one.
    let wanted = known_program_id();
    let app_index = wanted
        .and_then(|id| nx.application(id))
        .or_else(|| (nx.applications().len() == 1).then_some(0));
    let Some(app_index) = app_index else {
        g.checks.push(Check::new(
            Severity::Error,
            "Game file",
            "the .nx holds no application for this game",
        ));
        return Package::Failed;
    };
    let app = &nx.applications()[app_index];
    let Some(snapshot) = nx.current(app_index) else {
        g.checks.push(Check::new(
            Severity::Error,
            "Game file",
            "the .nx holds no snapshot for this game",
        ));
        return Package::Failed;
    };
    let snap = &nx.snapshots()[snapshot];
    let version = if snap.display_version.is_empty() {
        snap.version.to_string()
    } else {
        snap.display_version.clone()
    };
    let tree = match nx.tree(snapshot) {
        Ok(t) => t,
        Err(e) => {
            g.checks
                .push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return Package::Failed;
        }
    };
    let (compressed, _) = nx.compression();
    g.checks.push(Check::new(
        Severity::Ok,
        "Game file",
        format!(
            ".nx, {} {:016X} v{version}, {} files, checked against its digests",
            if app.title.is_empty() {
                "title"
            } else {
                app.title.as_str()
            },
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
        g.checks.push(Check::new(
            Severity::Error,
            "ExeFS",
            "the .nx has no exefs/ folder at the root of this version",
        ));
        return Package::Failed;
    }
    if !modules.iter().any(|(name, _, _)| name == MAIN_MODULE) {
        g.checks.push(Check::new(
            Severity::Error,
            "ExeFS",
            "the .nx has no exefs/main module",
        ));
        return Package::Failed;
    }
    let Some(work) = Config::work_dir() else {
        return Package::Failed;
    };
    let base = work
        .join("nx")
        .join(format!("{:016X}-v{}", app.id, snap.version));
    let dir = base.join("exefs");
    let current = modules
        .iter()
        .all(|(name, _, size)| file_len(&dir.join(name)) == Some(*size));
    if !current {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            g.checks.push(Check::new(
                Severity::Error,
                "ExeFS",
                format!("creating {}: {e}", dir.display()),
            ));
            return Package::Failed;
        }
        for (name, id, _) in &modules {
            match nx
                .read(*id)
                .map_err(|e| format!("{e:#}"))
                .and_then(|b| std::fs::write(dir.join(name), b).map_err(|e| e.to_string()))
            {
                Ok(()) => {}
                Err(e) => {
                    g.checks.push(Check::new(
                        Severity::Error,
                        "ExeFS",
                        format!("copying {name}: {e}"),
                    ));
                    return Package::Failed;
                }
            }
        }
    }

    let romfs: Vec<&(String, u32, u64)> = tree
        .iter()
        .filter(|(p, _, _)| p.starts_with("romfs/"))
        .collect();
    if romfs.is_empty() {
        g.checks.push(Check::new(
            Severity::Error,
            "RomFS",
            "the .nx has no romfs/ folder at the root of this version",
        ));
        return Package::Failed;
    }
    let mut tops: Vec<&str> = romfs
        .iter()
        .filter_map(|(p, _, _)| p.strip_prefix("romfs/"))
        .filter_map(|rest| rest.split('/').next())
        .collect();
    tops.sort_unstable();
    tops.dedup();
    g.romfs_entries = tops.len();

    // The engine reads NXFormat's container itself (runtime/src/loader/nx.cpp): the archive is handed over as it is, a RomFS read is a
    // block read of the file, decoded and checked when the game asks for it, and nothing is copied out first (issue 28).
    g.romfs = Some(path.to_path_buf());
    g.nx = Some(path.to_path_buf());
    g.title_id = Some(app.id);
    if !app.title.is_empty() {
        g.title = Some((app.title.clone(), version));
    }
    Package::Ready(dir)
}

/// What came of the package the user pointed at.
enum Package {
    Ready(PathBuf),
    /// The user stopped the unpack; there is nothing to check yet.
    Cancelled,
    Failed,
}

/// The two single-file packages whose NCAs can be unpacked without keys. They
/// differ only in how the NCAs are found; the unpack and the resulting folder
/// layout are identical.
enum Container {
    Nsp(ew_format::nsp::Nsp),
    Xci(ew_format::xci::Xci),
}

impl Container {
    fn size(&self) -> u64 {
        match self {
            Container::Nsp(n) => n.size,
            Container::Xci(x) => x.size,
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Container::Nsp(_) => ".nsp",
            Container::Xci(_) => ".xci",
        }
    }
    fn folder(&self) -> &'static str {
        match self {
            Container::Nsp(_) => "nsp",
            Container::Xci(_) => "xci",
        }
    }
    fn program(&self) -> anyhow::Result<ew_format::nsp::Nca> {
        match self {
            Container::Nsp(n) => n.program(),
            Container::Xci(x) => x.program(),
        }
    }
    fn summary(&self) -> anyhow::Result<String> {
        match self {
            Container::Nsp(n) => n.summary(),
            Container::Xci(x) => x.summary(),
        }
    }
    fn dump(
        &self,
        dest: &Path,
        progress: &mut dyn FnMut(u64, u64),
        cancelled: &dyn Fn() -> bool,
    ) -> anyhow::Result<ew_format::nsp::Dumped> {
        match self {
            Container::Nsp(n) => n.dump(dest, progress, cancelled),
            Container::Xci(x) => x.dump(dest, progress, cancelled),
        }
    }
}

/// Unpack a package's ExeFS, RomFS and control metadata once into the
/// launcher's work folder, then use that folder like a dump. Returns the ExeFS
/// folder and keeps the dump counts for the checks.
fn use_package(
    g: &mut GameData,
    path: &Path,
    package: Container,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Package {
    let program = match package.program() {
        Ok(p) => p,
        Err(e) => {
            g.checks
                .push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            return Package::Failed;
        }
    };
    g.checks.push(Check::new(
        Severity::Ok,
        "Game file",
        package
            .summary()
            .unwrap_or_else(|e| format!("{}, {e:#}", package.kind())),
    ));

    // The dump is keyed by the package's title and size, and re-done when the
    // package changes, so pointing at a new build never reuses stale files.
    let stamp = format!("{}-{}", package.size(), modified_stamp(path));
    let Some(work) = Config::work_dir() else {
        return Package::Failed;
    };
    let dir =
        work.join(package.folder())
            .join(format!("{:016X}-{}", program.title_id, package.size()));
    let marker = dir.join("dump.txt");
    let cached = std::fs::read_to_string(&marker).ok().and_then(|t| {
        let mut lines = t.lines();
        let same = lines.next() == Some(stamp.as_str());
        let counts = lines.next().and_then(|l| {
            let mut parts = l.split_whitespace();
            Some((
                parts.next()?.parse::<usize>().ok()?,
                parts.next()?.parse::<usize>().ok()?,
                parts.next()?.parse::<u64>().ok()?,
            ))
        });
        same.then_some(counts).flatten()
    });

    let (exefs_files, romfs_files, romfs_bytes) = match cached {
        Some(c) => c,
        None => {
            g.checks.push(Check::new(
                Severity::Info,
                "Game file",
                format!(
                    "unpacking {} into {}, which takes a while the first time",
                    path.display(),
                    dir.display()
                ),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let dumped = match package.dump(&dir, progress, cancelled) {
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
                    g.checks.push(Check::new(
                        Severity::Error,
                        "Game file",
                        format!("unpacking {}: {e:#}", path.display()),
                    ));
                    return Package::Failed;
                }
            };
            let counts = (dumped.exefs, dumped.romfs, dumped.romfs_bytes);
            if std::fs::write(
                &marker,
                format!("{stamp}\n{} {} {}\n", counts.0, counts.1, counts.2),
            )
            .is_err()
            {
                g.checks.push(Check::new(
                    Severity::Warn,
                    "Game file",
                    format!("unpack not recorded in {}", marker.display()),
                ));
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
    if let Some(nacp) = dir
        .join("data")
        .join("control")
        .join("control.nacp")
        .is_file()
        .then(|| dir.join("data").join("control").join("control.nacp"))
    {
        g.nacp = Some(nacp);
    }
    g.title_id = Some(program.title_id);
    g.romfs = Some(dir.join("romfs"));
    match package {
        Container::Nsp(_) => g.nsp = Some(path.to_path_buf()),
        Container::Xci(_) => g.xci = Some(path.to_path_buf()),
    }
    g.romfs_entries = std::fs::read_dir(dir.join("romfs"))
        .map(|rd| rd.count())
        .unwrap_or(0);
    Package::Ready(dir.join("exefs"))
}

fn use_nsp(
    g: &mut GameData,
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Package {
    match ew_format::nsp::Nsp::open(path) {
        Ok(nsp) => use_package(g, path, Container::Nsp(nsp), progress, cancelled),
        Err(e) => {
            g.checks
                .push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            Package::Failed
        }
    }
}

fn use_xci(
    g: &mut GameData,
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Package {
    match ew_format::xci::Xci::open(path) {
        Ok(xci) => use_package(g, path, Container::Xci(xci), progress, cancelled),
        Err(e) => {
            g.checks
                .push(Check::new(Severity::Error, "Game file", format!("{e:#}")));
            Package::Failed
        }
    }
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
/// The title id of the dump this launcher was built for (data/dump_hashes.json).
pub fn known_program_id() -> Option<u64> {
    let known: KnownDump = serde_json::from_str(KNOWN_DUMP).ok()?;
    u64::from_str_radix(known.program_id.trim_start_matches("0x"), 16).ok()
}

pub fn detect_with(
    paths: &Paths,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> GameData {
    let mut g = GameData::default();
    let root = non_empty(&paths.game_root);

    // A .nsp package is unpacked once, then used like a dump.
    if let Some(r) = root
        .as_ref()
        .filter(|r| r.is_file() && ew_format::nsp::Nsp::is_nsp(r))
    {
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
    if let Some(r) = root
        .as_ref()
        .filter(|r| r.is_file() && ew_format::nup::Nup::is_nup(r))
    {
        let r = r.clone();
        if let Some(exefs) = use_nup(&mut g, &r, progress, cancelled) {
            g.exefs = non_empty(&paths.exefs).or(Some(exefs));
        }
        return finish(g, paths);
    }

    // An .nx archive does the same, in NXFormat's own container.
    if let Some(r) = root
        .as_ref()
        .filter(|r| r.is_file() && ew_format::nx::is_nx(r))
    {
        let r = r.clone();
        match use_nx(&mut g, &r) {
            Package::Ready(exefs) => g.exefs = non_empty(&paths.exefs).or(Some(exefs)),
            Package::Cancelled => return g,
            Package::Failed => {}
        }
        return finish(g, paths);
    }

    // A plaintext .xci game-card image: its secure partition holds NCAs in the
    // same form as an .nsp, so it is unpacked with the same code (issue 28's
    // container work made this a small addition).
    if let Some(r) = root
        .as_ref()
        .filter(|r| r.is_file() && ew_format::xci::Xci::is_xci(r))
    {
        let r = r.clone();
        match use_xci(&mut g, &r, progress, cancelled) {
            Package::Ready(exefs) => g.exefs = non_empty(&paths.exefs).or(Some(exefs)),
            Package::Cancelled => return g,
            Package::Failed => {}
        }
        return finish(g, paths);
    }

    if let Some(r) = &root {
        if !r.is_dir() {
            g.checks.push(Check::new(
                Severity::Error,
                "Game folder",
                format!("{} does not exist", r.display()),
            ));
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
        let near = g
            .exefs
            .as_ref()
            .and_then(|e| e.parent())
            .map(|p| p.join("romfs"));
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
        let base = g
            .exefs
            .as_ref()
            .and_then(|e| e.parent())
            .map(Path::to_path_buf)
            .or(root.clone())?;
        let nsp = base.join("data").join("control").join("control.nacp");
        if nsp.is_file() {
            return Some(nsp);
        }
        find_named(&base, "control.nacp", false, 3).or_else(|| {
            root.as_ref()
                .and_then(|r| find_named(r, "control.nacp", false, 3))
        })
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
        candidates
            .into_iter()
            .find(|p| p.join("text.bin").is_file())
    });

    // ExeFS contents.
    match &g.exefs {
        None => g.checks.push(Check::new(
            Severity::Error,
            "ExeFS",
            "not found; choose the folder with your extracted exefs",
        )),
        Some(dir) => match ew_format::ExeFs::scan_dir(dir) {
            Ok(exefs) => {
                if let Ok(Some(npdm)) = exefs.read_npdm() {
                    g.npdm = Some(npdm);
                }
                match exefs.find(MAIN_MODULE) {
                    Some(m) => g.checks.push(Check::new(
                        Severity::Ok,
                        "ExeFS",
                        format!(
                            "{} modules; `main` is `{}` ({})",
                            exefs.modules.len(),
                            m.module_name,
                            m.compressions.join(" + ")
                        ),
                    )),
                    None => g.checks.push(Check::new(
                        Severity::Error,
                        "ExeFS",
                        "no `main` module in this ExeFS",
                    )),
                }
                g.modules = Some(exefs);
            }
            Err(e) => g.checks.push(Check::new(
                Severity::Error,
                "ExeFS",
                format!("{}: {e:#}", dir.display()),
            )),
        },
    }

    // Is this the build the engine was tested against?
    if let Some(dir) = g.exefs.clone() {
        check_known_dump(&mut g, &dir);
    }

    // RomFS.
    match &g.romfs {
        None => g.checks.push(Check::new(
            Severity::Error,
            "RomFS",
            "not found; the game cannot load its assets",
        )),
        Some(_) if g.nup.is_some() => g.checks.push(Check::new(
            Severity::Ok,
            "RomFS",
            format!(
                "{} top-level entries, read in place from the .nup",
                g.romfs_entries
            ),
        )),
        // A package is a file, not a folder: reading it with read_dir would
        // report no entries and turn a good archive into an empty RomFS.
        Some(_) if g.nx.is_some() => g.checks.push(Check::new(
            Severity::Ok,
            "RomFS",
            format!(
                "{} top-level entries, read in place from the .nx",
                g.romfs_entries
            ),
        )),
        Some(dir) => {
            g.romfs_entries = std::fs::read_dir(dir).map(|rd| rd.count()).unwrap_or(0);
            if g.romfs_entries == 0 {
                g.checks.push(Check::new(
                    Severity::Error,
                    "RomFS",
                    format!("{} is empty", dir.display()),
                ));
            } else if dir.join("Fs.bin.zst").is_file() {
                g.checks.push(Check::new(
                    Severity::Ok,
                    "RomFS",
                    format!(
                        "{} top-level entries, content index present",
                        g.romfs_entries
                    ),
                ));
            } else {
                g.checks.push(Check::new(
                    Severity::Warn,
                    "RomFS",
                    "Fs.bin.zst (the content index) is missing",
                ));
            }
        }
    }

    // Add-on content: the folder the engine serves as [aoc] dir. A pack is
    // `<title id>/romfs` (or one numbered folder down); the explicit path wins
    // over the dlc/ folder beside the ExeFS, RomFS or game root.
    let explicit_dlc = non_empty(&paths.dlc);
    if let Some((dir, ids)) = find_dlc(&g, paths) {
        g.checks.push(Check::new(
            Severity::Ok,
            "Add-on content",
            format!(
                "{} pack(s) in {}: {}",
                ids.len(),
                dir.display(),
                ids.join(", ")
            ),
        ));
        g.dlc = Some(dir);
        g.dlc_ids = ids;
    } else if let Some(dir) = explicit_dlc {
        g.checks.push(Check::new(
            Severity::Warn,
            "Add-on content",
            format!(
                "{} has no <title id>/romfs pack; the game will start without DLC",
                dir.display()
            ),
        ));
    }

    // Title metadata: the .nup's own, or control.nacp.
    let nacp_title = if g.nup.is_some() {
        g.title.clone()
    } else {
        g.nacp.as_deref().and_then(ew_boot::read_nacp)
    };
    match nacp_title {
        Some((name, version)) => {
            let sev = if name == TITLE_NAME { Severity::Ok } else { Severity::Warn };
            g.checks.push(Check::new(sev, "Title", format!("{name}, version {version}")));
            g.title = Some((name, version));
        }
        None => g.checks.push(Check::new(
            Severity::Warn,
            "Title metadata",
            "control.nacp not found: the title name and display version stay blank, and neither the build nor the game needs it. An extracted .nsp keeps it at data\\control\\control.nacp",
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
            g.segments.push(SegmentCheck {
                name,
                actual: file_len(&dir.join(format!("{name}.bin"))),
                declared,
            });
        }
        let ok = g.segments.iter().all(|s| s.ok());
        g.checks.push(if ok {
            Check::new(
                Severity::Ok,
                "Decoded module",
                "every segment matches the size its header declares",
            )
        } else if main.is_none() {
            Check::new(
                Severity::Warn,
                "Decoded module",
                "present, but there is no ExeFS to check it against",
            )
        } else {
            Check::new(
                Severity::Error,
                "Decoded module",
                "segment sizes do not match its header; decode again",
            )
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
        let paths = Paths {
            game_root: root.to_string_lossy().into(),
            ..Default::default()
        };
        let g = detect_with(&paths, &mut |_, _| {}, &|| false);
        assert_eq!(
            g.exefs.as_deref(),
            Some(root.join("dump").join("exefs").as_path())
        );
        assert_eq!(
            g.romfs.as_deref(),
            Some(root.join("dump").join("romfs").as_path())
        );
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
        sha256.insert(
            "main".to_string(),
            format!("{:x}", sha2::Sha256::digest(b"module bytes")),
        );
        sha256.insert(
            "sdk".to_string(),
            format!("{:x}", sha2::Sha256::digest(b"expected sdk")),
        );
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
        let g = detect_with(
            &Paths {
                game_root: path.clone(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        for c in &g.checks {
            eprintln!("{:?} {}: {}", c.severity, c.label, c.detail);
        }
        assert_eq!(g.nup.as_deref(), Some(std::path::Path::new(&path)));
        assert_eq!(g.romfs.as_deref(), Some(std::path::Path::new(&path)));
        assert!(g
            .exefs
            .as_ref()
            .is_some_and(|e| e.join("main").is_file() && e.join("sdk").is_file()));
        assert_eq!(g.title.as_ref().map(|t| t.0.as_str()), Some(TITLE_NAME));
        assert!(g.romfs_entries > 0);
    }

    /// On a real package: `EW_NSP=<game.nsp> cargo test -p emblemweave -- --ignored`.
    #[test]
    #[ignore]
    fn detects_an_nsp_package() {
        let path = std::env::var("EW_NSP").expect("EW_NSP");
        let g = detect_with(
            &Paths {
                game_root: path.clone(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        for c in &g.checks {
            eprintln!("{:?} {}: {}", c.severity, c.label, c.detail);
        }
        assert_eq!(g.nsp.as_deref(), Some(std::path::Path::new(&path)));
        assert!(g
            .exefs
            .as_ref()
            .is_some_and(|e| e.join("main").is_file() && e.join("sdk").is_file()));
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
            &Paths {
                game_root: path.clone(),
                ..Default::default()
            },
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
        assert!(
            g.checks.iter().any(|c| c.detail.contains("cancelled")),
            "no cancellation check"
        );
        assert!(
            !g.checks.iter().any(|c| c.severity == Severity::Error),
            "a cancel reported an error: {:?}",
            g.checks
        );
        assert!(
            g.exefs.is_none() && g.romfs.is_none(),
            "a cancelled unpack was used anyway"
        );
        let dir = Config::work_dir().expect("work dir").join("nsp");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        for d in &left {
            eprintln!("left behind: {}", d.display());
        }
        assert!(left.is_empty(), "a cancelled unpack left files behind");
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let paths = Paths {
            game_root: "Z:/definitely/not/here".into(),
            ..Default::default()
        };
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
        let paths = Paths {
            game_root: p.to_string_lossy().into(),
            ..Default::default()
        };
        let g = detect_with(&paths, &mut |_, _| {}, &|| false);
        assert!(
            g.checks
                .iter()
                .any(|c| c.label == "Game file" && c.severity == Severity::Error),
            "{:?}",
            g.checks
        );
        assert!(g.exefs.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `.nx` reader that issue 28 asked for is not only in `ew-format`: the
    /// picker has to route the archive to `use_nx` before it looks for a folder.
    #[test]
    fn an_nx_archive_is_taken_as_a_package_for_issue_28() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/ew-format/tests/nx-fixtures/minimal_valid.nx");
        assert!(fixture.is_file(), "missing {}", fixture.display());
        let g = detect_with(
            &Paths {
                game_root: fixture.to_string_lossy().into(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        assert!(
            g.checks
                .iter()
                .any(|c| c.detail.contains(".nx") && c.detail.contains("exefs")),
            "the .nx was not opened as a package: {:?}",
            g.checks
        );
        assert!(
            g.exefs.is_none(),
            "the empty fixture should not produce an ExeFS"
        );
    }

    #[test]
    fn a_card_header_is_taken_as_an_xci_package() {
        let dir = std::env::temp_dir().join(format!("ew-xci-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("broken.xci");
        let mut image = vec![0u8; 0x200];
        image[0x100..0x104].copy_from_slice(b"HEAD");
        image[0x130..0x138].copy_from_slice(&0x200u64.to_le_bytes());
        std::fs::write(&p, &image).unwrap();
        let paths = Paths {
            game_root: p.to_string_lossy().into(),
            ..Default::default()
        };
        let g = detect_with(&paths, &mut |_, _| {}, &|| false);
        assert!(
            g.checks
                .iter()
                .any(|c| c.label == "Game file" && c.severity == Severity::Error),
            "{:?}",
            g.checks
        );
        assert!(g.exefs.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_warning_names_the_check_that_raised_it() {
        let mut g = GameData::default();
        g.checks.push(Check::new(
            Severity::Warn,
            "Title metadata",
            "control.nacp not found",
        ));
        g.checks
            .push(Check::new(Severity::Warn, "RomFS", "Fs.bin.zst is missing"));
        g.checks
            .push(Check::new(Severity::Ok, "ExeFS", "seven modules"));
        assert_eq!(g.worst(), Severity::Warn);
        assert_eq!(
            g.attention_line().as_deref(),
            Some("Title metadata: control.nacp not found (+1 more)")
        );
        assert_eq!(
            g.attention_text().as_deref(),
            Some("Title metadata: control.nacp not found\nRomFS: Fs.bin.zst is missing")
        );
        g.checks.clear();
        g.checks
            .push(Check::new(Severity::Ok, "ExeFS", "seven modules"));
        assert_eq!(g.attention_line(), None);
        assert_eq!(g.attention_text(), None);
    }

    #[test]
    fn a_long_reason_is_clipped_to_the_card_line() {
        let mut g = GameData::default();
        let path = format!(
            "C:\\Users\\someone\\AppData\\Roaming\\EmblemWeave\\{}",
            "work\\".repeat(20)
        );
        g.checks.push(Check::new(
            Severity::Warn,
            "Game file",
            format!("unpack not recorded in {path}"),
        ));
        let line = g.attention_line().unwrap();
        assert!(
            line.chars().count() <= CARD_LINE,
            "{} chars: {line}",
            line.chars().count()
        );
        assert!(line.ends_with('…'), "{line}");
        // The tooltip keeps the whole message, clipping is only for the card.
        assert!(g.attention_text().unwrap().contains("work\\work\\work\\"));
        assert_eq!(clip("short", CARD_LINE), "short");
    }

    /// A tiny format-1 container with a base package, one add-on for this
    /// game and one add-on for another title. Each package has its own RomFS
    /// with one file.
    fn write_addon_nup(path: &Path) {
        use sha2::{Digest, Sha256};

        let names = ["romfs", "base.bin", "dlc.bin", "other.bin"];
        let mut pool = Vec::new();
        let mut name_at = std::collections::HashMap::new();
        for name in names {
            name_at.insert(name, (pool.len() as u32, name.len() as u32));
            pool.extend_from_slice(name.as_bytes());
        }

        let n_pkg = 3usize;
        let n_dir = 3usize;
        let n_file = 3usize;
        let pkg_at = 0x30usize;
        let dir_at = pkg_at + n_pkg * 0x40;
        let file_at = dir_at + n_dir * 0x18;
        let child_at = file_at + n_file * 0x50;
        let pool_off = child_at + n_file * 4;
        let toc_size = pool_off + pool.len();
        let data_off = 0x80 + toc_size;
        let payloads: [&[u8]; 3] = [b"BASE", b"GOOD", b"BAD"];
        let mut payload = Vec::new();
        let mut file_offsets = [0u64; 3];
        for (i, bytes) in payloads.iter().enumerate() {
            file_offsets[i] = (data_off + payload.len()) as u64;
            payload.extend_from_slice(bytes);
        }
        let total = data_off + payload.len();

        let mut toc = vec![0u8; toc_size];
        toc[0..4].copy_from_slice(b"TOC\x01");
        toc[4..8].copy_from_slice(&(n_pkg as u32).to_le_bytes());
        toc[8..16].copy_from_slice(&(n_dir as u64).to_le_bytes());
        toc[16..24].copy_from_slice(&(n_file as u64).to_le_bytes());
        toc[24..32].copy_from_slice(&(pool_off as u64).to_le_bytes());
        toc[32..40].copy_from_slice(&(pool.len() as u64).to_le_bytes());

        let packages = [
            (0x0400_7EF0_1F8A_E000u64, 0u8, 0u32, 0u32, 0u32, 1u32),
            (0x0400_7EF0_1F8A_F001u64, 2u8, 1u32, 1u32, 1u32, 0u32),
            (0x0400_7EF0_1F8B_F001u64, 2u8, 2u32, 2u32, 2u32, 0u32),
        ];
        for (i, (title, kind, root, exefs, romfs, version)) in packages.iter().enumerate() {
            let at = pkg_at + i * 0x40;
            toc[at..at + 8].copy_from_slice(&title.to_le_bytes());
            toc[at + 8..at + 12].copy_from_slice(&root.to_le_bytes());
            toc[at + 12..at + 16].copy_from_slice(&exefs.to_le_bytes());
            toc[at + 16..at + 20].copy_from_slice(&romfs.to_le_bytes());
            toc[at + 20..at + 24].copy_from_slice(&version.to_le_bytes());
            toc[at + 24] = *kind;
        }

        let dirs: [(&str, u32, u32); 3] = [("romfs", 0, 1), ("romfs", 1, 1), ("romfs", 2, 1)];
        for (i, (name, first, count)) in dirs.iter().enumerate() {
            let at = dir_at + i * 0x18;
            let (off, len) = name_at[*name];
            toc[at..at + 4].copy_from_slice(&off.to_le_bytes());
            toc[at + 4..at + 8].copy_from_slice(&len.to_le_bytes());
            toc[at + 8..at + 12].copy_from_slice(&first.to_le_bytes());
            toc[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        }

        let files: [(&str, usize); 3] = [("base.bin", 0), ("dlc.bin", 1), ("other.bin", 2)];
        for (i, (name, payload_index)) in files.iter().enumerate() {
            let at = file_at + i * 0x50;
            let bytes = payloads[*payload_index];
            let (off, len) = name_at[*name];
            toc[at..at + 4].copy_from_slice(&off.to_le_bytes());
            toc[at + 4..at + 8].copy_from_slice(&len.to_le_bytes());
            toc[at + 8..at + 16].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
            toc[at + 16..at + 24].copy_from_slice(&file_offsets[*payload_index].to_le_bytes());
            toc[at + 24..at + 32].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
            toc[at + 32..at + 64].copy_from_slice(&Sha256::digest(bytes));
            toc[at + 64] = 0;
        }

        for (i, file) in [0u32, 1, 2].into_iter().enumerate() {
            let at = child_at + i * 4;
            toc[at..at + 4].copy_from_slice(&(n_dir as u32 + file).to_le_bytes());
        }
        toc[pool_off..].copy_from_slice(&pool);

        let mut head = vec![0u8; 0x80];
        head[0..4].copy_from_slice(b"NUP\x01");
        head[4..6].copy_from_slice(&0x80u16.to_le_bytes());
        head[6..8].copy_from_slice(&1u16.to_le_bytes());
        head[8..16].copy_from_slice(&0x80u64.to_le_bytes());
        head[16..24].copy_from_slice(&(toc_size as u64).to_le_bytes());
        head[24..56].copy_from_slice(&Sha256::digest(&toc));
        head[56..64].copy_from_slice(&(data_off as u64).to_le_bytes());
        head[0x40..0x48].copy_from_slice(&(total as u64).to_le_bytes());
        head[0x48..0x4C].copy_from_slice(&(n_pkg as u32).to_le_bytes());

        let mut out = head;
        out.extend(toc);
        out.extend(payload);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn a_synthetic_nup_unpacks_only_this_titles_addons_and_reuses_them() {
        let dir = std::env::temp_dir().join(format!("ew-nup-dlc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("addons.nup");
        write_addon_nup(&path);
        let nup = ew_format::nup::Nup::open(&path).unwrap();

        let base = 0x0400_7EF0_1F8A_E000u64;
        let dest = dir.join("dlc");
        let mut progress = Vec::new();
        let first = unpack_nup_addons(
            &nup,
            base,
            &dest,
            &mut |done, total| progress.push((done, total)),
            &|| false,
        )
        .unwrap();
        assert!(matches!(first, AddonOutcome::Ready));
        assert_eq!(
            std::fs::read(dest.join("04007EF01F8AF001/romfs/dlc.bin")).unwrap(),
            b"GOOD".as_slice()
        );
        assert!(!dest.join("04007EF01F8BF001").exists());
        assert_eq!(progress.first(), Some(&(0, 4)));
        assert_eq!(progress.last(), Some(&(4, 4)));
        assert_eq!(scan_dlc(&dest, base), vec!["04007EF01F8AF001".to_string()]);
        let marker = std::fs::read_to_string(dest.join(ADDON_MARKER)).unwrap();
        assert!(marker.contains("04007EF01F8AF001"));
        assert!(!marker.contains("04007EF01F8BF001"));

        // A cancelled callback would make an unpack fail; Ready here proves the
        // marker and the pack folders were enough to reuse the first run.
        let reused = unpack_nup_addons(
            &nup,
            base,
            &dest,
            &mut |_, _| panic!("a reused unpack reported progress"),
            &|| true,
        )
        .unwrap();
        assert!(matches!(reused, AddonOutcome::Ready));

        let cancelled_dest = dir.join("cancelled-dlc");
        let cancelled =
            unpack_nup_addons(&nup, base, &cancelled_dest, &mut |_, _| {}, &|| true).unwrap();
        assert!(matches!(cancelled, AddonOutcome::Cancelled));
        assert!(!cancelled_dest.exists());

        // The folder-dump path still sees valid packs without a marker, and
        // the title filter keeps another game's folder out of the list.
        let folder = dir.join("folder-dlc");
        std::fs::create_dir_all(folder.join("04007EF01F8AF002/romfs")).unwrap();
        std::fs::create_dir_all(folder.join("04007EF01F8BF002/romfs")).unwrap();
        assert_eq!(
            scan_dlc(&folder, base),
            vec!["04007EF01F8AF002".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_on_packs_are_found_by_title_id_and_romfs() {
        let dir = std::env::temp_dir().join(format!("ew-dlc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dlc = dir.join("dlc");
        std::fs::create_dir_all(dlc.join("04007EF01F8AF002/0/romfs")).unwrap();
        std::fs::create_dir_all(dlc.join("04007EF01F8AF001/romfs")).unwrap();
        // Not a pack: another name, a title id with no RomFS, and another game's pack.
        std::fs::create_dir_all(dlc.join("not-a-title/romfs")).unwrap();
        std::fs::create_dir_all(dlc.join("04007EF01F8AF003/empty")).unwrap();
        std::fs::create_dir_all(dlc.join("04007EF01F8BF001/romfs")).unwrap();
        assert_eq!(
            scan_dlc(&dlc, 0x0400_7EF0_1F8A_E000),
            vec![
                "04007EF01F8AF001".to_string(),
                "04007EF01F8AF002".to_string()
            ]
        );
        // The auto-detection finds dlc/ beside the ExeFS/RomFS; an explicit path wins.
        std::fs::create_dir_all(dir.join("exefs")).unwrap();
        std::fs::create_dir_all(dir.join("romfs")).unwrap();
        let g = GameData {
            exefs: Some(dir.join("exefs")),
            romfs: Some(dir.join("romfs")),
            ..Default::default()
        };
        let paths = Paths {
            game_root: dir.to_string_lossy().into(),
            ..Default::default()
        };
        assert_eq!(
            find_dlc(&g, &paths),
            Some((
                dlc,
                vec![
                    "04007EF01F8AF001".to_string(),
                    "04007EF01F8AF002".to_string()
                ]
            ))
        );
        let other = dir.join("other-dlc");
        std::fs::create_dir_all(other.join("04007EF01F8AF004/romfs")).unwrap();
        let explicit = Paths {
            dlc: other.to_string_lossy().into(),
            ..Default::default()
        };
        assert_eq!(
            find_dlc(&g, &explicit),
            Some((other, vec!["04007EF01F8AF004".to_string()]))
        );
        // A folder holding only another game's pack is not accepted.
        let wrong = dir.join("wrong-dlc");
        std::fs::create_dir_all(wrong.join("04007EF01F8BF002/romfs")).unwrap();
        let wrong_path = Paths {
            dlc: wrong.to_string_lossy().into(),
            ..Default::default()
        };
        assert_eq!(find_dlc(&g, &wrong_path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
