//! Save data: importing a Switch save from an emulator or a folder, and
//! exporting EmblemWeave's.
//!
//! The engine keeps the game's account save in
//! `<save folder>/account/<user id>/`, with a `<user id>.meta` file beside it
//! (size, journal size, owner and the save's 64-byte attribute). A Switch
//! emulator keeps the same files under
//! `nand/user/save/0000000000000000/<user>/<title id>`, so a save moves between
//! them as a copy of that folder's contents.
//!
//! Import never touches the source. If the account folder already holds a
//! save, it is moved to `<save folder>/backups/<time>/` first, so an import
//! can always be undone. When the game has not created its save yet, the
//! import writes the `.meta` the engine expects.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Fire Emblem: Fortune's Weave's program id: the save folder's name in an
/// emulator's NAND. Used when the dump does not state it.
pub const FALLBACK_TITLE_ID: u64 = 0x0400_7EF0_1F8A_E000;

/// Emulators that keep saves as `<root>/nand/user/save/...` under
/// `%APPDATA%`: yuzu and the forks that inherited its layout, including
/// Lichii, Fritchii and Eden.
const EMULATOR_DIRS: &[&str] = &[
    "yuzu",
    "suyu",
    "sudachi",
    "citron",
    "eden",
    "torzu",
    "Ryujinx-yuzu",
    "lichii",
    "fritchii",
];

/// A save found on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The emulator folder it was found under, such as `yuzu`.
    pub source: String,
    /// The title's save folder.
    pub path: PathBuf,
    /// Number of files and their newest modification time.
    pub files: usize,
    pub modified: Option<std::time::SystemTime>,
}

