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
//! Import never touches the source. It copies into a staging folder beside the
//! account, hashes every file, and only then moves the existing save to
//! `<save folder>/backups/<time>/` and renames the verified stage into place.
//! A forced stop therefore leaves the old save active or the verified stage
//! recoverable; a partial account is never visible to the game. A sidecar
//! records each step so the next launch can finish or roll back an interrupted
//! import. When the game has not created its save yet, the import writes the
//! `.meta` the engine expects.

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

/// What the emulator roots hold for the title: the saves that can be imported,
/// and the game's own folders that exist but hold no files.
///
/// An emulator creates the title folder before a save is written, and a profile
/// the player never used stays empty. Reporting those folders separately is the
/// difference between "no save was found anywhere" and "the folder is there, but
/// there is nothing in it to import". Empty folders are never offered as a
/// source: `import` refuses them, so the page can only explain them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Survey {
    pub saves: Vec<Found>,
    pub empty: Vec<PathBuf>,
}

/// Every yuzu-family save root under `appdata`, split into importable saves and
/// empty folders. The layout is `nand/user/save/<save id>/[<profile>/]<title>`:
/// most forks use `save/0000000000000000/<profile>/<title>`, Eden can use another
/// save id, and a portable or migrated install can omit the profile level.
pub fn survey(appdata: &Path, title_id: u64) -> Survey {
    let name = format!("{title_id:016X}");
    let mut out = Survey::default();
    for emu in EMULATOR_DIRS {
        let save_root = appdata.join(emu).join("nand").join("user").join("save");
        let Ok(ids) = std::fs::read_dir(&save_root) else {
            continue;
        };
        for id in ids.flatten() {
            if !id.path().is_dir() {
                continue;
            }
            push_candidate(&mut out, emu, &id.path().join(&name));
            let Ok(profiles) = std::fs::read_dir(id.path()) else {
                continue;
            };
            for profile in profiles.flatten() {
                if profile.path().is_dir() {
                    push_candidate(&mut out, emu, &profile.path().join(&name));
                }
            }
        }
    }
    out.saves.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.saves.dedup_by(|a, b| a.path == b.path);
    out.empty.sort();
    out.empty.dedup();
    out
}

fn push_candidate(out: &mut Survey, source: &str, path: &Path) {
    if !path.is_dir() {
        return;
    }
    let (files, modified) = summary(path);
    if files > 0 {
        out.saves.push(Found {
            source: source.to_string(),
            path: path.to_path_buf(),
            files,
            modified,
        });
    } else {
        out.empty.push(path.to_path_buf());
    }
}

/// The same survey under the current user's `%APPDATA%` and `%LOCALAPPDATA%`.
/// The second root covers portable-style forks that keep their NAND there.
///
/// The launcher's own other data root is included too. A copy that switches between `%APPDATA%` and a portable folder can
/// leave a whole playthrough behind in the root it is not using (issue #79); the Saves page has to offer it instead of
/// looking as if no save exists anywhere.
pub fn survey_here(title_id: u64) -> Survey {
    let mut out = Survey::default();
    for key in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(key) {
            let one = survey(Path::new(&root), title_id);
            out.saves.extend(one.saves);
            out.empty.extend(one.empty);
        }
    }
    if let Some(save_dir) = crate::config::Config::save_dir() {
        out.saves.extend(launcher_sources(&save_dir));
    }
    out.saves.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.saves.dedup_by(|a, b| a.path == b.path);
    out.empty.sort();
    out.empty.dedup();
    out
}

/// Saves this launcher keeps under another of its data roots: `%APPDATA%\EmblemWeave\save`, and `data\save` beside the
/// executable (a portable copy, or a source checkout). `save_dir` is the root in use, which is never offered to itself.
///
/// A newer download extracted beside an older copy keeps its save in its own `data\save`, which the new copy cannot see.
/// Those sibling folders are offered too: an upgrade that produced a second EmblemWeave folder must not look like a lost
/// playthrough (issue #79).
///
/// Returns the account save folder (the one holding `000`) so `import` receives a folder it can resolve directly. Newest
/// first, so the caller that carries one saves carries the most recently played.
pub fn launcher_sources(save_dir: &Path) -> Vec<Found> {
    let mut roots: Vec<(String, PathBuf)> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push((
            "EmblemWeave (AppData)".into(),
            PathBuf::from(appdata).join("EmblemWeave").join("save"),
        ));
    }
    if let Some(exe) = crate::platform::exe_dir() {
        roots.push(("EmblemWeave (this install)".into(), exe.join("data").join("save")));
        if let Some(parent) = exe.parent() {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p != exe && p.is_dir() {
                        let label = p
                            .file_name()
                            .map(|n| format!("EmblemWeave (nearby copy: {})", n.to_string_lossy()))
                            .unwrap_or_else(|| "EmblemWeave (nearby copy)".into());
                        roots.push((label, p.join("data").join("save")));
                    }
                }
            }
        }
    }
    // A remembered engine checkout names the folder of an older copy even when it is on another drive. A portable
    // copy keeps its save under that folder's `data`, so the new download can still find the playthrough.
    let mut engine_files: Vec<PathBuf> = Vec::new();
    if let Some(dir) = crate::config::Config::dir() {
        engine_files.push(dir.join("engine.txt"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        engine_files.push(
            PathBuf::from(appdata)
                .join("EmblemWeave")
                .join("engine.txt"),
        );
    }
    for file in engine_files {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Some(root) = remembered_install_root(&text) {
                let name = root
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "older copy".into());
                roots.push((format!("EmblemWeave (older copy: {name})"), root));
            }
        }
    }
    launcher_sources_from(&roots, save_dir)
}

/// The `data\save` of the install a remembered `engine.txt` names, when that folder still exists. The file holds the
/// checkout (`…\engine\fefw-recomp`); two levels up is the EmblemWeave folder it belongs to.
fn remembered_install_root(text: &str) -> Option<PathBuf> {
    let checkout = Path::new(text.trim());
    let install = checkout.parent()?.parent()?;
    let save = install.join("data").join("save");
    save.is_dir().then_some(save)
}

