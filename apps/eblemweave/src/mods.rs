//! RomFS mods: the game's `mods` folder, importing archives and folders, and
//! GameBanana.
//!
//! The engine lays every `<mods>/<name>/romfs` over the game's RomFS when the
//! game starts. A mod folder whose name starts with `_` is skipped, which is
//! how a mod is switched off. `exefs/` patches (`.ips`, `.pchtxt`) cannot
//! apply, because the game's code is compiled ahead of time; an import keeps
//! them but says so.
//!
//! Archives are unpacked with Windows' own `tar.exe` (libarchive), which reads
//! zip, 7z and rar. GameBanana is read through its public API with Windows'
//! own `curl.exe`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What EmblemWeave records about a mod it installed, in `emblemweave.json`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Info {
    pub title: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub gamebanana_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mod {
    pub path: PathBuf,
    pub name: String,
    pub enabled: bool,
    pub info: Info,
    pub files: usize,
    pub has_romfs: bool,
    pub has_exefs: bool,
}

pub fn dir() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("mods"))
}

/// Where removed mods go (outside the mods folder, so the engine never sees them).
pub fn removed_dir() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("mods-removed"))
}

fn find_named(root: &Path, name: &str, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name)) {
                out.push(p);
            } else if depth > 0 {
                find_named(&p, name, depth - 1, out);
            }
        }
    }
}