/// Saves for `title_id` under every known emulator in `appdata`.
///
/// The save tree changed slightly between yuzu forks: most use
/// `save/0000000000000000/<profile>/<title>`, Eden can use another save id,
/// and a portable or migrated install can omit the profile level. Scanning
/// the save directory rather than assuming one profile id finds all of them.
pub fn find(appdata: &Path, title_id: u64) -> Vec<Found> {
    let name = format!("{title_id:016X}");
    let mut out = Vec::new();
    for emu in EMULATOR_DIRS {
        let save_root = appdata.join(emu).join("nand").join("user").join("save");
        let Ok(ids) = std::fs::read_dir(&save_root) else {
            continue;
        };
        for id in ids.flatten() {
            if !id.path().is_dir() {
                continue;
            }
            push_found(&mut out, emu, &id.path().join(&name));
            let Ok(profiles) = std::fs::read_dir(id.path()) else {
                continue;
            };
            for profile in profiles.flatten() {
                if profile.path().is_dir() {
                    push_found(&mut out, emu, &profile.path().join(&name));
                }
            }
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn push_found(out: &mut Vec<Found>, source: &str, path: &Path) {
    if !path.is_dir() {
        return;
    }
    let (files, modified) = summary(path);
    if files > 0 {
        out.push(Found {
            source: source.to_string(),
            path: path.to_path_buf(),
            files,
            modified,
        });
    }
}

/// Saves under the current user's `%APPDATA%` and `%LOCALAPPDATA%`.
/// The second root covers portable-style forks that keep their NAND there.
pub fn find_here(title_id: u64) -> Vec<Found> {
    let mut out = Vec::new();
    for key in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(key) {
            out.extend(find(Path::new(&root), title_id));
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// File count and newest modification time below `dir`.
pub fn summary(dir: &Path) -> (usize, Option<std::time::SystemTime>) {
    let mut files = 0;
    let mut newest = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(m) = e.metadata() {
                files += 1;
                if let Ok(t) = m.modified() {
                    newest = Some(newest.map_or(t, |n: std::time::SystemTime| n.max(t)));
                }
            }
        }
    }
    (files, newest)
}

pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let mut copied = 0;
    for e in std::fs::read_dir(from)
        .map_err(|e| format!("reading {}: {e}", from.display()))?
        .flatten()
    {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copied += copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("copying {} to {}: {e}", src.display(), dst.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Copy the files of a flat save (the contents of `000`) into `to`, without taking a `.meta` or profile marker with them.
/// The account's own `.meta` is written by [`import`] when the target does not have one, and a `.meta` from an older account
/// would name the wrong user.
fn copy_flat(from: &Path, to: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let mut copied = 0;
    for e in std::fs::read_dir(from)
        .map_err(|e| format!("reading {}: {e}", from.display()))?
        .flatten()
    {
        let src = e.path();
        if src.is_dir() || e.file_name().to_str().is_some_and(is_metadata_file) {
            continue;
        }
        let dst = to.join(e.file_name());
        std::fs::copy(&src, &dst)
            .map_err(|e| format!("copying {} to {}: {e}", src.display(), dst.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub files: usize,
    /// Where the previous save was moved, if there was one.
    pub backup: Option<PathBuf>,
    /// The folder the files were read from, after resolving what the player picked.
    pub from: Option<PathBuf>,
    /// Whether the game's own data folder (`<account save>/000`) received files. The game reads that folder, so an import
    /// whose files all landed beside it is one that reports success and shows nothing in game.
    pub readable: bool,
}

/// The engine's built-in local user id (`[account] uid` unset), as its save
/// folder names it.
pub const DEFAULT_USER: &str = "0100000000000000706d635257464546";
/// Save data and journal size the game asks for: 64 MiB each.
const SAVE_SIZE: u64 = 64 << 20;

/// The account save folder the game reads: `account/DEFAULT_USER`.
///
/// The engine names that folder after the save's owner, and more than one can be present: its built-in
/// user, a console user id, or a folder an earlier import left empty. Picking the first by name, or the
/// newest, put a loaded profile into a folder the game never reads, which is why loading a profile looked
/// like it did nothing (the save in the game was unchanged).
pub fn account_dir(save_dir: &Path) -> PathBuf {
    // The engine's local user id is fixed (the title's `user_uid`, written as `account/<uid>`), and the game always opens that
    // folder. More than one account folder can exist after imports or an older build. Choosing the newest one put a loaded profile
    // into a folder the game never reads, which is why loading looked like it did nothing for some players (issue #10).
    save_dir.join("account").join(DEFAULT_USER)
}

/// The `.meta` the engine writes for an account save of `title_id`.
fn meta_text(title_id: u64, user_hex: &str) -> Option<String> {
    if user_hex.len() != 32 {
        return None;
    }
    let mut attr = [0u8; 0x40];
    attr[0..8].copy_from_slice(&title_id.to_le_bytes());
    for i in 0..16 {
        attr[8 + i] = u8::from_str_radix(user_hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    attr[32] = 1; // SaveDataType::Account
    let hex: String = attr.iter().map(|b| format!("{b:02x}")).collect();
    Some(format!(
        "size={SAVE_SIZE}\njournal={SAVE_SIZE}\nowner={title_id:x}\nattr={hex}\n"
    ))
}

/// What the picked folder turned out to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveSource {
    /// A folder to copy beside the game's `000`: it already holds `000` (and maybe the account's `.meta`).
    Folder(PathBuf),
    /// The contents of a `000` folder without their parent: the files go directly inside the account save's `000`.
    Flat(PathBuf),
}

/// Whether `dir` is an account save folder the game can read: it holds `000` with files.
fn is_account_save(dir: &Path) -> bool {
    dir.join("000").is_dir() && summary(&dir.join("000")).0 > 0
}

/// `dir` itself when it holds `000`, or the only folder directly below it that does.
///
/// The second case is an import left one level too deep by an older build: the player picked the folder above the save, so the
/// save ended up as `account/<user>/<user>/000`. The files the game reads are still in that `000`, so the folder holding it is
/// the save to use.
fn save_below(dir: &Path) -> Option<PathBuf> {
    if is_account_save(dir) {
        return Some(dir.to_path_buf());
    }
    let saves: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && is_account_save(p))
        .collect();
    (saves.len() == 1).then(|| saves[0].clone())
}

/// The account save in an `account` folder, preferring the engine's fixed user id.
fn account_save(account: &Path) -> Option<PathBuf> {
    let preferred = account.join(DEFAULT_USER);
    if let Some(found) = save_below(&preferred) {
        return Some(found);
    }
    let mut found: Vec<(Option<SystemTime>, String, PathBuf)> = std::fs::read_dir(account)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let save = save_below(&p)?;
            let modified = summary(&save.join("000")).1;
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Some((modified, name, save))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().next().map(|(_, _, p)| p)
}

/// The account save in a picked save folder, or in an install folder above one.
///
/// **Import from folder…** is the fallback for a portable install, so the player can point it at the save folder
/// (`data\save`), the `data` folder, the install folder, or the `account` folder itself. All of them describe the same save.
fn account_below(source: &Path) -> Option<PathBuf> {
    if has_name(source, "account") {
        if let Some(found) = account_save(source) {
            return Some(found);
        }
    }
    let candidates = [
        source.to_path_buf(),
        source.join("save"),
        source.join("data"),
        source.join("data").join("save"),
    ];
    for save in candidates {
        if let Some(found) = account_save(&save.join("account")) {
            return Some(found);
        }
    }
    None
}

/// Whether `dir` holds the contents of a `000` folder directly: save slots and `Global`, with no `000` level of their own.
fn flat_save(dir: &Path) -> bool {
    let mut has_slot = false;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = e.path();
        if path.is_dir() {
            return false;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if is_metadata_file(&name) {
            continue;
        }
        if is_slot_candidate(&path) || is_global_save_file(&name) {
            has_slot = true;
        }
    }
    has_slot
}

/// Where an import should read from, when the picked folder may sit above the save's own files.
///
/// The game opens `<account>/<user>/000`, so the files have to end up exactly there. A player can point the folder picker at
/// many shapes of the same save: the save folder itself, an `account` folder, an install folder above either, a console user
/// folder holding `<title id>/000`, a whole NAND save tree, or a folder holding the contents of `000`. Copying any of them
/// wholesale nested everything one level deeper, under a folder the game never opens: the import reported success and the game
/// still showed no save (issue #43). Every shape is resolved here; a folder that holds no save at all returns `None`, so the
/// import can refuse before it touches the current save.
pub fn resolve_source(source: &Path, title_id: u64) -> Option<SaveSource> {
    let title = format!("{title_id:016X}");
    // An EmblemWeave save folder, an account folder, or an install folder above them.
    if let Some(dir) = account_below(source) {
        return Some(SaveSource::Folder(dir));
    }
    // Already the title's own save folder.
    if is_account_save(source) {
        return Some(SaveSource::Folder(source.to_path_buf()));
    }
    // The title folder, at the documented levels only: directly in a picked user folder, one level below it, or under a NAND
    // save tree (`nand/user/save/<id>/[<profile>/]<title>`). A picked folder can be a whole drive; walking it depth-first for
    // the name took as long as the drive had folders, so the search looks at these levels and nowhere else.
    if let Some(dir) = child_named(source, &title)
        .or_else(|| nested_named(source, &title))
        .or_else(|| nand_named(source, &title))
    {
        return Some(if is_account_save(&dir) {
            SaveSource::Folder(dir)
        } else {
            SaveSource::Flat(dir)
        });
    }
    // The contents of `000` without their parent.
    if flat_save(source) {
        return Some(SaveSource::Flat(source.to_path_buf()));
    }
    None
}

fn has_name(p: &Path, name: &str) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
}

/// A child folder of `dir` called `name` that holds files.
fn child_named(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir() && has_name(p, name) && summary(p).0 > 0)
}

/// A folder called `name` one level below `root`, which is where a console keeps the title folder under a profile id.
fn nested_named(root: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(found) = child_named(&p, name) {
                return Some(found);
            }
        }
    }
    None
}

/// The title folder in a NAND save tree, `nand/user/save/<save id>/[<profile>/]<title>`, from a picked emulator root, the
/// `save` folder, or the `nand` folder.
fn nand_named(root: &Path, name: &str) -> Option<PathBuf> {
    for save in [
        root.join("nand").join("user").join("save"),
        root.to_path_buf(),
    ] {
        let Ok(ids) = std::fs::read_dir(&save) else {
            continue;
        };
        for id in ids.flatten() {
            let p = id.path();
            if p.is_dir() {
                if let Some(found) = child_named(&p, name).or_else(|| nested_named(&p, name)) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Copy `source` into the game's account save, moving any existing save to
/// `<save folder>/backups/<stamp>/` first.
pub fn import(
    source: &Path,
    save_dir: &Path,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    if !source.is_dir() {
        return Err(format!("{} is not a folder", source.display()));
    }
    if summary(source).0 == 0 {
        return Err(format!("{} holds no save files", source.display()));
    }
    // The picked folder can be a whole save folder, an account folder, an install folder above either, or a NAND root; read
    // the title's files out of it instead of copying the nesting into the account save.
    let resolved = resolve_source(source, title_id).ok_or_else(|| {
        format!(
            "{} does not hold a save this game can read: pick the folder that contains its 000 save folder, its account folder, or the save folder an older install used",
            source.display()
        )
    })?;
    let (from, flat) = match &resolved {
        SaveSource::Folder(dir) => (dir.as_path(), false),
        SaveSource::Flat(dir) => (dir.as_path(), true),
    };
    let target = account_dir(save_dir);
    // A profile or a backup under the save folder is fine; the current save, its account folder, or a folder holding the save
    // folder, is not.
    if from.starts_with(&target)
        || target.starts_with(from)
        || save_dir.starts_with(source)
        || source.starts_with(save_dir.join("account"))
    {
        return Err("the source and the save folder overlap".into());
    }
    let backup = if target.is_dir() && summary(&target).0 > 0 {
        let to = save_dir.join("backups").join(stamp);
        std::fs::create_dir_all(save_dir.join("backups"))
            .map_err(|e| format!("creating the backups folder: {e}"))?;
        std::fs::rename(&target, &to)
            .map_err(|e| format!("moving the current save to {}: {e}", to.display()))?;
        Some(to)
    } else {
        None
    };
    let copied = if flat {
        copy_flat(from, &target.join("000"))
    } else {
        copy_tree(from, &target)
    };
    match copied {
        Ok(files) => {
            let user = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let meta = target.with_extension("meta");
            if !meta.is_file() {
                if let Some(text) = meta_text(title_id, &user) {
                    if let Err(e) = std::fs::write(&meta, text) {
                        // The copied tree is not a usable installed save without
                        // its journal metadata. Put the previous save back.
                        let _ = std::fs::remove_dir_all(&target);
                        if let Some(b) = &backup {
                            let _ = std::fs::rename(b, &target);
                        }
                        return Err(format!("writing {}: {e}", meta.display()));
                    }
                }
            }
            let readable = summary(&target.join("000")).0 > 0;
            if !readable {
                // Never call an import that landed outside `000` a success: the game would open an empty save. Put the
                // previous save back instead of leaving the wrong tree in place.
                let _ = std::fs::remove_dir_all(&target);
                if let Some(b) = &backup {
                    let _ = std::fs::rename(b, &target);
                }
                return Err(format!(
                    "{} did not leave the save in {}; the previous save was put back",
                    from.display(),
                    target.join("000").display()
                ));
            }
            Ok(Imported {
                files,
                backup,
                from: Some(from.to_path_buf()),
                readable,
            })
        }
        Err(e) => {
            // Put the previous save back rather than leave a partial copy.
            let _ = std::fs::remove_dir_all(&target);
            if let Some(b) = &backup {
                let _ = std::fs::rename(b, &target);
            }
            Err(e)
        }
    }
}

/// What to say about an import result, and whether it is good news.
///
/// The wording lives here rather than on a page so `--import-save` prints the same sentence the window shows without reaching
/// into a UI module. `false` means the import needs the player's attention: the files landed, but somewhere the game does not
/// read.
pub fn import_message(r: Result<Imported, String>) -> (String, bool) {
    match r {
        Ok(i) => {
            let from = i
                .from
                .as_ref()
                .map(|p| format!(" from {}", p.display()))
                .unwrap_or_default();
            let mut text = format!("Imported {} save files{from}", i.files);
            if let Some(b) = &i.backup {
                text.push_str(&format!("; the previous save is in {}", b.display()));
            }
            if !i.readable {
                // The old message said success either way, and the player found an empty game afterwards.
                return (
                    format!("{text} — but the game reads only <save>/000, which holds none of them; import the folder that contains 000"),
                    false,
                );
            }
            (text, true)
        }
        Err(e) => (format!("Import failed: {e}"), false),
    }
}

/// Copy the game's account save into `dest/<title id>`, the layout an
/// emulator's user folder expects. Returns the folder written.
pub fn export(save_dir: &Path, dest: &Path, title_id: u64) -> Result<(PathBuf, usize), String> {
    let from = account_dir(save_dir);
    if summary(&from).0 == 0 {
        return Err("there is no EmblemWeave save to export yet".into());
    }
    let to = dest.join(format!("{title_id:016X}"));
    if to.exists() {
        return Err(format!(
            "{} already exists; choose an empty folder",
            to.display()
        ));
    }
    copy_tree(&from, &to).map(|n| (to, n))
}

/// A named copy of the account save, kept in `<save folder>/profiles/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub path: PathBuf,
    pub files: usize,
    pub modified: Option<std::time::SystemTime>,
}

/// A marker in a profile that has no save yet. It is not a game file: the game
/// never sees it, and the slot scanner ignores it.
pub const EMPTY_PROFILE_MARKER: &str = ".emblemweave-empty";

/// The kind of save file a profile or the current save holds. The game's
/// filenames are the only stable identifiers available without guessing at
/// the payload, so the display type is derived from those names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Auto,
    Manual,
    Other,
}

impl SlotKind {
    pub fn label(self) -> &'static str {
        match self {
            SlotKind::Auto => "Autosave",
            SlotKind::Manual => "Manual save",
            SlotKind::Other => "Other save data",
        }
    }
}

/// What one save file contributes to a profile summary. Parsing failures are
/// data, not errors: the file is kept and shown as unreadable rather than
/// hidden or deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotInfo {
    pub path: PathBuf,
    pub relative: String,
    pub name: String,
    pub kind: SlotKind,
    pub modified: Option<SystemTime>,
    pub size: u64,
    pub gold: Option<u32>,
    pub renown: Option<u32>,
    pub units: usize,
    /// The first main character this slot names, when it has one. Display-only:
    /// the carousel uses it to choose a portrait when no explicit one is set.
    pub leader: Option<String>,
    pub valid: bool,
    pub problems: Vec<String>,
    /// The read-only summary: difficulty, playthrough, mode and how many flags the slot stores.
    pub summary: crate::savefile::Summary,
}

/// The cached metadata for a profile. Values that this build cannot read from
/// the save format stay `None`; they are never filled with guesses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileMeta {
    pub slots: Vec<SlotInfo>,
    pub gold: Option<u32>,
    pub renown: Option<u32>,
    pub units: usize,
    pub leader: Option<String>,
    pub valid_slots: usize,
}

/// A user-facing profile name that is also a safe directory name. Unlike the
/// older silent clean-up, this reports why a name cannot be used.
pub fn checked_profile_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("a profile needs a name".into());
    }
    let clean = crate::mods::clean_name(trimmed);
    if clean != trimmed || clean.is_empty() {
        return Err(
            "use a name without path separators, control characters or a trailing dot".into(),
        );
    }
    Ok(clean)
}

pub fn is_metadata_file(name: &str) -> bool {
    name == EMPTY_PROFILE_MARKER || name.ends_with(".meta")
}

/// `Global` is the game's global data beside the slots, not a slot file: it has
/// a different header, so validating it as a slot would reject every profile
/// that contains one. It is still copied with the profile.
fn is_global_save_file(name: &str) -> bool {
    name.eq_ignore_ascii_case("Global")
}

fn is_slot_candidate(path: &Path) -> bool {
    path.extension().is_none()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| !is_metadata_file(n) && !is_global_save_file(n))
}

