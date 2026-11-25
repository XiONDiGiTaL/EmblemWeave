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
use sha2::{Digest, Sha256};
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

/// The title as the container stores it, with typographic punctuation folded
/// to the ASCII form used by `TITLE_NAME`. The shipped `.nup` spells the colon
/// as U+A789, which is the same title to a reader but not to `str::eq`.
fn canonical_title(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| match c {
            '\u{A789}' | '\u{FF1A}' | '\u{2236}' => ':',
            c => c,
        })
        .collect()
}

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
/// and how many bytes. A file already at its full size is left alone, so a run of the same package resumes where it stopped; the caller
/// is what guarantees a destination folder only ever holds the package its identity names. `say` gets progress.
///
/// Each file is written into `<dest>/.emblemweave-parts` and renamed into place, so a file at its final path is always complete and the
/// length check below is a safe resume test. The staging folder lives inside the folder being written: it is covered by the same
/// ownership decision that let the write touch `dest` at all, so it can never collide with a sibling of a user-named destination.
fn unpack_packed_romfs(
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
    let staging = dest.join(".emblemweave-parts");
    if let Err(e) = std::fs::remove_dir_all(&staging) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("clearing {}: {e}", staging.display()));
        }
    }
    let (mut written, mut bytes) = (0usize, 0u64);
    for (n, (rel, index)) in entries.iter().enumerate() {
        let file = &nup.files[*index as usize];
        if !file.compressed {
            continue; // the engine serves stored files from the package itself
        }
        // A path from the container is data, not a path: it must stay inside `dest`.
        let Some(out) = safe_relative(dest, rel) else {
            return Err(format!("the .nup has an unsafe packed path {rel:?}"));
        };
        if std::fs::metadata(&out).map(|m| m.len()).ok() == Some(file.size) {
            continue;
        }
        let data = nup
            .read(*index)
            .map_err(|e| format!("reading {rel} from {}: {e}", nup_path.display()))?;
        let staged = staging.join(rel);
        if let Some(parent) = staged.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        std::fs::write(&staged, &data).map_err(|e| format!("writing {}: {e}", staged.display()))?;
        // `rename` refuses an existing target on Windows, so a file left at the destination goes first.
        match std::fs::remove_file(&out) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("replacing {}: {e}", out.display())),
        }
        // Ensure the destination parent directory exists before renaming.
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        std::fs::rename(&staged, &out).map_err(|e| format!("writing {}: {e}", out.display()))?;
        written += 1;
        bytes += file.size;
        if n % 500 == 0 {
            say(format!("{n} of {} unpacked", entries.len()));
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
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

/// The hidden files the launcher keeps in an overlay: the identity of the package it was written for, and the marker that the write
/// finished. Neither is game data.
const OVERLAY_SOURCE: &str = ".source";
const OVERLAY_WRITTEN: &str = ".written";

/// What an overlay is written from. A path, a packed-file count and a total byte count are not enough: a replaced package that happens to
/// line up reuses the old folder, and a packed path the new package no longer holds keeps its old file, which then wins over the package
/// because the overlay is merged last. The key therefore covers the package's own size and modification time and, for every packed file,
/// its path, decompressed size and entry SHA-256. It changes whenever any packed content does.
#[derive(Debug, Clone)]
pub struct OverlayIdentity {
    pub key: String,
    pub count: usize,
    pub bytes: u64,
}

/// What one call to [`write_romfs_overlay`] did.
#[derive(Debug, Clone, Copy)]
pub struct OverlayWrite {
    pub written: usize,
    pub written_bytes: u64,
    pub reused: bool,
}

fn read_trim(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_string())
}