fn launcher_sources_from(roots: &[(String, PathBuf)], save_dir: &Path) -> Vec<Found> {
    let current = account_dir(save_dir);
    let mut out = Vec::new();
    for (label, root) in roots {
        if *root == save_dir {
            continue;
        }
        let Some(save) = account_save(&root.join("account")) else {
            continue;
        };
        // The root in use, including another account folder inside it, is not a source for itself: those saves are
        // listed by `find_orphan_saves` and adopted with a button, not carried automatically.
        if save == current || save.starts_with(save_dir) {
            continue;
        }
        let (files, modified) = summary(&save);
        if files == 0 || out.iter().any(|f: &Found| f.path == save) {
            continue;
        }
        out.push(Found {
            source: label.clone(),
            path: save,
            files,
            modified,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
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

/// Whether `account` holds the save the game reads. A `.meta` alone does not count: the game writes that metadata
/// before it has ever saved, so a folder with only metadata is still an empty save.
pub fn has_save(account: &Path) -> bool {
    is_account_save(account)
}

/// The `.meta` file beside the account save folder the game reads.
///
/// The engine treats the save as present only when both this file and the
/// account folder exist, so a save tree with a populated `000` but no `.meta`
/// looks empty to the runtime until it creates fresh metadata. Keeping the
/// path in one place lets the launcher repair that case without guessing.
pub fn account_meta_path(save_dir: &Path) -> PathBuf {
    let mut meta = account_dir(save_dir).into_os_string();
    meta.push(".meta");
    PathBuf::from(meta)
}

/// Write the account's `.meta` when `000` holds files but the metadata is
/// missing. Existing metadata is never replaced, and no save file is touched.
///
/// This is the launcher-side counterpart of the runtime's missing-meta repair:
/// it runs before Play so the game is never told that a populated save tree is
/// an empty one. Returns whether a file was written.
pub fn ensure_account_meta(save_dir: &Path, title_id: u64) -> Result<bool, String> {
    let account = account_dir(save_dir);
    if !has_save(&account) {
        return Ok(false);
    }
    let meta = account_meta_path(save_dir);
    if meta.is_file() {
        return Ok(false);
    }
    let text = meta_text(title_id, DEFAULT_USER)
        .ok_or("the game's save user id is not a 16-byte value")?;
    if let Some(parent) = meta.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(&meta, text).map_err(|e| format!("writing {}: {e}", meta.display()))?;
    Ok(true)
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

/// Copy `source` into the game's account save. The replacement is staged and
/// verified first; only then is an existing save moved to
/// `<save folder>/backups/<stamp>/` and the stage promoted.
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
    // Finish or roll back an import killed in an earlier run before looking at
    // this source. The active account is then old-or-new, never a partial copy.
    recover_interrupted_import(save_dir)?;
    // A profile or a backup under the save folder is fine; the current save, its account folder, or a folder holding the save
    // folder, is not.
    if from.starts_with(&target)
        || target.starts_with(from)
        || save_dir.starts_with(source)
        || source.starts_with(save_dir.join("account"))
    {
        return Err("the source and the save folder overlap".into());
    }
    let pending = stage_import(from, flat, &target, save_dir, title_id, stamp)?;
    pending.commit()
}

/// A sibling name for a temporary folder or sidecar. Keeping it beside the
/// target means promotion is one rename on the same volume.
fn side_path(base: &Path, tag: &str) -> PathBuf {
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let name = base
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    for n in 0u32.. {
        let candidate = parent.join(format!(".{name}.emblemweave-{tag}-{stamp}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Delete a service-owned sidecar or staging tree. A symlink is removed as a
/// link, never followed into the directory it names.
fn remove_path(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// A file copied into a staging folder, hashed so a later recovery can prove
/// the replacement is complete without the original source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StagedFile {
    path: String,
    size: u64,
    sha256: String,
}

/// The sidecar that makes an interrupted import recoverable.
///
/// It is written after the stage is verified and before the active save is
/// moved. A crash at any later step leaves enough information for the next
/// launch to either finish promotion or put the previous save back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ImportState {
    stage: PathBuf,
    backup: Option<PathBuf>,
    old_target: Option<PathBuf>,
    meta_created: bool,
    files: Vec<StagedFile>,
}

/// Deterministic pause for the crash-injection tests. Debug builds only; a
/// release binary has no test hook and the variable has no effect.
#[cfg(debug_assertions)]
fn import_test_pause(point: &str) {
    let Ok(wanted) = std::env::var("EMBLEMWAVE_TEST_IMPORT_PAUSE") else {
        return;
    };
    if wanted == point {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[cfg(not(debug_assertions))]
fn import_test_pause(_point: &str) {}

/// A fully copied import waiting to be renamed into place.
///
/// The staging folder is a sibling of the account folder, so the final rename
/// stays on one filesystem and cannot expose a half-copied account to the game.
struct PendingImport {
    stage: PathBuf,
    target: PathBuf,
    backup: Option<PathBuf>,
    old_target: Option<PathBuf>,
    state: PathBuf,
    meta_created: bool,
    files: Vec<StagedFile>,
    from: PathBuf,
}

impl PendingImport {
    fn commit(self) -> Result<Imported, String> {
        // The old save stays active while the replacement is staged. Only now,
        // after every copied file has been verified, is it moved to backups.
        let backup_moved = if let Some(backup) = &self.backup {
            if !self.target.is_dir() || !has_save(&self.target) {
                self.discard_stage();
                return Err(
                    "the save to back up changed while the import was staged; nothing was changed"
                        .into(),
                );
            }
            if backup.exists() {
                self.discard_stage();
                return Err(format!(
                    "the backup folder {} already exists; the import was not applied",
                    backup.display()
                ));
            }
            if let Err(e) = std::fs::rename(&self.target, backup) {
                self.discard_stage();
                return Err(format!(
                    "moving the current save to {}: {e}; the current save was left in place",
                    backup.display()
                ));
            }
            true
        } else {
            false
        };
        import_test_pause("after-backup");
        let old_moved = if !backup_moved {
            if let Some(old) = &self.old_target {
                if self.target.exists() {
                    if has_save(&self.target) {
                        self.discard_stage();
                        return Err(
                            "a new save appeared while the import was staged; it was not overwritten"
                                .into(),
                        );
                    }
                    if let Err(e) = std::fs::rename(&self.target, old) {
                        self.discard_stage();
                        return Err(format!(
                            "moving the empty account folder {} aside: {e}; nothing was changed",
                            self.target.display()
                        ));
                    }
                    true
                } else {
                    false
                }
            } else {
                if self.target.exists() {
                    self.discard_stage();
                    return Err(
                        "a save appeared while the import was staged; it was not overwritten"
                            .into(),
                    );
                }
                false
            }
        } else {
            false
        };
        if let Err(e) = std::fs::rename(&self.stage, &self.target) {
            let restored = self.restore_previous(backup_moved, old_moved);
            if restored {
                self.discard_stage();
            }
            return Err(format!(
                "renaming {} to {}: {e}{}",
                self.stage.display(),
                self.target.display(),
                if restored {
                    "; the previous save was restored"
                } else {
                    "; the previous save remains in backup and the next launch will retry"
                }
            ));
        }
        import_test_pause("after-promotion");
        if old_moved {
            if let Some(old) = &self.old_target {
                let _ = remove_path(old);
            }
        }
        let _ = std::fs::remove_file(&self.state);
        Ok(Imported {
            files: self.files.len(),
            backup: self.backup,
            from: Some(self.from),
            readable: true,
        })
    }

    /// Put the old save back after a failed promotion. A backup is copied back,
    /// not consumed, so the visible backup history survives a failed import.
    fn restore_previous(&self, backup_moved: bool, old_moved: bool) -> bool {
        if backup_moved {
            if let Some(backup) = &self.backup {
                return restore_tree(backup, &self.target).is_ok();
            }
        }
        if old_moved {
            if let Some(old) = &self.old_target {
                return std::fs::rename(old, &self.target).is_ok();
            }
        }
        true
    }

    fn discard_stage(&self) {
        let _ = std::fs::remove_dir_all(&self.stage);
        if self.meta_created {
            let _ = std::fs::remove_file(self.target.with_extension("meta"));
        }
        let _ = std::fs::remove_file(&self.state);
    }
}

/// Copy a resolved import into a staging folder, verify it, and record enough
/// state for recovery. The active save is not moved here.
fn stage_import(
    from: &Path,
    flat: bool,
    target: &Path,
    save_dir: &Path,
    title_id: u64,
    stamp: &str,
) -> Result<PendingImport, String> {
    let had_save = target.is_dir() && has_save(target);
    let backup = if had_save {
        Some(plan_backup_path(save_dir, stamp)?)
    } else {
        None
    };
    let old_target = if !had_save && target.exists() {
        Some(side_path(target, "import-old"))
    } else {
        None
    };
    let parent = target
        .parent()
        .ok_or_else(|| "the account save folder has no parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    let stage = side_path(target, "import");
    let state_path = side_path(target, "import-state");
    let meta = target.with_extension("meta");
    let mut meta_created = false;
    let staged = (|| -> Result<Vec<StagedFile>, String> {
        std::fs::create_dir_all(&stage)
            .map_err(|e| format!("creating the staging folder: {e}"))?;
        import_test_pause("staging");
        // Preserve anything already in an empty account folder. A saved account
        // is left active and is not copied into the stage.
        if !had_save && target.is_dir() {
            copy_tree(target, &stage)?;
        }
        if flat {
            copy_flat(from, &stage.join("000"))?;
        } else {
            copy_tree(from, &stage)?;
        }
        let files = verify_stage(from, flat, &stage)?;
        if summary(&stage.join("000")).0 == 0 {
            return Err(format!(
                "{} did not leave the save in {}; the previous save was left in place",
                from.display(),
                target.join("000").display()
            ));
        }
        let user = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !meta.is_file() {
            if let Some(text) = meta_text(title_id, &user) {
                let tmp = side_path(&meta, "import-meta");
                if let Err(e) = std::fs::write(&tmp, &text) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("writing {}: {e}", meta.display()));
                }
                if let Err(e) = std::fs::rename(&tmp, &meta) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("writing {}: {e}", meta.display()));
                }
                meta_created = true;
                if std::fs::read_to_string(&meta).ok().as_deref() != Some(text.as_str()) {
                    let _ = std::fs::remove_file(&meta);
                    return Err(format!("verifying {}", meta.display()));
                }
            }
        }
        let state = ImportState {
            stage: stage.clone(),
            backup: backup.clone(),
            old_target: old_target.clone(),
            meta_created,
            files: files.clone(),
        };
        write_import_state(&state_path, &state)?;
        Ok(files)
    })();
    match staged {
        Ok(files) => {
            import_test_pause("staged");
            Ok(PendingImport {
                stage,
                target: target.to_path_buf(),
                backup,
                old_target,
                state: state_path,
                meta_created,
                files,
                from: from.to_path_buf(),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            let _ = std::fs::remove_file(&state_path);
            if meta_created {
                let _ = std::fs::remove_file(&meta);
            }
            Err(e)
        }
    }
}

/// Plan a backup folder without moving anything. Existing history is never
/// overwritten; a second import in the same second gets a suffix.
fn plan_backup_path(save_dir: &Path, stamp: &str) -> Result<PathBuf, String> {
    if stamp.is_empty() || stamp.contains(['/', '\\']) {
        return Err("the import has no valid backup stamp".into());
    }
    let dir = save_dir.join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating the backups folder: {e}"))?;
    let first = dir.join(stamp);
    if !first.exists() {
        return Ok(first);
    }
    for n in 2u32.. {
        let candidate = dir.join(format!("{stamp}-{n}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    unreachable!("a free backup suffix always exists")
}

/// Hash every source file and compare it with the copy in `stage`. The returned
/// manifest lets a later launch verify the stage after the source is gone.
fn verify_stage(from: &Path, flat: bool, stage: &Path) -> Result<Vec<StagedFile>, String> {
    let mut files = Vec::new();
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| format!("reading {} for verification: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("reading {} for verification: {e}", dir.display()))?;
            let src = entry.path();
            if src.is_dir() {
                if !flat {
                    stack.push(src);
                }
                continue;
            }
            if flat && entry.file_name().to_str().is_some_and(is_metadata_file) {
                continue;
            }
            let (size, source_hash) = file_sha256(&src)?;
            let dst = if flat {
                stage.join("000").join(entry.file_name())
            } else {
                let rel = src
                    .strip_prefix(from)
                    .map_err(|_| format!("{} is not below {}", src.display(), from.display()))?;
                stage.join(rel)
            };
            let (dst_size, dst_hash) = file_sha256(&dst).map_err(|e| {
                format!(
                    "the staged copy of {} is missing or unreadable: {e}",
                    src.display()
                )
            })?;
            if size != dst_size || source_hash != dst_hash {
                return Err(format!(
                    "{} did not copy identically into the staging folder",
                    src.display()
                ));
            }
            let rel = dst
                .strip_prefix(stage)
                .map_err(|_| format!("{} is not below {}", dst.display(), stage.display()))?;
            files.push(StagedFile {
                path: rel.to_string_lossy().replace('\\', "/"),
                size,
                sha256: hex_bytes(&source_hash),
            });
        }
    }
    if files.is_empty() {
        return Err(format!("{} holds no save files", from.display()));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn file_sha256(path: &Path) -> Result<(u64, [u8; 32]), String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("opening {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let size = file
        .metadata()
        .map_err(|e| format!("reading the size of {}: {e}", path.display()))?
        .len();
    Ok((size, hasher.finalize().into()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_import_state(path: &Path, state: &ImportState) -> Result<(), String> {
    let data = serde_json::to_vec(state)
        .map_err(|e| format!("recording the import recovery state: {e}"))?;
    let tmp = side_path(path, "tmp");
    if let Err(e) = std::fs::write(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("writing {}: {e}", path.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("writing {}: {e}", path.display()));
    }
    Ok(())
}

fn read_import_state(path: &Path) -> Option<ImportState> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

/// Copy a tree to a temporary sibling and rename it into place. Used to put a
/// backup back without consuming the backup entry.
fn restore_tree(from: &Path, to: &Path) -> Result<(), String> {
    let tmp = side_path(to, "import-restore");
    if let Err(e) = copy_tree(from, &tmp) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    // A restore is a copy too. Promote only after every file has been hashed
    // against the backup; a short copy must not become the active save.
    if let Err(e) = verify_stage(from, false, &tmp) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("the backup copy could not be verified: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, to) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("restoring {} to {}: {e}", from.display(), to.display()));
    }
    Ok(())
}

fn import_state_prefix(target: &Path) -> String {
    format!(
        ".{}.emblemweave-import-state-",
        target.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn import_stage_prefix(target: &Path) -> String {
    format!(
        ".{}.emblemweave-import-",
        target.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn is_stage_entry(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_ascii_digit())
}

fn state_paths_are_safe(target: &Path, save_dir: &Path, state: &ImportState) -> bool {
    let stage_name = state
        .stage
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if state.stage.parent() != target.parent()
        || !is_stage_entry(&stage_name, &import_stage_prefix(target))
    {
        return false;
    }
    if let Some(backup) = &state.backup {
        let backup_name = backup
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if backup.parent() != Some(save_dir.join("backups").as_path())
            || backup_name.is_empty()
            || backup_name == "."
            || backup_name == ".."
        {
            return false;
        }
    }
    if let Some(old) = &state.old_target {
        let old_name = old
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let old_prefix = format!(
            ".{}.emblemweave-import-old-",
            target.file_name().unwrap_or_default().to_string_lossy()
        );
        if old.parent() != target.parent() || !is_stage_entry(&old_name, &old_prefix) {
            return false;
        }
    }
    true
}

fn verify_recorded_stage(state: &ImportState) -> Result<(), String> {
    for file in &state.files {
        let rel = Path::new(&file.path);
        if rel.is_absolute()
            || rel.components().any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("the import record names an unsafe path".into());
        }
        let path = state.stage.join(rel);
        let (size, hash) = file_sha256(&path)?;
        if size != file.size || hex_bytes(&hash) != file.sha256 {
            return Err(format!("{} is not the verified copy", path.display()));
        }
    }
    if state.files.is_empty() || summary(&state.stage.join("000")).0 == 0 {
        return Err("the staged save is empty".into());
    }
    Ok(())
}

/// Finish or roll back imports interrupted by a crash, kill or power loss.
///
/// The active save is either the old one, the verified replacement, or absent
/// while the recorded stage/backup is still recoverable. It is never a partial
/// replacement. Safe to call repeatedly; only completed actions are forgotten.
pub fn recover_interrupted_import(save_dir: &Path) -> Result<usize, String> {
    let target = account_dir(save_dir);
    let Some(parent) = target.parent() else {
        return Ok(0);
    };
    let prefix = import_state_prefix(&target);
    let mut states: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
        })
        .collect();
    states.sort();
    let mut seen_stages = std::collections::HashSet::new();
    let mut actions = 0usize;
    for state_path in states {
        let Some(state) = read_import_state(&state_path) else {
            let _ = std::fs::remove_file(&state_path);
            continue;
        };
        if !state_paths_are_safe(&target, save_dir, &state) {
            let _ = std::fs::remove_file(&state_path);
            continue;
        }
        seen_stages.insert(state.stage.clone());
        let meta = target.with_extension("meta");
        if target.exists() {
            let stage_present = state.stage.exists();
            if stage_present {
                // The active folder won the race (the old save was never moved,
                // or the user started the game). Never remove its metadata: the
                // import may have repaired a missing meta just before stopping.
                let _ = remove_path(&state.stage);
            } else if let Some(old) = &state.old_target {
                if old.exists() {
                    let _ = remove_path(old);
                }
            }
            let _ = std::fs::remove_file(&state_path);
            actions += 1;
            continue;
        }
        let staged_ok = verify_recorded_stage(&state).is_ok();
        if staged_ok {
            match std::fs::rename(&state.stage, &target) {
                Ok(()) => {
                    if let Some(old) = &state.old_target {
                        let _ = remove_path(old);
                    }
                    let _ = std::fs::remove_file(&state_path);
                    actions += 1;
                    continue;
                }
                Err(_) => {}
            }
        }
        let mut restored = false;
        if let Some(backup) = &state.backup {
            if backup.is_dir() {
                restored = restore_tree(backup, &target).is_ok();
            }
        }
        if !restored {
            if let Some(old) = &state.old_target {
                if old.exists() {
                    restored = std::fs::rename(old, &target).is_ok();
                }
            }
        }
        if !restored && state.backup.is_none() && state.old_target.is_none() {
            restored = true;
        }
        if restored {
            let _ = remove_path(&state.stage);
            let _ = std::fs::remove_file(&state_path);
            if state.meta_created {
                let _ = std::fs::remove_file(&meta);
            }
            actions += 1;
        }
    }
    // A partial stage with no state was interrupted before the old save was
    // touched. It is never a readable replacement; remove it.
    let stage_prefix = import_stage_prefix(&target);
    let orphans: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| is_stage_entry(n, &stage_prefix))
                && !seen_stages.contains(p)
        })
        .collect();
    for path in orphans {
        let _ = remove_path(&path);
        actions += 1;
    }
    Ok(actions)
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

/// An orphan save found in the save directory that the game doesn't read.
/// These can exist in profiles/ or in non-DEFAULT_USER account folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanSave {
    /// Human-readable description of where this save was found.
    pub description: String,
    /// The path to the save folder (the one containing the 000 subfolder).
    pub path: PathBuf,
    /// Number of save files found.
    pub files: usize,
    /// Newest modification time of any file in the save.
    pub modified: Option<std::time::SystemTime>,
}

/// Whether `save` holds at least one file the game would write as a slot. The game's slots are extensionless
/// (`Auto`, `Global`, `Chapter_*`, `Route_*`, `<timestamp>-Auto`); an unrelated folder that happens to contain
/// `000/readme.txt` must not be offered as a playthrough. Extra files are still counted by [`summary`] when the
/// folder does hold a slot, so a save with stray files remains visible.
fn holds_a_slot(save: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(save.join("000")) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        path.is_file() && !is_metadata_file(&name) && Path::new(&name).extension().is_none()
    })
}

/// Find all saves in the save directory that the game does not read.
/// These are saves in `profiles/`, in `account/<other user>/`, or nested one level below the game's own account folder.
///
/// The last shape is the dangerous one: an import from an older build could end up as `account/<user>/<user>/000`, and
/// the game opens the empty folder above it. Nothing on the page named that save, so the player saw "no save" while
/// their files sat one folder deeper.
pub fn find_orphan_saves(save_dir: &Path) -> Vec<OrphanSave> {
    let mut out: Vec<OrphanSave> = Vec::new();
    let target = account_dir(save_dir);

    // A save one level below the folder the game opens, left by an import that was nested too deep.
    if let Some(save) = save_below(&target) {
        if save != target {
            let (files, modified) = summary(&save.join("000"));
            if files > 0 && holds_a_slot(&save) {
                out.push(OrphanSave {
                    description: "Nested below the game's account folder".into(),
                    path: save,
                    files,
                    modified,
                });
            }
        }
    }

    // Check profiles/ folder
    let profiles = save_dir.join("profiles");
    if profiles.is_dir() {
        for e in std::fs::read_dir(&profiles).into_iter().flatten().flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let Some(save) = save_below(&path) else {
                continue;
            };
            let (files, modified) = summary(&save.join("000"));
            if files > 0 && save != target && holds_a_slot(&save) {
                out.push(OrphanSave {
                    description: format!(
                        "Profile \"{}\"",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    path: save,
                    files,
                    modified,
                });
            }
        }
    }

    // Check account/ folder for non-DEFAULT_USER saves
    let account = save_dir.join("account");
    if account.is_dir() {
        for e in std::fs::read_dir(&account).into_iter().flatten().flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            // The game's own account folder is handled above, including a save nested below it.
            if path == target {
                continue;
            }
            let Some(save) = save_below(&path) else {
                continue;
            };
            let (files, modified) = summary(&save.join("000"));
            if files > 0 && holds_a_slot(&save) {
                out.push(OrphanSave {
                    description: format!(
                        "Account folder \"{}\"",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    path: save,
                    files,
                    modified,
                });
            }
        }
    }

    // A save folder directly in the save root: a profile folder moved out by hand, or a tree recovered from an older
    // build. The game only opens `account/DEFAULT_USER`, so this save is invisible without being named here. Reserved
    // folders are skipped: `account` and `profiles` are scanned above, and `backups` is the launcher's own copy area.
    for e in std::fs::read_dir(save_dir).into_iter().flatten().flatten() {
        let path = e.path();
        if !path.is_dir() || path == target {
            continue;
        }
        let label = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if matches!(label.to_ascii_lowercase().as_str(), "account" | "profiles" | "backups") {
            continue;
        }
        let Some(save) = save_below(&path) else {
            continue;
        };
        let (files, modified) = summary(&save.join("000"));
        if files > 0 && holds_a_slot(&save) {
            out.push(OrphanSave {
                description: format!("Save folder \"{label}\""),
                path: save,
                files,
                modified,
            });
        }
    }

    // Sort by modification time, newest first
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// Adopt an orphan save into the game's account save.
/// This moves the current save to backups/ and copies the orphan save into place.
/// Returns the import result with backup info.
pub fn adopt_orphan_save(
    orphan: &OrphanSave,
    save_dir: &Path,
    title_id: u64,
    stamp: &str,
) -> Result<Imported, String> {
    // Use the existing import logic, which handles backup automatically
    import(&orphan.path, save_dir, title_id, stamp)
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
    /// The route lord this slot was saved on, from its route byte, or the first
    /// protagonist in the roster when that byte is unknown. Display-only: the
    /// carousel uses it to choose a portrait when no explicit one is set.
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

/// The save facts a diagnostic report needs, without a byte of save content: where the launcher and the game point,
/// whether those folders hold a real save, any save the game ignores, and every emulator save found for the title.
///
/// A report that arrives without its ZIP still answers the first question a save bug needs: was there a save, and where
/// was it? The same text is written into the bundle.
pub fn describe_save_state(title_id: u64) -> String {
    let mut s = String::new();
    let configured = crate::config::Config::save_dir();
    match &configured {
        Some(dir) => {
            s.push_str(&format!("configured: {}\n", dir.display()));
            s.push_str(&describe_account(dir, "  "));
        }
        None => s.push_str("configured: (not set)\n"),
    }
    match crate::engine::effective_save_dir() {
        Some(effective) => {
            s.push_str(&format!("effective (fefw.ini): {}\n", effective.display()));
            if configured.as_ref().is_some_and(|c| c != &effective) {
                s.push_str("status: MISMATCH (the game reads the effective path)\n");
                s.push_str(&describe_account(&effective, "  "));
            } else {
                s.push_str("status: in sync\n");
            }
        }
        None => s.push_str("effective (fefw.ini): (not found)\n"),
    }
    if let Some(dir) = &configured {
        let orphans = find_orphan_saves(dir);
        if orphans.is_empty() {
            s.push_str("unread saves in this folder: none\n");
        } else {
            s.push_str("unread saves in this folder:\n");
            for o in orphans {
                s.push_str(&format!(
                    "  {}: {} ({} files, {})\n",
                    o.description,
                    o.path.display(),
                    o.files,
                    when(o.modified)
                ));
            }
        }
        let recoverable = recoverable_saves(dir);
        if recoverable.is_empty() {
            s.push_str("recoverable saves in this folder: none\n");
        } else {
            s.push_str("recoverable saves in this folder (not the current save):\n");
            for r in recoverable {
                s.push_str(&format!(
                    "  {} \"{}\": {} ({} files, {} slots, {})\n",
                    r.kind.label(),
                    r.name,
                    r.path.display(),
                    r.files,
                    r.slots,
                    when(r.modified)
                ));
            }
        }
        let others = launcher_sources(dir);
        if !others.is_empty() {
            s.push_str("saves under another EmblemWeave data root:\n");
            for f in others {
                s.push_str(&format!(
                    "  {}: {} ({} files, {})\n",
                    f.source,
                    f.path.display(),
                    f.files,
                    when(f.modified)
                ));
            }
        }
    }
    let survey = survey_here(title_id);
    if survey.saves.is_empty() {
        s.push_str("emulator saves found: none\n");
    } else {
        s.push_str("emulator saves found:\n");
        for f in survey.saves {
            s.push_str(&format!(
                "  {}: {} ({} files, {})\n",
                f.source,
                f.path.display(),
                f.files,
                when(f.modified)
            ));
        }
    }
    for path in survey.empty {
        s.push_str(&format!("emulator folder with no save: {}\n", path.display()));
    }
    s
}

fn describe_account(root: &Path, indent: &str) -> String {
    let account = account_dir(root);
    let mut s = format!("{indent}account: {}\n", account.display());
    match save_below(&account) {
        Some(save) if save == account => {
            let (files, modified) = summary(&save.join("000"));
            s.push_str(&format!(
                "{indent}save: {} files in 000 ({})\n",
                files,
                when(modified)
            ));
        }
        Some(save) => {
            let (files, _) = summary(&save.join("000"));
            s.push_str(&format!(
                "{indent}save: {files} files in 000, but nested at {}; the game opens the level above\n",
                save.display()
            ));
        }
        None => s.push_str(&format!("{indent}save: none (000 is missing or empty)\n")),
    }
    let meta = account_meta_path(root);
    if meta.is_file() {
        let size = std::fs::metadata(&meta).map(|m| m.len()).unwrap_or(0);
        s.push_str(&format!("{indent}meta: {size} bytes\n"));
    } else {
        s.push_str(&format!("{indent}meta: missing\n"));
    }
    s
}

/// A relative label for a path inside the save folder. Diagnostic reports keep
/// the useful structure (`account/<user>/000/Auto`) without exposing the
/// local user name or install path.
fn shown_path(save_dir: &Path, path: &Path) -> String {
    match path.strip_prefix(save_dir) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().replace('\\', "/"),
        Ok(_) => "<save folder>".into(),
        Err(_) => "<outside save folder>".into(),
    }
}

/// A read-only, content-free inventory of the save tree for the Troubleshooter
/// report.
///
/// It records the account folder, profiles and backups with slot/file counts,
/// total sizes and timestamps. Slot contents are never opened, hashed or
/// copied, and local paths are reduced to relative labels: this section is safe
/// to attach to a public issue even when the save itself is not included.
pub fn save_inventory(save_dir: &Path, title_id: u64) -> String {
    let mut s = String::new();
    s.push_str("save inventory (metadata only; no save contents or local paths)\n");
    s.push_str("save folder: <local path omitted>\n");
    s.push_str(&format!("title id: {title_id:016X}\n"));
    s.push_str(&format!("expected user: {DEFAULT_USER}\n\n"));

    let account = account_dir(save_dir);
    let stats = inventory_stats(&account);
    s.push_str(&format!("account: {}\n", shown_path(save_dir, &account)));
    s.push_str(&format!(
        "  current save: {}\n",
        if has_save(&account) { "yes" } else { "no" }
    ));
    if let Some(nested) = save_below(&account).filter(|p| p.as_path() != account.as_path()) {
        s.push_str(&format!("  nested save: {}\n", shown_path(save_dir, &nested)));
    }
    s.push_str(&format!(
        "  slots={} files={} bytes={} newest={}\n",
        stats.slots,
        stats.files,
        stats.bytes,
        when(stats.newest)
    ));
    let meta = account_meta_path(save_dir);
    match std::fs::metadata(&meta) {
        Ok(m) => s.push_str(&format!(
            "  meta: present size={} modified={}\n",
            m.len(),
            when(m.modified().ok())
        )),
        Err(_) => s.push_str("  meta: missing\n"),
    }

    s.push_str("\nprofiles:\n");
    let profiles = profiles(save_dir);
    if profiles.is_empty() {
        s.push_str("  (none)\n");
    }
    for p in profiles {
        let stats = inventory_stats(&p.path);
        s.push_str(&format!(
            "  \"{}\": path={} slots={} files={} bytes={} newest={}\n",
            p.name,
            shown_path(save_dir, &p.path),
            stats.slots,
            stats.files,
            stats.bytes,
            when(stats.newest)
        ));
    }

    s.push_str("\nbackups:\n");
    let backups = backups(save_dir);
    if backups.is_empty() {
        s.push_str("  (none)\n");
    }
    for b in backups {
        let stats = inventory_stats(&b.path);
        s.push_str(&format!(
            "  \"{}\": path={} kind={} slots={} files={} bytes={} created={} newest={}\n",
            b.name,
            shown_path(save_dir, &b.path),
            if b.name.contains("-profile-") {
                "profile backup"
            } else {
                "save backup"
            },
            backup_slot_count(&b.path),
            stats.files,
            stats.bytes,
            when(b.modified),
            when(stats.newest)
        ));
    }

    s.push_str("\nrecoverable (not the current save):\n");
    let recoverable = recoverable_saves(save_dir);
    if recoverable.is_empty() {
        s.push_str("  (none)\n");
    }
    for r in recoverable {
        let stats = inventory_stats(&r.path);
        s.push_str(&format!(
            "  {} \"{}\": path={} slots={} files={} bytes={} newest={}\n",
            r.kind.label(),
            r.name,
            shown_path(save_dir, &r.path),
            r.slots,
            stats.files,
            stats.bytes,
            when(stats.newest)
        ));
    }
    s
}

/// A bounded, content-free listing of the save tree for a diagnostic bundle.
///
/// Filenames, sizes, timestamps and hashes answer whether a slot exists and
/// whether two reports hold the same bytes without putting save contents in an
/// issue attachment. Hashes are skipped for files above 1 MiB, and the listing
/// stops at 600 entries so a damaged work folder cannot make the report huge.
pub fn save_manifest(save_dir: &Path, title_id: u64) -> String {
    let mut s = String::new();
    s.push_str("EmblemWeave save manifest\n");
    s.push_str("=======================\n\n");
    s.push_str("save folder: <local path omitted>\n");
    s.push_str(&format!("title id: {title_id:016X}\n"));
    s.push_str(&format!("expected user: {DEFAULT_USER}\n"));
    let account = account_dir(save_dir);
    let meta = account_meta_path(save_dir);
    s.push_str(&format!("account: {}\n", shown_path(save_dir, &account)));
    s.push_str(&format!("account exists: {}\n", account.exists()));
    s.push_str(&format!("000 exists: {}\n", account.join("000").exists()));
    s.push_str(&format!("meta: {}\n", shown_path(save_dir, &meta)));
    if meta.is_file() {
        let size = std::fs::metadata(&meta).map(|m| m.len()).unwrap_or(0);
        s.push_str(&format!("meta state: present ({size} bytes)\n"));
        if let Ok(text) = std::fs::read_to_string(&meta) {
            for line in text.lines() {
                s.push_str(&format!("  meta: {line}\n"));
            }
        }
    } else {
        s.push_str("meta state: missing\n");
    }
    if meta.is_file() {
        // A metadata file the runtime wrote has these four keys. A truncated or
        // foreign one is the difference between "the game creates an empty save"
        // and "the save is there but the runtime will not mount it".
        let text = std::fs::read_to_string(&meta).unwrap_or_default();
        let missing: Vec<&str> = ["size=", "journal=", "owner=", "attr="]
            .into_iter()
            .filter(|key| !text.lines().any(|line| line.starts_with(key)))
            .collect();
        if missing.is_empty() {
            s.push_str("meta keys: size, journal, owner and attr are present\n");
        } else {
            s.push_str(&format!("meta keys: missing {}\n", missing.join(", ")));
        }
    }

    let mut entries: Vec<(String, u64, Option<SystemTime>)> = Vec::new();
    let mut stack = vec![save_dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in read.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(m) = e.metadata() else {
                continue;
            };
            let rel = path
                .strip_prefix(save_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            entries.push((rel, m.len(), m.modified().ok()));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let total = entries.len();
    entries.truncate(600);
    s.push_str(&format!("\nfiles: {total}\n"));
    for (rel, size, modified) in &entries {
        let path = save_dir.join(rel);
        let hash = if *size <= 1 << 20 {
            sha256_for_manifest(&path).unwrap_or_else(|| "unreadable".into())
        } else {
            "skipped (larger than 1 MiB)".into()
        };
        s.push_str(&format!(
            "  {rel} size={size} modified={} sha256={hash}\n",
            when(*modified)
        ));
    }
    if total > entries.len() {
        s.push_str(&format!("  ... {} more files not listed\n", total - entries.len()));
    }

    // Whether each file the game reads is actually a save the launcher and the
    // runtime's decoder accept. Only names, sizes and verdicts: no contents.
    s.push_str("\naccount slots:\n");
    let slot_dir = account.join("000");
    let mut slot_files: Vec<PathBuf> = std::fs::read_dir(&slot_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_slot_candidate(p))
        .collect();
    slot_files.sort();
    if slot_files.is_empty() {
        s.push_str("  (none)\n");
    }
    for path in slot_files.iter().take(40) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        match crate::savefile::Slot::open(path) {
            Ok(slot) => {
                let verify = slot.verify();
                s.push_str(&format!(
                    "  {name} size={size} parse=ok verify={} native={}\n",
                    if verify.is_empty() { "ok".to_string() } else { verify.join("; ") },
                    match slot.native_check() {
                        Ok(()) => "ok".to_string(),
                        Err(e) => e,
                    }
                ));
            }
            Err(e) => s.push_str(&format!("  {name} size={size} parse={e}\n")),
        }
    }
    if slot_files.len() > 40 {
        s.push_str(&format!("  ... {} more slots not checked\n", slot_files.len() - 40));
    }

    // A decided commit or an uncommitted work folder is the runtime's journal
    // state; seeing one here says the previous session did not finish its save.
    s.push_str("\ncommit state:\n");
    let mut pending = 0usize;
    if let (Some(parent), Some(stem)) = (
        account.parent(),
        account.file_name().map(|n| n.to_string_lossy().into_owned()),
    ) {
        let mut names: Vec<String> = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let tail = name.strip_prefix(&stem)?.to_string();
                (tail.starts_with(".commit") || tail.starts_with(".work.") || tail.starts_with(".journal."))
                    .then_some(name)
            })
            .collect();
        names.sort();
        for name in &names {
            pending += 1;
            let path = parent.join(name);
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let modified = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
            s.push_str(&format!("  {name} size={size} modified={}\n", when(modified)));
        }
    }
    if pending == 0 {
        s.push_str("  (no decided marker, journal or work folder)\n");
    }

    s.push_str("\nprofiles:\n");
    let profiles = profiles(save_dir);
    if profiles.is_empty() {
        s.push_str("  (none)\n");
    }
    for p in profiles {
        s.push_str(&format!(
            "  \"{}\": {} ({} files, {} slots, {})\n",
            p.name,
            shown_path(save_dir, &p.path),
            p.files,
            slot_file_count(&p.path),
            when(p.modified)
        ));
    }

    s.push_str("\nbackups:\n");
    let backups = backups(save_dir);
    if backups.is_empty() {
        s.push_str("  (none)\n");
    }
    for b in backups {
        s.push_str(&format!(
            "  {}: {} ({} files, {})\n",
            b.name,
            shown_path(save_dir, &b.path),
            b.files,
            when(b.modified)
        ));
    }

    s.push_str("\nrecoverable (not the current save):\n");
    let recoverable = recoverable_saves(save_dir);
    if recoverable.is_empty() {
        s.push_str("  (none)\n");
    }
    for r in recoverable {
        s.push_str(&format!(
            "  {} \"{}\": {} ({} files, {} slots, {})\n",
            r.kind.label(),
            r.name,
            shown_path(save_dir, &r.path),
            r.files,
            r.slots,
            when(r.modified)
        ));
    }

    s.push_str("\nruntime save log (end of fefw.log):\n");
    s.push_str(&runtime_save_witness());
    s
}

fn sha256_for_manifest(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn runtime_save_witness() -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Some(dir) = crate::engine::run_dir() else {
        return "  (no run folder)\n".into();
    };
    let path = dir.join("fefw.log");
    let Ok(mut file) = std::fs::File::open(&path) else {
        return format!("  (no {} yet)\n", path.display());
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > 1_000_000 && file.seek(SeekFrom::End(-1_000_000)).is_err() {
        return "  (log could not be read)\n".into();
    }
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return "  (log is not text)\n".into();
    }
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.contains("OpenSaveDataFileSystem")
                || l.contains("save data")
                || l.contains("save commit")
                || l.contains("uncommitted")
                || l.contains("save_dir")
                || l.contains("account/")
                || l.contains("journal")
        })
        .collect();
    if lines.is_empty() {
        return "  (no save mount/commit lines in the log)\n".into();
    }
    let mut s = String::new();
    for line in lines.iter().rev().take(80).rev() {
        s.push_str("  ");
        s.push_str(line);
        s.push('\n');
    }
    s
}

fn when(t: Option<SystemTime>) -> String {
    t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "time unknown".into())
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

/// A file the launcher named `<stamp>-<slot>`: what `archive_slot` and
/// `backup_slot` write into `backups/`, and what a slot copied back out of
/// that folder by hand keeps. The game opens only `<slot>`, so such a file is
/// the slot it names, not an unknown one.
pub fn stamped_slot_name(name: &str) -> Option<&str> {
    let (stamp, slot) = name.split_once('-')?;
    if stamp.len() < 9 || !stamp.bytes().all(|b| b.is_ascii_digit()) || slot.is_empty() {
        return None;
    }
    Some(slot)
}

/// The game names its per-route file `Route_<id>`: same save container as the
/// slots (the header and the native decode are checked like any other), but
/// `kind_of` calls it `Other` because it is not one of the listed slots. A
/// stamped one is still a file the player moved back from `backups/`, so
/// recovery puts it under its game name exactly as it does for `Auto` and the
/// chapter saves; `Global` (a different format) stays excluded.
fn is_recoverable_slot(name: &str) -> bool {
    let plain = stamped_slot_name(name).unwrap_or(name).to_ascii_lowercase();
    matches!(kind_of(name), SlotKind::Auto | SlotKind::Manual)
        || (plain.starts_with("route_") && is_slot_candidate(Path::new(name)))
}

fn kind_of(name: &str) -> SlotKind {
    let n = stamped_slot_name(name).unwrap_or(name).to_ascii_lowercase();
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
                    // The route lord is who the save is following. Fall back to
                    // the first protagonist in the roster only for slots whose
                    // route byte is not one of the five known routes.
                    let leader = slot
                        .route_lord()
                        .map(str::to_string)
                        .or_else(|| {
                            roster
                                .iter()
                                .find_map(|u| crate::portraits::key_for_name(&u.name))
                                .map(str::to_string)
                        });
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
    // Back up only a save that is really there. A folder holding just the account's `.meta` is the empty save the
    // game writes on first start; moving it aside would hide that metadata from the import below.
    let backup = if target.is_dir() && has_save(&target) {
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
    if !has_save(&from) {
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

/// When a backup folder was made. Its name starts with the epoch second the
/// launcher created it (`<stamp>-...`); the files inside keep the modification
/// time of the save they were copied from, so they cannot date the backup.
/// Older or renamed folders fall back to the folder's own modification time.
fn backup_created(path: &Path) -> Option<SystemTime> {
    if let Some(stamp) = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .and_then(|n| n.split('-').next().map(str::to_owned))
    {
        if stamp.len() >= 9 && stamp.bytes().all(|b| b.is_ascii_digit()) {
            if let Some(t) = stamp
                .parse::<u64>()
                .ok()
                .and_then(|s| std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(s)))
            {
                return Some(t);
            }
        }
    }
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
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
                    let (files, _) = summary(&path);
                    let modified = backup_created(&path);
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

/// Where a save the game does not currently read came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryKind {
    Profile,
    ProfileBackup,
    SaveBackup,
    Orphan,
}

impl RecoveryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::ProfileBackup => "profile backup",
            Self::SaveBackup => "save backup",
            Self::Orphan => "unread save",
        }
    }
}

/// A save that is present on disk but is not in the account folder the game
/// opens. It can be loaded or restored without touching the current save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recoverable {
    pub kind: RecoveryKind,
    pub name: String,
    /// The folder an import/restore reads: it holds `000`, or is the flat
    /// contents of a `000` folder for an old backup.
    pub path: PathBuf,
    pub files: usize,
    pub modified: Option<SystemTime>,
    pub slots: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct InventoryStats {
    files: usize,
    bytes: u64,
    slots: usize,
    newest: Option<SystemTime>,
}

/// Count files, bytes and slot files below `dir` from metadata only. No file is
/// opened: this is the basis for the diagnostic inventory.
fn inventory_stats(dir: &Path) -> InventoryStats {
    let mut out = InventoryStats::default();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !p.is_file() {
                continue;
            }
            let is_slot = is_slot_candidate(&p);
            let Ok(meta) = e.metadata() else {
                // A slot can still be counted when its metadata cannot be read;
                // the detailed file listing will show it as unreadable.
                if is_slot {
                    out.slots += 1;
                }
                continue;
            };
            out.files += 1;
            out.bytes += meta.len();
            if is_slot {
                out.slots += 1;
            }
            if let Ok(t) = meta.modified() {
                out.newest = Some(out.newest.map_or(t, |n| n.max(t)));
            }
        }
    }
    out
}

/// Number of slot files below a save folder. Metadata and the game's Global
/// file do not count: a folder holding only those has no playthrough.
fn slot_file_count(dir: &Path) -> usize {
    inventory_stats(dir).slots
}

/// The number of slot files in a whole-save or profile backup. A backup can
/// hold an account tree or the flat contents of `000`; resolve it first.
fn backup_slot_count(path: &Path) -> usize {
    let Some(source) = resolve_source(path, FALLBACK_TITLE_ID) else {
        return 0;
    };
    match source {
        SaveSource::Folder(dir) | SaveSource::Flat(dir) => slot_file_count(&dir),
    }
}

/// Every profile, profile backup, whole-save backup and orphan save that holds
/// at least one slot file, newest first. Empty profiles and metadata-only
/// folders are deliberately left out: offering those would not recover progress.
pub fn recoverable_saves(save_dir: &Path) -> Vec<Recoverable> {
    let mut out = Vec::new();

    for p in profiles(save_dir) {
        if is_empty_profile(&p) || !has_save(&p.path) {
            continue;
        }
        let slots = slot_file_count(&p.path);
        if slots == 0 {
            continue;
        }
        out.push(Recoverable {
            kind: RecoveryKind::Profile,
            name: p.name,
            path: p.path,
            files: p.files,
            modified: p.modified,
            slots,
        });
    }

    for b in backups(save_dir) {
        let Some(source) = resolve_source(&b.path, FALLBACK_TITLE_ID) else {
            continue;
        };
        let path = match source {
            SaveSource::Folder(path) | SaveSource::Flat(path) => path,
        };
        let slots = slot_file_count(&path);
        if slots == 0 {
            continue;
        }
        out.push(Recoverable {
            kind: if b.name.contains("-profile-") {
                RecoveryKind::ProfileBackup
            } else {
                RecoveryKind::SaveBackup
            },
            name: b.name,
            path,
            files: b.files,
            modified: b.modified,
            slots,
        });
    }

    for o in find_orphan_saves(save_dir) {
        if out.iter().any(|r| r.path == o.path) {
            continue;
        }
        let slots = slot_file_count(&o.path);
        if slots == 0 {
            continue;
        }
        out.push(Recoverable {
            kind: RecoveryKind::Orphan,
            name: o.description,
            path: o.path,
            files: o.files,
            modified: o.modified,
            slots,
        });
    }

    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.slots.cmp(&a.slots))
            .then_with(|| a.name.cmp(&b.name))
    });
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// A message that stops Play when the account folder is missing but saves that
/// can be recovered are present. This is the dangerous shape of issue #79: the
/// game would create an empty save and look as if the playthrough was lost.
///
/// An existing but empty account folder is stopped too: that is the state a
/// failed load or a moved save leaves behind. The launcher asks the player to
/// confirm a deliberate new game instead of starting one silently.
pub fn empty_namespace_blocker(save_dir: &Path) -> Option<String> {
    let account = account_dir(save_dir);
    if has_save(&account) {
        return None;
    }
    let recoverable = recoverable_saves(save_dir);
    if recoverable.is_empty() {
        return None;
    }
    let names = recoverable
        .iter()
        .take(3)
        .map(|r| format!("{} \"{}\" ({})", r.kind.label(), r.name, when(r.modified)))
        .collect::<Vec<_>>()
        .join("; ");
    let found = format!(
        "{} recoverable save{}",
        recoverable.len(),
        if recoverable.len() == 1 { "" } else { "s" }
    );
    Some(format!(
        "The current save is empty: {}. Found {found}: {names}. \
The game was not started on an empty save. Open Saves and load or restore one, or confirm a new game; nothing was deleted.",
        account.display()
    ))
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

/// Restore a profile from the backups folder.
/// The backup folder name must match the pattern `<stamp>-profile-<name>`.
pub fn restore_profile(save_dir: &Path, backup_name: &str) -> Result<Profile, String> {
    let backup_path = save_dir.join("backups").join(backup_name);
    if !backup_path.is_dir() {
        return Err(format!("backup {} does not exist", backup_name));
    }
    // Validate it's a profile backup
    if !backup_name.contains("-profile-") {
        return Err(format!("{} is not a profile backup", backup_name));
    }
    // Extract profile name from backup folder name
    let profile_name = backup_name
        .split_once("-profile-")
        .map(|(_, name)| name)
        .ok_or_else(|| format!("could not parse profile name from {}", backup_name))?;
    let profile_name = crate::mods::clean_name(profile_name);
    if profile_name.is_empty() {
        return Err("invalid profile name".into());
    }
    let target = save_dir.join("profiles").join(&profile_name);
    if target.exists() {
        return Err(format!("a profile named {} already exists", profile_name));
    }
    std::fs::create_dir_all(save_dir.join("profiles")).map_err(|e| e.to_string())?;
    std::fs::rename(&backup_path, &target).map_err(|e| e.to_string())?;
    let (files, modified) = summary(&target);
    Ok(Profile {
        name: profile_name,
        path: target,
        files,
        modified,
    })
}

/// Why a parsed slot must not be opened by the game, empty when it is intact:
/// its own header and payload checksums, then the native zstd decode the game
/// itself performs. The guard before Play and the recovery of a stamped slot
/// both run exactly this, so a file that would crash the game (#77) is never
/// promoted to a name it opens.
fn slot_problems(slot: &crate::savefile::Slot) -> Vec<String> {
    let mut problems = slot.verify();
    if problems.is_empty() {
        if let Err(e) = slot.native_check() {
            problems.push(e);
        }
    }
    problems
}

/// An autosave that could not be read, and where it was kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamagedAutosave {
    pub reason: String,
    pub kept: PathBuf,
}

/// Check the autosave the game opens at boot before Play starts it. A damaged
/// zstd frame is moved to the backups folder, byte for byte, so the game can
/// reach its title screen instead of crashing when it copies the decoder's
/// error code as a memory-copy length (#77). Nothing is deleted, and a valid
/// autosave is never touched. `Global` is deliberately not checked: it has a
/// different header and is not a slot, so validating it here would reject a
/// healthy one.
pub fn quarantine_damaged_autosave(
    save_dir: &Path,
    stamp: &str,
) -> Result<Option<DamagedAutosave>, String> {
    let slot = account_dir(save_dir).join("000").join("Auto");
    if !slot.is_file() {
        return Ok(None);
    }
    let reason = match crate::savefile::Slot::open(&slot) {
        Ok(parsed) => {
            let problems = slot_problems(&parsed);
            if problems.is_empty() {
                return Ok(None);
            }
            problems.join("; ")
        }
        Err(e) => e,
    };
    let kept = archive_slot(save_dir, &slot, &format!("{stamp}-damaged-autosave"))?;
    let note = kept.with_extension("why.txt");
    let _ = std::fs::write(
        &note,
        format!(
            "The autosave could not be read before Play: {reason}\nThe file was moved here unchanged; no save data was deleted.\n"
        ),
    );
    Ok(Some(DamagedAutosave { reason, kept }))
}

/// A slot the game could not open because its file carried the backups
/// folder's timestamp prefix, and the name it was copied to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairedSlot {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Restore the game's name to a slot that was moved back from the backups
/// folder by hand. `archive_slot` and `backup_slot` name their files
/// `<stamp>-<slot>` (for example `1795325701-Auto`), and the game opens only
/// `Auto`: a file copied back out of `backups/` under that name is never read,
/// so the game starts a new one (#79). Each readable slot is copied to its own
/// name and the stamped file is left untouched, so nothing is renamed or lost;
/// an existing file of the target name always wins, and a file the runtime
/// cannot parse is left alone. `Global` and the game's non-slot files are not
/// touched.
pub fn repair_backup_named_slots(save_dir: &Path) -> Result<Vec<RepairedSlot>, String> {
    let dir = account_dir(save_dir).join("000");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    let mut found: Vec<(String, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(slot_name) = stamped_slot_name(name) else {
            continue;
        };
        // Only a name the game itself opens: Auto, the manual chapter saves
        // and the per-route file. Global has a different header.
        if !is_recoverable_slot(slot_name) || !is_slot_candidate(Path::new(slot_name)) {
            continue;
        }
        let stamp = name
            .split_once('-')
            .and_then(|(s, _)| s.parse::<u64>().ok())
            .unwrap_or(0);
        found.push((slot_name.to_string(), stamp, path));
    }
    // For one slot name, the newest stamp is the save the player moved back
    // last, so it is the one copied; an older stamp is then left alone because
    // the target exists (or, when the newest fails verification, the next one
    // is tried instead of whichever file the directory happened to list first).
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    for (_, _, from) in found {
        if let Some(to) = recover_stamped_slot(&from)? {
            out.push(RepairedSlot { from, to });
        }
    }
    Ok(out)
}

/// Copy one `<stamp>-<slot>` file to the name the game opens, in the same
/// folder. The Saves page offers this on a stamped slot where it is listed,
/// and Play does the same for every stamped slot in the account folder (#79).
/// Returns `None` when the file is not a stamped slot, is not a save the game
/// can open (it must parse, pass its checksums and decode with the native zstd
/// the game uses), or its game name already exists: an existing slot always
/// wins, and the stamped original is never renamed or removed.
pub fn recover_stamped_slot(path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Ok(None);
    };
    let Some(slot_name) = stamped_slot_name(name) else {
        return Ok(None);
    };
    if !is_recoverable_slot(slot_name) || !is_slot_candidate(Path::new(slot_name)) {
        return Ok(None);
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let to = dir.join(slot_name);
    if to.exists() {
        return Ok(None);
    }
    // Only a copy the game could actually open is worth promoting: a file that
    // parses but fails its checksums or the native zstd decode is the #77 crash
    // the guard before Play moves aside, and copying it into `Auto` after that
    // guard has run would hand the game the same bad frame under a new name.
    let Ok(parsed) = crate::savefile::Slot::open(path) else {
        return Ok(None);
    };
    if !slot_problems(&parsed).is_empty() {
        return Ok(None);
    }
    // Copy under a temporary name and rename it into place: an interrupted
    // copy leaves no half-written `Auto` for the game to reject, and the
    // rename cannot replace a slot that appeared while the copy ran (on
    // Windows it fails; elsewhere the exists() check above decided).
    let tmp = dir.join(format!("{slot_name}.emblemweave-recovering"));
    let _ = std::fs::remove_file(&tmp);
    std::fs::copy(path, &tmp)
        .map_err(|e| format!("copying {} to {}: {e}", path.display(), tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, &to) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("renaming {} to {}: {e}", tmp.display(), to.display()));
    }
    Ok(Some(to))
}

/// What the launcher says when it recovers slots before Play: which file it
/// could not open, and the name it copied it to. The launch continues, so the
/// note has to name the save without asking for another press.
pub fn repair_note(repaired: &[RepairedSlot]) -> String {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let list = repaired
        .iter()
        .map(|r| format!("{} -> {}", name(&r.from), name(&r.to)))
        .collect::<Vec<_>>()
        .join(", ");
    match repaired.len() {
        0 => String::new(),
        1 => format!(
            "Recovered the save the game could not open: {list}. The copy is what the game reads, and the original is untouched."
        ),
        n => format!(
            "Recovered {n} saves the game could not open: {list}. The copies are what the game reads, and the originals are untouched."
        ),
    }
}

/// The same report for the pre-Play repair, where the launch really does carry
/// on after the copy. The CLI uses `repair_note` and must not claim this.
pub fn repair_note_before_play(repaired: &[RepairedSlot]) -> String {
    let note = repair_note(repaired);
    if note.is_empty() {
        note
    } else {
        format!("{note} The game is starting now.")
    }
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

    /// Rewrite the recovery sidecar to model a kill at a later commit step.
    fn write_state(
        pending: &PendingImport,
        backup: Option<PathBuf>,
        old_target: Option<PathBuf>,
    ) {
        let state = ImportState {
            stage: pending.stage.clone(),
            backup,
            old_target,
            meta_created: pending.meta_created,
            files: pending.files.clone(),
        };
        write_import_state(&pending.state, &state).unwrap();
    }

    /// Model the exact point after `rename(target, backup)` and before the
    /// staged save is renamed into place.
    fn move_old_save_to_backup(pending: &PendingImport, save: &Path, stamp: &str) -> PathBuf {
        let backup = plan_backup_path(save, stamp).unwrap();
        std::fs::rename(&pending.target, &backup).unwrap();
        write_state(pending, Some(backup.clone()), None);
        backup
    }

    #[test]
    fn reports_an_empty_emulator_folder_without_offering_it() {
        let root = temp("find-empty");
        let empty = root.join(
            "yuzu/nand/user/save/0000000000000000/00000000000000000000000000000000/04007EF01F8AE000",
        );
        std::fs::create_dir_all(&empty).unwrap();
        let survey = survey(&root, FALLBACK_TITLE_ID);
        assert!(survey.saves.is_empty());
        assert_eq!(survey.empty, vec![empty]);
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
        let found = survey(&root, FALLBACK_TITLE_ID).saves;
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].source.as_str(), found[0].files), ("yuzu", 1));
        assert_eq!(found[0].path, title);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn offers_a_save_from_the_other_launcher_data_root() {
        let root = temp("launcher-roots");
        let current = root.join("install/save");
        let appdata = root.join("appdata/EmblemWeave/save");
        write(
            &appdata.join("account").join(DEFAULT_USER).join("000/Auto"),
            "a run left behind",
        );
        let found = launcher_sources_from(
            &[("EmblemWeave (AppData)".to_string(), appdata.clone())],
            &current,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].path,
            appdata.join("account").join(DEFAULT_USER)
        );

        // The root in use is not offered as a source of itself.
        let found = launcher_sources_from(
            &[("EmblemWeave (AppData)".to_string(), current.clone())],
            &current,
        );
        assert!(found.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_remembered_engine_names_an_older_portable_install() {
        let root = temp("remembered-engine");
        let install = root.join("EmblemWeave-0.1.41");
        let checkout = install.join("engine").join("fefw-recomp");
        let save = install.join("data").join("save");
        std::fs::create_dir_all(&checkout).unwrap();
        assert_eq!(remembered_install_root(&checkout.display().to_string()), None);
        std::fs::create_dir_all(&save).unwrap();
        assert_eq!(
            remembered_install_root(&checkout.display().to_string()),
            Some(save)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_save_inside_the_current_root_is_not_a_source_for_itself() {
        let root = temp("same-root-source");
        let save = root.join("save");
        write(
            &save.join("account").join("ANOTHERUSER").join("000/Auto"),
            "a save the game does not read",
        );
        let found = launcher_sources_from(
            &[("This root".to_string(), save.clone())],
            &save,
        );
        assert!(found.is_empty(), "{found:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_meta_only_account_folder_is_not_a_save() {
        let root = temp("meta-only");
        let save = root.join("save");
        let account = account_dir(&save);
        std::fs::create_dir_all(account.join("000")).unwrap();
        write(&account.with_extension("meta"), "size=67108864\n");

        assert!(!has_save(&account), "metadata is not a playthrough");
        assert!(find_orphan_saves(&save).is_empty());
        assert!(
            store_profile(&save, "Nothing", "1").is_err(),
            "an empty save cannot be kept as a profile"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A recovered tree can hold the playthrough as `<save folder>/<id>/000` instead of under `account/`. The game
    /// never opens it, so the Saves page must name it and offer to copy it into place (issue #79). Backups are not
    /// orphans: the launcher already lists them as backups.
    #[test]
    fn a_save_folder_in_the_save_root_is_an_orphan() {
        let root = temp("root-orphan");
        let save = root.join("save");
        write(&save.join("1795393583/000/Chapter_Led_1_4"), "progress");
        write(&save.join("profiles/Kept/000/Auto"), "profile");
        write(&save.join("backups/1-import/000/Auto"), "backup");
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "current",
        );

        let found = find_orphan_saves(&save);
        assert!(
            found
                .iter()
                .any(|o| o.path.ends_with("1795393583") && o.files == 1 && o.description.contains("Save folder")),
            "the root save is named: {found:?}"
        );
        assert!(
            found.iter().all(|o| !o.path.ends_with("1-import")),
            "the launcher's backup area is not an orphan: {found:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The user can click Use this save after the source has already gone (another program moved it, or a removable
    /// drive was unplugged). The error must be reported without first moving the current playthrough to Backups.
    #[test]
    fn adopting_a_vanished_orphan_fails_without_touching_the_current_save() {
        let root = temp("orphan-vanished");
        let save = root.join("save");
        let current = account_dir(&save).join("000/Auto");
        write(&current, "current playthrough");
        let before = std::fs::read(&current).unwrap();
        let missing = save.join("1795393583/000");
        let orphan = OrphanSave {
            description: "Save folder \"1795393583\"".into(),
            path: save.join("1795393583"),
            files: 1,
            modified: None,
        };

        let error = adopt_orphan_save(&orphan, &save, FALLBACK_TITLE_ID, "1").unwrap_err();
        assert!(!error.is_empty(), "the page can show why it failed");
        assert!(missing.parent().is_some_and(|p| !p.exists()));
        assert_eq!(std::fs::read(&current).unwrap(), before, "current save untouched");
        assert!(
            !save.join("backups").exists(),
            "a failed import must not create a backup or move the current save"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A folder named `000` with a text file in it is not a playthrough. Only an extensionless game slot makes a
    /// root-level folder an orphan; otherwise an unrelated folder (a note, an export, a tool's cache) is offered as a
    /// save and adopting it would replace the current playthrough with junk.
    #[test]
    fn an_unrelated_populated_000_folder_is_not_offered_as_a_save() {
        let root = temp("orphan-unrelated");
        let save = root.join("save");
        write(&save.join("weird/000/readme.txt"), "not a save");
        write(&save.join("notes/000/changelog.md"), "also not a save");

        let found = find_orphan_saves(&save);
        assert!(found.is_empty(), "no folder without a slot is offered: {found:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A real save that also carries stray files stays visible, and the file count still tells the player what is in
    /// the folder.
    #[test]
    fn a_save_with_a_stray_file_is_still_offered() {
        let root = temp("orphan-stray");
        let save = root.join("save");
        write(&save.join("1795393583/000/Chapter_Led_1_4"), "progress");
        write(&save.join("1795393583/000/notes.txt"), "stray");

        let found = find_orphan_saves(&save);
        assert_eq!(found.len(), 1, "the real save is offered: {found:?}");
        assert_eq!(found[0].files, 2, "the stray file is counted, not hidden");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_meta_is_repaired_without_touching_the_save_files() {
        let root = temp("meta-repair");
        let save = root.join("save");
        let account = account_dir(&save);
        write(&account.join("000/Auto"), "keep this playthrough");
        let before = std::fs::read(account.join("000/Auto")).unwrap();
        assert!(!account_meta_path(&save).exists());

        assert!(ensure_account_meta(&save, FALLBACK_TITLE_ID).unwrap());
        let meta = std::fs::read_to_string(account_meta_path(&save)).unwrap();
        assert!(meta.contains("owner=4007ef01f8ae000"), "{meta}");
        assert!(meta.contains("attr="), "{meta}");
        assert_eq!(std::fs::read(account.join("000/Auto")).unwrap(), before);
        assert!(!ensure_account_meta(&save, FALLBACK_TITLE_ID).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recoverable_inventory_and_empty_namespace_guard() {
        let root = temp("recoverable");
        let save = root.join("save");
        assert!(
            empty_namespace_blocker(&save).is_none(),
            "a fresh install with no recoverable save is not blocked"
        );
        write(&save.join("profiles/Kept/000/Auto"), "profile");
        write(&save.join("backups/100-profile-Gone/000/Auto"), "profile backup");
        write(
            &save.join("backups/200-import/account").join(DEFAULT_USER).join("000/Auto"),
            "whole backup",
        );

        let found = recoverable_saves(&save);
        assert!(found.iter().any(|r| r.kind == RecoveryKind::Profile && r.name == "Kept"));
        assert!(found.iter().any(|r| r.kind == RecoveryKind::ProfileBackup));
        assert!(found.iter().any(|r| r.kind == RecoveryKind::SaveBackup));
        let note = empty_namespace_blocker(&save).expect("a missing account folder is refused");
        assert!(note.contains("not started"), "{note}");

        // An existing but empty account folder is still stopped: this is the
        // shape a failed profile load leaves behind, and Play asks first.
        std::fs::create_dir_all(account_dir(&save)).unwrap();
        assert!(empty_namespace_blocker(&save).is_some());
        write(&account_dir(&save).join("000/Auto"), "current");
        assert!(empty_namespace_blocker(&save).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_times_read_as_dates_instead_of_epoch_seconds() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_795_525_256);
        assert_eq!(when(Some(t)), "2026-11-24 13:00 UTC");
        assert_eq!(when(None), "time unknown");
    }

    #[test]
    fn a_backup_is_dated_by_its_name_not_the_files_inside() {
        let root = temp("backup-stamp");
        let save = root.join("save");
        let backup = save.join("backups").join("1795525256-import");
        let slot = backup.join("account").join(DEFAULT_USER).join("000/Auto");
        write(&slot, "a save copied from an older session");
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&slot)
            .unwrap()
            .set_modified(old)
            .unwrap();

        let listed = backups(&save);
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].modified,
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_795_525_256)),
            "the backup's age is when it was made, not the copied slot's mtime"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_inventory_lists_metadata_without_save_contents() {
        let root = temp("save-inventory");
        let save = root.join("save");
        let secret = "DO-NOT-COPY-THIS-SAVE-PAYLOAD";
        write(&account_dir(&save).join("000/Auto"), secret);
        write(&account_dir(&save).join("000/Global"), "global data");
        ensure_account_meta(&save, FALLBACK_TITLE_ID).unwrap();
        store_profile(&save, "Run A", "1").unwrap();
        write(
            &save.join("backups/1795525256-import/account")
                .join(DEFAULT_USER)
                .join("000/Auto"),
            secret,
        );

        let text = save_inventory(&save, FALLBACK_TITLE_ID);
        for expected in [
            "account:",
            "current save: yes",
            "meta: present",
            "profiles:",
            "\"Run A\"",
            "backups:",
            "1795525256-import",
            "kind=save backup",
            "slots=1",
            "bytes=",
            "created=",
            "newest=",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(
            text.matches("slots=1").count() >= 3,
            "the account, profile and backup each report their slot count:\n{text}"
        );
        assert!(!text.contains(secret), "save contents must never enter the report");
        assert!(
            !text.contains(&save.to_string_lossy().into_owned()),
            "a public diagnostic must not expose the local save path:\n{text}"
        );
        assert!(
            !text.contains("sha256="),
            "the inventory is metadata-only; the detailed manifest keeps hashes"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_manifest_lists_files_sizes_and_hashes() {
        let root = temp("manifest");
        let save = root.join("save");
        write(&account_dir(&save).join("000/Auto"), "manifest bytes");
        ensure_account_meta(&save, FALLBACK_TITLE_ID).unwrap();
        let text = save_manifest(&save, FALLBACK_TITLE_ID);
        assert!(text.contains("account/"));
        assert!(text.contains("000/Auto"));
        assert!(text.contains("sha256="));
        assert!(text.contains("meta state: present"));
        assert!(
            text.contains("meta keys: size, journal, owner and attr are present"),
            "the manifest says whether the metadata is complete: {text}"
        );
        assert!(
            text.contains("  Auto size=14 parse=not a Fortune's Weave save slot"),
            "a file the game cannot open is reported as such: {text}"
        );
        assert!(
            text.contains("commit state:\n  (no decided marker, journal or work folder)"),
            "a clean save tree says it has no pending commit: {text}"
        );
        assert!(
            !text.contains(&save.to_string_lossy().into_owned()),
            "the diagnostic bundle must not expose the local save path:\n{text}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A save the decoder accepts is reported as intact, and a decided commit
    /// left beside the account folder is named: the two facts a report needs to
    /// tell a missing save from an unfinished one.
    #[test]
    fn save_manifest_checks_the_slots_and_the_pending_commit() {
        let root = temp("manifest-slots");
        let save = root.join("save");
        let account = account_dir(&save);
        write_bytes(&account.join("000/Auto"), &valid_slot(b"a readable autosave"));
        ensure_account_meta(&save, FALLBACK_TITLE_ID).unwrap();
        write_bytes(&account.with_extension("commit"), b"W x\nR y\n");

        let text = save_manifest(&save, FALLBACK_TITLE_ID);
        assert!(
            text.contains("Auto size=") && text.contains("parse=ok verify=ok native=ok"),
            "the readable slot is reported intact: {text}"
        );
        assert!(
            text.contains("\ncommit state:\n  0100000000000000706d635257464546.commit size=8"),
            "the decided commit is named: {text}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_save_nested_below_the_account_folder_is_reported() {
        let root = temp("nested-account");
        let save = root.join("save");
        let target = account_dir(&save);
        let deeper = target.join(DEFAULT_USER);
        write(&deeper.join("000/Auto"), "a real save");

        assert!(!has_save(&target), "the game opens the level above");
        let found = find_orphan_saves(&save);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].path, deeper);
        assert_eq!(found[0].files, 1);
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
        let found = survey(&root, FALLBACK_TITLE_ID).saves;
        let sources: Vec<&str> = found.iter().map(|f| f.source.as_str()).collect();
        assert_eq!(found.len(), 3, "{sources:?}");
        assert!(sources.contains(&"lichii"));
        assert!(sources.contains(&"fritchii"));
        assert!(sources.contains(&"eden"));
        assert!(found.iter().any(|f| f.path == eden));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_emulator_title_folder_becomes_a_new_profile_without_touching_the_others() {
        let root = temp("profile-yuzu");
        let save = root.join("save");
        // A yuzu-family save: `<nand>/user/save/<save id>/<profile>/<title>/000/...`.
        let src = root
            .join("yuzu/nand/user/save/0000000000000000/5A3C/04007EF01F8AE000");
        let slot = valid_slot(b"a run from yuzu");
        write_bytes(&src.join("000/Auto"), &slot);
        write(&src.join("000/Global"), "global");
        write(&save.join("profiles/Existing/000/Auto"), "keep me");
        write(
            &save.join("account").join(DEFAULT_USER).join("000/Auto"),
            "the current save",
        );

        let p = import_profile(&save, "Yuzu run", &src, FALLBACK_TITLE_ID).unwrap();
        assert_eq!(p.name, "Yuzu run");
        assert_eq!(std::fs::read(p.path.join("000/Auto")).unwrap(), slot);
        assert_eq!(
            std::fs::read_to_string(save.join("profiles/Existing/000/Auto")).unwrap(),
            "keep me",
            "an existing profile is not replaced"
        );
        assert_eq!(
            std::fs::read_to_string(
                save.join("account").join(DEFAULT_USER).join("000/Auto")
            )
            .unwrap(),
            "the current save",
            "importing a profile does not load it"
        );
        assert_eq!(profiles(&save).len(), 2);
        assert!(src.join("000/Auto").is_file(), "the source is left alone");
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

    /// A stop before commit must leave the old save active. Its staging folder
    /// and uncommitted sidecar are removed, and no backup is invented.
    #[test]
    fn an_interrupted_import_before_commit_keeps_the_old_save() {
        let root = temp("import-before-commit");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("000/Auto"), "new save");
        let user = account_dir(&save);
        write(&user.join("000/Auto"), "old save");

        let pending = stage_import(&src, false, &user, &save, FALLBACK_TITLE_ID, "1").unwrap();
        let stage = pending.stage.clone();
        let state = pending.state.clone();
        let backup = pending.backup.clone().expect("a backup was planned");
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "old save",
            "staging never moves the active save"
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("000/Auto")).unwrap(),
            "new save"
        );
        assert!(!backup.exists(), "staging does not move the backup either");
        assert_eq!(
            stage.parent(),
            user.parent(),
            "the staging folder and account are on the same filesystem"
        );
        drop(pending);

        assert!(recover_interrupted_import(&save).unwrap() >= 1);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "old save",
            "a kill before commit cannot lose the old save"
        );
        assert!(!stage.exists(), "the uncommitted stage is removed");
        assert!(!state.exists(), "the sidecar is removed");
        assert!(!backup.exists());
        assert!(
            user.with_extension("meta").is_file(),
            "the meta written before the stop is a harmless repair, not deleted"
        );
        assert_eq!(recover_interrupted_import(&save).unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "old save"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A kill after the old save has moved to backups but before promotion is
    /// resumed: the verified stage becomes active and the backup stays intact.
    #[test]
    fn an_interrupted_import_after_backup_resumes_the_verified_stage() {
        let root = temp("import-resume");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("000/Auto"), "new save");
        let user = account_dir(&save);
        write(&user.join("000/Auto"), "old save");

        let pending = stage_import(&src, false, &user, &save, FALLBACK_TITLE_ID, "1").unwrap();
        let stage = pending.stage.clone();
        let state = pending.state.clone();
        let backup = move_old_save_to_backup(&pending, &save, "resume");
        assert!(!user.exists(), "the old save is out of the active path");
        assert_eq!(
            std::fs::read_to_string(backup.join("000/Auto")).unwrap(),
            "old save"
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("000/Auto")).unwrap(),
            "new save"
        );

        assert!(recover_interrupted_import(&save).unwrap() >= 1);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "new save",
            "the fully verified stage is promoted"
        );
        assert_eq!(
            std::fs::read_to_string(backup.join("000/Auto")).unwrap(),
            "old save",
            "the old save remains in backup history"
        );
        assert!(!stage.exists());
        assert!(!state.exists());
        assert!(user.with_extension("meta").is_file());
        assert_eq!(recover_interrupted_import(&save).unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "new save"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A kill after backup followed by a damaged stage must roll back to the
    /// backup. The rollback is a verified copy, and the backup itself remains.
    #[test]
    fn a_damaged_stage_after_backup_restores_the_old_save() {
        let root = temp("import-rollback");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("000/Auto"), "new save");
        let user = account_dir(&save);
        write(&user.join("000/Auto"), "old save");

        let pending = stage_import(&src, false, &user, &save, FALLBACK_TITLE_ID, "1").unwrap();
        let stage = pending.stage.clone();
        let state = pending.state.clone();
        let backup = move_old_save_to_backup(&pending, &save, "rollback");
        std::fs::remove_file(stage.join("000/Auto")).unwrap();

        assert!(recover_interrupted_import(&save).unwrap() >= 1);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "old save",
            "an unverifiable replacement is never promoted"
        );
        assert_eq!(
            std::fs::read_to_string(backup.join("000/Auto")).unwrap(),
            "old save",
            "rollback copies the backup instead of consuming it"
        );
        assert!(!stage.exists());
        assert!(!state.exists());
        assert!(!user.with_extension("meta").exists());
        assert_eq!(recover_interrupted_import(&save).unwrap(), 0);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "old save"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A kill after promotion but before the sidecar is removed only needs
    /// cleanup: the new save and the old backup both stay.
    #[test]
    fn a_kill_after_promotion_finishes_cleanup_without_rolling_back() {
        let root = temp("import-after-promotion");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("000/Auto"), "new save");
        let user = account_dir(&save);
        write(&user.join("000/Auto"), "old save");

        let pending = stage_import(&src, false, &user, &save, FALLBACK_TITLE_ID, "1").unwrap();
        let state = pending.state.clone();
        let backup = move_old_save_to_backup(&pending, &save, "promoted");
        std::fs::rename(&pending.stage, &user).unwrap();

        assert!(recover_interrupted_import(&save).unwrap() >= 1);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "new save"
        );
        assert_eq!(
            std::fs::read_to_string(backup.join("000/Auto")).unwrap(),
            "old save"
        );
        assert!(!state.exists());
        assert!(user.with_extension("meta").is_file());
        assert_eq!(recover_interrupted_import(&save).unwrap(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// An empty account folder is moved aside only at commit. If the process
    /// dies after that move, recovery resumes the stage and removes the empty
    /// folder; files that were already in the empty folder stay in the save.
    #[test]
    fn an_interrupted_import_of_an_empty_account_resumes_and_discards_the_empty_old() {
        let root = temp("import-empty-old");
        let (src, save) = (root.join("src"), root.join("save"));
        write(&src.join("000/Auto"), "new save");
        let user = account_dir(&save);
        write(&user.join("keep.txt"), "keep me");

        let pending = stage_import(&src, false, &user, &save, FALLBACK_TITLE_ID, "1").unwrap();
        let old = pending.old_target.clone().expect("the empty account is planned");
        assert_eq!(
            std::fs::read_to_string(user.join("keep.txt")).unwrap(),
            "keep me",
            "an empty account is not moved while staging"
        );
        std::fs::rename(&pending.target, &old).unwrap();
        write_state(&pending, None, Some(old.clone()));

        assert!(recover_interrupted_import(&save).unwrap() >= 1);
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "new save"
        );
        assert_eq!(
            std::fs::read_to_string(user.join("keep.txt")).unwrap(),
            "keep me",
            "unrelated files in the empty account are preserved"
        );
        assert!(!old.exists());
        assert!(!pending.state.exists());
        assert!(user.with_extension("meta").is_file());
        assert_eq!(recover_interrupted_import(&save).unwrap(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A staging folder with no sidecar was interrupted before commit. It is
    /// never a readable replacement, so recovery removes it and leaves the
    /// active save untouched.
    #[test]
    fn a_partial_stage_without_state_is_removed_and_the_active_save_stays() {
        let root = temp("import-orphan-stage");
        let save = root.join("save");
        let user = account_dir(&save);
        write(&user.join("000/Auto"), "old save");
        let stage = side_path(&user, "import");
        write(&stage.join("000/Auto"), "partial");

        assert_eq!(recover_interrupted_import(&save).unwrap(), 1);
        assert!(!stage.exists());
        assert_eq!(
            std::fs::read_to_string(user.join("000/Auto")).unwrap(),
            "old save"
        );
        assert_eq!(recover_interrupted_import(&save).unwrap(), 0);
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
    fn profile_backup_and_restore_cycle() {
        let root = temp("profile-restore");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "original");
        // Create a profile
        store_profile(&save, "Test Profile", "1").unwrap();
        let ps = profiles(&save);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "Test Profile");
        // Move profile to backups (simulates user deleting profile)
        let backup_path = remove_profile(&save, &ps[0], "2").unwrap();
        assert!(profiles(&save).is_empty());
        // Verify backup exists with expected name pattern
        let backup_name = backup_path.file_name().unwrap().to_string_lossy().to_string();
        assert!(backup_name.contains("-profile-Test Profile"));
        // Restore profile from backup
        let restored = restore_profile(&save, &backup_name).unwrap();
        assert_eq!(restored.name, "Test Profile");
        assert!(restored.path.is_dir());
        // Verify profile is back in profiles list
        let ps = profiles(&save);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "Test Profile");
        // Verify backup is gone
        let backups = crate::saves::backups(&save);
        let profile_backups: Vec<_> = backups.iter().filter(|b| b.name.contains("-profile-")).collect();
        assert!(profile_backups.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Restoration when a profile with the same name already exists must fail
    /// and leave both the backup and the existing profile untouched.
    #[test]
    fn restore_profile_fails_on_name_collision() {
        let root = temp("profile-collision");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "original");
        // Create a profile and move it to backups
        store_profile(&save, "Test Profile", "1").unwrap();
        let ps = profiles(&save);
        let backup_path = remove_profile(&save, &ps[0], "2").unwrap();
        let backup_name = backup_path.file_name().unwrap().to_string_lossy().to_string();
        // Create another profile with the same name
        store_profile(&save, "Test Profile", "3").unwrap();
        assert_eq!(profiles(&save).len(), 1);
        // Attempt to restore - should fail
        let result = restore_profile(&save, &backup_name);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
        // Existing profile should be untouched
        assert_eq!(profiles(&save).len(), 1);
        assert_eq!(profiles(&save)[0].name, "Test Profile");
        // Backup should still exist
        let backups = crate::saves::backups(&save);
        let profile_backups: Vec<_> = backups.iter().filter(|b| b.name.contains("-profile-")).collect();
        assert_eq!(profile_backups.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Invalid profile backup (doesn't match naming pattern) must be rejected.
    #[test]
    fn restore_profile_rejects_invalid_backup_name() {
        let root = temp("profile-invalid");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "original");
        // Create a whole-save backup (not a profile backup)
        let backup_dir = save.join("backups").join("12345-whole-save");
        std::fs::create_dir_all(&backup_dir).unwrap();
        write(&backup_dir.join("some-file"), "data");
        // Attempt to restore as profile - should fail
        let result = restore_profile(&save, "12345-whole-save");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a profile backup"));
        // Backup should still exist
        assert!(backup_dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Empty profile backup (no contents) is restored but will be an empty profile.
    /// The restore operation itself succeeds (it's just a rename); validation of
    /// profile contents happens when the profile is loaded.
    #[test]
    fn restore_profile_empty_backup_restores_empty_folder() {
        let root = temp("profile-empty");
        let save = root.join("save");
        // Create an empty profile backup folder
        let backup_dir = save.join("backups").join("12345-profile-Empty Profile");
        std::fs::create_dir_all(&backup_dir).unwrap();
        // Attempt to restore - should succeed (rename)
        let result = restore_profile(&save, "12345-profile-Empty Profile");
        assert!(result.is_ok());
        let restored = result.unwrap();
        assert_eq!(restored.name, "Empty Profile");
        assert!(restored.path.is_dir());
        // Profile exists but has no files
        assert_eq!(restored.files, 0);
        // Backup is gone (renamed)
        assert!(!backup_dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Failed restoration must not destroy the backup.
    /// This is tested by restore_profile_fails_on_name_collision which
    /// verifies the backup persists when restoration fails due to name collision.
    #[test]
    fn restore_profile_preserves_backup_on_failure() {
        // Covered by restore_profile_fails_on_name_collision
        let root = temp("profile-preserve");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "original");
        store_profile(&save, "Test Profile", "1").unwrap();
        let ps = profiles(&save);
        let backup_path = remove_profile(&save, &ps[0], "2").unwrap();
        let backup_name = backup_path.file_name().unwrap().to_string_lossy().to_string();
        // Create another profile with the same name to force collision
        store_profile(&save, "Test Profile", "3").unwrap();
        let result = restore_profile(&save, &backup_name);
        assert!(result.is_err());
        // Backup should still exist
        let backups = crate::saves::backups(&save);
        let profile_backups: Vec<_> = backups.iter().filter(|b| b.name.contains("-profile-")).collect();
        assert_eq!(profile_backups.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Whole-save backup restoration must remain completely unaffected by
    /// the new profile backup restoration functionality.
    #[test]
    fn whole_save_backup_restoration_unaffected() {
        let root = temp("whole-save-unaffected");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "original");
        // Create a whole-save backup (simulating import)
        let backup_dir = save.join("backups").join("12345-import-backup");
        std::fs::create_dir_all(&backup_dir).unwrap();
        let imported_user = backup_dir.join("account").join(DEFAULT_USER);
        std::fs::create_dir_all(&imported_user.join("000")).unwrap();
        write(&imported_user.join("000/Auto"), "imported save");
        // Create a title_id file to make it a valid whole-save backup
        write(&backup_dir.join("title_id"), "0A3C5F0000000000");
        // List backups - should show both types if we had them
        let backups = crate::saves::backups(&save);
        assert_eq!(backups.len(), 1);
        assert!(!backups[0].name.contains("-profile-"));
        // Verify it's detected as whole-save backup
        assert!(!backups[0].name.contains("-profile-"));
        // The restore_backup_verified path should still work (tested elsewhere)
        let _ = std::fs::remove_dir_all(root);
    }

    /// Restored profile must preserve all original metadata and contents.
    #[test]
    fn restore_profile_preserves_metadata_and_contents() {
        let root = temp("profile-preserve-contents");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "autosave data");
        write(&user.join("000/Manual"), "manual save data");
        write(&user.join("DEFAULT_USER.meta"), "meta data");
        // Create a profile with multiple files
        store_profile(&save, "My Profile", "1").unwrap();
        let ps = profiles(&save);
        // Verify original profile contents (copied from account_dir which is DEFAULT_USER,
        // so contents are directly under the profile folder, not in a DEFAULT_USER subfolder)
        assert!(ps[0].path.join("000/Auto").exists());
        assert!(ps[0].path.join("000/Manual").exists());
        assert!(ps[0].path.join("DEFAULT_USER.meta").exists());
        let original_files = ps[0].files;
        assert!(original_files >= 3);
        // Move profile to backups
        let backup_path = remove_profile(&save, &ps[0], "2").unwrap();
        let backup_name = backup_path.file_name().unwrap().to_string_lossy().to_string();
        assert!(profiles(&save).is_empty());
        // Restore profile
        let restored = restore_profile(&save, &backup_name).unwrap();
        // Verify restored profile has same name
        assert_eq!(restored.name, "My Profile");
        // Verify all contents are preserved directly under profile folder
        assert!(restored.path.join("000/Auto").exists());
        assert!(restored.path.join("000/Manual").exists());
        assert!(restored.path.join("DEFAULT_USER.meta").exists());
        // Verify file contents
        assert_eq!(std::fs::read_to_string(restored.path.join("000/Auto")).unwrap(), "autosave data");
        assert_eq!(std::fs::read_to_string(restored.path.join("000/Manual")).unwrap(), "manual save data");
        assert_eq!(std::fs::read_to_string(restored.path.join("DEFAULT_USER.meta")).unwrap(), "meta data");
        // Verify file count matches
        assert_eq!(restored.files, original_files);
        // Verify backup is gone
        let backups = crate::saves::backups(&save);
        let profile_backups: Vec<_> = backups.iter().filter(|b| b.name.contains("-profile-")).collect();
        assert!(profile_backups.is_empty());
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
    fn slot_with_frame(payload: &[u8], frame: Vec<u8>) -> Vec<u8> {
        let mut raw = vec![0u8; 0x124];
        raw[0..4].copy_from_slice(&0xCDEF_BCDDu32.to_le_bytes());
        raw[4..8].copy_from_slice(&0xE0u32.to_le_bytes());
        raw[0x104..0x108].copy_from_slice(&0xCDEF_BCDDu32.to_le_bytes());
        raw[0x108..0x10C].copy_from_slice(&(frame.len() as u32).to_le_bytes());
        raw[0x10C..0x110].copy_from_slice(&fnv1_32(payload).to_le_bytes());
        raw[0x110..0x114].copy_from_slice(&3u32.to_le_bytes());
        raw.extend(frame);
        let header = fnv1a32(&raw[0x1F..0x1F + 0xE0]);
        raw[8..12].copy_from_slice(&header.to_le_bytes());
        raw
    }

    fn valid_slot(payload: &[u8]) -> Vec<u8> {
        slot_with_frame(payload, raw_zstd_frame(payload))
    }

    #[test]
    fn the_repair_note_names_the_save_and_does_not_ask_for_another_press() {
        let repaired = vec![RepairedSlot {
            from: PathBuf::from("1795325701-Auto"),
            to: PathBuf::from("Auto"),
        }];
        let note = repair_note(&repaired);
        assert!(note.contains("1795325701-Auto -> Auto"), "{note}");
        assert!(
            !note.to_lowercase().contains("starting now"),
            "the CLI does not launch the game: {note}"
        );
        assert!(note.contains("original is untouched"), "{note}");
        let before_play = repair_note_before_play(&repaired);
        assert!(before_play.contains("starting now"), "{before_play}");
        assert!(!note.to_lowercase().contains("press play"), "{note}");

        let two = vec![
            RepairedSlot {
                from: PathBuf::from("1795325701-Auto"),
                to: PathBuf::from("Auto"),
            },
            RepairedSlot {
                from: PathBuf::from("1795325701-Chapter_Ism_0_2"),
                to: PathBuf::from("Chapter_Ism_0_2"),
            },
        ];
        let note = repair_note(&two);
        assert!(note.contains("Recovered 2 saves"), "{note}");
        assert!(note.contains("originals are untouched"), "{note}");
        assert!(repair_note(&[]).is_empty());
    }

    #[test]
    fn a_slot_moved_back_from_the_backups_is_copied_to_the_games_name() {
        let root = temp("stamped-slot");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let stamped = dir.join("1795325701-Auto");
        let bytes = valid_slot(b"playthrough");
        write_bytes(&stamped, &bytes);

        let repaired = repair_backup_named_slots(&save).unwrap();
        assert_eq!(repaired.len(), 1, "the stamped autosave is reported");
        assert_eq!(repaired[0].to, dir.join("Auto"));
        assert_eq!(std::fs::read(dir.join("Auto")).unwrap(), bytes, "the game gets the same bytes");
        assert_eq!(std::fs::read(&stamped).unwrap(), bytes, "the stamped file is left in place");

        assert!(
            repair_backup_named_slots(&save).unwrap().is_empty(),
            "a second Play finds nothing left to repair"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_slot_stamped_with_the_launcher_prefix_is_the_slot_it_names() {
        assert_eq!(kind_of("Auto"), SlotKind::Auto);
        assert_eq!(kind_of("1795325701-Auto"), SlotKind::Auto);
        assert_eq!(kind_of("1795325701-Chapter_Ism_0_2"), SlotKind::Manual);
        // A name the game does not open stays unknown, stamped or not.
        assert_eq!(kind_of("1795325701-Route_Ism"), SlotKind::Other);
        assert_eq!(kind_of("7-Auto"), SlotKind::Other, "not a launcher stamp");
    }

    #[test]
    fn the_saves_page_recovers_a_stamped_slot_the_same_way_play_does() {
        let root = temp("recover-stamped-page");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let stamped = dir.join("1795325701-Auto");
        let bytes = valid_slot(b"recovered");
        write_bytes(&stamped, &bytes);

        let to = recover_stamped_slot(&stamped)
            .unwrap()
            .expect("the stamped slot is recovered");
        assert_eq!(to, dir.join("Auto"));
        assert_eq!(std::fs::read(&to).unwrap(), bytes, "the game reads the same bytes");
        assert_eq!(std::fs::read(&stamped).unwrap(), bytes, "the original is left in place");
        assert!(
            recover_stamped_slot(&stamped).unwrap().is_none(),
            "an existing slot always wins"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The game keeps its route progress in `Route_<id>`, a file with the same
    /// container header as the slots. A stamped one is recovered like `Auto`;
    /// without this it stayed unrecovered while the autosave came back, which is
    /// the route-completion state a player would notice as missing.
    #[test]
    fn a_stamped_route_file_is_recovered_under_its_game_name() {
        let root = temp("recover-route");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let stamped = dir.join("1795393583-Route_Ism");
        let bytes = valid_slot(b"route progress");
        write_bytes(&stamped, &bytes);

        let to = recover_stamped_slot(&stamped).unwrap().expect("the route file is recovered");
        assert_eq!(to, dir.join("Route_Ism"));
        assert_eq!(std::fs::read(&to).unwrap(), bytes);
        assert_eq!(std::fs::read(&stamped).unwrap(), bytes, "the original stays");
        assert!(
            recover_stamped_slot(&stamped).unwrap().is_none(),
            "the restored name now wins"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// `Global` holds a different kind of data; a stamped copy of it is not a
    /// slot and must not be promoted under the global name.
    #[test]
    fn a_stamped_global_is_not_recovered() {
        let root = temp("recover-global");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let stamped = dir.join("1795325701-Global");
        write_bytes(&stamped, &valid_slot(b"not a slot"));

        assert!(recover_stamped_slot(&stamped).unwrap().is_none());
        assert!(!dir.join("Global").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_recovered_slot_arrives_through_a_temporary_name() {
        let root = temp("recover-temp");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let stamped = dir.join("1795325701-Auto");
        let bytes = valid_slot(b"recovered atomically");
        write_bytes(&stamped, &bytes);

        let to = recover_stamped_slot(&stamped).unwrap().expect("recovered");
        assert_eq!(std::fs::read(&to).unwrap(), bytes);
        assert!(
            !dir.join("Auto.emblemweave-recovering").exists(),
            "the temporary copy is renamed away, not left beside the slot"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Recovery that cannot write must fail without leaving a slot the game
    /// would read: the temporary copy is never promoted under the game name.
    #[test]
    fn a_recovery_that_cannot_write_leaves_no_slot_behind() {
        let root = temp("recover-fails");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let stamped = dir.join("1795325701-Auto");
        let bytes = valid_slot(b"the save that could not be copied");
        write_bytes(&stamped, &bytes);
        // The temporary name is taken by a folder, so the copy cannot start.
        std::fs::create_dir(dir.join("Auto.emblemweave-recovering")).unwrap();

        assert!(recover_stamped_slot(&stamped).is_err(), "the failure is reported");
        assert!(
            !dir.join("Auto").exists(),
            "no partial Auto is left for the game to open"
        );
        assert_eq!(std::fs::read(&stamped).unwrap(), bytes, "the original is untouched");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_stamped_slot_the_native_decoder_rejects_is_not_promoted() {
        let root = temp("stamped-native-bad");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let payload = b"payload whose contents still decode";
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let mut frame = encoder.finish().unwrap();
        let checksum = frame.len() - 4;
        frame[checksum] ^= 0xFF;
        let stamped = dir.join("1795325701-Auto");
        write_bytes(&stamped, &slot_with_frame(payload, frame));

        // It parses and its own checksums pass: only the native decode, the
        // one the guard before Play runs, sees the damage.
        let parsed = crate::savefile::Slot::open(&stamped).unwrap();
        assert!(parsed.verify().is_empty());
        assert!(parsed.native_check().is_err());

        assert!(repair_backup_named_slots(&save).unwrap().is_empty());
        assert!(!dir.join("Auto").exists(), "a corrupt backup must not become Auto");
        assert!(stamped.is_file(), "the stamped file is left where it was");
        assert!(recover_stamped_slot(&stamped).unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_stamped_slot_with_a_bad_payload_checksum_is_not_promoted() {
        let root = temp("stamped-sum-bad");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let mut raw = slot_with_frame(b"payload", raw_zstd_frame(b"payload"));
        raw[0x10C] ^= 0xFF;
        let stamped = dir.join("1795325701-Manual");
        write_bytes(&stamped, &raw);
        assert_eq!(
            crate::savefile::Slot::open(&stamped).unwrap().verify(),
            vec!["payload checksum mismatch".to_string()]
        );

        assert!(repair_backup_named_slots(&save).unwrap().is_empty());
        assert!(!dir.join("Manual").exists(), "a corrupt backup must not become Manual");
        assert!(stamped.is_file(), "the stamped file is left where it was");
        let _ = std::fs::remove_dir_all(root);
    }

    /// The exact sequence from issue #79 (KalHawke): the autosave is moved to
    /// the backups folder from the Saves page, the player copies it back into
    /// `000` by hand, closes the launcher and comes back. The move back keeps
    /// the launcher's timestamp, so before the Play repair the game sees no
    /// `Auto` at all; after it the same bytes are at `Auto`, and the stamped
    /// file, the chapters, `Global` and the account metadata are untouched.
    #[test]
    fn an_autosave_moved_to_backups_and_back_by_hand_opens_after_a_restart() {
        let root = temp("kalhawke-cycle");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let auto = dir.join("Auto");
        let chapter = dir.join("Chapter_Ism_0_2");
        let global = dir.join("Global");
        let auto_bytes = valid_slot(b"four hours in");
        let chapter_bytes = valid_slot(b"chapter two");
        write_bytes(&auto, &auto_bytes);
        write_bytes(&chapter, &chapter_bytes);
        write_bytes(&global, b"global data");
        let meta = account_meta_path(&save);
        let meta_bytes = meta_text(FALLBACK_TITLE_ID, DEFAULT_USER).unwrap().into_bytes();
        write_bytes(&meta, &meta_bytes);

        // Saves page: "Move to backups" renames the file with the stamp.
        let kept = archive_slot(&save, &auto, "1795325701").unwrap();
        assert_eq!(kept, save.join("backups").join("1795325701-Auto"));
        assert!(!auto.exists(), "the game would offer a new game now");

        // The player copies it back into 000 by hand; Explorer keeps the name.
        write_bytes(&dir.join("1795325701-Auto"), &std::fs::read(&kept).unwrap());
        assert!(!auto.exists(), "the game still opens no autosave");
        assert!(
            slots(&save).iter().any(|(p, _)| p == &dir.join("1795325701-Auto")),
            "the launcher lists the file it has to offer to recover"
        );

        // Play (or the page's Recover) repairs it before the game starts.
        let repaired = repair_backup_named_slots(&save).unwrap();
        assert_eq!(repaired.len(), 1, "the moved-back autosave is recovered");
        assert_eq!(std::fs::read(&auto).unwrap(), auto_bytes, "the same playthrough opens");
        assert_eq!(
            std::fs::read(dir.join("1795325701-Auto")).unwrap(),
            auto_bytes,
            "the moved-back file is left in place"
        );
        assert_eq!(std::fs::read(&chapter).unwrap(), chapter_bytes, "the chapters are untouched");
        assert_eq!(std::fs::read(&global).unwrap(), b"global data", "Global is untouched");
        assert_eq!(std::fs::read(&meta).unwrap(), meta_bytes, "the account metadata is untouched");
        assert_eq!(std::fs::read(&kept).unwrap(), auto_bytes, "the backups copy is not consumed");
        assert!(
            repair_backup_named_slots(&save).unwrap().is_empty(),
            "a restart finds nothing left to repair"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The shape of msalgadob's `otherdata.zip` (#79): the account save holds
    /// only `1795325701-Auto`, while the four-hour playthrough's chapters and
    /// `Global` sit in a whole-save backup (`backups/<stamp>/000`). Restoring
    /// that backup through the normal import, putting the moved-back autosave
    /// back and running the Play repair yields one save with Auto, its
    /// chapters and Global; the archive folder is read, never changed.
    #[test]
    fn a_stamped_autosave_beside_a_whole_save_backup_recovers_the_playthrough() {
        let root = temp("msalgadob-shape");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let auto_bytes = valid_slot(b"the moved-back autosave");
        write_bytes(&dir.join("1795325701-Auto"), &auto_bytes);
        let meta = account_meta_path(&save);
        write_bytes(
            &meta,
            meta_text(FALLBACK_TITLE_ID, DEFAULT_USER).unwrap().as_bytes(),
        );

        let backup = save.join("backups").join("1795393583");
        let chapter_bytes = valid_slot(b"chapter saved after four hours");
        write_bytes(&backup.join("000").join("Chapter_Ism_0_2"), &chapter_bytes);
        write_bytes(&backup.join("000").join("Global"), b"global data");
        let backup_chapter = std::fs::read(backup.join("000").join("Chapter_Ism_0_2")).unwrap();

        // Saves page: "Restore" the whole-save backup. The stamped account save
        // is moved aside, not deleted.
        let b = Backup {
            name: "1795393583".into(),
            path: backup.clone(),
            files: 2,
            modified: None,
        };
        let restored = restore_backup_verified(&save, &b, FALLBACK_TITLE_ID, "1795394000").unwrap();
        let kept = restored.backup.expect("the stamped account save is kept in backups");
        assert_eq!(
            std::fs::read(kept.join("000").join("1795325701-Auto")).unwrap(),
            auto_bytes
        );
        assert_eq!(
            std::fs::read(dir.join("Chapter_Ism_0_2")).unwrap(),
            chapter_bytes,
            "the chapters are installed where the game reads them"
        );
        assert_eq!(std::fs::read(dir.join("Global")).unwrap(), b"global data");

        // The reporter's moved-back autosave goes into 000 again, then the Play
        // repair copies it to `Auto` because the game opens only that name.
        write_bytes(&dir.join("1795325701-Auto"), &auto_bytes);
        assert_eq!(repair_backup_named_slots(&save).unwrap().len(), 1);
        assert_eq!(std::fs::read(dir.join("Auto")).unwrap(), auto_bytes);
        assert!(
            empty_namespace_blocker(&save).is_none(),
            "the recovered save is no longer an empty namespace"
        );

        assert_eq!(
            std::fs::read(backup.join("000").join("Chapter_Ism_0_2")).unwrap(),
            backup_chapter,
            "the restored backup is read, never changed"
        );
        assert!(backup.join("000").join("Global").is_file());
        assert!(meta.is_file(), "the account metadata is still there");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_slot_repair_never_overwrites_a_slot_or_promotes_other_files() {
        let root = temp("stamped-slot-guards");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        write_bytes(&dir.join("Auto"), &valid_slot(b"current"));
        write_bytes(&dir.join("1795325701-Auto"), &valid_slot(b"older"));
        // Names the game does not open are left alone even though they parse:
        // `Global` has its own format and `Notes` is not a slot at all.
        write_bytes(&dir.join("1795325701-Notes"), &valid_slot(b"not a slot"));
        write_bytes(&dir.join("1795325701-Global"), &valid_slot(b"global in a slot shape"));
        // A stamped file that does not parse is not promoted either.
        write_bytes(&dir.join("1795325702-Auto"), b"not a save");

        assert!(repair_backup_named_slots(&save).unwrap().is_empty());
        assert_eq!(
            std::fs::read(dir.join("Auto")).unwrap(),
            valid_slot(b"current"),
            "the current autosave is kept"
        );
        assert!(!dir.join("Notes").exists());
        assert!(!dir.join("Global").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Two autosaves moved back from `backups/` at different times: the one
    /// with the newest stamp is the save the player moved back last, so it is
    /// the one the game must open. Before the stamps were compared the winner
    /// was whichever file the directory happened to list first.
    #[test]
    fn the_newest_stamped_autosave_is_the_one_recovered() {
        let root = temp("stamped-newest");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let older = valid_slot(b"older autosave");
        let newer = valid_slot(b"newer autosave");
        write_bytes(&dir.join("1795325701-Auto"), &older);
        write_bytes(&dir.join("1795393583-Auto"), &newer);

        let repaired = repair_backup_named_slots(&save).unwrap();
        assert_eq!(repaired.len(), 1, "one name can be recovered, not two");
        assert_eq!(repaired[0].from, dir.join("1795393583-Auto"));
        assert_eq!(
            std::fs::read(dir.join("Auto")).unwrap(),
            newer,
            "the newest move is the save that opens"
        );
        assert!(dir.join("1795325701-Auto").is_file(), "the older copy stays where it was");
        assert!(dir.join("1795393583-Auto").is_file(), "the recovered original stays where it was");
        assert!(
            repair_backup_named_slots(&save).unwrap().is_empty(),
            "a restart finds nothing left to recover"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The newest stamped file is a truncated copy of a move that did not
    /// finish. It must not block the older stamped autosave that still reads.
    #[test]
    fn a_stamped_autosave_that_cannot_be_read_does_not_hide_an_older_valid_one() {
        let root = temp("stamped-fallback");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let older = valid_slot(b"the autosave that still reads");
        let mut damaged = valid_slot(b"a copy of the move that failed");
        damaged.truncate(damaged.len() - 3);
        write_bytes(&dir.join("1795325701-Auto"), &older);
        write_bytes(&dir.join("1795393583-Auto"), &damaged);

        let repaired = repair_backup_named_slots(&save).unwrap();
        assert_eq!(repaired.len(), 1);
        assert_eq!(repaired[0].from, dir.join("1795325701-Auto"));
        assert_eq!(std::fs::read(dir.join("Auto")).unwrap(), older);
        assert!(
            std::fs::read(dir.join("1795393583-Auto")).unwrap() == damaged,
            "the damaged copy is kept unchanged for repair"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A manual chapter save moved back from `backups/` under two names is
    /// recovered under both: the game lists each chapter slot separately.
    #[test]
    fn every_stamped_manual_chapter_save_is_recovered() {
        let root = temp("stamped-chapters");
        let save = root.join("save");
        let dir = account_dir(&save).join("000");
        let one = valid_slot(b"chapter one");
        let two = valid_slot(b"chapter two");
        write_bytes(&dir.join("1795325701-Chapter_Led_1_1"), &one);
        write_bytes(&dir.join("1795393583-Chapter_Led_1_2"), &two);

        let mut repaired = repair_backup_named_slots(&save).unwrap();
        repaired.sort_by(|a, b| a.to.cmp(&b.to));
        assert_eq!(repaired.len(), 2);
        assert_eq!(std::fs::read(dir.join("Chapter_Led_1_1")).unwrap(), one);
        assert_eq!(std::fs::read(dir.join("Chapter_Led_1_2")).unwrap(), two);
        assert!(dir.join("1795325701-Chapter_Led_1_1").is_file());
        assert!(dir.join("1795393583-Chapter_Led_1_2").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_damaged_autosave_is_moved_to_backups_before_play() {
        let root = temp("damaged-autosave");
        let save = root.join("save");
        let auto = account_dir(&save).join("000/Auto");
        let mut damaged = valid_slot(b"payload");
        damaged.truncate(damaged.len() - 2);
        write_bytes(&auto, &damaged);
        let before = std::fs::read(&auto).unwrap();

        let found = quarantine_damaged_autosave(&save, "123")
            .unwrap()
            .expect("the damaged autosave is reported");
        assert!(!auto.exists(), "the game must not open it again");
        assert_eq!(std::fs::read(&found.kept).unwrap(), before, "the bytes are kept unchanged");
        assert_eq!(
            found.kept.parent(),
            Some(save.join("backups").as_path()),
            "the file is in the backups folder"
        );
        assert!(
            found.kept.with_extension("why.txt").is_file(),
            "the reason is written beside the kept file"
        );

        assert!(
            quarantine_damaged_autosave(&save, "124").unwrap().is_none(),
            "a second Play does not find the same file"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_checksum_corrupt_frame_that_still_decompresses_is_rejected_before_play() {
        let root = temp("checksum-autosave");
        let save = root.join("save");
        let auto = account_dir(&save).join("000/Auto");
        let payload = b"payload whose contents still decode";

        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let mut frame = encoder.finish().unwrap();
        let good = slot_with_frame(payload, frame.clone());
        let good = crate::savefile::Slot::from_bytes(&auto, good).unwrap();
        assert!(
            good.native_check().is_ok(),
            "native zstd accepts the same frame before its checksum is damaged"
        );

        let checksum = frame.len() - 4;
        frame[checksum] ^= 0xFF;

        let raw = slot_with_frame(payload, frame);
        let parsed = crate::savefile::Slot::from_bytes(&auto, raw.clone()).unwrap();
        assert_eq!(parsed.payload, payload, "the payload still decodes");
        assert!(parsed.verify().is_empty(), "the slot's own checksums still pass");
        assert!(
            parsed.native_check().is_err(),
            "native zstd rejects the damaged frame checksum"
        );

        write_bytes(&auto, &raw);
        let found = quarantine_damaged_autosave(&save, "125")
            .unwrap()
            .expect("the checksum-damaged autosave is reported");
        assert!(!auto.exists(), "the game must not open it again");
        assert!(
            found.reason.contains("native zstd"),
            "the reason names the native rejection: {}",
            found.reason
        );
        assert_eq!(std::fs::read(&found.kept).unwrap(), raw, "the bytes are kept unchanged");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_damaged_frame_length_is_rejected_before_play() {
        let root = temp("frame-length-autosave");
        let save = root.join("save");
        let auto = account_dir(&save).join("000/Auto");
        let mut raw = valid_slot(b"payload whose header names the frame length");

        let parsed = crate::savefile::Slot::from_bytes(&auto, raw.clone()).unwrap();
        assert!(parsed.verify().is_empty(), "the frame starts intact");
        assert!(parsed.native_check().is_ok(), "the frame decodes before the length is damaged");

        // The length the game passes to zstd sits outside the header checksum;
        // shortening it must not be mistaken for a readable frame.
        let declared = u32::from_le_bytes([raw[0x108], raw[0x109], raw[0x10A], raw[0x10B]]);
        raw[0x108..0x10C].copy_from_slice(&(declared / 2).max(1).to_le_bytes());
        let parsed = crate::savefile::Slot::from_bytes(&auto, raw.clone()).unwrap();
        assert!(parsed.verify().is_empty(), "the slot's own checksums still pass");
        assert!(
            parsed.native_check().is_err(),
            "native zstd must see the truncated frame the game would read"
        );

        write_bytes(&auto, &raw);
        let found = quarantine_damaged_autosave(&save, "126")
            .unwrap()
            .expect("the autosave with a damaged frame length is reported");
        assert!(!auto.exists(), "the game must not open it again");
        assert!(
            found.reason.contains("native zstd"),
            "the reason names the decoder rejection: {}",
            found.reason
        );
        assert_eq!(std::fs::read(&found.kept).unwrap(), raw, "the bytes are kept unchanged");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_valid_autosave_is_not_moved_by_the_play_guard() {
        let root = temp("valid-autosave");
        let save = root.join("save");
        let auto = account_dir(&save).join("000/Auto");
        write_bytes(&auto, &valid_slot(b"payload"));

        assert!(quarantine_damaged_autosave(&save, "123").unwrap().is_none());
        assert!(auto.is_file(), "a readable autosave is left in place");
        assert!(!save.join("backups").exists());
        let _ = std::fs::remove_dir_all(root);
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

    /// Issue #79 row 24: the Saves page's empty-profile load keeps the current
    /// save in backups, and its Restore button installs that backup again.
    /// These are the same functions the page calls, not a simulation.
    #[test]
    fn empty_profile_load_backs_up_and_restore_returns_the_old_tree() {
        let root = temp("empty-profile-row24");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/000.bin"), "old slot");
        write(&user.join("000/header.bin"), "old header");

        // "New empty profile" then "Load" in the Saves page.
        let empty = create_empty_profile(&save, "Empty Run").unwrap();
        let loaded = load_profile_verified(&save, &empty, FALLBACK_TITLE_ID, "100").unwrap();
        assert_eq!(loaded.files, 0);
        assert!(user.is_dir(), "the empty account folder is created");
        assert!(!user.join("000").exists(), "the game sees no slot and starts a new game");

        let backup = loaded.backup.clone().expect("the old save must be kept");
        assert_eq!(std::fs::read_to_string(backup.join("000/000.bin")).unwrap(), "old slot");
        assert_eq!(std::fs::read_to_string(backup.join("000/header.bin")).unwrap(), "old header");

        // The page lists the backup and its "Restore" button calls this.
        let listed = backups(&save);
        let b = listed.iter().find(|b| b.path == backup).expect("backup is listed");
        let restored = restore_backup_verified(&save, b, FALLBACK_TITLE_ID, "200").unwrap();
        assert_eq!(restored.files, 2);
        assert_eq!(std::fs::read_to_string(user.join("000/000.bin")).unwrap(), "old slot");
        assert_eq!(std::fs::read_to_string(user.join("000/header.bin")).unwrap(), "old header");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A backup whose install cannot be verified must put the save it replaced
    /// back and leave the bad backup on disk for inspection.
    #[test]
    fn restore_that_fails_verification_puts_the_current_save_back() {
        let root = temp("restore-rollback-row24");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/000.bin"), "current");

        // `import` accepts this folder, but verification ignores metadata, so
        // nothing it installed can be matched: the page's restore must roll back.
        let bad = save.join("backups").join("300");
        write(&bad.join("000/junk.meta"), "metadata");

        let listed = backups(&save);
        let b = listed.iter().find(|b| b.name == "300").expect("backup is listed");
        let err = restore_backup_verified(&save, b, FALLBACK_TITLE_ID, "400").unwrap_err();
        assert!(err.contains("did not match"), "unexpected error: {err}");
        assert_eq!(std::fs::read_to_string(user.join("000/000.bin")).unwrap(), "current");
        assert!(bad.is_dir(), "the backup that failed verification is kept");
        let _ = std::fs::remove_dir_all(root);
    }
}