fn kind_of(name: &str) -> SlotKind {
    let n = name.to_ascii_lowercase();
    if n.starts_with("auto") {
        SlotKind::Auto
    } else if n.starts_with("chapter") || n.starts_with("manual") || n.starts_with("save") {
        SlotKind::Manual
    } else {
        SlotKind::Other
    }
}

fn file_paths(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The save files in a profile, with the metadata this build can parse. The
/// list is sorted by modified time, newest first, and includes unknown files so
/// the manager can show them instead of silently treating them as disposable.
pub fn profile_slots(p: &Profile, ids: &crate::savefile::Ids) -> Vec<SlotInfo> {
    let mut out = Vec::new();
    for path in file_paths(&p.path) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if is_metadata_file(&name)
            || is_global_save_file(&name)
            || name.ends_with(".tmp")
            || name.ends_with(".orig")
            || name.ends_with(".before-restore")
            // The game keeps `<slot>.bak` beside a slot. The current-save list
            // already ignores them because they have an extension; the profile
            // list did not, so a profile looked like it held two more slots
            // than the save it was copied from.
            || name.to_ascii_lowercase().ends_with(".bak")
        {
            continue;
        }
        let rel = path
            .strip_prefix(&p.path)
            .unwrap_or(&path)
            .display()
            .to_string();
        let metadata = std::fs::metadata(&path).ok();
        let modified = metadata.as_ref().and_then(|m| m.modified().ok());
        let size = metadata.as_ref().map_or(0, |m| m.len());
        let (gold, renown, units, leader, valid, problems, summary) =
            match crate::savefile::Slot::open(&path) {
                Ok(slot) => {
                    let player = slot.player();
                    let roster = slot.units(ids);
                    let leader = roster
                        .iter()
                        .find_map(|u| crate::portraits::key_for_name(&u.name))
                        .map(str::to_string);
                    let problems = slot.verify();
                    let summary = slot.summary();
                    (
                        player.map(|p| p.gold),
                        player.map(|p| p.renown),
                        roster.len(),
                        leader,
                        problems.is_empty(),
                        problems,
                        summary,
                    )
                }
                Err(e) => (
                    None,
                    None,
                    0,
                    None,
                    false,
                    vec![e],
                    crate::savefile::Summary::default(),
                ),
            };
        out.push(SlotInfo {
            path,
            relative: rel,
            name: name.clone(),
            kind: kind_of(&name),
            modified,
            size,
            gold,
            renown,
            units,
            leader,
            valid,
            problems,
            summary,
        });
    }
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.relative.cmp(&b.relative))
    });
    out
}

/// A summary of a profile for the carousel. The newest readable slot supplies
/// gold, renown and the number of units; missing values remain missing.
pub fn profile_meta(p: &Profile, ids: &crate::savefile::Ids) -> ProfileMeta {
    let slots = profile_slots(p, ids);
    let newest = slots.iter().find(|s| s.valid).or_else(|| slots.first());
    ProfileMeta {
        gold: newest.and_then(|s| s.gold),
        renown: newest.and_then(|s| s.renown),
        units: newest.map_or(0, |s| s.units),
        leader: newest.and_then(|s| s.leader.clone()),
        valid_slots: slots.iter().filter(|s| s.valid).count(),
        slots,
    }
}

pub fn is_empty_profile(p: &Profile) -> bool {
    p.path.join(EMPTY_PROFILE_MARKER).is_file()
        && file_paths(&p.path)
            .iter()
            .all(|f| f.file_name().and_then(|n| n.to_str()) == Some(EMPTY_PROFILE_MARKER))
}

/// Whether every file in `profile` is byte-for-byte present in the current
/// account save. The `.meta` beside the account folder is not part of a profile
/// and is ignored.
fn profile_is_installed(p: &Profile, save_dir: &Path) -> bool {
    let target = account_dir(save_dir);
    let files = file_paths(&p.path);
    let mut compared = 0;
    for from in files {
        let Some(name) = from.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_metadata_file(name) {
            continue;
        }
        let Ok(rel) = from.strip_prefix(&p.path) else {
            continue;
        };
        let to = target.join(rel);
        let (Ok(a), Ok(b)) = (std::fs::read(&from), std::fs::read(&to)) else {
            return false;
        };
        if a != b {
            return false;
        }
        compared += 1;
    }
    compared > 0
}

/// The profile installed in the game's current save, found by comparing the
/// actual files. This is what makes the active badge correct after a restart,
/// not only in the session that loaded the profile.
pub fn installed_profile(save_dir: &Path, profiles: &[Profile]) -> Option<String> {
    profiles
        .iter()
        .find(|p| profile_is_installed(p, save_dir))
        .map(|p| p.name.clone())
}