pub fn list(mods: &Path) -> Vec<Mod> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods) else { return out };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let folder = e.file_name().to_string_lossy().into_owned();
        let enabled = !(folder.starts_with('_') || folder.starts_with('-'));
        let name = folder.trim_start_matches(['_', '-']).to_string();
        let info = std::fs::read_to_string(path.join("emblemweave.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| Info { title: name.clone(), ..Info::default() });
        let (mut r, mut x) = (Vec::new(), Vec::new());
        find_named(&path, "romfs", 3, &mut r);
        find_named(&path, "exefs", 3, &mut x);
        out.push(Mod { files: crate::saves::summary(&path).0, path, name, enabled, info, has_romfs: !r.is_empty(), has_exefs: !x.is_empty() });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn set_enabled(m: &Mod, on: bool) -> Result<PathBuf, String> {
    let parent = m.path.parent().ok_or("no parent folder")?;
    let to = parent.join(if on { m.name.clone() } else { format!("_{}", m.name) });
    if to == m.path {
        return Ok(to);
    }
    std::fs::rename(&m.path, &to).map_err(|e| format!("renaming {}: {e}", m.path.display()))?;
    Ok(to)
}

/// Move a mod out of the mods folder into `mods-removed`.
pub fn remove(m: &Mod) -> Result<PathBuf, String> {
    let bin = removed_dir().ok_or("APPDATA is not set")?;
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let mut to = bin.join(&m.name);
    let mut n = 1;
    while to.exists() {
        n += 1;
        to = bin.join(format!("{} ({n})", m.name));
    }
    std::fs::rename(&m.path, &to).map_err(|e| format!("moving {}: {e}", m.path.display()))?;
    Ok(to)
}

/// A folder name safe on Windows.
pub fn clean_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let t = t.trim().trim_matches('.').trim_start_matches(['_', '-']).trim().to_string();
    if t.is_empty() { "mod".into() } else { t.chars().take(80).collect() }
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn system_tool(name: &str) -> PathBuf {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let p = sys.join("System32").join(name);
    if p.is_file() { p } else { PathBuf::from(name) }
}

fn unpack(archive: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let out = hidden(Command::new(system_tool("tar.exe")).arg("-xf").arg(archive).arg("-C").arg(to))
        .output()
        .map_err(|e| format!("running tar: {e}"))?;
    if !out.status.success() {
        return Err(format!("could not unpack {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let (s, d) = (e.path(), to.join(e.file_name()));
        if s.is_dir() {
            copy_tree(&s, &d)?;
        } else {
            std::fs::copy(&s, &d).map_err(|e| format!("copying {}: {e}", s.display()))?;
        }
    }
    Ok(())
}

/// Top-level RomFS folders of this game, used to recognise a mod packed
/// without a `romfs` folder.
const ROMFS_TOP: &[&str] = &["Animation", "Character", "Effect", "Font", "GameData", "Graphics", "Layout", "Map", "Movie", "Msg", "Sound", "Shader"];

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub mods: Vec<String>,
    pub exefs_only: bool,
}

/// Install the mod in `staged` (an unpacked archive or a folder) into `mods`.
/// Every `romfs` folder found becomes a mod; when there are several (an
/// archive offering variants), all but the first are installed switched off.
pub fn install_from(staged: &Path, mods: &Path, info: &Info) -> Result<Installed, String> {
    let mut romfs = Vec::new();
    find_named(staged, "romfs", 4, &mut romfs);
    romfs.sort();
    if romfs.is_empty() {
        // A folder whose top level looks like the game's RomFS.
        let mut root = staged.to_path_buf();
        for _ in 0..3 {
            let looks = std::fs::read_dir(&root)
                .map(|d| d.flatten().any(|e| ROMFS_TOP.iter().any(|t| e.file_name().to_string_lossy().eq_ignore_ascii_case(t))))
                .unwrap_or(false);
            if looks {
                romfs.push(root.clone());
                break;
            }
            let subdirs: Vec<PathBuf> = std::fs::read_dir(&root).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
            if subdirs.len() != 1 {
                break;
            }
            root = subdirs[0].clone();
        }
    }
    let mut exefs = Vec::new();
    find_named(staged, "exefs", 4, &mut exefs);
    if romfs.is_empty() {
        if !exefs.is_empty() {
            return Ok(Installed { mods: Vec::new(), exefs_only: true });
        }
        return Err("no romfs folder or game files were found in it".into());
    }
    std::fs::create_dir_all(mods).map_err(|e| e.to_string())?;
    let base = clean_name(&info.title);
    let mut names = Vec::new();
    for (i, r) in romfs.iter().enumerate() {
        let variant = r.parent().filter(|p| *p != staged && romfs.len() > 1).and_then(|p| p.strip_prefix(staged).ok()).map(|p| p.display().to_string());
        let mut name = match &variant {
            Some(v) if !v.is_empty() => clean_name(&format!("{base} - {v}")),
            _ => base.clone(),
        };
        let first = name.clone();
        let mut n = 1;
        while mods.join(&name).exists() || mods.join(format!("_{name}")).exists() {
            n += 1;
            name = format!("{first} ({n})");
        }
        let folder = mods.join(if i == 0 { name.clone() } else { format!("_{name}") });
        copy_tree(r, &folder.join("romfs"))?;
        let mut info = info.clone();
        info.title = name.clone();
        let _ = std::fs::write(folder.join("emblemweave.json"), serde_json::to_string_pretty(&info).unwrap_or_default());
        names.push(name);
    }
    Ok(Installed { mods: names, exefs_only: false })
}

fn staging() -> Result<PathBuf, String> {
    let base = crate::engine::run_dir().ok_or("APPDATA is not set")?.join("mods-incoming");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let d = base.join(stamp.to_string());
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Install from an archive (.zip, .7z, .rar) or a folder.
pub fn install(source: &Path, mods: &Path, mut info: Info) -> Result<Installed, String> {
    if info.title.is_empty() {
        info.title = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "mod".into());
    }
    if source.is_dir() {
        return install_from(source, mods, &info);
    }
    let stage = staging()?;
    let r = unpack(source, &stage).and_then(|_| install_from(&stage, mods, &info));
    let _ = std::fs::remove_dir_all(&stage);
    r
}

// ---------------------------------------------------------------- GameBanana

/// Fire Emblem: Fortune's Weave on GameBanana.
pub const GAMEBANANA_GAME: u64 = 26274;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GbMod {
    pub id: u64,
    pub kind: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub likes: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GbFile {
    pub name: String,
    pub size: u64,
    pub url: String,
    pub description: String,
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let out = hidden(Command::new(system_tool("curl.exe")).args(["-sSfL", "--max-time", "600", "-A", "EmblemWeave", url]))
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn json(url: &str) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&fetch(url)?).map_err(|e| format!("GameBanana answered with something unexpected: {e}"))
}

fn s(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

/// One page of the game's submissions, newest first.
pub fn gb_list(page: u32) -> Result<(Vec<GbMod>, bool), String> {
    let v = json(&format!("https://gamebanana.com/apiv11/Game/{GAMEBANANA_GAME}/Subfeed?_nPage={page}&_sSort=new"))?;
    let done = v.pointer("/_aMetadata/_bIsComplete").and_then(|b| b.as_bool()).unwrap_or(true);
    let mods = v
        .get("_aRecords")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter(|r| r.get("_bHasFiles").and_then(|b| b.as_bool()).unwrap_or(true))
                .map(|r| GbMod {
                    id: r.get("_idRow").and_then(|x| x.as_u64()).unwrap_or(0),
                    kind: s(r, "_sModelName"),
                    name: s(r, "_sName"),
                    author: r.get("_aSubmitter").map(|a| s(a, "_sName")).unwrap_or_default(),
                    version: s(r, "_sVersion"),
                    likes: r.get("_nLikeCount").and_then(|x| x.as_u64()).unwrap_or(0),
                    url: s(r, "_sProfileUrl"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((mods, !done))
}

/// The mod id in a GameBanana link (`https://gamebanana.com/mods/738457`).
pub fn gb_id(link: &str) -> Option<u64> {
    let t = link.trim().trim_end_matches('/');
    if let Ok(n) = t.parse() {
        return Some(n);
    }
    let i = t.find("/mods/")?;
    t[i + 6..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

pub fn gb_mod(id: u64) -> Result<(GbMod, Vec<GbFile>), String> {
    let v = json(&format!("https://gamebanana.com/apiv11/Mod/{id}?_csvProperties=_sName,_aFiles,_aSubmitter,_sVersion,_sProfileUrl"))?;
    let m = GbMod {
        id,
        kind: "Mod".into(),
        name: s(&v, "_sName"),
        author: v.get("_aSubmitter").map(|a| s(a, "_sName")).unwrap_or_default(),
        version: s(&v, "_sVersion"),
        likes: 0,
        url: s(&v, "_sProfileUrl"),
    };
    let files = v
        .get("_aFiles")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .map(|f| GbFile {
                    name: s(f, "_sFile"),
                    size: f.get("_nFilesize").and_then(|x| x.as_u64()).unwrap_or(0),
                    url: s(f, "_sDownloadUrl"),
                    description: s(f, "_sDescription"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((m, files))
}

/// Download one of a mod's files and install it.
pub fn gb_install(m: &GbMod, file: &GbFile, mods: &Path) -> Result<Installed, String> {
    if !file.url.starts_with("https://gamebanana.com/") {
        return Err("that download is not on gamebanana.com".into());
    }
    let stage = staging()?;
    let archive = stage.join(clean_name(&file.name));
    let r = fetch(&file.url).and_then(|bytes| {
        std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let unpacked = stage.join("unpacked");
        unpack(&archive, &unpacked)?;
        let info = Info { title: m.name.clone(), source: m.url.clone(), author: m.author.clone(), version: m.version.clone(), gamebanana_id: Some(m.id) };
        install_from(&unpacked, mods, &info)
    });
    let _ = std::fs::remove_dir_all(&stage);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ew-mods-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    #[test]
    fn installs_romfs_layouts() {
        let root = temp("layouts");
        let mods = root.join("mods");
        // romfs folder below a top folder
        let a = root.join("a");
        write(&a.join("FEFW-Witch/romfs/Character/x.bin"));
        let r = install_from(&a, &mods, &Info { title: "Witch".into(), ..Info::default() }).unwrap();
        assert_eq!(r.mods, vec!["Witch".to_string()]);
        assert!(mods.join("Witch/romfs/Character/x.bin").is_file());
        // bare game folders
        let b = root.join("b");
        write(&b.join("Mod/GameData/Person.bin"));
        install_from(&b, &mods, &Info { title: "Recruit".into(), ..Info::default() }).unwrap();
        assert!(mods.join("Recruit/romfs/GameData/Person.bin").is_file());
        // exefs only
        let c = root.join("c");
        write(&c.join("60 FPS/exefs/fps.pchtxt"));
        assert!(install_from(&c, &mods, &Info { title: "fps".into(), ..Info::default() }).unwrap().exefs_only);
        // variants: second one off
        let d = root.join("d");
        write(&d.join("Light/romfs/Msg/a"));
        write(&d.join("Dark/romfs/Msg/a"));
        let r = install_from(&d, &mods, &Info { title: "Theme".into(), ..Info::default() }).unwrap();
        assert_eq!(r.mods.len(), 2);
        let l = list(&mods);
        assert_eq!(l.iter().filter(|m| m.name.starts_with("Theme")).filter(|m| m.enabled).count(), 1);
        // toggle
        let w = l.iter().find(|m| m.name == "Witch").unwrap();
        let off = set_enabled(w, false).unwrap();
        assert!(off.ends_with("_Witch"));
        assert!(!list(&mods).iter().find(|m| m.name == "Witch").unwrap().enabled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_gamebanana_links() {
        assert_eq!(gb_id("https://gamebanana.com/mods/738457"), Some(738457));
        assert_eq!(gb_id("gamebanana.com/mods/737235/"), Some(737235));
        assert_eq!(gb_id("737235"), Some(737235));
        assert_eq!(gb_id("https://example.com"), None);
    }

    /// Installs a real mod from GameBanana (network).
    #[test]
    #[ignore]
    fn live_gamebanana_install() {
        let (feed, _) = gb_list(1).unwrap();
        assert!(!feed.is_empty());
        let mods = temp("gb");
        let (m, files) = gb_mod(737235).unwrap();
        println!("{} by {}: {:?}", m.name, m.author, files.iter().map(|f| &f.name).collect::<Vec<_>>());
        let r = gb_install(&m, &files[0], &mods).unwrap();
        println!("{r:?}");
        let l = list(&mods);
        assert!(l[0].has_romfs && l[0].enabled);
        assert_eq!(l[0].info.gamebanana_id, Some(737235));
        let _ = std::fs::remove_dir_all(mods);
    }
}