pub fn overlay_identity(nup_path: &Path) -> Result<OverlayIdentity, String> {
    let nup = ew_format::nup::Nup::open(nup_path)
        .map_err(|e| format!("reading {}: {e}", nup_path.display()))?;
    let Some(pkg) = nup.program_package() else {
        return Err(format!(
            "{} holds no base game or update, so it has no RomFS to write",
            nup_path.display()
        ));
    };
    // fefw.ini spells the path with `/`, the build with `\`; both must give one key, or Play re-checks the overlay forever.
    let shown = nup_path.components().collect::<PathBuf>().display().to_string();
    let modified = std::fs::metadata(nup_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut digest = Sha256::new();
    digest.update(b"emblemweave romfs overlay 2\0");
    digest.update(shown.as_bytes());
    digest.update(nup.size.to_le_bytes());
    digest.update(modified.to_le_bytes());
    let (mut count, mut bytes) = (0usize, 0u64);
    for (rel, index) in nup.walk(pkg.romfs) {
        let file = &nup.files[index as usize];
        if !file.compressed {
            continue;
        }
        count += 1;
        bytes += file.size;
        digest.update(rel.as_bytes());
        digest.update([0u8]);
        digest.update(file.size.to_le_bytes());
        digest.update(file.sha256);
    }
    let sum: [u8; 32] = digest.finalize().into();
    let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    Ok(OverlayIdentity {
        key: format!("{shown}|{count} files|{bytes} bytes|{hex}"),
        count,
        bytes,
    })
}

/// Whether `dest` holds a finished write of exactly the package at `nup_path`. Cheap (the package's TOC and metadata, no game file is
/// hashed or read), so Play can check it before starting instead of serving files an older package wrote.
pub fn overlay_is_current(nup_path: &Path, dest: &Path) -> bool {
    overlay_identity(nup_path).is_ok_and(|id| {
        id.count > 0 && read_trim(&dest.join(OVERLAY_WRITTEN)).as_deref() == Some(id.key.as_str())
    })
}

/// Bring `dest` up to date for the package at `nup_path`, and say what happened through `say`.
///
/// A finished write of the same package (the marker matches the identity) is reused as it is. A write of the same package that did not
/// finish keeps its files and resumes. Anything else, including a folder written by an older launcher with no identity, is cleared first:
/// the folder then holds only this package's files, so a path the old package packed can never shadow the one the game reads now.
pub fn write_romfs_overlay(
    nup_path: &Path,
    dest: &Path,
    say: &mut dyn FnMut(String),
) -> Result<OverlayWrite, String> {
    let id = overlay_identity(nup_path)?;
    if id.count == 0 {
        return Ok(OverlayWrite {
            written: 0,
            written_bytes: 0,
            reused: false,
        });
    }
    let source = dest.join(OVERLAY_SOURCE);
    if read_trim(&dest.join(OVERLAY_WRITTEN)).as_deref() == Some(id.key.as_str()) {
        say(format!(
            "{} packed files; using the overlay already written in {}",
            id.count,
            dest.display()
        ));
        return Ok(OverlayWrite {
            written: 0,
            written_bytes: 0,
            reused: true,
        });
    }
    say(format!(
        "{} files of this package ({:.1} GB) are packed with zstd, which the engine cannot read in place, so those files are written once into {} and read from there; the package is used as it is for everything else",
        id.count,
        id.bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        dest.display()
    ));
    if read_trim(&source).as_deref() != Some(id.key.as_str()) {
        reset_managed_dir(dest)?;
        std::fs::write(&source, &id.key)
            .map_err(|e| format!("writing {}: {e}", source.display()))?;
    }
    let (written, written_bytes) = unpack_packed_romfs(nup_path, dest, say)?;
    std::fs::write(dest.join(OVERLAY_WRITTEN), &id.key)
        .map_err(|e| format!("writing {}: {e}", dest.join(OVERLAY_WRITTEN).display()))?;
    say(format!(
        "{written} packed files written ({:.1} GB) to {}; the package supplies the rest",
        written_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        dest.display()
    ));
    Ok(OverlayWrite {
        written,
        written_bytes,
        reused: false,
    })
}

/// `--unpack-romfs`: write the same overlay the launcher serves, with the same identity marker and staging, so a folder that held an
/// older package is rebuilt instead of resumed by matching sizes, and paths the older package packed are cleared.
///
/// The overlay writer clears `dest` when the package's identity changes. That is safe for the launcher's own `<engine-root>/work/...`
/// folder, but not for a folder the user names on the command line. A folder counts as the launcher's only when it is empty or carries
/// an overlay marker (`.source` or `.written`); anything else is refused rather than cleared.
pub fn unpack_romfs_command(
    nup_path: &Path,
    dest: &Path,
    say: &mut dyn FnMut(String),
) -> Result<OverlayWrite, String> {
    if dest.exists() {
        if !dest.is_dir() {
            return Err(format!(
                "refusing to write {}: it exists and is not a folder",
                dest.display()
            ));
        }
        let managed = dest.join(OVERLAY_SOURCE).is_file() || dest.join(OVERLAY_WRITTEN).is_file();
        if !managed {
            let mut entries =
                std::fs::read_dir(dest).map_err(|e| format!("reading {}: {e}", dest.display()))?;
            if entries.next().is_some() {
                return Err(format!(
                    "refusing to write {}: it is not empty and has no EmblemWeave overlay marker (.source or .written), so it may \
                     be a folder EmblemWeave did not write; clearing it for this package would delete its files. Point --unpack-romfs \
                     at a new or empty folder, or remove that folder yourself first",
                    dest.display()
                ));
            }
        }
    }
    write_romfs_overlay(nup_path, dest, say)
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

/// Files and bytes under `root`, so a managed folder's disk cost can be reported and reclaimed deliberately. A missing or unreadable
/// root is `(0, 0)`; symbolic links are not followed.
pub fn folder_disk_use(root: &Path) -> (usize, u64) {
    fn walk(dir: &Path, files: &mut usize, bytes: &mut u64) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(&entry.path(), files, bytes);
            } else if meta.is_file() {
                *files += 1;
                *bytes += meta.len();
            }
        }
    }
    let (mut files, mut bytes) = (0, 0);
    walk(root, &mut files, &mut bytes);
    (files, bytes)
}

fn reset_managed_dir(dest: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::remove_dir_all(dest) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("clearing {}: {e}", dest.display()));
        }
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("creating {}: {e}", dest.display()))
}

/// Removes the managed DLC folder unless the unpack armed it off after writing
/// its completion marker. Every early return, error and panic after the folder
/// is reset therefore leaves no partial pack for a later detection to find.
struct UnpackGuard<'a> {
    dest: &'a Path,
    armed: bool,
}

impl Drop for UnpackGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            discard_dir(self.dest);
        }
    }
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
            p.kind == ew_format::nup::PackageKind::AddOn
                && is_this_titles_addon(base, p.title_id)
                && (p.romfs as usize) < nup.dirs.len()
        })
        .map(|p| (format!("{:016X}", p.title_id), p))
        .collect();

    if packs.is_empty() {
        // A package that has lost its add-ons, or whose only add-on has no
        // RomFS, must not leave last run's packs for the engine to serve as if
        // they were still part of it.
        discard_dir(dest);
        return Ok(AddonOutcome::Ready);
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

    if let Err(e) = reset_managed_dir(dest) {
        discard_dir(dest);
        return Err(e);
    }
    let mut cleanup = UnpackGuard { dest, armed: true };
    progress(0, total);
    let mut done = 0u64;
    for (id, pkg) in &packs {
        let root = dest.join(id).join("romfs");
        if let Err(e) = std::fs::create_dir_all(&root) {
            return Err(format!("creating {}: {e}", root.display()));
        }
        for (rel, index) in nup.walk(pkg.romfs) {
            if cancelled() {
                return Ok(AddonOutcome::Cancelled);
            }
            let Some(out) = safe_relative(&root, &rel) else {
                return Err(format!("the .nup has an unsafe add-on path {rel:?}"));
            };
            if let Some(parent) = out.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(format!("creating {}: {e}", parent.display()));
                }
            }
            let data = match nup.read(index) {
                Ok(data) => data,
                Err(e) => return Err(format!("reading {rel} from {}: {e}", nup.path.display())),
            };
            if let Err(e) = std::fs::write(&out, &data) {
                return Err(format!("writing {}: {e}", out.display()));
            }
            done += nup.files[index as usize].size;
            progress(done, total);
            // A progress callback may have set the stop flag; do not go on to
            // finish the marker for an unpack the user just cancelled.
            if cancelled() {
                return Ok(AddonOutcome::Cancelled);
            }
        }
    }
    if let Err(e) = std::fs::write(&marker, format!("{stamp}\n")) {
        return Err(format!("writing {}: {e}", marker.display()));
    }
    cleanup.armed = false;
    Ok(AddonOutcome::Ready)
}