pub fn create_empty_profile(save_dir: &Path, name: &str) -> Result<Profile, String> {
    let name = checked_profile_name(name)?;
    let path = save_dir.join("profiles").join(&name);
    if path.exists() {
        return Err(format!("there is already a profile called {name}"));
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("creating {}: {e}", path.display()))?;
    if let Err(e) = std::fs::write(
        path.join(EMPTY_PROFILE_MARKER),
        "EmblemWeave empty profile\n",
    ) {
        let _ = std::fs::remove_dir_all(&path);
        return Err(format!("writing the profile marker: {e}"));
    }
    let (files, modified) = summary(&path);
    Ok(Profile {
        name,
        path,
        files,
        modified,
    })
}

/// Copy an external folder into a new profile and refuse to keep it unless its
/// slot files can be read. The source is never modified.
///
/// The picker is the same kind of file dialog as **Import from folder…** on Game Setup, so the same shapes are accepted: an
/// old save folder, its `account` folder, a title folder, a NAND tree, or a flat folder holding the contents of `000`. A flat
/// profile that kept its slots at the top level used to pass validation and then fail when it was loaded; resolving first puts
/// every shape into the `000` layout the loader compares against.
pub fn import_profile(
    save_dir: &Path,
    name: &str,
    source: &Path,
    title_id: u64,
) -> Result<Profile, String> {
    let name = checked_profile_name(name)?;
    if !source.is_dir() {
        return Err(format!("{} is not a folder", source.display()));
    }
    let path = save_dir.join("profiles").join(&name);
    if path.exists() {
        return Err(format!("there is already a profile called {name}"));
    }
    if source == path || source.starts_with(&path) {
        return Err("the import source overlaps the profile folder".into());
    }
    let resolved = resolve_source(source, title_id).ok_or_else(|| {
        format!(
            "{} does not hold a save this game can read: pick the folder that contains its 000 save folder, its account folder, or an exported profile",
            source.display()
        )
    })?;
    let (from, flat) = match &resolved {
        SaveSource::Folder(dir) => (dir.as_path(), false),
        SaveSource::Flat(dir) => (dir.as_path(), true),
    };
    if from == path || from.starts_with(&path) {
        return Err("the import source overlaps the profile folder".into());
    }
    if flat {
        copy_flat(from, &path.join("000"))?
    } else {
        copy_tree(from, &path)?
    };
    let (files, modified) = summary(&path);
    let p = Profile {
        name: name.clone(),
        path,
        files,
        modified,
    };
    let check = check_profile(&p);
    if !check.ok() {
        let _ = std::fs::remove_dir_all(&p.path);
        return Err(check
            .problems
            .first()
            .cloned()
            .unwrap_or_else(|| "the backup has no readable save slots".into()));
    }
    Ok(p)
}

/// Export a profile in the same title-id layout `export` uses for the current
/// save, without touching the current account.
pub fn export_profile(p: &Profile, dest: &Path, title_id: u64) -> Result<(PathBuf, usize), String> {
    let to = dest.join(format!("{title_id:016X}"));
    if to.exists() {
        return Err(format!(
            "{} already exists; choose an empty folder",
            to.display()
        ));
    }
    copy_tree(&p.path, &to).map(|n| (to, n))
}

/// Load a profile and verify the installed tree before reporting success. If
/// the copy differs, put the previous save back and return an error.
pub fn load_profile_verified(
    save_dir: &Path,
    p: &Profile,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    if is_empty_profile(p) {
        return load_empty_profile(save_dir, p, title_id, stamp);
    }
    let check = check_profile(p);
    if !check.ok() {
        return Err(check
            .problems
            .first()
            .cloned()
            .unwrap_or_else(|| "the profile has no readable save slots".into()));
    }
    let imported = load_profile(save_dir, p, title_id, stamp)?;
    if profile_is_installed(p, save_dir) {
        return Ok(imported);
    }
    let target = account_dir(save_dir);
    let _ = std::fs::remove_dir_all(&target);
    if let Some(backup) = &imported.backup {
        let _ = std::fs::rename(backup, &target);
    }
    Err("the restored save did not match the profile; the previous save was put back".into())
}

