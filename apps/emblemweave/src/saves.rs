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

/// Fire Emblem: Fortune's Weave's program id: the save folder's name in an
/// emulator's NAND. Used when the dump does not state it.
pub const FALLBACK_TITLE_ID: u64 = 0x0400_7EF0_1F8A_E000;

/// Emulators that keep saves as `<root>/nand/user/save/...` under
/// `%APPDATA%`.
const EMULATOR_DIRS: &[&str] = &["yuzu", "suyu", "sudachi", "citron", "eden", "torzu", "Ryujinx-yuzu"];

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
pub fn find(appdata: &Path, title_id: u64) -> Vec<Found> {
    let name = format!("{title_id:016X}");
    let mut out = Vec::new();
    for emu in EMULATOR_DIRS {
        let users = appdata.join(emu).join("nand").join("user").join("save").join("0000000000000000");
        let Ok(entries) = std::fs::read_dir(&users) else { continue };
        for user in entries.flatten() {
            let path = user.path().join(&name);
            if path.is_dir() {
                let (files, modified) = summary(&path);
                if files > 0 {
                    out.push(Found { source: emu.to_string(), path, files, modified });
                }
            }
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// Saves under the current user's `%APPDATA%`.
pub fn find_here(title_id: u64) -> Vec<Found> {
    std::env::var_os("APPDATA").map(|a| find(Path::new(&a), title_id)).unwrap_or_default()
}

/// File count and newest modification time below `dir`.
pub fn summary(dir: &Path) -> (usize, Option<std::time::SystemTime>) {
    let mut files = 0;
    let mut newest = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
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
    for e in std::fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copied += copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("copying {} to {}: {e}", src.display(), dst.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub files: usize,
    /// Where the previous save was moved, if there was one.
    pub backup: Option<PathBuf>,
}

/// The engine's built-in local user id (`[account] uid` unset), as its save
/// folder names it.
pub const DEFAULT_USER: &str = "0100000000000000706d635257464546";
/// Save data and journal size the game asks for: 64 MiB each.
const SAVE_SIZE: u64 = 64 << 20;

/// The account save folder in the engine's save folder: the one the game last wrote, else the default
/// user's.
///
/// The engine names that folder after the save's owner, and more than one can be present: its built-in
/// user, a console user id, or a folder an earlier import left empty. Picking the first by name put a
/// loaded profile into a folder the game never reads, which is why loading a profile looked like it did
/// nothing (the save in the game was unchanged). The one the game last wrote is the one it reads.
pub fn account_dir(save_dir: &Path) -> PathBuf {
    let account = save_dir.join("account");
    let mut best: Option<(std::time::SystemTime, usize, String, PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(&account) {
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let (files, modified) = summary(&path);
            if files == 0 {
                continue;
            }
            let when = modified.unwrap_or(std::time::UNIX_EPOCH);
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let better = match &best {
                None => true,
                Some((t, f, n, _)) => (when, files, &name) > (*t, *f, n),
            };
            if better {
                best = Some((when, files, name, path));
            }
        }
    }
    best.map(|(_, _, _, p)| p).unwrap_or_else(|| account.join(DEFAULT_USER))
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
    Some(format!("size={SAVE_SIZE}\njournal={SAVE_SIZE}\nowner={title_id:x}\nattr={hex}\n"))
}

/// Copy `source` into the game's account save, moving any existing save to
/// `<save folder>/backups/<stamp>/` first.
pub fn import(source: &Path, save_dir: &Path, title_id: u64, stamp: &str) -> Result<Imported, String> {
    if !source.is_dir() {
        return Err(format!("{} is not a folder", source.display()));
    }
    if summary(source).0 == 0 {
        return Err(format!("{} holds no save files", source.display()));
    }
    let target = account_dir(save_dir);
    // A profile under the save folder is fine; the current save, or a folder
    // holding the save folder, is not.
    if source.starts_with(&target) || target.starts_with(source) || save_dir.starts_with(source) {
        return Err("the source and the save folder overlap".into());
    }
    let backup = if target.is_dir() && summary(&target).0 > 0 {
        let to = save_dir.join("backups").join(stamp);
        std::fs::create_dir_all(save_dir.join("backups")).map_err(|e| format!("creating the backups folder: {e}"))?;
        std::fs::rename(&target, &to).map_err(|e| format!("moving the current save to {}: {e}", to.display()))?;
        Some(to)
    } else {
        None
    };
    match copy_tree(source, &target) {
        Ok(files) => {
            let user = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let meta = target.with_extension("meta");
            if !meta.is_file() {
                if let Some(text) = meta_text(title_id, &user) {
                    std::fs::write(&meta, text).map_err(|e| format!("writing {}: {e}", meta.display()))?;
                }
            }
            Ok(Imported { files, backup })
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

/// Copy the game's account save into `dest/<title id>`, the layout an
/// emulator's user folder expects. Returns the folder written.
pub fn export(save_dir: &Path, dest: &Path, title_id: u64) -> Result<(PathBuf, usize), String> {
    let from = account_dir(save_dir);
    if summary(&from).0 == 0 {
        return Err("there is no EmblemWeave save to export yet".into());
    }
    let to = dest.join(format!("{title_id:016X}"));
    if to.exists() {
        return Err(format!("{} already exists; choose an empty folder", to.display()));
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

pub fn profiles(save_dir: &Path) -> Vec<Profile> {
    let mut out: Vec<Profile> = std::fs::read_dir(save_dir.join("profiles"))
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .map(|path| {
                    let (files, modified) = summary(&path);
                    Profile { name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(), path, files, modified }
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
        let b = save_dir.join("backups").join(format!("{stamp}-profile-{name}"));
        std::fs::create_dir_all(save_dir.join("backups")).map_err(|e| e.to_string())?;
        std::fs::rename(&to, &b).map_err(|e| format!("moving the old profile aside: {e}"))?;
    }
    copy_tree(&from, &to)
}

/// Make profile `p` the current save. The current save goes to the backups.
pub fn load_profile(save_dir: &Path, p: &Profile, title_id: u64, stamp: &str) -> Result<Imported, String> {
    import(&p.path, save_dir, title_id, stamp)
}

/// Move a profile to the backups folder.
pub fn remove_profile(save_dir: &Path, p: &Profile, stamp: &str) -> Result<PathBuf, String> {
    let b = save_dir.join("backups").join(format!("{stamp}-profile-{}", p.name));
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
    let name = slot.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() {
        return Err(format!("{} is not a save slot", slot.display()));
    }
    if !slot.is_file() {
        return Err(format!("{} is not there any more", slot.display()));
    }
    let to = save_dir.join("backups").join(format!("{stamp}-{name}"));
    std::fs::create_dir_all(save_dir.join("backups")).map_err(|e| format!("creating the backups folder: {e}"))?;
    std::fs::rename(slot, &to).map_err(|e| format!("moving {} to {}: {e}", slot.display(), to.display()))?;
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
    Ok(Profile { name, path: to, files, modified })
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
    Ok(Profile { name, path: to, files, modified })
}

/// The save slots in the current save: files directly in its folders, newest first.
pub fn slots(save_dir: &Path) -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let mut stack = vec![account_dir(save_dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_none() {
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

    #[test]
    fn finds_emulator_saves_by_title() {
        let root = temp("find");
        let title = root.join("yuzu/nand/user/save/0000000000000000/5A3C/04007EF01F8AE000");
        write(&title.join("000/Auto"), "a");
        write(&root.join("yuzu/nand/user/save/0000000000000000/5A3C/0100000000000000/x"), "other game");
        let found = find(&root, FALLBACK_TITLE_ID);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].source.as_str(), found[0].files), ("yuzu", 1));
        assert_eq!(found[0].path, title);
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
        assert_eq!(std::fs::read_to_string(user.join("000/Auto")).unwrap(), "new");
        let backup = r.backup.unwrap();
        assert_eq!(std::fs::read_to_string(backup.join("000/Auto")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(src.join("000/Auto")).unwrap(), "new");
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
        let meta = std::fs::read_to_string(save.join("account").join(format!("{DEFAULT_USER}.meta"))).unwrap();
        // The same attribute the engine wrote for this game's save.
        assert!(meta.starts_with("size=67108864\njournal=67108864\nowner=4007ef01f8ae000\n"));
        assert!(meta.contains("attr=00e08a1ff07e00040100000000000000706d6352574645460000000000000000010000"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn profiles_store_and_load() {
        let root = temp("profiles");
        let save = root.join("save");
        let user = save.join("account").join(DEFAULT_USER);
        write(&user.join("000/Auto"), "first");
        store_profile(&save, "Run A", "1").unwrap();
        write(&user.join("000/Auto"), "second");
        let ps = profiles(&save);
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "Run A");
        let r = load_profile(&save, &ps[0], FALLBACK_TITLE_ID, "2").unwrap();
        assert_eq!(std::fs::read_to_string(user.join("000/Auto")).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(r.backup.unwrap().join("000/Auto")).unwrap(), "second");
        assert_eq!(slots(&save).len(), 1);
        remove_profile(&save, &ps[0], "3").unwrap();
        assert!(profiles(&save).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_account_folder_is_the_one_the_game_last_wrote() {
        let root = temp("account");
        let save = root.join("save");
        // The built-in user's folder sorts first but is older; the game writes the other one.
        let older = save.join("account").join(DEFAULT_USER);
        let newer = save.join("account").join("0A3C5F00000000000000000000000001");
        write(&older.join("000/Auto"), "old");
        write(&newer.join("000/Auto"), "new");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let new = old + std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(older.join("000/Auto")).unwrap().set_modified(old).unwrap();
        std::fs::File::options().write(true).open(newer.join("000/Auto")).unwrap().set_modified(new).unwrap();
        assert_eq!(account_dir(&save), newer);
        // With nothing in either, the default user's folder is the answer (a first save goes there).
        let empty = temp("account-empty");
        let s2 = empty.join("save");
        std::fs::create_dir_all(s2.join("account").join("0A3C5F00000000000000000000000001")).unwrap();
        assert_eq!(account_dir(&s2), s2.join("account").join(DEFAULT_USER));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(empty);
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
        write(&save.join("account").join(DEFAULT_USER).join("000/Auto"), "a");
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
        write(&save.join("account").join(DEFAULT_USER).join("000/Auto"), "x");
        let (to, n) = export(&save, &root.join("out"), FALLBACK_TITLE_ID).unwrap();
        assert_eq!(n, 1);
        assert!(to.ends_with("04007EF01F8AE000"));
        assert!(to.join("000/Auto").is_file());
        assert!(export(&save, &root.join("out"), FALLBACK_TITLE_ID).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