/// Write every data NCA for `base` from a `.nsp` or `.xci` into
/// `<dest>/<title id>/romfs`, the same layout `.nup` add-ons use. Reuses a
/// complete unpack by its source stamp; on any failure or cancellation the
/// cleanup guard removes the folder, so detection cannot serve a stale pack.
fn unpack_package_addons(
    package: &Container,
    base: u64,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<AddonOutcome, String> {
    let mut packs = std::collections::BTreeMap::new();
    // A failed scan must not leave a previous complete unpack where detection
    // would serve it as part of this package.
    let addons = package.addons().map_err(|e| {
        discard_dir(dest);
        format!("{e:#}")
    })?;
    for nca in addons {
        if is_this_titles_addon(base, nca.title_id) && nca.has_romfs() {
            packs.entry(nca.title_id).or_insert(nca);
        }
    }
    if packs.is_empty() {
        discard_dir(dest);
        return Ok(AddonOutcome::Ready);
    }

    let source = package.path();
    let ids: Vec<String> = packs.keys().map(|id| format!("{id:016X}")).collect();
    let marker = dest.join(ADDON_MARKER);
    let stamp = format!(
        "{:016X}|{}|{}|{}|{}",
        base,
        source.display(),
        package.size(),
        modified_stamp(source),
        ids.join(",")
    );
    let reusable = std::fs::read_to_string(&marker)
        .map(|s| s.trim() == stamp)
        .unwrap_or(false)
        && ids.iter().all(|id| dest.join(id).join("romfs").is_dir());
    if reusable {
        return Ok(AddonOutcome::Ready);
    }

    if let Err(e) = reset_managed_dir(dest) {
        discard_dir(dest);
        return Err(e);
    }
    let mut cleanup = UnpackGuard { dest, armed: true };
    for (id, nca) in &packs {
        let root = dest.join(format!("{id:016X}")).join("romfs");
        if let Err(e) = package.dump_addon(nca, &root, progress, cancelled) {
            if e.downcast_ref::<ew_format::nsp::Cancelled>().is_some() {
                return Ok(AddonOutcome::Cancelled);
            }
            return Err(format!("{e:#}"));
        }
    }
    if let Err(e) = std::fs::write(&marker, format!("{stamp}\n")) {
        return Err(format!("writing {}: {e}", marker.display()));
    }
    cleanup.armed = false;
    Ok(AddonOutcome::Ready)
}

/// Whether a copied ExeFS folder holds exactly the modules the package names. Lengths alone let a same-version repack, a truncated
/// copy or an edited file pass as current; compare the package's SHA-256 for each module instead. `files` carries the name, size and
/// entry hash from the package.
fn exefs_copy_matches(dir: &Path, files: &[(String, u64, [u8; 32])]) -> bool {
    files.iter().all(|(name, size, sha)| {
        safe_relative(dir, name).is_some_and(|path| {
            std::fs::read(path)
                .is_ok_and(|data| data.len() as u64 == *size && Sha256::digest(&data)[..] == sha[..])
        })
    })
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
    let wanted: Vec<(String, u64, [u8; 32])> = exefs
        .iter()
        .map(|(name, i)| {
            let file = &nup.files[*i as usize];
            (name.clone(), file.size, file.sha256)
        })
        .collect();
    let current = exefs_copy_matches(&dir, &wanted);
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
            let Some(out) = safe_relative(&dir, name) else {
                g.checks.push(Check::new(
                    Severity::Error,
                    "ExeFS",
                    format!("the .nup has an unsafe ExeFS path {name:?}"),
                ));
                return None;
            };
            match nup
                .read(*i)
                .map_err(|e| format!("{e:#}"))
                .and_then(|b| std::fs::write(&out, b).map_err(|e| e.to_string()))
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
            // The build does this automatically. It is useful context, not a
            // problem the player has to resolve, so it must not turn the Game
            // data card into "Needs attention" after a successful build.
            Severity::Info,
            "Game file",
            format!(
                "{} of {} files are zstd-compressed: the engine reads packages in place and cannot read into a zstd frame, so the game's files are unpacked into your work folder once, during the build (about {:.1} GB of disk, and the package itself is never rewritten)",
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
    fn path(&self) -> &Path {
        match self {
            Container::Nsp(n) => &n.path,
            Container::Xci(x) => &x.path,
        }
    }
    fn addons(&self) -> anyhow::Result<Vec<ew_format::nsp::Nca>> {
        match self {
            Container::Nsp(n) => n.addons(),
            Container::Xci(x) => x.addons(),
        }
    }
    fn dump_addon(
        &self,
        nca: &ew_format::nsp::Nca,
        dest: &Path,
        progress: &mut dyn FnMut(u64, u64),
        cancelled: &dyn Fn() -> bool,
    ) -> anyhow::Result<(usize, u64)> {
        ew_format::nsp::dump_nca_romfs(self.path(), nca, dest, progress, cancelled)
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
            // A completed unpack is normal build state; the note explains the
            // disk copy and how to reclaim it, and is not an open problem.
            Severity::Info,
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
    // Add-on content is optional: a package with none clears an older unpack,
    // and a failure leaves the base game usable but never a partial DLC folder.
    match unpack_package_addons(&package, program.title_id, &dir.join("dlc"), progress, cancelled) {
        Ok(AddonOutcome::Ready) => {}
        Ok(AddonOutcome::Cancelled) => g.checks.push(Check::new(
            Severity::Info,
            "Add-on content",
            "the add-on unpack was cancelled; those packs are not used",
        )),
        Err(e) => g.checks.push(Check::new(
            Severity::Warn,
            "Add-on content",
            format!("could not unpack the add-on content ({e}); those packs are not used"),
        )),
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
            let name = canonical_title(&name);
            // A display-name difference is cosmetic: the title id, ExeFS hashes
            // and RomFS checks are what establish that this is the right game.
            let sev = if name == TITLE_NAME { Severity::Ok } else { Severity::Info };
            g.checks.push(Check::new(sev, "Title", format!("{name}, version {version}")));
            g.title = Some((name, version));
        }
        None => g.checks.push(Check::new(
            // The message itself says the build and game do not need this. It
            // only leaves the displayed name and version blank.
            Severity::Info,
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
                // This is a limit of the check, not a second problem: when the
                // ExeFS is actually required, its own error is already present.
                Severity::Info,
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
            "Game build",
            "a different build of the game",
        ));
        g.checks
            .push(Check::new(Severity::Warn, "RomFS", "Fs.bin.zst is missing"));
        g.checks
            .push(Check::new(Severity::Ok, "ExeFS", "seven modules"));
        assert_eq!(g.worst(), Severity::Warn);
        assert_eq!(
            g.attention_line().as_deref(),
            Some("Game build: a different build of the game (+1 more)")
        );
        assert_eq!(
            g.attention_text().as_deref(),
            Some("Game build: a different build of the game\nRomFS: Fs.bin.zst is missing")
        );
        g.checks.clear();
        g.checks
            .push(Check::new(Severity::Ok, "ExeFS", "seven modules"));
        assert_eq!(g.attention_line(), None);
        assert_eq!(g.attention_text(), None);
    }

    #[test]
    fn a_typographic_colon_in_the_title_is_the_same_game() {
        assert_eq!(canonical_title("Fire Emblem꞉ Fortune's Weave"), TITLE_NAME);
        assert_eq!(canonical_title("  Fire Emblem: Fortune's Weave  "), TITLE_NAME);
        assert_ne!(canonical_title("Fire Emblem: Three Houses"), TITLE_NAME);
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

    /// A zstd frame with one raw block: `ruzstd` decodes it back to `payload`. The tests need a package `Nup::read` accepts, without
    /// depending on an encoder.
    fn zstd_raw(payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 256, "the test frame uses a one-byte content size");
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, payload.len() as u8];
        let block = ((payload.len() as u32) << 3) | 1; // last block, raw, this size
        frame.extend_from_slice(&block.to_le_bytes()[..3]);
        frame.extend_from_slice(payload);
        frame
    }

    /// A real `.nup` with one base package whose RomFS holds `files`, each packed as one zstd frame. The bytes are deliberately not game
    /// data; the point is that `Nup::read` decompresses and hash-checks them like a real package's packed files.
    fn write_overlay_nup(path: &Path, files: &[(&str, &[u8])]) {
        use sha2::{Digest, Sha256};
        let mut names = vec!["root".to_string(), "romfs".to_string()];
        names.extend(files.iter().map(|(name, _)| (*name).to_string()));
        let mut pool = Vec::new();
        let mut at = std::collections::HashMap::new();
        for name in &names {
            at.insert(name.clone(), (pool.len() as u32, name.len() as u32));
            pool.extend_from_slice(name.as_bytes());
        }
        let frames: Vec<Vec<u8>> = files.iter().map(|(_, data)| zstd_raw(data)).collect();
        let (n_pkg, n_dir, n_file) = (1usize, 2usize, files.len());
        let toc_size =
            0x30 + n_pkg * 0x40 + n_dir * 0x18 + n_file * 0x50 + (1 + n_file) * 4 + pool.len();
        let data_off = 0x80 + toc_size;
        let mut payload = Vec::new();
        let mut offsets = Vec::new();
        for frame in &frames {
            offsets.push((data_off + payload.len()) as u64);
            payload.extend_from_slice(frame);
        }
        let total = data_off + payload.len();

        let mut toc = vec![0u8; toc_size];
        toc[0..4].copy_from_slice(b"TOC\x01");
        toc[4..8].copy_from_slice(&(n_pkg as u32).to_le_bytes());
        toc[8..16].copy_from_slice(&(n_dir as u64).to_le_bytes());
        toc[16..24].copy_from_slice(&(n_file as u64).to_le_bytes());
        let pool_off = toc_size - pool.len();
        toc[24..32].copy_from_slice(&(pool_off as u64).to_le_bytes());
        toc[32..40].copy_from_slice(&(pool.len() as u64).to_le_bytes());

        let pkg = 0x30;
        toc[pkg..pkg + 8].copy_from_slice(&0x0400_7EF0_1F8A_E000u64.to_le_bytes());
        toc[pkg + 16..pkg + 20].copy_from_slice(&1u32.to_le_bytes()); // romfs directory

        let dir_at = pkg + 0x40;
        // root (dir 0): one child, the romfs directory.
        toc[dir_at + 12..dir_at + 16].copy_from_slice(&1u32.to_le_bytes());
        // romfs (dir 1): its files, which follow the romfs directory in the child list.
        let romfs_dir = dir_at + 0x18;
        toc[romfs_dir + 4..romfs_dir + 8].copy_from_slice(&5u32.to_le_bytes()); // "romfs"
        toc[romfs_dir + 8..romfs_dir + 12].copy_from_slice(&1u32.to_le_bytes());
        toc[romfs_dir + 12..romfs_dir + 16].copy_from_slice(&(n_file as u32).to_le_bytes());

        let file_at = dir_at + n_dir * 0x18;
        for (i, (name, data)) in files.iter().enumerate() {
            let e = file_at + i * 0x50;
            let (off, len) = at[*name];
            toc[e..e + 4].copy_from_slice(&off.to_le_bytes());
            toc[e + 4..e + 8].copy_from_slice(&len.to_le_bytes());
            toc[e + 8..e + 16].copy_from_slice(&(data.len() as u64).to_le_bytes());
            toc[e + 16..e + 24].copy_from_slice(&offsets[i].to_le_bytes());
            toc[e + 24..e + 32].copy_from_slice(&(frames[i].len() as u64).to_le_bytes());
            toc[e + 32..e + 64].copy_from_slice(&Sha256::digest(data));
            toc[e + 64] = 1; // one zstd frame
        }
        let child_at = file_at + n_file * 0x50;
        toc[child_at..child_at + 4].copy_from_slice(&1u32.to_le_bytes()); // root -> romfs
        for i in 0..n_file {
            let at_child = child_at + (1 + i) * 4;
            toc[at_child..at_child + 4]
                .copy_from_slice(&(n_dir as u32 + i as u32).to_le_bytes());
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

    /// A package replaced under the same path by one with the same packed count and the same total decompressed bytes must not reuse the
    /// overlay: the old key (path, count, bytes) matched this case, so the stale file was accepted and the engine read the old voice.
    #[test]
    fn a_replaced_package_with_the_same_count_and_bytes_rebuilds_the_overlay() {
        let dir = std::env::temp_dir().join(format!("ew-overlay-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        let dest = dir.join("work").join("romfs-overlay");

        write_overlay_nup(&nup, &[("voice.bin", b"VOICE\n")]);
        assert_eq!(packed_romfs(&nup), Some((1, 6)));
        let before = overlay_identity(&nup).unwrap();
        let first = write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!first.reused);
        assert_eq!(std::fs::read(dest.join("voice.bin")).unwrap(), b"VOICE\n");

        write_overlay_nup(&nup, &[("voice.bin", b"VOICX\n")]);
        assert_eq!(packed_romfs(&nup), Some((1, 6)));
        let after = overlay_identity(&nup).unwrap();
        assert_ne!(before.key, after.key, "the identity ignored the content");
        assert!(
            !overlay_is_current(&nup, &dest),
            "an overlay written for the old content still looked current"
        );
        let second = write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!second.reused);
        assert_eq!(std::fs::read(dest.join("voice.bin")).unwrap(), b"VOICX\n");

        assert!(overlay_is_current(&nup, &dest));
        let third = write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert!(third.reused);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Play reads the package path from fefw.ini (`/`); the build wrote the marker for the `\` path.
    #[cfg(windows)]
    #[test]
    fn an_overlay_written_for_a_backslash_path_is_current_for_the_slash_path() {
        let dir = std::env::temp_dir().join(format!("ew-overlay-slashes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        let dest = dir.join("work").join("romfs-overlay");
        write_overlay_nup(&nup, &[("voice.bin", b"VOICE\n")]);
        write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();

        let slashed = PathBuf::from(nup.display().to_string().replace('\\', "/"));
        assert_ne!(slashed.as_os_str(), nup.as_os_str());
        assert!(
            overlay_is_current(&slashed, &dest),
            "the overlay the build wrote did not count for the path fefw.ini holds"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A newer package with fewer packed files must not leave the old ones in the overlay: the overlay is merged last, so an orphan path
    /// would win over the package.
    #[test]
    fn an_overlay_from_another_package_leaves_no_orphans() {
        let dir = std::env::temp_dir().join(format!("ew-overlay-orphans-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        let dest = dir.join("work").join("romfs-overlay");

        write_overlay_nup(
            &nup,
            &[("voice.bin", b"VOICE\n"), ("movie.bin", b"MOVIE\n")],
        );
        write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert!(dest.join("movie.bin").is_file());

        write_overlay_nup(&nup, &[("voice.bin", b"VOICE\n")]);
        write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(dest.join("voice.bin")).unwrap(), b"VOICE\n");
        assert!(
            !dest.join("movie.bin").exists(),
            "the old package's movie stayed in the overlay"
        );
        assert!(dest.join(OVERLAY_WRITTEN).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--unpack-romfs` takes a folder the user names. A dotted name used to produce `dest.with_extension("parts")`, which was then
    /// deleted, so an unrelated sibling folder could disappear; the tool now keeps its staging inside the destination and touches
    /// nothing beside it.
    #[test]
    fn unpacking_to_a_dotted_folder_does_not_touch_its_sibling() {
        let dir = std::env::temp_dir().join(format!("ew-unpack-dotted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        write_overlay_nup(&nup, &[("voice.bin", b"VOICE\n")]);
        // `out.rel` used to map to this folder through with_extension("parts").
        let sibling = dir.join("out.parts");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("keep.txt"), b"user data").unwrap();

        let dest = dir.join("out.rel");
        let write = unpack_romfs_command(&nup, &dest, &mut |_| {}).unwrap();
        assert_eq!((write.written, write.written_bytes), (1, 6));
        assert!(!write.reused);
        assert_eq!(std::fs::read(dest.join("voice.bin")).unwrap(), b"VOICE\n");
        assert_eq!(
            std::fs::read(sibling.join("keep.txt")).unwrap(),
            b"user data",
            "a folder beside the destination was modified"
        );
        assert!(!sibling.join("voice.bin").exists());
        assert!(dest.join(OVERLAY_WRITTEN).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The overlay writer clears `dest` when the package identity changes. The command must not do that to a non-empty folder it
    /// cannot prove the launcher wrote.
    #[test]
    fn the_command_refuses_a_folder_it_did_not_write() {
        let dir = std::env::temp_dir().join(format!("ew-unpack-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        write_overlay_nup(&nup, &[("voice.bin", b"VOICE\n")]);
        let dest = dir.join("user-folder");
        std::fs::create_dir_all(dest.join("sub")).unwrap();
        std::fs::write(dest.join("keep.txt"), b"user data").unwrap();

        let err = unpack_romfs_command(&nup, &dest, &mut |_| {}).unwrap_err();
        assert!(
            err.contains("no EmblemWeave overlay marker") && err.contains("new or empty folder"),
            "the refusal did not say why: {err}"
        );
        assert_eq!(std::fs::read(dest.join("keep.txt")).unwrap(), b"user data");
        assert!(!dest.join(OVERLAY_SOURCE).exists());
        assert!(!dest.join("voice.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--unpack-romfs` used to resume into an existing folder by matching file sizes, so a replaced package with the same packed count
    /// and total bytes was served from the old files, and paths the old package packed were never removed.
    #[test]
    fn the_command_clears_orphans_and_rebuilds_a_replaced_package() {
        let dir = std::env::temp_dir().join(format!("ew-unpack-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        let dest = dir.join("romfs-unpacked");

        write_overlay_nup(
            &nup,
            &[("voice.bin", b"VOICE\n"), ("movie.bin", b"MOVIE\n")],
        );
        let first = unpack_romfs_command(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!first.reused);
        assert_eq!(first.written, 2);
        assert!(dest.join("movie.bin").is_file());

        // Fewer packed files: the old movie path must not survive and shadow the package.
        write_overlay_nup(&nup, &[("voice.bin", b"VOICE\n")]);
        let second = unpack_romfs_command(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!second.reused);
        assert!(
            !dest.join("movie.bin").exists(),
            "the old package's movie stayed in the overlay"
        );

        // Same count and bytes, different content: the old file must be replaced, not reused by its size.
        write_overlay_nup(&nup, &[("voice.bin", b"VOICX\n")]);
        assert_eq!(packed_romfs(&nup), Some((1, 6)));
        let third = unpack_romfs_command(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!third.reused);
        assert_eq!(third.written, 1);
        assert_eq!(std::fs::read(dest.join("voice.bin")).unwrap(), b"VOICX\n");

        // The same package again is a finished overlay and is reused.
        let fourth = unpack_romfs_command(&nup, &dest, &mut |_| {}).unwrap();
        assert!(fourth.reused);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A copied ExeFS is reused on the next detect, so its modules must match the package by hash, not only by length: a
    /// same-version repack or an edited/truncated copy has to be rewritten.
    #[test]
    fn an_exefs_copy_is_current_only_when_every_module_hash_matches() {
        let dir = std::env::temp_dir().join(format!("ew-exefs-reuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sha = |b: &[u8]| -> [u8; 32] {
            let mut out = [0u8; 32];
            out.copy_from_slice(Sha256::digest(b).as_slice());
            out
        };
        let main = b"main module bytes".to_vec();
        let npdm = b"npdm".to_vec();
        let files = vec![
            ("main".to_string(), main.len() as u64, sha(&main)),
            ("main.npdm".to_string(), npdm.len() as u64, sha(&npdm)),
        ];
        std::fs::write(dir.join("main"), &main).unwrap();
        std::fs::write(dir.join("main.npdm"), &npdm).unwrap();
        assert!(exefs_copy_matches(&dir, &files));

        // Same length, different bytes: the case a length check accepted.
        std::fs::write(dir.join("main"), vec![b'X'; main.len()]).unwrap();
        assert!(!exefs_copy_matches(&dir, &files));
        std::fs::write(dir.join("main"), &main).unwrap();

        std::fs::write(dir.join("main.npdm"), &npdm[..2]).unwrap();
        assert!(!exefs_copy_matches(&dir, &files));
        std::fs::remove_file(dir.join("main.npdm")).unwrap();
        assert!(!exefs_copy_matches(&dir, &files));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_disk_use_counts_nested_files_and_bytes() {
        let dir = std::env::temp_dir().join(format!("ew-disk-use-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
        std::fs::write(dir.join("one.bin"), b"1234").unwrap();
        std::fs::write(dir.join("a").join("b").join("two.bin"), b"12").unwrap();
        assert_eq!(folder_disk_use(&dir), (2, 6));
        assert_eq!(folder_disk_use(&dir.join("missing")), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    static NUP_DETECT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Break the add-on package's RomFS index in the TOC and re-sign the TOC.
    /// The container still opens; only its add-on extraction is impossible.
    fn break_addon_romfs(path: &Path) {
        use sha2::{Digest, Sha256};
        let mut bytes = std::fs::read(path).unwrap();
        let toc_size = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
        let mut toc = bytes[0x80..0x80 + toc_size].to_vec();
        // Package 0 is the base; package 1 is the valid add-on. Its romfs field
        // is at +0x10 in the 0x40-byte package record, after the 0x30-byte TOC header.
        let romfs = 0x30 + 0x40 + 0x10;
        toc[romfs..romfs + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[0x80..0x80 + toc_size].copy_from_slice(&toc);
        bytes[24..56].copy_from_slice(&Sha256::digest(&toc));
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_synthetic_nup_detection_uses_only_this_titles_addons() {
        let _lock = NUP_DETECT.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ew-nup-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("addons.nup");
        write_addon_nup(&path);

        let base = 0x0400_7EF0_1F8A_E000u64;
        let dump = Config::work_dir()
            .expect("work dir")
            .join("nup")
            .join(format!("{base:016X}-v1"));
        let dest = dump.join("dlc");
        let _ = std::fs::remove_dir_all(&dump);
        let g = detect_with(
            &Paths {
                game_root: path.to_string_lossy().into(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        assert_eq!(g.dlc.as_deref(), Some(dest.as_path()), "{:?}", g.checks);
        assert_eq!(g.dlc_ids, vec!["04007EF01F8AF001".to_string()]);
        assert_eq!(
            std::fs::read(dest.join("04007EF01F8AF001/romfs/dlc.bin")).unwrap(),
            b"GOOD".as_slice()
        );
        assert!(!dest.join("04007EF01F8BF001").exists());

        // A failed add-on extraction must also clear the previous complete
        // folder: otherwise detection would serve the old pack as current.
        break_addon_romfs(&path);
        let g = detect_with(
            &Paths {
                game_root: path.to_string_lossy().into(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        assert!(g.dlc.is_none(), "a stale add-on folder survived: {:?}", g.checks);
        assert!(!dest.exists(), "the failed unpack left {}", dest.display());

        let _ = std::fs::remove_dir_all(&dump);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn xci_hfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        for (name, _) in files {
            offsets.push(names.len() as u32);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
        }
        let mut out = b"HFS0".to_vec();
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut data_off = 0u64;
        for (i, (_, data)) in files.iter().enumerate() {
            out.extend_from_slice(&data_off.to_le_bytes());
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(&offsets[i].to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&[0u8; 8]);
            out.extend_from_slice(&[0u8; 32]);
            data_off += data.len() as u64;
        }
        out.extend_from_slice(&names);
        for (_, data) in files {
            out.extend_from_slice(data);
        }
        out
    }

    /// A plaintext card image whose secure partition holds `files`.
    fn xci_image(files: &[(&str, &[u8])]) -> Vec<u8> {
        let secure = xci_hfs0(files);
        let root = xci_hfs0(&[("secure", &secure)]);
        let mut image = vec![0u8; 0x200];
        image[0x100..0x104].copy_from_slice(b"HEAD");
        image[0x130..0x138].copy_from_slice(&0x200u64.to_le_bytes());
        image.extend_from_slice(&root);
        image
    }

    fn nsp_pfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        for (name, _) in files {
            offsets.push(names.len() as u32);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
        }
        let mut out = b"PFS0".to_vec();
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut data_off = 0u64;
        for (i, (_, data)) in files.iter().enumerate() {
            out.extend_from_slice(&data_off.to_le_bytes());
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(&offsets[i].to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            data_off += data.len() as u64;
        }
        out.extend_from_slice(&names);
        for (_, data) in files {
            out.extend_from_slice(data);
        }
        out
    }

    /// A plaintext program NCA with a minimal ExeFS; enough for the NSP
    /// detection path to dump and find its `main`.
    fn nsp_exefs_nca() -> Vec<u8> {
        let exefs = nsp_pfs0(&[("main", b"main-bytes"), ("main.npdm", b"npdm")]);
        let section_at = 0xc00u64;
        let data_at = 0x200u64;
        let section_end = (section_at + data_at + exefs.len() as u64 + 0x1ff) & !0x1ff;
        let mut nca = vec![0u8; section_end as usize];
        nca[0x200..0x204].copy_from_slice(b"NCA3");
        nca[0x205] = 0; // program
        nca[0x208..0x210].copy_from_slice(&section_end.to_le_bytes());
        nca[0x210..0x218].copy_from_slice(&0x0400_7EF0_1F8A_E000u64.to_le_bytes());
        nca[0x240..0x244].copy_from_slice(&((section_at / 0x200) as u32).to_le_bytes());
        nca[0x244..0x248].copy_from_slice(&((section_end / 0x200) as u32).to_le_bytes());
        let fs = 0x400usize;
        nca[fs..fs + 2].copy_from_slice(&2u16.to_le_bytes());
        nca[fs + 0x40..fs + 0x48].copy_from_slice(&data_at.to_le_bytes());
        nca[fs + 0x48..fs + 0x50].copy_from_slice(&(exefs.len() as u64).to_le_bytes());
        let at = (section_at + data_at) as usize;
        nca[at..at + exefs.len()].copy_from_slice(&exefs);
        nca
    }

    /// A plaintext data NCA with one RomFS file, mirroring the fixture in
    /// `ew-format`'s NCA tests.
    fn nsp_romfs_nca(name: &str, data: &[u8], title: u64) -> Vec<u8> {
        let dir_meta_at = 0x50usize;
        let file_meta_at = dir_meta_at + 0x18;
        let file_meta_len = 0x20 + name.len();
        let data_at = file_meta_at + file_meta_len;
        let level_off = 0x200usize;
        let romfs_len = data_at + data.len();
        let mut romfs = vec![0u8; romfs_len];
        for (field, value) in [
            (3, dir_meta_at as u64),
            (4, 0x18u64),
            (7, file_meta_at as u64),
            (8, file_meta_len as u64),
            (9, data_at as u64),
        ] {
            romfs[8 * field..8 * field + 8].copy_from_slice(&value.to_le_bytes());
        }
        romfs[dir_meta_at + 8..dir_meta_at + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        romfs[dir_meta_at + 12..dir_meta_at + 16].copy_from_slice(&0u32.to_le_bytes());
        romfs[file_meta_at + 4..file_meta_at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        romfs[file_meta_at + 16..file_meta_at + 24]
            .copy_from_slice(&(data.len() as u64).to_le_bytes());
        romfs[file_meta_at + 28..file_meta_at + 32]
            .copy_from_slice(&(name.len() as u32).to_le_bytes());
        romfs[file_meta_at + 0x20..file_meta_at + 0x20 + name.len()]
            .copy_from_slice(name.as_bytes());
        romfs[data_at..].copy_from_slice(data);

        let section_at = 0xc00u64;
        let section_size = ((level_off + romfs_len + 0x1ff) & !0x1ff) as u64;
        let section_end = section_at + section_size;
        let mut nca = vec![0u8; section_end as usize];
        nca[0x200..0x204].copy_from_slice(b"NCA3");
        nca[0x205] = 4; // data
        nca[0x208..0x210].copy_from_slice(&section_end.to_le_bytes());
        nca[0x210..0x218].copy_from_slice(&title.to_le_bytes());
        nca[0x240..0x244].copy_from_slice(&((section_at / 0x200) as u32).to_le_bytes());
        nca[0x244..0x248].copy_from_slice(&((section_end / 0x200) as u32).to_le_bytes());
        let fs = 0x400usize;
        nca[fs + 8..fs + 12].copy_from_slice(b"IVFC");
        nca[fs + 0x14..fs + 0x18].copy_from_slice(&1u32.to_le_bytes());
        nca[fs + 0x18..fs + 0x20].copy_from_slice(&(level_off as u64).to_le_bytes());
        nca[fs + 0x20..fs + 0x28].copy_from_slice(&(romfs_len as u64).to_le_bytes());
        let at = (section_at + level_off as u64) as usize;
        nca[at..at + romfs.len()].copy_from_slice(&romfs);
        nca
    }

    #[test]
    fn a_synthetic_xci_detects_its_addon() {
        let _lock = NUP_DETECT.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ew-xci-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = nsp_exefs_nca();
        let addon = nsp_romfs_nca("dlc.bin", b"GOOD", 0x0400_7EF0_1F8A_F001);
        let other = nsp_romfs_nca("other.bin", b"OTHER", 0x0400_7EF0_1F8B_F001);
        let path = dir.join("addons.xci");
        std::fs::write(
            &path,
            xci_image(&[
                ("program.nca", &program),
                ("addon.nca", &addon),
                ("other.nca", &other),
            ]),
        )
        .unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        let dump = Config::work_dir()
            .expect("work dir")
            .join("xci")
            .join(format!("04007EF01F8AE000-{size}"));
        let dest = dump.join("dlc");
        let _ = std::fs::remove_dir_all(&dump);

        let g = detect_with(
            &Paths {
                game_root: path.to_string_lossy().into(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        assert_eq!(g.dlc.as_deref(), Some(dest.as_path()), "{:?}", g.checks);
        assert_eq!(g.dlc_ids, vec!["04007EF01F8AF001".to_string()]);
        assert_eq!(
            std::fs::read(dest.join("04007EF01F8AF001/romfs/dlc.bin")).unwrap(),
            b"GOOD".as_slice()
        );
        assert!(!dest.join("04007EF01F8BF001").exists());
        let marker = std::fs::read_to_string(dest.join(ADDON_MARKER)).unwrap();
        assert!(marker.contains("04007EF01F8AF001"));
        assert!(!marker.contains("04007EF01F8BF001"));

        // A cancelled callback would make a fresh unpack fail. Ready here proves
        // the dump and add-on marker were both reused.
        let g = detect_with(
            &Paths {
                game_root: path.to_string_lossy().into(),
                ..Default::default()
            },
            &mut |_, _| panic!("a reused unpack reported progress"),
            &|| true,
        );
        assert_eq!(g.dlc.as_deref(), Some(dest.as_path()), "{:?}", g.checks);

        let _ = std::fs::remove_dir_all(&dump);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_package_addon_scan_clears_the_managed_dlc() {
        let _lock = NUP_DETECT.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ew-addon-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = nsp_exefs_nca();
        let mut encrypted = nsp_romfs_nca("dlc.bin", b"GOOD", 0x0400_7EF0_1F8A_F001);
        encrypted[0x206] = 1; // an encrypted NCA cannot be parsed without keys
        let nsp_path = dir.join("addons.nsp");
        std::fs::write(
            &nsp_path,
            nsp_pfs0(&[("program.nca", &program), ("addon.nca", &encrypted)]),
        )
        .unwrap();
        let xci_path = dir.join("addons.xci");
        std::fs::write(
            &xci_path,
            xci_image(&[("program.nca", &program), ("addon.nca", &encrypted)]),
        )
        .unwrap();
        let nsp_size = std::fs::metadata(&nsp_path).unwrap().len();
        let xci_size = std::fs::metadata(&xci_path).unwrap().len();

        for (kind, path, size) in [
            ("nsp", &nsp_path, nsp_size),
            ("xci", &xci_path, xci_size),
        ] {
            let dump = Config::work_dir()
                .expect("work dir")
                .join(kind)
                .join(format!("04007EF01F8AE000-{size}"));
            let dest = dump.join("dlc");
            let _ = std::fs::remove_dir_all(&dump);
            std::fs::create_dir_all(dest.join("04007EF01F8AF001/romfs")).unwrap();
            std::fs::write(dest.join("04007EF01F8AF001/romfs/stale.bin"), b"OLD").unwrap();
            std::fs::write(dest.join(ADDON_MARKER), "stale\n").unwrap();
            // The base unpack is already recorded, so detection reuses it and
            // reaches the add-on scan with the previous DLC still in place.
            let stamp = format!("{size}-{}", modified_stamp(path));
            std::fs::write(dump.join("dump.txt"), format!("{stamp}\n0 0 0\n")).unwrap();

            let g = detect_with(
                &Paths {
                    game_root: path.to_string_lossy().into(),
                    ..Default::default()
                },
                &mut |_, _| {},
                &|| false,
            );
            assert!(g.dlc.is_none(), "{kind}: a stale add-on folder survived: {:?}", g.checks);
            assert!(!dest.exists(), "{kind}: the failed unpack left {}", dest.display());
            assert!(
                g.checks.iter().any(|c| {
                    c.label == "Add-on content" && c.detail.contains("could not unpack")
                }),
                "{kind}: no add-on warning: {:?}",
                g.checks
            );
            let _ = std::fs::remove_dir_all(&dump);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_package_with_an_encrypted_addon_still_detects_its_base() {
        let _lock = NUP_DETECT.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ew-addon-encrypted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = nsp_exefs_nca();
        let mut encrypted = nsp_romfs_nca("dlc.bin", b"GOOD", 0x0400_7EF0_1F8A_F001);
        encrypted[0x206] = 1;
        let nsp_path = dir.join("addons.nsp");
        std::fs::write(
            &nsp_path,
            nsp_pfs0(&[("program.nca", &program), ("addon.nca", &encrypted)]),
        )
        .unwrap();
        let xci_path = dir.join("addons.xci");
        std::fs::write(
            &xci_path,
            xci_image(&[("program.nca", &program), ("addon.nca", &encrypted)]),
        )
        .unwrap();

        for (kind, path) in [("nsp", &nsp_path), ("xci", &xci_path)] {
            let size = std::fs::metadata(path).unwrap().len();
            let dump = Config::work_dir()
                .expect("work dir")
                .join(kind)
                .join(format!("04007EF01F8AE000-{size}"));
            let dest = dump.join("dlc");
            let _ = std::fs::remove_dir_all(&dump);
            let g = detect_with(
                &Paths {
                    game_root: path.to_string_lossy().into(),
                    ..Default::default()
                },
                &mut |_, _| {},
                &|| false,
            );
            assert!(g.exefs.is_some(), "{kind}: the base game was not detected: {:?}", g.checks);
            assert!(g.romfs.is_some(), "{kind}: the base dump was not kept: {:?}", g.checks);
            assert!(g.dlc.is_none(), "{kind}: an encrypted add-on was served: {:?}", g.checks);
            assert!(!dest.exists(), "{kind}: the failed unpack left {}", dest.display());
            assert!(
                g.checks.iter().any(|c| {
                    c.label == "Add-on content" && c.detail.contains("could not unpack")
                }),
                "{kind}: no add-on warning: {:?}",
                g.checks
            );
            let _ = std::fs::remove_dir_all(&dump);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_synthetic_nsp_detects_its_addon() {
        let _lock = NUP_DETECT.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ew-nsp-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = nsp_exefs_nca();
        let addon = nsp_romfs_nca("dlc.bin", b"GOOD", 0x0400_7EF0_1F8A_F001);
        let path = dir.join("addons.nsp");
        std::fs::write(
            &path,
            nsp_pfs0(&[("program.nca", &program), ("addon.nca", &addon)]),
        )
        .unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        let dump = Config::work_dir()
            .expect("work dir")
            .join("nsp")
            .join(format!("04007EF01F8AE000-{size}"));
        let dest = dump.join("dlc");
        let _ = std::fs::remove_dir_all(&dump);

        let g = detect_with(
            &Paths {
                game_root: path.to_string_lossy().into(),
                ..Default::default()
            },
            &mut |_, _| {},
            &|| false,
        );
        assert_eq!(g.dlc.as_deref(), Some(dest.as_path()), "{:?}", g.checks);
        assert_eq!(g.dlc_ids, vec!["04007EF01F8AF001".to_string()]);
        assert_eq!(
            std::fs::read(dest.join("04007EF01F8AF001/romfs/dlc.bin")).unwrap(),
            b"GOOD".as_slice()
        );

        let _ = std::fs::remove_dir_all(&dump);
        let _ = std::fs::remove_dir_all(&dir);
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

    /// The RomFS overlay write must create parent directories for nested files.
    /// Regression test for the exact failing path: Animation/AC/MV0002/AC_MV0002_01_CD_107_Face.matanimb
    #[test]
    fn romfs_overlay_creates_nested_parent_directories() {
        let dir = std::env::temp_dir().join(format!("ew-overlay-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        let dest = dir.join("work").join("romfs-overlay");

        // Create a .nup with a deeply nested packed file path
        write_overlay_nup(
            &nup,
            &[("Animation/AC/MV0002/AC_MV0002_01_CD_107_Face.matanimb", b"MATANIMB\n")],
        );
        assert_eq!(packed_romfs(&nup), Some((1, 9)));

        // This should not fail with "The system cannot find the path specified"
        let write = write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!write.reused);
        assert_eq!(write.written, 1);

        // Verify the file was written at the correct nested path
        let written_file = dest.join("Animation/AC/MV0002/AC_MV0002_01_CD_107_Face.matanimb");
        assert!(written_file.is_file(), "nested file was not created");
        assert_eq!(std::fs::read(&written_file).unwrap(), b"MATANIMB\n");

        // Verify parent directories exist
        assert!(dest.join("Animation").is_dir());
        assert!(dest.join("Animation/AC").is_dir());
        assert!(dest.join("Animation/AC/MV0002").is_dir());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A packed file with multiple levels of nesting must have all parent directories created.
    #[test]
    fn romfs_overlay_creates_multiple_nested_parent_directories() {
        let dir = std::env::temp_dir().join(format!("ew-overlay-multi-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("game.nup");
        let dest = dir.join("work").join("romfs-overlay");

        // Create a .nup with multiple deeply nested packed file paths
        write_overlay_nup(
            &nup,
            &[
                ("a/b/c/d/e/file1.bin", b"FILE1\n"),
                ("x/y/z/file2.bin", b"FILE2\n"),
                ("alpha/beta/gamma/delta/epsilon/file3.bin", b"FILE3\n"),
            ],
        );
        assert_eq!(packed_romfs(&nup), Some((3, 18)));

        let write = write_romfs_overlay(&nup, &dest, &mut |_| {}).unwrap();
        assert!(!write.reused);
        assert_eq!(write.written, 3);

        // Verify all files were written at the correct nested paths
        assert!(dest.join("a/b/c/d/e/file1.bin").is_file());
        assert!(dest.join("x/y/z/file2.bin").is_file());
        assert!(dest.join("alpha/beta/gamma/delta/epsilon/file3.bin").is_file());

        assert_eq!(std::fs::read(dest.join("a/b/c/d/e/file1.bin")).unwrap(), b"FILE1\n");
        assert_eq!(std::fs::read(dest.join("x/y/z/file2.bin")).unwrap(), b"FILE2\n");
        assert_eq!(std::fs::read(dest.join("alpha/beta/gamma/delta/epsilon/file3.bin")).unwrap(), b"FILE3\n");

        // Verify parent directories exist
        assert!(dest.join("a/b/c/d/e").is_dir());
        assert!(dest.join("x/y/z").is_dir());
        assert!(dest.join("alpha/beta/gamma/delta/epsilon").is_dir());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A path that comes from a container must stay inside the folder it is written into.
    #[test]
    fn container_paths_cannot_escape_the_destination() {
        let root = std::env::temp_dir().join("ew-safe-relative");
        assert_eq!(safe_relative(&root, "a/b/c.bin"), Some(root.join("a/b/c.bin")));
        let absolute = std::env::temp_dir().join("abs.bin");
        for bad in [
            "../evil.bin",
            "a/../../evil.bin",
            "a/b/../c.bin",
            ".",
            "",
            &absolute.to_string_lossy(),
        ] {
            assert_eq!(safe_relative(&root, bad), None, "{bad:?} must be rejected");
        }
    }
}