/// Install a fresh, empty account save. The previous save is kept in backups,
/// so starting a new playthrough never destroys the old one.
pub fn load_empty_profile(
    save_dir: &Path,
    p: &Profile,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    if !is_empty_profile(p) {
        return Err("this profile is not empty".into());
    }
    let target = account_dir(save_dir);
    let backup = if target.is_dir() && summary(&target).0 > 0 {
        let to = save_dir.join("backups").join(stamp);
        std::fs::create_dir_all(save_dir.join("backups"))
            .map_err(|e| format!("creating the backups folder: {e}"))?;
        std::fs::rename(&target, &to)
            .map_err(|e| format!("moving the current save to {}: {e}", to.display()))?;
        Some(to)
    } else {
        None
    };
    let result = (|| {
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("creating {}: {e}", target.display()))?;
        let user = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meta = target.with_extension("meta");
        if let Some(text) = meta_text(title_id, &user) {
            std::fs::write(&meta, text).map_err(|e| format!("writing {}: {e}", meta.display()))?;
        }
        Ok(Imported {
            files: 0,
            backup: backup.clone(),
            from: None,
            readable: true,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&target);
        if let Some(b) = &backup {
            let _ = std::fs::rename(b, &target);
        }
    }
    result
}

pub fn profiles(save_dir: &Path) -> Vec<Profile> {
    let mut out: Vec<Profile> = std::fs::read_dir(save_dir.join("profiles"))
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .map(|path| {
                    let (files, modified) = summary(&path);
                    Profile {
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path,
                        files,
                        modified,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Keep a copy of the current save as profile `name` (replacing a profile of
/// that name, which is moved to the backups first).
pub fn store_profile(save_dir: &Path, name: &str, stamp: &str) -> Result<usize, String> {
    let name = crate::mods::clean_name(name);
    let from = account_dir(save_dir);
    if summary(&from).0 == 0 {
        return Err("there is no save to keep yet".into());
    }
    let to = save_dir.join("profiles").join(&name);
    if to.exists() {
        let b = save_dir
            .join("backups")
            .join(format!("{stamp}-profile-{name}"));
        std::fs::create_dir_all(save_dir.join("backups")).map_err(|e| e.to_string())?;
        std::fs::rename(&to, &b).map_err(|e| format!("moving the old profile aside: {e}"))?;
    }
    copy_tree(&from, &to)
}

/// Make profile `p` the current save. The current save goes to the backups.
pub fn load_profile(
    save_dir: &Path,
    p: &Profile,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    import(&p.path, save_dir, title_id, stamp)
}

/// What a profile's slot files look like before they replace the current save.
/// A folder copied from a game version whose slot format this build cannot read
/// would otherwise be installed, and the failure would only show up in play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCheck {
    pub slots: usize,
    pub readable: usize,
    pub problems: Vec<String>,
}

impl ProfileCheck {
    pub fn ok(&self) -> bool {
        self.problems.is_empty() && self.slots > 0 && self.readable == self.slots
    }
}

pub fn check_profile(p: &Profile) -> ProfileCheck {
    let mut slots = 0;
    let mut readable = 0;
    let mut problems = Vec::new();
    let mut stack = vec![p.path.clone()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !is_slot_candidate(&path) {
                continue;
            }
            slots += 1;
            match crate::savefile::Slot::open(&path) {
                Ok(_) => readable += 1,
                Err(err) => problems.push(format!(
                    "{}: {err}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
    }
    if slots == 0 {
        problems.push("the profile holds no slot files".into());
    }
    ProfileCheck {
        slots,
        readable,
        problems,
    }
}

/// A whole save kept by an import or a profile removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub name: String,
    pub path: PathBuf,
    pub files: usize,
    pub modified: Option<std::time::SystemTime>,
}

/// Save folders under `backups`, newest first. Slot files moved aside by the
/// Saves page live there too, but a slot is restored by copying it back into
/// the save folder; only whole folders are listed here.
pub fn backups(save_dir: &Path) -> Vec<Backup> {
    let mut out: Vec<Backup> = std::fs::read_dir(save_dir.join("backups"))
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .map(|path| {
                    let (files, modified) = summary(&path);
                    Backup {
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path,
                        files,
                        modified,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.name.cmp(&a.name))
    });
    out
}

/// Put a whole backup folder back as the current save. The save it replaces is
/// moved to the backups first, so a restore can be undone.
pub fn restore_backup(
    save_dir: &Path,
    b: &Backup,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    import(&b.path, save_dir, title_id, stamp)
}

/// Restore a whole backup and verify the installed tree. If the copy differs,
/// the save that was replaced is put back before the error is returned.
pub fn restore_backup_verified(
    save_dir: &Path,
    b: &Backup,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    let imported = restore_backup(save_dir, b, title_id, stamp)?;
    let p = Profile {
        name: b.name.clone(),
        path: b.path.clone(),
        files: b.files,
        modified: b.modified,
    };
    if profile_is_installed(&p, save_dir) {
        return Ok(imported);
    }
    let target = account_dir(save_dir);
    let _ = std::fs::remove_dir_all(&target);
    if let Some(previous) = &imported.backup {
        let _ = std::fs::rename(previous, &target);
    }
    Err("the restored backup did not match its files; the previous save was put back".into())
}

/// Keep a copy of one slot beside the other whole-save backups. The slot stays
/// where it is, so this is safe for a profile that is currently selected.
pub fn backup_slot(
    save_dir: &Path,
    profile: &Profile,
    slot: &Path,
    stamp: &str,
) -> Result<PathBuf, String> {
    let name = slot
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "slot".into());
    let to = save_dir
        .join("backups")
        .join(format!("{stamp}-{}-{name}", profile.name));
    std::fs::create_dir_all(save_dir.join("backups"))
        .map_err(|e| format!("creating the backups folder: {e}"))?;
    std::fs::copy(slot, &to)
        .map_err(|e| format!("copying {} to {}: {e}", slot.display(), to.display()))?;
    Ok(to)
}

/// The untouched original kept by the slot editor, when there is one. Older folders keep it as `<slot>.orig`; the numbered
/// scheme below writes `<slot>.bak` instead and still reads the old name.
pub fn original_slot_path(slot: &Path) -> PathBuf {
    let mut o = slot.as_os_str().to_owned();
    o.push(".orig");
    PathBuf::from(o)
}

/// The names a slot's backups use: `<slot>.bak` for the untouched original, then `<slot>.bak1`, `.bak2`, … for the copies
/// each save replaced, the highest number the newest.
fn slot_backup_names(slot: &Path) -> Option<(std::path::PathBuf, String)> {
    let name = slot.file_name()?.to_string_lossy().into_owned();
    let dir = slot.parent().unwrap_or(Path::new(".")).to_path_buf();
    Some((dir, name))
}

/// Every backup beside a slot, oldest first: the original, then the numbered copies by ascending number. A folder that
/// still has the older `<slot>.orig` is read as having that original. Nothing is renamed or removed to build the list.
pub fn slot_backups(slot: &Path) -> Vec<PathBuf> {
    let Some((dir, name)) = slot_backup_names(slot) else {
        return Vec::new();
    };
    let mut original: Option<PathBuf> = None;
    let mut numbered: Vec<(u32, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(tail) = file.strip_prefix(&format!("{name}.bak")) else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if tail.is_empty() {
            original = Some(path);
        } else if let Ok(n) = tail.parse::<u32>() {
            if n > 0 {
                numbered.push((n, path));
            }
        }
    }
    if original.is_none() {
        let legacy = dir.join(format!("{name}.orig"));
        if legacy.is_file() {
            original = Some(legacy);
        }
    }
    numbered.sort_by_key(|(n, _)| *n);
    let mut out: Vec<PathBuf> = original.into_iter().collect();
    out.extend(numbered.into_iter().map(|(_, p)| p));
    out
}

/// Keep the file a save is about to replace: the first save takes `<slot>.bak`, so the untouched original survives, and
/// every save after that takes the next number. Nothing already on disk is renamed, overwritten or deleted.
pub fn keep_slot_backup(slot: &Path) -> Result<PathBuf, String> {
    let Some((dir, name)) = slot_backup_names(slot) else {
        return Err("that slot has no file name".into());
    };
    if !slot.is_file() {
        return Err(format!("{} is not there to back up", slot.display()));
    }
    let original = dir.join(format!("{name}.bak"));
    let legacy = dir.join(format!("{name}.orig"));
    let to = if !original.exists() && !legacy.exists() {
        original
    } else {
        let mut n = 1u32;
        loop {
            let candidate = dir.join(format!("{name}.bak{n}"));
            if !candidate.exists() {
                break candidate;
            }
            n += 1;
        }
    };
    std::fs::copy(slot, &to)
        .map_err(|e| format!("copying {} to {}: {e}", slot.display(), to.display()))?;
    Ok(to)
}

/// Put the newest backup back. The file it replaces is kept as the next backup first, and the answer names both, so the
/// page can say which file it restored from instead of just claiming success.
pub fn restore_slot_backup(slot: &Path) -> Result<(PathBuf, PathBuf), String> {
    let Some(newest) = slot_backups(slot).pop() else {
        return Err("this slot has no backup to restore".into());
    };
    let kept = keep_slot_backup(slot)?;
    std::fs::copy(&newest, slot).map_err(|e| format!("restoring {}: {e}", newest.display()))?;
    Ok((newest, kept))
}

/// Move a profile to the backups folder.
pub fn remove_profile(save_dir: &Path, p: &Profile, stamp: &str) -> Result<PathBuf, String> {
    let b = save_dir
        .join("backups")
        .join(format!("{stamp}-profile-{}", p.name));
    std::fs::create_dir_all(save_dir.join("backups")).map_err(|e| e.to_string())?;
    std::fs::rename(&p.path, &b).map_err(|e| e.to_string())?;
    Ok(b)
}

/// A slot in the current save, with the size the page shows beside it.
pub fn slots_with_sizes(save_dir: &Path) -> Vec<(PathBuf, Option<std::time::SystemTime>, u64)> {
    slots(save_dir)
        .into_iter()
        .map(|(p, t)| {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            (p, t, size)
        })
        .collect()
}

/// Move one slot file into `<save folder>/backups/<stamp>-<file>`: the game offers a new game there
/// again, and the old slot is still on disk if it was the wrong one.
pub fn archive_slot(save_dir: &Path, slot: &Path, stamp: &str) -> Result<PathBuf, String> {
    let name = slot
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return Err(format!("{} is not a save slot", slot.display()));
    }
    if !slot.is_file() {
        return Err(format!("{} is not there any more", slot.display()));
    }
    let to = save_dir.join("backups").join(format!("{stamp}-{name}"));
    std::fs::create_dir_all(save_dir.join("backups"))
        .map_err(|e| format!("creating the backups folder: {e}"))?;
    std::fs::rename(slot, &to)
        .map_err(|e| format!("moving {} to {}: {e}", slot.display(), to.display()))?;
    Ok(to)
}

/// Rename a profile: its folder is moved, so a name that sorts better does not need a copy and a delete.
pub fn rename_profile(save_dir: &Path, p: &Profile, new_name: &str) -> Result<Profile, String> {
    let name = crate::mods::clean_name(new_name);
    if name.is_empty() {
        return Err("a profile needs a name".into());
    }
    if name == p.name {
        return Ok(p.clone());
    }
    let to = save_dir.join("profiles").join(&name);
    if to.exists() {
        return Err(format!("there is already a profile called {name}"));
    }
    std::fs::rename(&p.path, &to).map_err(|e| format!("renaming the profile: {e}"))?;
    let (files, modified) = summary(&to);
    Ok(Profile {
        name,
        path: to,
        files,
        modified,
    })
}

/// Keep a second copy of a profile under another name, for a branch in the story.
pub fn duplicate_profile(save_dir: &Path, p: &Profile, new_name: &str) -> Result<Profile, String> {
    let name = crate::mods::clean_name(new_name);
    if name.is_empty() {
        return Err("a profile needs a name".into());
    }
    let to = save_dir.join("profiles").join(&name);
    if to.exists() {
        return Err(format!("there is already a profile called {name}"));
    }
    copy_tree(&p.path, &to)?;
    let (files, modified) = summary(&to);
    Ok(Profile {
        name,
        path: to,
        files,
        modified,
    })
}

/// The save slots in the current save: files directly in its folders, newest first.
pub fn slots(save_dir: &Path) -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let mut stack = vec![account_dir(save_dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if is_slot_candidate(&p) {
                out.push((p, e.metadata().ok().and_then(|m| m.modified().ok())));
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// The program id a `control.nacp` names as its save data owner.
pub fn nacp_save_owner(nacp: &Path) -> Option<u64> {
    let b = std::fs::read(nacp).ok()?;
    let v = u64::from_le_bytes(b.get(0x3078..0x3080)?.try_into().ok()?);
    (v != 0).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::savefile::{fnv1_32, fnv1a32, raw_zstd_frame};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ew-saves-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn write_bytes(p: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn finds_emulator_saves_by_title() {
        let root = temp("find");
        let title = root.join("yuzu/nand/user/save/0000000000000000/5A3C/04007EF01F8AE000");
        write(&title.join("000/Auto"), "a");
        write(
            &root.join("yuzu/nand/user/save/0000000000000000/5A3C/0100000000000000/x"),
            "other game",
        );
        let found = find(&root, FALLBACK_TITLE_ID);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].source.as_str(), found[0].files), ("yuzu", 1));
        assert_eq!(found[0].path, title);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_lichii_fritchii_eden_and_alternate_save_ids() {
        let root = temp("find-forks");
        let lichii = root.join("lichii/nand/user/save/0000000000000000/5A3C/04007EF01F8AE000");
        let fritchii = root.join("fritchii/nand/user/save/0000000000000000/04007EF01F8AE000");
        let eden = root.join("eden/nand/user/save/0000000000000001/5A3C/04007EF01F8AE000");
        write(&lichii.join("000/Auto"), "l");
        write(&fritchii.join("000/Auto"), "f");
        write(&eden.join("000/Auto"), "e");
        let found = find(&root, FALLBACK_TITLE_ID);
        let sources: Vec<&str> = found.iter().map(|f| f.source.as_str()).collect();
        assert_eq!(found.len(), 3, "{sources:?}");
        assert!(sources.contains(&"lichii"));
        assert!(sources.contains(&"fritchii"));
        assert!(sources.contains(&"eden"));
        assert!(found.iter().any(|f| f.path == eden));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_backs_up_and_leaves_the_source_alone() {
        let root = temp("import");
        let (src, save) = (root.join("src"), root.join("app/save"));
        write(&src.join("000/Auto"), "new");
        write(&src.join("000/Global"), "g");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "old");
        let r = import(&src, &save, FALLBACK_TITLE_ID, "1").unwrap();
        assert_eq!(r.files, 2);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "new"
        );
        let backup = r.backup.unwrap();
        assert_eq!(
            std::fs::read_to_string(backup.join("000/Auto")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(src.join("000/Auto")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_reads_the_save_out_of_the_folder_that_was_picked() {
        let root = temp("unwrap");
        let (src, save) = (root.join("src"), root.join("save"));
        // A whole EmblemWeave save folder: the account save belongs inside `account/`, not `account/account/`.
        write(
            &src.join("account").join(DEFAULT_USER).join("000/Auto"),
            "a",
        );
        let r = import(&src, &save, FALLBACK_TITLE_ID, "1").unwrap();
        assert_eq!(
            r.from.as_deref(),
            Some(src.join("account").join(DEFAULT_USER).as_path())
        );
        assert!(r.readable, "the files landed in the folder the game reads");
        let user = save.join("account").join(DEFAULT_USER);
        assert_eq!(std::fs::read_to_string(user.join("000/Auto")).unwrap(), "a");

        // A console user folder: `<title id>/000` is the save, not a folder to nest inside the account save.
        let picked = root.join("user");
        write(&picked.join("04007EF01F8AE000/000/Auto"), "b");
        let r = import(&picked, &save, FALLBACK_TITLE_ID, "2").unwrap();
        assert!(r.readable, "the title folder was unwrapped");
        assert_eq!(std::fs::read_to_string(user.join("000/Auto")).unwrap(), "b");

        // A whole NAND save tree: the title folder is below nand/user/save/<id>/<profile>/.
        let nand = root.join("nand-root");
        write(
            &nand.join("nand/user/save/0000000000000000/5A3C/04007EF01F8AE000/000/Auto"),
            "c",
        );
        let r = import(&nand, &save, FALLBACK_TITLE_ID, "3").unwrap();
        assert!(r.readable, "the NAND tree was walked to the title folder");
        assert_eq!(std::fs::read_to_string(user.join("000/Auto")).unwrap(), "c");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_title_search_stops_at_the_documented_levels() {
        // The title folder four levels down is not where a console or a NAND tree puts it, and a picked drive must not be walked
        // to find it: there is no save to import, rather than a guess made by reading the whole folder.
        let root = temp("levels");
        write(&root.join("a/b/c/04007EF01F8AE000/000/Auto"), "x");
        assert_eq!(resolve_source(&root, FALLBACK_TITLE_ID), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_with_no_save_is_refused_before_the_current_save_is_touched() {
        let root = temp("unreadable");
        let (src, save) = (root.join("src"), root.join("save"));
        // A file with an extension is not a slot and there is no `000` anywhere, so this is not a save the game can open.
        write(&src.join("save.dat"), "x");
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "keep me",
        );
        assert!(import(&src, &save, FALLBACK_TITLE_ID, "1").is_err());
        assert_eq!(
            std::fs::read_to_string(account_dir(&save).join("000/Auto")).unwrap(),
            "keep me"
        );
        assert!(!save.join("backups").join("1").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_flat_save_folder_is_placed_under_000() {
        // A player can pick the contents of `000` without the folder itself. Those files belong inside `000`; copying them
        // one level above it is the import that reported success and showed nothing in game (issue #43).
        let root = temp("flat");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("Auto"), "a");
        write(&src.join("Global"), "g");
        let r = import(&src, &save, FALLBACK_TITLE_ID, "1").unwrap();
        assert!(r.readable, "the files landed in the folder the game reads");
        assert_eq!(r.files, 2);
        let user = account_dir(&save);
        assert_eq!(std::fs::read_to_string(user.join("000/Auto")).unwrap(), "a");
        assert_eq!(
            std::fs::read_to_string(user.join("000/Global")).unwrap(),
            "g"
        );
        assert!(!user.join("Auto").exists(), "nothing was left beside 000");
        let (text, ok) = import_message(Ok(r));
        assert!(ok, "{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_refuses_empty_or_overlapping_sources() {
        let root = temp("refuse");
        let save = root.join("save");
        std::fs::create_dir_all(root.join("empty")).unwrap();
        assert!(import(&root.join("empty"), &save, FALLBACK_TITLE_ID, "1").is_err());
        write(&save.join("account/u/000/Auto"), "x");
        assert!(import(&save.join("account"), &save, FALLBACK_TITLE_ID, "1").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_first_import_writes_the_engines_meta() {
        let root = temp("meta");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("000/Auto"), "a");
        import(&src, &save, FALLBACK_TITLE_ID, "1").unwrap();
        let meta =
            std::fs::read_to_string(save.join("account").join(format!("{DEFAULT_USER}.meta")))
                .unwrap();
        // The same attribute the engine wrote for this game's save.
        assert!(meta.starts_with("size=67108864\njournal=67108864\nowner=4007ef01f8ae000\n"));
        assert!(meta.contains(
            "attr=00e08a1ff07e00040100000000000000706d6352574645460000000000000000010000"
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn profiles_store_and_load() {
        let root = temp("profiles");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "first");
        // The test's slot is not a real save container; check_profile is exercised separately.
        store_profile(&save, "Run A", "1").unwrap();
        write(&user.join("000/Auto"), "second");
        let ps = profiles(&save);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "Run A");
        let r = load_profile(&save, &ps[0], FALLBACK_TITLE_ID, "2").unwrap();
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "first"
        );
        assert_eq!(
            std::fs::read_to_string(r.backup.unwrap().join("000/Auto")).unwrap(),
            "second"
        );
        assert_eq!(slots(&save).len(), 1);
        remove_profile(&save, &ps[0], "3").unwrap();
        assert!(profiles(&save).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_account_folder_is_always_the_titles_user() {
        let root = temp("account");
        let save = root.join("save");
        // A newer folder for another user must not win: the game always opens the title's fixed user id.
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "old",
        );
        let newer = save
            .join("account")
            .join("0A3C5F00000000000000000000000001");
        write(&newer.join("000/Auto"), "new");
        let now = std::time::SystemTime::now();
        std::fs::File::options()
            .write(true)
            .open(newer.join("000/Auto"))
            .unwrap()
            .set_modified(now)
            .unwrap();
        assert_eq!(account_dir(&save), save.join("account").join(DEFAULT_USER));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_saves_page_keeps_reading_the_games_own_account_folder() {
        let root = temp("account-fixed");
        let save = root.join("save");
        // An import once left a newer folder for another user: the game still reads DEFAULT_USER.
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "mine",
        );
        let other = save
            .join("account")
            .join("0A3C5F00000000000000000000000001");
        write(&other.join("000/Auto"), "theirs");
        let now = std::time::SystemTime::now();
        std::fs::File::options()
            .write(true)
            .open(other.join("000/Auto"))
            .unwrap()
            .set_modified(now)
            .unwrap();
        assert_eq!(account_dir(&save), save.join("account").join(DEFAULT_USER));
        assert_eq!(
            crate::saves::account_dir(&save).join("000/Auto"),
            save.join("account").join(DEFAULT_USER).join("000/Auto")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_slot_moves_to_the_backups_and_can_come_back() {
        let root = temp("slot");
        let save = root.join("save");
        let slot = save.join("account").join(DEFAULT_USER).join("000/Auto");
        write(&slot, "save");
        let listed = slots_with_sizes(&save);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].2 > 0, "the slot has a size the page can show");
        let to = archive_slot(&save, &slot, "7").unwrap();
        assert!(!slot.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "save");
        assert!(slots(&save).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn profiles_rename_and_duplicate() {
        let root = temp("profiles2");
        let save = root.join("save");
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "a",
        );
        store_profile(&save, "Run A", "1").unwrap();
        let p = profiles(&save).into_iter().next().unwrap();
        let renamed = rename_profile(&save, &p, "Run B").unwrap();
        assert_eq!(renamed.name, "Run B");
        assert_eq!(profiles(&save).len(), 1);
        let copy = duplicate_profile(&save, &renamed, "Run B copy").unwrap();
        assert_eq!(profiles(&save).len(), 2);
        assert!(copy.path.join("000/Auto").is_file());
        assert!(duplicate_profile(&save, &renamed, "Run B copy").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn export_writes_the_emulator_layout() {
        let root = temp("export");
        let save = root.join("save");
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "x",
        );
        let (to, n) = export(&save, &root.join("out"), FALLBACK_TITLE_ID).unwrap();
        assert_eq!(n, 1);
        assert!(to.ends_with("04007EF01F8AE000"));
        assert!(to.join("000/Auto").is_file());
        assert!(export(&save, &root.join("out"), FALLBACK_TITLE_ID).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A minimal valid container for metadata tests. It has no GameUserData,
    /// so gold and renown are correctly absent rather than invented.
    fn valid_slot(payload: &[u8]) -> Vec<u8> {
        let mut raw = vec![0u8; 0x124];
        raw[0..4].copy_from_slice(&0xCDEF_BCDDu32.to_le_bytes());
        raw[4..8].copy_from_slice(&0xE0u32.to_le_bytes());
        raw[0x104..0x108].copy_from_slice(&0xCDEF_BCDDu32.to_le_bytes());
        let frame = raw_zstd_frame(payload);
        raw[0x108..0x10C].copy_from_slice(&(frame.len() as u32).to_le_bytes());
        raw[0x10C..0x110].copy_from_slice(&fnv1_32(payload).to_le_bytes());
        raw[0x110..0x114].copy_from_slice(&3u32.to_le_bytes());
        raw.extend(frame);
        let header = fnv1a32(&raw[0x1F..0x1F + 0xE0]);
        raw[8..12].copy_from_slice(&header.to_le_bytes());
        raw
    }

    #[test]
    fn empty_profile_load_backs_up_the_current_save() {
        let root = temp("empty-profile");
        let save = root.join("save");
        let p = create_empty_profile(&save, "Fresh Run").unwrap();
        assert!(is_empty_profile(&p));
        assert!(profile_slots(&p, &crate::savefile::Ids::default()).is_empty());
        let old = account_dir(&save).join("000/Auto");
        write(&old, "old save");
        let loaded = load_empty_profile(&save, &p, FALLBACK_TITLE_ID, "9").unwrap();
        assert_eq!(loaded.files, 0);
        let backup = loaded.backup.expect("the previous save is kept");
        assert_eq!(
            std::fs::read_to_string(backup.join("000/Auto")).unwrap(),
            "old save"
        );
        assert!(account_dir(&save).is_dir());
        assert!(!account_dir(&save).join("000/Auto").exists());
        assert!(save
            .join("account")
            .join(format!("{DEFAULT_USER}.meta"))
            .is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn slot_metadata_keeps_corrupt_files_and_does_not_invent_values() {
        let root = temp("profile-meta");
        let save = root.join("save");
        let p = create_empty_profile(&save, "Run A").unwrap();
        std::fs::write(p.path.join("Auto"), valid_slot(b"payload")).unwrap();
        std::fs::write(p.path.join("Chapter_1"), b"not a save").unwrap();
        let slots = profile_slots(&p, &crate::savefile::Ids::default());
        assert_eq!(slots.len(), 2, "both files stay visible");
        assert!(slots.iter().any(|s| s.name == "Auto" && s.valid));
        assert!(slots.iter().any(|s| s.name == "Chapter_1" && !s.valid));
        let meta = profile_meta(&p, &crate::savefile::Ids::default());
        assert_eq!(meta.valid_slots, 1);
        assert_eq!((meta.gold, meta.renown), (None, None));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn game_bak_files_are_not_counted_as_slots() {
        let root = temp("profile-bak");
        let save = root.join("save");
        let p = create_empty_profile(&save, "Run A").unwrap();
        for i in 0..8 {
            write_bytes(
                &p.path.join(format!("Chapter_{i}")),
                &valid_slot(b"payload"),
            );
        }
        // The game's own backups have an extension, so the current-save list
        // never counted them. A profile copied from that save did.
        write_bytes(&p.path.join("Auto.bak"), &valid_slot(b"backup"));
        write_bytes(&p.path.join("Chapter_0.bak"), &valid_slot(b"backup"));

        let listed = profile_slots(&p, &crate::savefile::Ids::default());
        assert_eq!(
            listed.len(),
            8,
            "the eight slots, not the eight plus two .bak files"
        );
        assert!(listed
            .iter()
            .all(|s| !s.name.to_ascii_lowercase().ends_with(".bak")));
        let check = check_profile(&p);
        assert_eq!((check.slots, check.readable), (8, 8));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn global_data_beside_the_slots_is_not_a_broken_slot() {
        let root = temp("profile-global");
        let save = root.join("save");
        let p = create_empty_profile(&save, "Run A").unwrap();
        write_bytes(&p.path.join("000/Auto"), &valid_slot(b"payload"));
        write_bytes(&p.path.join("000/Global"), b"global data");

        let listed = profile_slots(&p, &crate::savefile::Ids::default());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Auto");
        let check = check_profile(&p);
        assert_eq!((check.slots, check.readable), (1, 1));
        assert!(check.ok());

        write(&account_dir(&save).join("000/Auto"), "old save");
        let loaded = load_profile_verified(&save, &p, FALLBACK_TITLE_ID, "7").unwrap();
        assert_eq!(
            std::fs::read(account_dir(&save).join("000/Auto")).unwrap(),
            valid_slot(b"payload")
        );
        assert_eq!(
            std::fs::read(account_dir(&save).join("000/Global")).unwrap(),
            b"global data"
        );
        assert!(loaded.backup.is_some());
        let current = slots(&save);
        assert_eq!(
            current.len(),
            1,
            "the marker and Global are data, not slots in the current save either"
        );
        assert_eq!(
            current[0].0.file_name().and_then(|n| n.to_str()),
            Some("Auto")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_profile_is_detected_from_the_installed_files() {
        let root = temp("active-profile");
        let save = root.join("save");
        let account = account_dir(&save);
        write_bytes(&account.join("000/Auto"), &valid_slot(b"same"));
        store_profile(&save, "Run A", "1").unwrap();
        let p = profiles(&save).into_iter().next().unwrap();
        assert_eq!(
            installed_profile(&save, &profiles(&save)).as_deref(),
            Some("Run A")
        );
        write_bytes(&account.join("000/Auto"), &valid_slot(b"changed"));
        assert_eq!(installed_profile(&save, &profiles(&save)), None);
        let loaded = load_profile_verified(&save, &p, FALLBACK_TITLE_ID, "2").unwrap();
        assert_eq!(
            std::fs::read(account.join("000/Auto")).unwrap(),
            valid_slot(b"same")
        );
        assert!(loaded.backup.is_some());
        assert_eq!(
            installed_profile(&save, &profiles(&save)).as_deref(),
            Some("Run A")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_and_duplicate_are_independent() {
        let root = temp("profile-import");
        let save = root.join("save");
        let source = root.join("backup");
        write_bytes(&source.join("000/Auto"), &valid_slot(b"source"));
        let imported = import_profile(&save, "Imported", &source, FALLBACK_TITLE_ID).unwrap();
        let duplicate = duplicate_profile(&save, &imported, "Copy").unwrap();
        std::fs::write(duplicate.path.join("000/Auto"), valid_slot(b"copy")).unwrap();
        assert_eq!(
            std::fs::read(imported.path.join("000/Auto")).unwrap(),
            valid_slot(b"source")
        );
        assert_ne!(
            std::fs::read(imported.path.join("000/Auto")).unwrap(),
            std::fs::read(duplicate.path.join("000/Auto")).unwrap()
        );
        let (to, n) = export_profile(&imported, &root.join("out"), FALLBACK_TITLE_ID).unwrap();
        assert_eq!(n, 1);
        assert!(to.join("000/Auto").is_file());
        assert!(to.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_profile_adapts_an_old_save_folder_and_a_flat_000_folder() {
        // The profile picker is the other way a save gets in, so it has to unwrap the same shapes before it validates. An old
        // `data/save` used to be copied with its `account/<user>` nesting, which passed validation and failed on load.
        let root = temp("profile-adapt");
        let save = root.join("save");
        let old = root.join("old-install");
        let account = old.join("data/save/account").join(DEFAULT_USER);
        write_bytes(&account.join("000/Auto"), &valid_slot(b"old"));
        let p = import_profile(&save, "Old", &old.join("data/save"), FALLBACK_TITLE_ID).unwrap();
        assert!(p.path.join("000/Auto").is_file());
        assert!(
            !p.path.join("account").exists(),
            "the profile holds the save, not the old folder's nesting"
        );
        load_profile_verified(&save, &p, FALLBACK_TITLE_ID, "1").unwrap();
        assert_eq!(
            std::fs::read(account_dir(&save).join("000/Auto")).unwrap(),
            valid_slot(b"old")
        );

        // A flat folder holding the contents of `000` belongs under `000` in the profile too, or loading compares files that are
        // not in the loaded save and puts the previous save back.
        let flat = root.join("flat");
        write_bytes(&flat.join("Auto"), &valid_slot(b"flat"));
        write(&flat.join("Global"), "global");
        let p = import_profile(&save, "Flat", &flat, FALLBACK_TITLE_ID).unwrap();
        assert_eq!(
            std::fs::read(p.path.join("000/Auto")).unwrap(),
            valid_slot(b"flat")
        );
        assert!(p.path.join("000/Global").is_file());
        load_profile_verified(&save, &p, FALLBACK_TITLE_ID, "2").unwrap();
        assert_eq!(
            std::fs::read(account_dir(&save).join("000/Auto")).unwrap(),
            valid_slot(b"flat")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_invalid_import_leaves_no_profile_behind() {
        let root = temp("profile-bad-import");
        let save = root.join("save");
        let source = root.join("bad");
        write(&source.join("Auto"), "not a slot");
        assert!(import_profile(&save, "Bad", &source, FALLBACK_TITLE_ID).is_err());
        assert!(profiles(&save).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn profile_names_and_collisions_are_refused() {
        let root = temp("profile-names");
        let save = root.join("save");
        assert!(create_empty_profile(&save, "../outside").is_err());
        assert!(create_empty_profile(&save, "  ").is_err());
        let p = create_empty_profile(&save, "Run A").unwrap();
        assert!(p.path.is_dir());
        assert!(create_empty_profile(&save, "Run A").is_err());
        assert_eq!(profiles(&save).len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_profile_load_leaves_the_current_save_alone() {
        let root = temp("profile-bad-load");
        let save = root.join("save");
        let current = account_dir(&save).join("000/Auto");
        write(&current, "keep me");
        let p = create_empty_profile(&save, "Corrupt").unwrap();
        write(&p.path.join("Auto"), "not a save");
        assert!(load_profile_verified(&save, &p, FALLBACK_TITLE_ID, "4").is_err());
        assert_eq!(std::fs::read_to_string(&current).unwrap(), "keep me");
        assert!(installed_profile(&save, &profiles(&save)).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_slot_original_restore_keeps_the_replaced_copy() {
        let root = temp("slot-original");
        let slot = root.join("Auto");
        write(&slot, "new");
        write(&original_slot_path(&slot), "old");
        let (from, replaced) = restore_slot_backup(&slot).unwrap();
        assert!(
            from.to_string_lossy().ends_with("Auto.orig"),
            "an old folder restores its .orig: {from:?}"
        );
        assert_eq!(std::fs::read_to_string(&slot).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(replaced).unwrap(), "new");
        let _ = std::fs::remove_dir_all(root);
    }

    // The scheme the save editor ships: the untouched original is `<slot>.bak`, each replaced copy `<slot>.bak1`, `.bak2`
    // …, the highest number newest, and no save ever renames or deletes the ones before it.
    #[test]
    fn slot_backups_number_upwards_and_never_reuse_a_name() {
        let root = temp("slot-numbered");
        let slot = root.join("Chapter_0");
        write(&slot, "original");
        let first = keep_slot_backup(&slot).unwrap();
        assert!(
            first.to_string_lossy().ends_with("Chapter_0.bak"),
            "first keep is the bare .bak: {first:?}"
        );
        write(&slot, "second");
        let second = keep_slot_backup(&slot).unwrap();
        assert!(
            second.to_string_lossy().ends_with("Chapter_0.bak1"),
            "then .bak1: {second:?}"
        );
        write(&slot, "third");
        let third = keep_slot_backup(&slot).unwrap();
        assert!(
            third.to_string_lossy().ends_with("Chapter_0.bak2"),
            "then .bak2: {third:?}"
        );
        let listed = slot_backups(&slot);
        let names: Vec<String> = listed
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["Chapter_0.bak", "Chapter_0.bak1", "Chapter_0.bak2"]
        );
        assert_eq!(
            std::fs::read_to_string(&listed[0]).unwrap(),
            "original",
            "the original is never overwritten"
        );

        // Restoring takes the newest, keeps the file it replaces as the next number, and says where it read from.
        let (from, kept) = restore_slot_backup(&slot).unwrap();
        assert!(
            from.to_string_lossy().ends_with("Chapter_0.bak2"),
            "newest first: {from:?}"
        );
        assert!(
            kept.to_string_lossy().ends_with("Chapter_0.bak3"),
            "the replaced copy takes the next number: {kept:?}"
        );
        assert_eq!(std::fs::read_to_string(&slot).unwrap(), "third");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "third");
        assert_eq!(slot_backups(&slot).len(), 4, "nothing was dropped");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn import_unwraps_an_account_folder_and_the_install_above_it() {
        // The old install's `account` folder and the install folder itself are both common picks. Both describe the same save:
        // `<install>/data/save/account/<user>/000`.
        let root = temp("install");
        let old = root.join("old-install");
        let user = old.join("data/save/account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "old save");
        let save = root.join("new-install/save");

        let r = import(
            &old.join("data/save/account"),
            &save,
            FALLBACK_TITLE_ID,
            "1",
        )
        .unwrap();
        assert_eq!(r.from.as_deref(), Some(user.as_path()));
        assert!(r.readable);
        assert_eq!(
            std::fs::read_to_string(account_dir(&save).join("000/Auto")).unwrap(),
            "old save"
        );

        let r = import(&old, &save, FALLBACK_TITLE_ID, "2").unwrap();
        assert_eq!(r.from.as_deref(), Some(user.as_path()));
        assert!(r.readable);
        assert_eq!(
            std::fs::read_to_string(account_dir(&save).join("000/Auto")).unwrap(),
            "old save"
        );

        // An account folder that an older import left nested is still found: the folder holding `000` is the save.
        let nested = root.join("nested-install/data/save/account");
        let inner = nested
            .join(DEFAULT_USER)
            .join("0A3C5F00000000000000000000000001");
        write(&inner.join("000/Auto"), "nested save");
        let r = import(&nested, &save, FALLBACK_TITLE_ID, "3").unwrap();
        assert_eq!(r.from.as_deref(), Some(inner.as_path()));
        assert!(r.readable);
        assert_eq!(
            std::fs::read_to_string(account_dir(&save).join("000/Auto")).unwrap(),
            "nested save"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
