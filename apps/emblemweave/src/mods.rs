//! RomFS mods: the game's `mods` folder, importing archives and folders, and
//! GameBanana.
//!
//! The engine lays every `<mods>/<name>/romfs` over the game's RomFS when the
//! game starts. A mod folder whose name starts with `_` is skipped, which is
//! how a mod is switched off. A mod's `exefs/` may hold code mods: DLLs the
//! engine loads at start (`runtime/include/nxrt/mod_api.h`) and the hook sites
//! they need (`*.sites`, built in by Build the game; see `sites.rs`).
//! `exefs/` patches (`.ips`, `.ips32`, `.pchtxt`) are applied to the launcher's
//! uncompressed copy of `main` before lifting and compiling; see `patches.rs`.
//! They are never applied to the user's dump or to the built Windows exe.
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
    /// The GameBanana file this copy came from, as uploaded. A page can hold
    /// two mods; their files' names keep them apart (`family`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_name: String,
    /// The variant picked when the download offered several (one per platform, say).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub variant: String,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
    /// `exefs/` holds a code mod: a `.dll` or `.sites`.
    pub has_code: bool,
    /// `exefs/` holds `.ips` / `.ips32` / `.pchtxt` patches.
    pub has_patches: bool,
}

/// The `exefs` folders of a mod (up to three folders deep).
pub fn exefs_dirs(path: &Path) -> Vec<PathBuf> {
    let mut x = Vec::new();
    find_named(path, "exefs", 3, &mut x);
    x
}

fn exefs_has(dir: &Path, exts: &[&str]) -> bool {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten().any(|e| {
                let p = e.path();
                kind(&e).is_some_and(|t| t.is_file()) && p.extension().is_some_and(|x| exts.iter().any(|w| x.to_string_lossy().eq_ignore_ascii_case(w)))
            })
        })
        .unwrap_or(false)
}

/// Patch files may sit in a subfolder of `exefs/`; `patches::inspect` walks
/// that tree, so installation and the Mods page must see the same files.
fn exefs_has_patches(dir: &Path) -> bool {
    exefs_has(dir, &["ips", "ips32", "pchtxt"])
        || std::fs::read_dir(dir)
            .map(|d| d.flatten().filter(|e| kind(e).is_some_and(|t| t.is_dir())).any(|e| exefs_has_patches(&e.path())))
            .unwrap_or(false)
}

/// An `exefs` folder that holds a code mod for this engine (a DLL or hook sites).
pub fn is_code_exefs(dir: &Path) -> bool {
    exefs_has(dir, &["dll", "sites"])
}

pub fn dir() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("mods"))
}

/// Where removed mods go (outside the mods folder, so the engine never sees them).
pub fn removed_dir() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("mods-removed"))
}

/// A directory entry's kind without following it: symbolic links and
/// junctions (reparse points that name another place) are skipped by every
/// walk, so a mod cannot reach outside its own folder.
fn kind(e: &std::fs::DirEntry) -> Option<std::fs::FileType> {
    e.file_type().ok().filter(|t| !t.is_symlink())
}

fn find_named(root: &Path, name: &str, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if kind(&e).is_some_and(|t| t.is_dir()) {
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
        // `.partial-…` is an install being built; the engine skips dot folders too.
        if folder.starts_with('.') {
            continue;
        }
        let enabled = !(folder.starts_with('_') || folder.starts_with('-'));
        let name = folder.trim_start_matches(['_', '-']).to_string();
        let info = std::fs::read_to_string(path.join("emblemweave.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| Info { title: name.clone(), ..Info::default() });
        let (mut r, mut x) = (Vec::new(), Vec::new());
        find_named(&path, "romfs", 3, &mut r);
        find_named(&path, "exefs", 3, &mut x);
        let has_code = x.iter().any(|d| is_code_exefs(d));
        let has_patches = x.iter().any(|d| exefs_has_patches(d));
        out.push(Mod {
            files: crate::saves::summary(&path).0,
            path,
            name,
            enabled,
            info,
            has_romfs: !r.is_empty(),
            has_exefs: !x.is_empty(),
            has_code,
            has_patches,
        });
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
    let bin = removed_dir().ok_or(crate::platform::NO_DATA_DIR)?;
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

/// A file that two or more switched-on mods each replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The file inside the RomFS, as `Graphics/Character/x.bin`.
    pub file: String,
    /// The mods that replace it, in the order the engine lays them down.
    pub mods: Vec<String>,
}

fn romfs_of(m: &Mod) -> Option<PathBuf> {
    let mut found = Vec::new();
    find_named(&m.path, "romfs", 3, &mut found);
    found.into_iter().min()
}

fn walk_rel(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let Some(t) = kind(&e) else { continue };
        if t.is_dir() {
            walk_rel(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            // The engine's file system ignores case, so two spellings of one
            // path are one file.
            out.push(rel.to_string_lossy().replace('\\', "/").to_lowercase());
        }
    }
}

/// Every file that two or more switched-on mods replace, with the mods that
/// replace it. The engine's overlay order decides which one wins; this says
/// which files are decided that way.
pub fn conflicts(list: &[Mod]) -> Vec<Conflict> {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in list.iter().filter(|m| m.enabled) {
        let Some(root) = romfs_of(m) else { continue };
        let mut files = Vec::new();
        walk_rel(&root, &root, &mut files);
        for f in files {
            seen.entry(f).or_default().push(m.info.title.clone());
        }
    }
    seen.into_iter()
        .filter(|(_, mods)| mods.len() > 1)
        .map(|(file, mods)| Conflict { file, mods })
        .collect()
}

/// Switch mods on or off so the set that is on matches `want` (mod names
/// without the `_` a switched-off one carries). Returns how many changed.
pub fn apply_profile(list: &[Mod], want: &[String]) -> Result<usize, String> {
    let mut changed = 0;
    for m in list {
        let on = want.iter().any(|w| w.eq_ignore_ascii_case(&m.name));
        if on != m.enabled {
            set_enabled(m, on)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// A folder name safe on Windows.
pub fn clean_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let t = t.trim().trim_matches('.').trim_start_matches(['_', '-']).trim().to_string();
    // Windows refuses a name that ends in a dot or a space, and cutting at 80 can leave one.
    let t: String = t.chars().take(80).collect::<String>().trim_end_matches(['.', ' ']).to_string();
    if t.is_empty() {
        return "mod".into();
    }
    // `CON`, `NUL`, `COM1`… are devices on Windows, with or without an extension.
    let (stem, rest) = t.split_at(t.find('.').unwrap_or(t.len()));
    let up = stem.trim_end().to_ascii_uppercase();
    let device = matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((up.starts_with("COM") || up.starts_with("LPT")) && up.len() == 4 && up.as_bytes()[3].is_ascii_digit() && up.as_bytes()[3] != b'0');
    if device { format!("{stem}_{rest}") } else { t }
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
        let Some(t) = kind(&e) else { continue };
        if t.is_dir() {
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
    /// A mod with only hook sites or a DLL and no RomFS.
    pub exefs_only: bool,
    /// A mod with only ExeFS patches and no RomFS or code.
    pub patch_only: bool,
    /// A mod brought hook sites or patches: the game needs building again before they take effect.
    pub needs_build: bool,
    /// The variants installed were optional parts of one mod, not variants
    /// that exclude each other (`Prepared::exclusive`): every one is
    /// switched on.
    pub parts: bool,
    /// The new copy was installed switched off because this mod, already on,
    /// is the same one (the same GameBanana mod, or a code mod with a DLL of
    /// the same name): two copies switched on would both load.
    pub off_for: Option<String>,
}

/// One thing a download can install: a `romfs` and/or an `exefs` from one
/// folder. A download that offers several (one per platform, or a light
/// and a dark theme) has one `Choice` each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What tells it apart from the other choices (the folder name that
    /// differs), empty when there is only one.
    pub label: String,
    pub romfs: Option<PathBuf>,
    pub exefs: Option<PathBuf>,
    /// How well it fits this PC build, from its folder names: above zero
    /// for EmblemWeave / recomp / PC builds, below zero for console or
    /// emulator builds.
    pub fit: i32,
    pub files: usize,
    /// Its files, relative and in lower case (`romfs/msg/a`, `exefs/x.dll`):
    /// variants that share one replace each other (`Prepared::exclusive`).
    pub paths: Vec<String>,
}

/// A download or folder unpacked and looked at, before anything is written
/// to the mods folder. `discard` removes the unpacked copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The unpacked copy, removed by `discard` (none for a folder installed in place).
    pub stage: Option<PathBuf>,
    pub info: Info,
    pub choices: Vec<Choice>,
    /// The download has an `exefs` but nothing this engine can use.
    pub unusable_exefs: bool,
    pub patch_only: bool,
    pub exefs_only: bool,
}

impl Prepared {
    pub fn discard(&self) {
        if let Some(s) = &self.stage {
            let _ = std::fs::remove_dir_all(s);
        }
    }

    /// The choice to preselect: the best fit, the first on a tie.
    pub fn recommended(&self) -> usize {
        let mut best = 0;
        for (i, c) in self.choices.iter().enumerate() {
            if c.fit > self.choices[best].fit {
                best = i;
            }
        }
        best
    }

    /// The download's variants are builds of one mod for different platforms
    /// (Switch, emulator, PC, Obelisk, recomp…), which exclude each other,
    /// rather than optional parts to install together: two or more of them
    /// name a platform. One that alone names one (`PC portraits` beside
    /// `Main`) is a part.
    pub fn platform_builds(&self) -> bool {
        named_platforms(self.choices.iter()) >= 2
    }

    /// The picked variants exclude each other, so only one of them may be on:
    /// they are platform builds (two or more name a platform), or two of them
    /// share a file (`romfs/…` or `exefs/…` at the same relative path), as a
    /// light and a dark theme of the same files do. Only variants whose
    /// files are all different are optional parts, all switched on.
    pub fn exclusive(&self, picks: &[usize]) -> bool {
        let chosen: Vec<&Choice> = picks.iter().filter_map(|&i| self.choices.get(i)).collect();
        if chosen.len() < 2 {
            return false;
        }
        if named_platforms(chosen.iter().copied()) >= 2 {
            return true;
        }
        let mut seen = std::collections::HashSet::new();
        chosen.iter().any(|c| c.paths.iter().any(|p| !seen.insert(p.as_str())))
    }
}

/// How many of these variants name a platform, by folder names.
fn named_platforms<'a>(choices: impl Iterator<Item = &'a Choice>) -> usize {
    choices.filter(|c| c.fit != 0 || platform_label(&c.label)).count()
}

/// A variant's folder name that names a platform: anything `fit` scores,
/// and the bare platform words it leaves alone because they are ordinary
/// words in mod names too (`Switch`, `Windows`).
pub fn platform_label(label: &str) -> bool {
    let l = label.to_lowercase();
    fit(&l) != 0 || l.split(|c: char| !c.is_alphanumeric()).any(|w| matches!(w, "switch" | "nx" | "console" | "windows" | "win64" | "linux" | "steam" | "desktop"))
}

/// How well a file or folder name fits this PC build of the game. Mod pages
/// often carry a Switch / emulator build beside the PC one, and the source
/// code beside the mod.
pub fn fit(text: &str) -> i32 {
    let t = text.to_lowercase();
    // Whole words, so "Weapon Switch", "NPC" or "Sweden" do not count.
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let mut n = 0;
    for (w, score) in [
        ("emblemweave", 3),
        ("recomp", 2),
        ("recomps", 2),
        ("recompiled", 2),
        ("obelisk", 2),
        ("pc", 2),
        ("dll", 1),
        ("atmosphere", -3),
        ("layeredfs", -3),
        ("emulator", -3),
        ("emulators", -3),
        ("emu", -3),
        ("emus", -3),
        ("yuzu", -3),
        ("ryujinx", -3),
        ("eden", -2),
        ("sudachi", -3),
        ("citron", -3),
        ("nro", -2),
        ("runtimepatcher", -3),
    ] {
        if has(w) {
            n += score;
        }
    }
    // Phrases that only name a platform: "(Switch)", "for Switch", "Windows build".
    let pair = |a: &str, b: &str| words.windows(2).any(|w| w[0] == a && w[1] == b);
    let bracketed = |w: &str| {
        let chars: Vec<char> = t.chars().collect();
        let wc: Vec<char> = w.chars().collect();
        (0..chars.len().saturating_sub(wc.len() - 1)).any(|i| {
            chars[i..].starts_with(&wc)
                && i > 0
                && chars[i - 1] == '('
                && chars.get(i + wc.len()).is_none_or(|c| !c.is_alphanumeric())
        })
    };
    if bracketed("switch") || pair("nintendo", "switch") || pair("for", "switch") {
        n -= 2;
    }
    if t.contains("contents/0") {
        n -= 2;
    }
    if bracketed("windows") || pair("for", "windows") || pair("windows", "build") {
        n += 2;
    }
    n
}

/// Look at an unpacked download (or a folder) and list what it can install.
pub fn analyse(staged: &Path) -> Result<(Vec<Choice>, bool, bool, bool), String> {
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
            let subdirs: Vec<PathBuf> = std::fs::read_dir(&root).map(|d| d.flatten().filter(|e| kind(e).is_some_and(|t| t.is_dir())).map(|e| e.path()).collect()).unwrap_or_default();
            if subdirs.len() != 1 {
                break;
            }
            root = subdirs[0].clone();
        }
    }
    let mut exefs = Vec::new();
    find_named(staged, "exefs", 4, &mut exefs);
    exefs.sort();
    let code: Vec<PathBuf> = exefs.iter().filter(|e| is_code_exefs(e)).cloned().collect();
    let patches: Vec<PathBuf> = exefs.iter().filter(|e| exefs_has_patches(e)).cloned().collect();
    let content: Vec<PathBuf> = exefs.iter().filter(|e| is_code_exefs(e) || exefs_has_patches(e)).cloned().collect();
    if romfs.is_empty() && content.is_empty() {
        if !exefs.is_empty() {
            return Ok((Vec::new(), true, false, false));
        }
        return Err("no romfs folder, patches or game files were found in it".into());
    }
    let patch_only = romfs.is_empty() && code.is_empty() && !patches.is_empty();
    let exefs_only = romfs.is_empty() && patches.is_empty() && !code.is_empty();
    // What each mod is made of: its romfs and/or an exefs from the same folder.
    let mut units: Vec<(Option<PathBuf>, Option<PathBuf>)> = if romfs.is_empty() {
        content.iter().map(|x| (None, Some(x.clone()))).collect()
    } else {
        romfs.iter().map(|r| (Some(r.clone()), content.iter().find(|x| pairs(r, x)).cloned())).collect()
    };
    if !romfs.is_empty() {
        for x in &content {
            let paired = romfs.iter().any(|r| pairs(r, x));
            if !paired {
                units.push((None, Some(x.clone())));
            }
        }
    }
    // Each choice's folder below the download, as path parts.
    let parts: Vec<Vec<String>> = units
        .iter()
        .map(|(r, x)| {
            let key = r.as_ref().or(x.as_ref()).expect("a unit has a romfs or an exefs");
            let home = if romfs.iter().any(|q| Some(q) == r.as_ref()) && !key.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("romfs")) {
                // A bare RomFS folder (Graphics/, Msg/…) is its own home.
                key.clone()
            } else {
                key.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            home.strip_prefix(staged).map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()).unwrap_or_default()
        })
        .collect();
    let labels = labels(&parts);
    let choices = units
        .into_iter()
        .zip(labels)
        .zip(parts)
        .map(|(((r, x), label), parts)| {
            let mut names = Vec::new();
            let mut paths = Vec::new();
            for (d, top) in [(r.as_deref(), "romfs"), (x.as_deref(), "exefs")] {
                let Some(d) = d else { continue };
                let at = names.len();
                walk_rel(d, d, &mut names);
                paths.extend(names[at..].iter().map(|n| format!("{top}/{n}")));
            }
            paths.sort();
            paths.dedup();
            // An NRO plugin only runs on an emulator's plugin loader.
            let nro = if names.iter().any(|n| n.ends_with(".nro")) && !names.iter().any(|n| n.ends_with(".dll")) { -3 } else { 0 };
            Choice { fit: fit(&parts.join("/")) + nro, label, romfs: r, exefs: x, files: names.len(), paths }
        })
        .collect();
    Ok((choices, false, patch_only, exefs_only))
}

/// An `exefs` belongs with a `romfs` beside it, or inside a bare RomFS
/// folder (one that holds `Graphics/`, `Msg/`… itself).
fn pairs(romfs: &Path, exefs: &Path) -> bool {
    exefs.parent() == romfs.parent() || exefs.parent() == Some(romfs)
}

/// Short names that tell the choices apart: the first folder in which their
/// paths differ (`Emulators`, `Obelisk Chamber (PC recomp)`), more of the
/// path only where that is not enough. One choice needs no name.
fn labels(parts: &[Vec<String>]) -> Vec<String> {
    if parts.len() < 2 {
        return vec![String::new(); parts.len()];
    }
    let mut common = 0;
    while parts.iter().all(|p| p.len() > common) && parts.iter().all(|p| p[common].eq_ignore_ascii_case(&parts[0][common])) {
        common += 1;
    }
    let first: Vec<String> = parts.iter().map(|p| p.get(common).cloned().unwrap_or_default()).collect();
    let mut out: Vec<String> = first
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let unique = first.iter().filter(|g| g.eq_ignore_ascii_case(f)).count() == 1;
            let label = if unique && !f.is_empty() { f.clone() } else { parts[i][common.min(parts[i].len())..].join(" / ") };
            if label.is_empty() { "main".to_string() } else { label }
        })
        .collect();
    // Whatever is still the same gets a number, so every choice has its own name.
    for i in 1..out.len() {
        let base = out[i].clone();
        let mut n = 1;
        while out[..i].iter().any(|o| o.eq_ignore_ascii_case(&out[i])) {
            n += 1;
            out[i] = format!("{base} ({n})");
        }
    }
    out
}

/// Installs run on worker threads; this keeps two of them from
/// picking the same folder name at once.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn free_name(mods: &Path, want: &str) -> String {
    let mut name = want.to_string();
    let mut n = 1;
    while mods.join(&name).exists() || mods.join(format!("_{name}")).exists() || mods.join(format!("-{name}")).exists() {
        n += 1;
        name = format!("{want} ({n})");
    }
    name
}

/// The DLL file names in an `exefs` folder, lowercase. Two code mods that
/// ship the same one are most likely two copies of one mod.
fn dll_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| kind(e).is_some_and(|t| t.is_file()))
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .filter(|n| n.ends_with(".dll"))
                .collect()
        })
        .unwrap_or_default()
}

/// A switched-on mod in `mods` that a new copy would load beside: the same
/// file of the same GameBanana mod, else a code mod with a DLL of the same
/// name. Its title. The page id alone is not enough, as a page can hold two
/// mods: the file names must be of one family too (`family`), unless either
/// copy's file is not known.
fn already_on(mods: &Path, gb: Option<u64>, file_name: &str, dlls: &[String]) -> Option<String> {
    let fam = family(file_name);
    let mut same_code = None;
    for e in std::fs::read_dir(mods).ok()?.flatten() {
        let folder = e.file_name().to_string_lossy().into_owned();
        if !kind(&e).is_some_and(|t| t.is_dir()) || folder.starts_with(['.', '_', '-']) {
            continue;
        }
        let path = e.path();
        let info: Option<Info> = std::fs::read_to_string(path.join("emblemweave.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
        let same_mod = gb.is_some()
            && info.as_ref().is_some_and(|i| {
                let theirs = family(&i.file_name);
                i.gamebanana_id == gb && (fam.is_empty() || theirs.is_empty() || theirs == fam)
            });
        let same_dll = !dlls.is_empty() && exefs_dirs(&path).iter().any(|x| dll_names(x).iter().any(|n| dlls.contains(n)));
        let title = || info.clone().map(|i| i.title).filter(|t| !t.is_empty()).unwrap_or(folder.clone());
        if same_mod {
            return Some(title());
        }
        if same_dll && same_code.is_none() {
            same_code = Some(title());
        }
    }
    same_code
}

/// Code mods switched on together that ship a DLL of the same name: both
/// load, most likely two copies of one mod. One entry per DLL, as
/// `exefs/<name>.dll`.
pub fn code_clashes(list: &[Mod]) -> Vec<Conflict> {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in list.iter().filter(|m| m.enabled && m.has_code) {
        let mut names: Vec<String> = exefs_dirs(&m.path).iter().flat_map(|x| dll_names(x)).collect();
        names.sort();
        names.dedup();
        for n in names {
            seen.entry(n).or_default().push(m.info.title.clone());
        }
    }
    seen.into_iter()
        .filter(|(_, mods)| mods.len() > 1)
        .map(|(dll, mods)| Conflict { file: format!("exefs/{dll}"), mods })
        .collect()
}

fn build_folder(c: &Choice, folder: &Path) -> Result<bool, String> {
    let mut needs_build = false;
    if let Some(r) = &c.romfs {
        copy_tree(r, &folder.join("romfs"))?;
        // A bare RomFS folder may hold the mod's exefs too: it is not game data.
        if c.exefs.as_deref().is_some_and(|x| x.parent() == Some(r.as_path())) {
            let _ = std::fs::remove_dir_all(folder.join("romfs").join(c.exefs.as_deref().unwrap().file_name().unwrap_or_default()));
        }
    }
    if let Some(x) = &c.exefs {
        copy_tree(x, &folder.join("exefs"))?;
        needs_build |= exefs_has(x, &["sites"]) || exefs_has_patches(x);
    }
    Ok(needs_build)
}

/// Install the picked choices into `mods`. Variants that exclude each other
/// (`Prepared::exclusive`: platform builds, or variants sharing a file): one is switched on, the recommended one (`Prepared::recommended`,
/// the variant the dialog preselects) when it is picked, else the first
/// picked; it is installed first and named first in `Installed::mods`, and
/// any others are installed switched off.
/// Optional parts (variants whose files are all different) are all switched on.
/// Each is built in a `.partial-…` folder
/// (which the engine and `list` skip) and renamed into place, so a failed
/// install leaves nothing half copied; on an error every folder this call
/// made is removed again.
pub fn finish(p: &Prepared, picks: &[usize], mods: &Path) -> Result<Installed, String> {
    if p.unusable_exefs {
        return Ok(Installed { mods: Vec::new(), exefs_only: true, patch_only: false, needs_build: false, parts: false, off_for: None });
    }
    std::fs::create_dir_all(mods).map_err(|e| e.to_string())?;
    let mut partials: Vec<PathBuf> = Vec::new();
    let mut placed: Vec<PathBuf> = Vec::new();
    let r = finish_into(p, picks, mods, &mut partials, &mut placed);
    if r.is_err() {
        for d in partials.iter().chain(&placed) {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    r
}

fn finish_into(p: &Prepared, picks: &[usize], mods: &Path, partials: &mut Vec<PathBuf>, placed: &mut Vec<PathBuf>) -> Result<Installed, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let base = clean_name(&p.info.title);
    // Optional parts of one mod are all switched on; of variants that exclude each other only one,
    // built first: the recommended variant (the one the dialog preselects) when it was picked, else
    // the first picked.
    let parts = picks.len() > 1 && !p.exclusive(picks);
    let mut picks = picks.to_vec();
    if !parts {
        if let Some(at) = picks.iter().position(|&i| i == p.recommended()) {
            let rec = picks.remove(at);
            picks.insert(0, rec);
        }
    }
    let mut needs_build = false;
    let mut built = Vec::new();
    for &i in &picks {
        let c = p.choices.get(i).ok_or("that choice is not in the download")?;
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = mods.join(format!(".partial-{stamp}-{}-{n}", std::process::id()));
        partials.push(partial.clone());
        needs_build |= build_folder(c, &partial)?;
        built.push((c, partial));
    }
    // Only naming and the renames need the lock: two installs never pick one name.
    let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    // A second copy of a mod that is on (Install again, the same link, another file of its family)
    // comes in switched off. Only mods there before this install count: the variants it places
    // below share its page and file, and never switch each other off.
    let dlls = built.first().map(|(_, partial)| dll_names(&partial.join("exefs"))).unwrap_or_default();
    let off_for = already_on(mods, p.info.gamebanana_id, &p.info.file_name, &dlls);
    let mut names = Vec::new();
    for (n, (c, partial)) in built.into_iter().enumerate() {
        let want = if c.label.is_empty() || p.choices.len() < 2 { base.clone() } else { clean_name(&format!("{base} - {}", c.label)) };
        let name = free_name(mods, &want);
        let folder = mods.join(if (n == 0 || parts) && off_for.is_none() { name.clone() } else { format!("_{name}") });
        let mut info = p.info.clone();
        info.title = name.clone();
        info.variant = c.label.clone();
        let json = partial.join("emblemweave.json");
        std::fs::write(&json, serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?).map_err(|e| format!("writing {}: {e}", json.display()))?;
        rename_retry(&partial, &folder).map_err(|e| format!("moving {} into place: {e}", folder.display()))?;
        partials.retain(|x| x != &partial);
        placed.push(folder);
        names.push(name);
    }
    Ok(Installed { mods: names, exefs_only: p.exefs_only, patch_only: p.patch_only, needs_build, parts, off_for })
}

/// A rename, tried a few times: on Windows an indexer or a virus scanner can
/// hold a just-written folder for a moment.
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for _ in 0..4 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(last.expect("tried at least once"))
}

/// Install the mod in `staged` (an unpacked archive or a folder) into `mods`.
/// Every `romfs` folder found becomes a mod; when there are several (an
/// archive offering variants), they are installed as `finish` installs them:
/// platform builds with only the recommended one switched on, optional parts
/// all switched on.
/// A code mod's `exefs` (a DLL or hook sites) next to a `romfs` goes with it;
/// a code mod without a `romfs` is a mod of its own. The Mods page asks
/// which variant to install instead (`prepare` + `finish`).
#[cfg(test)]
pub fn install_from(staged: &Path, mods: &Path, info: &Info) -> Result<Installed, String> {
    let p = prepare_dir(staged, None, info.clone())?;
    let all: Vec<usize> = (0..p.choices.len()).collect();
    finish(&p, &all, mods)
}

fn prepare_dir(dir: &Path, stage: Option<PathBuf>, info: Info) -> Result<Prepared, String> {
    let (choices, unusable_exefs, patch_only, exefs_only) = analyse(dir)?;
    Ok(Prepared { stage, info, choices, unusable_exefs, patch_only, exefs_only })
}

fn staging() -> Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let base = crate::engine::run_dir().ok_or(crate::platform::NO_DATA_DIR)?.join("mods-incoming");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // Unique per download, even for several started in the same instant
    // or by two copies of the launcher.
    loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = base.join(format!("{stamp}-{}-{n}", std::process::id()));
        match std::fs::create_dir(&d) {
            Ok(()) => return Ok(d),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Mark an unpacked download as still wanted, so `clean_incoming` in another
/// copy of the launcher leaves it alone while its question waits.
pub fn keep_stage(p: &Prepared) {
    if let Some(s) = &p.stage {
        let _ = std::fs::write(s.join("waiting"), now().to_string());
    }
}

/// Remove unpacked downloads left in `mods-incoming` by a launcher that was
/// closed while it waited for a choice, and `.partial-…` installs left in
/// the mods folder by one that was closed mid-copy. Only folders older than
/// an hour go, so a download in progress in another copy of the launcher is
/// left alone; a `.partial-…` install is kept for a day. A folder whose
/// name carries the id of a launcher still running (another copy, whose
/// question may wait for days) is never removed; when the process list
/// cannot be read, no folder carrying another launcher's id is. It walks
/// folders and reads the process list: call it off the UI thread.
pub fn clean_incoming() {
    let Some(run) = crate::engine::run_dir() else { return };
    let own = std::process::id();
    let live = crate::cheats::process::pids().map(|mut l| {
        l.push(own);
        l
    });
    clean_in(&run, std::time::SystemTime::now(), live.as_deref(), own);
}

/// The launcher process that made a stage or `.partial-…` folder: its names
/// are `<stamp>-<pid>-<n>`.
fn stage_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(".partial-").unwrap_or(name);
    rest.split('-').nth(1)?.parse().ok()
}

/// `live` is the running processes, `None` when they are unknown: then
/// every folder made by another launcher (`own` is this one) is kept, and
/// only folders without an id, or with this launcher's, follow the age rule.
fn clean_in(run: &Path, now: std::time::SystemTime, live: Option<&[u32]>, own: u32) {
    let alive = |e: &std::fs::DirEntry| {
        stage_pid(&e.file_name().to_string_lossy()).is_some_and(|pid| match live {
            Some(live) => live.contains(&pid),
            None => pid != own,
        })
    };
    let hour_ago = now - std::time::Duration::from_secs(3600);
    let day_ago = now - std::time::Duration::from_secs(24 * 3600);
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    if let Ok(entries) = std::fs::read_dir(run.join("mods-incoming")) {
        for e in entries.flatten() {
            let p = e.path();
            if !kind(&e).is_some_and(|t| t.is_dir()) || alive(&e) {
                continue;
            }
            let newest = [modified(&p), modified(&p.join("waiting"))].into_iter().flatten().max();
            if newest.is_some_and(|t| t < hour_ago) {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(run.join("mods")) {
        for e in entries.flatten() {
            let p = e.path();
            let partial = e.file_name().to_string_lossy().starts_with(".partial-");
            if partial && kind(&e).is_some_and(|t| t.is_dir()) && !alive(&e) && modified(&p).is_some_and(|t| t < day_ago) {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }
}

/// Unpack an archive (.zip, .7z, .rar) or look at a folder, ready for `finish`.
pub fn prepare(source: &Path, mut info: Info) -> Result<Prepared, String> {
    if info.title.is_empty() {
        info.title = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "mod".into());
    }
    if source.is_dir() {
        return prepare_dir(source, None, info);
    }
    let stage = staging()?;
    let unpacked = stage.join("unpacked");
    let r = unpack(source, &unpacked).and_then(|_| prepare_dir(&unpacked, Some(stage.clone()), info));
    if r.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GbFile {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub url: String,
    pub description: String,
    /// The version the author gave this file, often empty.
    pub version: String,
    /// When it was uploaded (Unix seconds).
    pub date: u64,
    /// The author marked it as an old version.
    pub archived: bool,
}

impl GbFile {
    /// The mod's source code, offered beside the mod ("SOURCE CODE ONLY —
    /// Not required to play"): nothing to install.
    pub fn source_only(&self) -> bool {
        let name = self.name.to_lowercase();
        let desc = self.description.to_lowercase();
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
        let src_word = desc.split(|c: char| !c.is_alphanumeric()).any(|w| w == "src" || w == "sources");
        let desc_says = (desc.contains("source code") || desc.contains("source only") || src_word) && (desc.contains("only") || desc.contains("not required") || desc.contains("not needed"));
        desc_says
            || ["source-only", "source_only", "sourceonly", "source-code", "source_code", "sourcecode"].iter().any(|w| name.contains(w))
            || ["-src", "_src", "-source", "_source"].iter().any(|w| stem.ends_with(w))
    }

    /// A current download of the mod itself: not an old version, not source code.
    pub fn playable(&self) -> bool {
        !self.archived && !self.source_only()
    }

    /// How well it fits this PC build (see `fit`), from its name and description.
    pub fn fit(&self) -> i32 {
        fit(&format!("{} {}", self.name, self.description))
    }
}

/// The part of a file name that stays the same from one upload to the next:
/// `pleasenomoreclicking-v200-main-mod.zip` and `…-v210-…` are both
/// `pleasenomoreclicking`. Pages that hold two mods keep them apart this way.
pub fn family(file_name: &str) -> String {
    let l = file_name.to_lowercase();
    let mut stem = l.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(l);
    // GameBanana adds `_<5 hex>` to a name it has seen before (`quicksave-100_6e177`).
    if let Some((head, tail)) = stem.rsplit_once('_') {
        if tail.len() == 5 && tail.chars().all(|c| c.is_ascii_hexdigit()) && tail.chars().any(|c| c.is_ascii_digit()) && !head.is_empty() {
            stem = head.to_string();
        }
    }
    let mut out = String::new();
    let chars: Vec<char> = stem.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            break;
        }
        // `v` right before a version number belongs to the version.
        if c == 'v' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()) && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            break;
        }
        out.push(c);
    }
    out.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// The file to install from a mod's files: the current downloads only,
/// preferring the same file family as `like` (the file installed before),
/// then the best fit for this PC build, then the newest.
pub fn pick(files: &[GbFile], like: &str) -> Option<usize> {
    let fam = if like.is_empty() { String::new() } else { family(like) };
    let same = |f: &GbFile| !fam.is_empty() && family(&f.name) == fam;
    let family_left = files.iter().any(|f| f.playable() && same(f));
    files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.playable() && (!family_left || same(f)))
        .max_by_key(|(_, f)| (f.fit().signum(), f.date))
        .map(|(i, _)| i)
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    fetch_within(url, 600)
}

fn fetch_within(url: &str, secs: u32) -> Result<Vec<u8>, String> {
    let max = secs.to_string();
    let out = hidden(Command::new(system_tool("curl.exe")).args(["-sSfL", "--proto-redir", "=https", "--connect-timeout", "15", "--max-time", &max, "-A", "EmblemWeave", url]))
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn json(url: &str) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&fetch_within(url, 45)?).map_err(|e| format!("GameBanana answered with something unexpected: {e}"))
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
    if v.get("_sName").is_none() {
        return Err(format!("mod {id} was not found (it may have been removed or made private)"));
    }
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
                    id: f.get("_idRow").and_then(|x| x.as_u64()).unwrap_or(0),
                    name: s(f, "_sFile"),
                    size: f.get("_nFilesize").and_then(|x| x.as_u64()).unwrap_or(0),
                    url: s(f, "_sDownloadUrl"),
                    description: s(f, "_sDescription"),
                    version: s(f, "_sVersion"),
                    date: f.get("_tsDateAdded").and_then(|x| x.as_u64()).unwrap_or(0),
                    archived: f.get("_bIsArchived").and_then(|x| x.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((m, files))
}

/// Download one of a mod's files and unpack it, ready for `finish`.
pub fn gb_prepare(m: &GbMod, file: &GbFile) -> Result<Prepared, String> {
    if !file.url.starts_with("https://gamebanana.com/") {
        return Err("that download is not on gamebanana.com".into());
    }
    let stage = staging()?;
    let archive = stage.join(clean_name(&file.name));
    let r = fetch(&file.url).and_then(|bytes| {
        std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let unpacked = stage.join("unpacked");
        unpack(&archive, &unpacked)?;
        let _ = std::fs::remove_file(&archive);
        let info = Info {
            title: m.name.clone(),
            source: m.url.clone(),
            author: m.author.clone(),
            version: if file.version.is_empty() { m.version.clone() } else { file.version.clone() },
            gamebanana_id: Some(m.id),
            file_name: file.name.clone(),
            ..Info::default()
        };
        prepare_dir(&unpacked, Some(stage.clone()), info)
    });
    match r {
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            if file.source_only() {
                Err(format!("{e} ({} is the mod's source code, not the mod)", file.name))
            } else {
                Err(e)
            }
        }
        ok => ok,
    }
}

/// Download one of a mod's files and install every variant in it.
#[cfg(test)]
pub fn gb_install(m: &GbMod, file: &GbFile, mods: &Path) -> Result<Installed, String> {
    let p = gb_prepare(m, file)?;
    let all: Vec<usize> = (0..p.choices.len()).collect();
    let r = finish(&p, &all, mods);
    p.discard();
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
        write(&c.join("60 FPS/exefs/extra/nested.ips"));
        let patch = install_from(&c, &mods, &Info { title: "fps".into(), ..Info::default() }).unwrap();
        assert!(patch.patch_only && patch.needs_build && !patch.exefs_only, "{patch:?}");
        assert!(mods.join("fps/exefs/fps.pchtxt").is_file());
        assert!(mods.join("fps/exefs/extra/nested.ips").is_file(), "a patch in an exefs subfolder is installed too");
        // platform builds: the second one off
        let d = root.join("d");
        write(&d.join("PC/romfs/Msg/a"));
        write(&d.join("Switch/romfs/Msg/a"));
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

    /// Builds of a mod for different platforms exclude each other: of several
    /// picked only the recommended one is switched on, or the first picked
    /// when the recommended one is not among them. Optional parts of one mod
    /// are all switched on.
    #[test]
    fn platform_builds_switch_on_one_optional_parts_all() {
        for l in ["Switch", "Emulators", "Obelisk", "PC", "Switch (LayeredFS)", "Obelisk Chamber (PC recomp)", "Yuzu / Ryujinx", "Windows"] {
            assert!(platform_label(l), "{l}");
        }
        for l in ["main", "Light", "Dark theme", "Extra portraits", "Optional voice pack"] {
            assert!(!platform_label(l), "{l}");
        }
        let root = temp("parts");
        let mods = root.join("mods");
        let gift = root.join("gift");
        write(&gift.join("Emulators/romfs/Msg/a"));
        write(&gift.join("Obelisk Chamber (PC recomp)/romfs/Msg/a"));
        write(&gift.join("Switch/romfs/Msg/a"));
        let p = prepare_dir(&gift, None, Info { title: "Gift".into(), ..Info::default() }).unwrap();
        assert!(p.platform_builds());
        let at = |l: &str| p.choices.iter().position(|c| c.label == l).unwrap();
        let (emu, obelisk, switch) = (at("Emulators"), at("Obelisk Chamber (PC recomp)"), at("Switch"));
        assert_eq!(p.recommended(), obelisk);
        let mut all = vec![emu, obelisk, switch];
        all.sort();
        let r = finish(&p, &all, &mods).unwrap();
        assert!(!r.parts);
        assert_eq!(r.mods[0], "Gift - Obelisk Chamber (PC recomp)", "the one switched on is named first: {r:?}");
        let on: Vec<String> = list(&mods).into_iter().filter(|m| m.enabled).map(|m| m.name).collect();
        assert_eq!(on, vec!["Gift - Obelisk Chamber (PC recomp)".to_string()], "platform builds: the recommended one on, the rest off");
        assert_eq!(list(&mods).len(), 3);
        // The recommended one not picked: the first picked is on.
        let mods2 = root.join("mods2");
        let mut two = vec![switch, emu];
        two.sort();
        let r = finish(&p, &two, &mods2).unwrap();
        let first = format!("Gift - {}", p.choices[two[0]].label);
        assert_eq!(r.mods[0], first);
        let on: Vec<String> = list(&mods2).into_iter().filter(|m| m.enabled).map(|m| m.name).collect();
        assert_eq!(on, vec![first], "without the recommended one, the first picked on");
        let addon = root.join("addon");
        write(&addon.join("Main/romfs/Msg/a"));
        write(&addon.join("Extra portraits/romfs/Character/b"));
        let p = prepare_dir(&addon, None, Info { title: "Addon".into(), ..Info::default() }).unwrap();
        assert!(!p.platform_builds());
        let r = finish(&p, &[0, 1], &mods).unwrap();
        assert!(r.parts);
        let on: Vec<String> = list(&mods).into_iter().filter(|m| m.enabled && m.name.starts_with("Addon")).map(|m| m.name).collect();
        assert_eq!(on.len(), 2, "optional parts are all switched on: {on:?}");
        // One part alone is not reported as parts.
        let r = finish(&p, &[1], &mods).unwrap();
        assert!(!r.parts);
        // One variant alone naming a platform, with files of its own: still a part.
        let pc = root.join("pc");
        write(&pc.join("Main/romfs/Msg/a"));
        write(&pc.join("PC portraits/romfs/Character/b"));
        let p = prepare_dir(&pc, None, Info { title: "Portraits".into(), ..Info::default() }).unwrap();
        assert!(!p.platform_builds());
        assert!(!p.exclusive(&[0, 1]));
        let mods3 = root.join("mods3");
        let r = finish(&p, &[0, 1], &mods3).unwrap();
        assert!(r.parts);
        assert_eq!(list(&mods3).iter().filter(|m| m.enabled).count(), 2, "disjoint files: both on");
        // Variants that replace the same files exclude each other: one on.
        let theme = root.join("theme");
        write(&theme.join("Light/romfs/Msg/a"));
        write(&theme.join("Dark/romfs/Msg/a"));
        let p = prepare_dir(&theme, None, Info { title: "Theme".into(), ..Info::default() }).unwrap();
        assert!(!p.platform_builds());
        assert!(p.exclusive(&[0, 1]));
        let mods4 = root.join("mods4");
        let r = finish(&p, &[1, 0], &mods4).unwrap();
        assert!(!r.parts);
        let want = format!("Theme - {}", p.choices[p.recommended()].label);
        assert_eq!(r.mods[0], want);
        let on: Vec<String> = list(&mods4).into_iter().filter(|m| m.enabled).map(|m| m.name).collect();
        assert_eq!(on, vec![want], "the same files: the recommended one on, the other off");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Several files picked from one page each go into their own folder:
    /// switched on when their families differ, the second of one family off
    /// (a second copy). The variants of one install share its page and file
    /// and never switch each other off.
    #[test]
    fn several_files_of_one_page_and_their_variants() {
        let root = temp("twofiles");
        let mods = root.join("mods");
        let fetch = |i: usize, dll: &str, file: &str| {
            let d = root.join(format!("dl{i}"));
            write(&d.join("TurnControl/exefs").join(dll));
            let info = Info { title: "Turn Control".into(), gamebanana_id: Some(742827), file_name: file.into(), ..Info::default() };
            prepare_dir(&d, None, info).unwrap()
        };
        let r = finish(&fetch(0, "turn_control.dll", "turncontrol-030-emblemweave.zip"), &[0], &mods).unwrap();
        assert_eq!(r.off_for, None);
        let r = finish(&fetch(1, "speed_up.dll", "speedup-v1.zip"), &[0], &mods).unwrap();
        assert_eq!(r.off_for, None, "another family from the same page comes in on: {r:?}");
        let r = finish(&fetch(2, "turn_control_emu.dll", "turncontrol-030-emulator.zip"), &[0], &mods).unwrap();
        assert_eq!(r.off_for.as_deref(), Some("Turn Control"), "the same family again is a second copy: {r:?}");
        let mut l: Vec<(String, bool)> = list(&mods).into_iter().map(|m| (m.name, m.enabled)).collect();
        l.sort();
        assert_eq!(l, vec![("Turn Control".to_string(), true), ("Turn Control (2)".to_string(), true), ("Turn Control (3)".to_string(), false)]);
        // Optional parts of one file, all picked: all on, though they share the page and the file.
        let parts = root.join("parts");
        write(&parts.join("Main/romfs/Msg/a"));
        write(&parts.join("Extra portraits/romfs/Character/b"));
        let info = Info { title: "Portraits".into(), gamebanana_id: Some(5), file_name: "portraits-v1.zip".into(), ..Info::default() };
        let p = prepare_dir(&parts, None, info).unwrap();
        let r = finish(&p, &[0, 1], &mods).unwrap();
        assert!(r.parts && r.off_for.is_none(), "{r:?}");
        assert_eq!(list(&mods).iter().filter(|m| m.enabled && m.name.starts_with("Portraits")).count(), 2);
        // Installed again: a second copy, every part off.
        let r = finish(&p, &[0, 1], &mods).unwrap();
        assert_eq!(r.off_for.as_deref().map(|t| t.starts_with("Portraits")), Some(true), "{r:?}");
        assert_eq!(list(&mods).iter().filter(|m| m.enabled && m.name.starts_with("Portraits")).count(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Two mods that ship the same file are reported, and switched-off ones
    /// are not counted.
    #[test]
    fn finds_shared_files() {
        let root = temp("conflicts");
        let mods = root.join("mods");
        write(&mods.join("Alpha/romfs/Graphics/hair.bin"));
        write(&mods.join("Alpha/romfs/Msg/story.txt"));
        write(&mods.join("Beta/romfs/Graphics/Hair.bin"));
        write(&mods.join("_Off/romfs/Graphics/hair.bin"));
        let installed = list(&mods);
        let c = conflicts(&installed);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].file, "graphics/hair.bin");
        let mut who = c[0].mods.clone();
        who.sort();
        assert_eq!(who, vec!["Alpha".to_string(), "Beta".to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A second copy of a mod that is on, by GameBanana id or by a code mod's
    /// DLL name, is installed switched off and says which copy is on; the
    /// Conflicts card lists code mods with the same DLL switched on together.
    #[test]
    fn a_second_copy_of_an_on_mod_comes_in_off() {
        let root = temp("second-copy");
        let mods = root.join("mods");
        write(&mods.join("Turn Control/exefs/turn_control.dll"));
        let info = Info { title: "Turn Control".into(), gamebanana_id: Some(7), ..Info::default() };
        std::fs::write(mods.join("Turn Control/emblemweave.json"), serde_json::to_string(&info).unwrap()).unwrap();
        // The same GameBanana mod, though its DLL has another name.
        let a = root.join("a");
        write(&a.join("TC/exefs/turn_control_v2.dll"));
        let r = install_from(&a, &mods, &info).unwrap();
        assert_eq!(r.off_for.as_deref(), Some("Turn Control"), "{r:?}");
        assert!(mods.join("_Turn Control (2)/exefs/turn_control_v2.dll").is_file());
        // Another copy of the same GameBanana mod is named before a code mod that only shares the DLL.
        write(&mods.join("A Local Build/exefs/turn_control_v2.dll"));
        let r = install_from(&a, &mods, &info).unwrap();
        assert_eq!(r.off_for.as_deref(), Some("Turn Control"), "{r:?}");
        std::fs::remove_dir_all(mods.join("A Local Build")).unwrap();
        std::fs::remove_dir_all(mods.join("_Turn Control (3)")).unwrap();
        // A code mod from elsewhere with the same DLL name.
        let b = root.join("b");
        write(&b.join("Copy/exefs/Turn_Control.dll"));
        let r = install_from(&b, &mods, &Info { title: "Copy".into(), ..Info::default() }).unwrap();
        assert_eq!(r.off_for.as_deref(), Some("Turn Control"));
        assert!(mods.join("_Copy/exefs/Turn_Control.dll").is_file());
        // Anything else still comes in switched on.
        let c = root.join("c");
        write(&c.join("Other/exefs/other.dll"));
        let r = install_from(&c, &mods, &Info { title: "Other".into(), gamebanana_id: Some(8), ..Info::default() }).unwrap();
        assert_eq!(r.off_for, None);
        assert!(mods.join("Other/exefs/other.dll").is_file());
        // Nothing clashes while the copies are off; switched on, they do.
        assert!(code_clashes(&list(&mods)).is_empty());
        let copy = list(&mods).into_iter().find(|m| m.name == "Copy").unwrap();
        set_enabled(&copy, true).unwrap();
        let c = code_clashes(&list(&mods));
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].file, "exefs/turn_control.dll");
        let mut who = c[0].mods.clone();
        who.sort();
        assert_eq!(who, vec!["Copy".to_string(), "Turn Control".to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A page can hold two mods: a download from the page of a mod that is on
    /// only comes in switched off when it is a copy of the same file family,
    /// or ships a DLL of the same name.
    #[test]
    fn a_second_copy_is_told_apart_by_its_file() {
        let root = temp("second-copy-family");
        let mods = root.join("mods");
        write(&mods.join("Turn Control/exefs/turn_control.dll"));
        let on = Info { title: "Turn Control".into(), gamebanana_id: Some(7), file_name: "turncontrol-v100.zip".into(), ..Info::default() };
        std::fs::write(mods.join("Turn Control/emblemweave.json"), serde_json::to_string(&on).unwrap()).unwrap();
        // Same page, another family: a mod of its own, switched on.
        let a = root.join("a");
        write(&a.join("Speed/exefs/speed.dll"));
        let info = Info { title: "Speed".into(), gamebanana_id: Some(7), file_name: "speedup-v1.zip".into(), ..Info::default() };
        let r = install_from(&a, &mods, &info).unwrap();
        assert_eq!(r.off_for, None, "{r:?}");
        assert!(mods.join("Speed/exefs/speed.dll").is_file());
        // Same page, same family (a newer upload): a second copy, off.
        let b = root.join("b");
        write(&b.join("TC/exefs/turn_control_v2.dll"));
        let info = Info { title: "Turn Control".into(), gamebanana_id: Some(7), file_name: "turncontrol-v110_6e177.zip".into(), ..Info::default() };
        let r = install_from(&b, &mods, &info).unwrap();
        assert_eq!(r.off_for.as_deref(), Some("Turn Control"), "{r:?}");
        assert!(mods.join("_Turn Control (2)/exefs/turn_control_v2.dll").is_file());
        // Same page, another family, but the same DLL: still a copy, off.
        let c = root.join("c");
        write(&c.join("Fork/exefs/Turn_Control.dll"));
        let info = Info { title: "Fork".into(), gamebanana_id: Some(7), file_name: "tcfork-v1.zip".into(), ..Info::default() };
        let r = install_from(&c, &mods, &info).unwrap();
        assert_eq!(r.off_for.as_deref(), Some("Turn Control"), "{r:?}");
        assert!(mods.join("_Fork/exefs/Turn_Control.dll").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A profile is a whole set: applying it leaves exactly its mods on.
    #[test]
    fn a_profile_switches_the_set_on() {
        let root = temp("profile");
        let mods = root.join("mods");
        write(&mods.join("Alpha/romfs/a.bin"));
        write(&mods.join("_Beta/romfs/b.bin"));
        write(&mods.join("Gamma/romfs/c.bin"));
        let installed = list(&mods);
        let changed = apply_profile(&installed, &["Beta".to_string()]).unwrap();
        assert_eq!(changed, 3, "Alpha and Gamma off, Beta on");
        let after = list(&mods);
        let on = |n: &str| after.iter().find(|m| m.name == n).unwrap().enabled;
        assert!(!on("Alpha"));
        assert!(on("Beta"));
        assert!(!on("Gamma"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A download with one folder per platform offers each as a choice,
    /// named by the folder that differs, with the PC build recommended.
    #[test]
    fn a_download_with_platform_folders_offers_choices() {
        let root = temp("choices");
        let d = root.join("gift");
        write(&d.join("README.txt"));
        write(&d.join("Obelisk Chamber (PC recomp)/mods/GiftGuide/romfs/Msg/EUen/Item.msbt.zst"));
        write(&d.join("Emulators/GiftGuide/romfs/Msg/EUen/Item.msbt.zst"));
        write(&d.join("Switch (LayeredFS)/atmosphere/contents/04007EF01F8AE000/romfs/Msg/EUen/Item.msbt.zst"));
        let p = prepare(&d, Info { title: "Gift Guide".into(), ..Info::default() }).unwrap();
        let mut labels: Vec<&str> = p.choices.iter().map(|c| c.label.as_str()).collect();
        labels.sort();
        assert_eq!(labels, vec!["Emulators", "Obelisk Chamber (PC recomp)", "Switch (LayeredFS)"]);
        assert_eq!(p.choices[p.recommended()].label, "Obelisk Chamber (PC recomp)");
        // Only the picked one is installed, and it is on.
        let mods = root.join("mods");
        let r = finish(&p, &[p.recommended()], &mods).unwrap();
        assert_eq!(r.mods, vec!["Gift Guide - Obelisk Chamber (PC recomp)".to_string()]);
        let l = list(&mods);
        assert_eq!(l.len(), 1);
        assert!(l[0].enabled);
        assert_eq!(l[0].info.variant, "Obelisk Chamber (PC recomp)");
        // A single mod needs no name of its own.
        let one = root.join("one");
        write(&one.join("TurnControl/exefs/turn_control.dll"));
        write(&one.join("TurnControl/src/turn_control.cpp"));
        let p = prepare(&one, Info { title: "Turn Control".into(), ..Info::default() }).unwrap();
        assert_eq!(p.choices.len(), 1);
        assert_eq!(p.choices[0].label, "");
        let r = finish(&p, &[0], &mods).unwrap();
        assert_eq!(r.mods, vec!["Turn Control".to_string()]);
        assert!(r.exefs_only && !r.mods.is_empty(), "a code-only mod is installed");
        let _ = std::fs::remove_dir_all(root);
    }

    fn file(id: u64, name: &str, desc: &str, date: u64, archived: bool) -> GbFile {
        GbFile { id, name: name.into(), description: desc.into(), date, archived, ..GbFile::default() }
    }

    /// Old versions and source code are not offered as the download; the
    /// PC build wins over the emulator one.
    #[test]
    fn picks_the_current_pc_download() {
        let f = vec![
            file(1, "nativeappearance-v300-main-mod.zip", "F8 Appearance Editor MAIN MOD - Download this to play", 300, false),
            file(2, "appearance_editoruniversal_wardrobe.rar", "old version  F8 Appearance Editor MAIN MOD", 200, true),
            file(3, "nativeappearance-v300-source-only.zip", "SOURCE CODE ONLY — Not required to play.", 301, false),
        ];
        assert!(f[2].source_only() && !f[0].source_only());
        assert_eq!(pick(&f, ""), Some(0));
        let t = vec![
            file(10, "turncontrol-010-emblemweave.zip", ".dll mod", 100, true),
            file(11, "turncontrol-010-emulator.zip", "atmosphere for emus", 100, false),
            file(12, "turncontrol-030-menu-update.zip", "for recomps", 90, false),
        ];
        assert_eq!(pick(&t, ""), Some(2));
        assert_eq!(family("pleasenomoreclicking-v200-main-mod.zip"), "pleasenomoreclicking");
        assert_eq!(family("quicksave-100_33510.zip"), "quicksave");
        assert_eq!(family("rnc_fefw_qol_v130.zip"), "rnc_fefw_qol");
        // A page with two mods: the installed one's family decides.
        let two = vec![
            file(20, "pleasenomoreclicking-v210-main-mod.zip", "MAIN MOD", 500, false),
            file(21, "manualstacking-v120-main-mod.zip", "MAIN MOD", 600, false),
        ];
        assert_eq!(pick(&two, "pleasenomoreclicking-v200-main-mod.zip"), Some(0));
        assert_eq!(pick(&two, "manualstacking-v110-main-mod.zip"), Some(1));
        assert_eq!(family("mymod_e61a7.zip"), "mymod");
        assert_eq!(family("mymod_a1b2c.zip"), "mymod");
        // Ordinary words are not platforms.
        assert_eq!(fit("Weapon Switch v2"), 0);
        assert_eq!(fit("NPC portraits"), 0);
        assert_eq!(fit("Dialogue Windows"), 0);
        assert!(fit("Switch (LayeredFS)") < 0 && fit("for recomps") > 0 && fit("atmosphere for emus") < 0);
        assert!(!file(9, "light-source-shadows.zip", "Optional add-on, not required to play", 1, false).source_only());
        assert!(file(9, "mod-src.zip", "", 1, false).source_only());
    }

    /// A bare RomFS folder with an exefs inside is one mod, its code out of romfs.
    #[test]
    fn a_bare_romfs_folder_keeps_its_exefs_together() {
        let root = temp("bare");
        let d = root.join("d");
        write(&d.join("Mod/GameData/Person.bin"));
        write(&d.join("Mod/exefs/x.dll"));
        let p = prepare(&d, Info { title: "Bare".into(), ..Info::default() }).unwrap();
        assert_eq!(p.choices.len(), 1, "{:?}", p.choices);
        assert_eq!(labels(&[vec!["a".into(), "x".into()], vec!["a".into(), "x".into()], vec!["a".into(), "x".into()]]).len(), 3);
        let l = labels(&[vec!["x".into()], vec!["x".into()], vec!["x".into()]]);
        assert!(l[0] != l[1] && l[1] != l[2] && l[0] != l[2], "{l:?}");
        let mods = root.join("mods");
        finish(&p, &[0], &mods).unwrap();
        assert!(mods.join("Bare/exefs/x.dll").is_file());
        assert!(!mods.join("Bare/romfs/exefs").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A failed install leaves nothing behind, and a folder being built is never listed.
    #[test]
    fn a_failed_install_leaves_nothing() {
        let root = temp("atomic");
        let mods = root.join("mods");
        let d = root.join("d");
        write(&d.join("A/romfs/Msg/a.msbt"));
        let p = prepare_dir(&d, None, Info { title: "A".into(), ..Info::default() }).unwrap();
        assert!(finish(&p, &[0, 7], &mods).is_err());
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 0, "no partial or placed folder is left");
        write(&mods.join(".partial-1/romfs/Msg/a.msbt"));
        write(&mods.join("-Off/romfs/Msg/a.msbt"));
        assert_eq!(list(&mods).iter().map(|m| m.name.clone()).collect::<Vec<_>>(), vec!["Off".to_string()]);
        assert_eq!(free_name(&mods, "Off"), "Off (2)", "a '-' switched-off mod holds its name");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A junction inside a mod is never followed: a patch or DLL it points at does not count.
    #[cfg(windows)]
    #[test]
    fn links_inside_a_mod_are_not_followed() {
        let root = temp("links");
        write(&root.join("elsewhere").join("x.ips"));
        write(&root.join("elsewhere").join("inner").join("y.dll"));
        let exefs = root.join("M").join("exefs");
        std::fs::create_dir_all(&exefs).unwrap();
        let link = |at: &Path, to: &Path| {
            let ok = Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(at).arg(to).output().is_ok_and(|o| o.status.success());
            assert!(ok, "mklink /J {}", at.display());
        };
        link(&exefs.join("sub"), &root.join("elsewhere"));
        assert!(!exefs_has_patches(&exefs), "a patch behind a junction is not the mod's");
        // A folder that holds only a junction is not a single RomFS subfolder to descend into.
        let bare = root.join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        write(&root.join("game").join("Graphics").join("a.bin"));
        link(&bare.join("only"), &root.join("game"));
        assert!(analyse(&bare).is_err(), "{:?}", analyse(&bare));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Old stages and `.partial-…` folders go, unless the launcher that made them still runs.
    #[test]
    fn cleaning_leaves_a_running_launchers_folders() {
        let root = temp("clean");
        for d in ["mods-incoming/100-4242-0", "mods-incoming/100-777-1", "mods/.partial-100-4242-0", "mods/.partial-100-777-0", "mods/Keep"] {
            write(&root.join(d).join("x"));
        }
        assert_eq!(stage_pid("100-4242-3"), Some(4242));
        assert_eq!(stage_pid(".partial-100-4242-3"), Some(4242));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3 * 24 * 3600);
        clean_in(&root, later, Some(&[4242, 1]), 1);
        assert!(root.join("mods-incoming/100-4242-0").is_dir(), "a live launcher's waiting download stays");
        assert!(root.join("mods/.partial-100-4242-0").is_dir());
        assert!(!root.join("mods-incoming/100-777-1").exists(), "a closed launcher's old download goes");
        assert!(!root.join("mods/.partial-100-777-0").exists());
        assert!(root.join("mods/Keep").is_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Without a process list every other launcher's folder is kept; unnamed ones and this launcher's still age out.
    #[test]
    fn cleaning_keeps_other_launchers_folders_when_the_process_list_is_unknown() {
        let root = temp("clean-unknown");
        for d in ["mods-incoming/100-4242-0", "mods-incoming/100-1-0", "mods-incoming/old-download", "mods/.partial-100-4242-0", "mods/.partial-100-1-0", "mods/.partial-x", "mods/Keep"] {
            write(&root.join(d).join("x"));
        }
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3 * 24 * 3600);
        clean_in(&root, later, None, 1);
        assert!(root.join("mods-incoming/100-4242-0").is_dir(), "another launcher's download may still be wanted");
        assert!(root.join("mods/.partial-100-4242-0").is_dir());
        assert!(!root.join("mods-incoming/100-1-0").exists(), "this launcher's own old folder follows the age rule");
        assert!(!root.join("mods/.partial-100-1-0").exists());
        assert!(!root.join("mods-incoming/old-download").exists(), "a name without an id follows the age rule");
        assert!(!root.join("mods/.partial-x").exists());
        assert!(root.join("mods/Keep").is_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn folder_names_are_safe_on_windows() {
        assert_eq!(clean_name("CON"), "CON_");
        assert_eq!(clean_name("nul.txt"), "nul_.txt");
        assert_eq!(clean_name("Com3"), "Com3_");
        assert_eq!(clean_name("COM0"), "COM0");
        assert_eq!(clean_name("Console"), "Console");
        let long = format!("{}. x", "a".repeat(79));
        assert_eq!(clean_name(&long), "a".repeat(79));
        assert_eq!(clean_name(" ... "), "mod");
    }

    #[test]
    fn reads_gamebanana_links() {
        assert_eq!(gb_id("https://gamebanana.com/mods/738457"), Some(738457));
        assert_eq!(gb_id("gamebanana.com/mods/737235/"), Some(737235));
        assert_eq!(gb_id("737235"), Some(737235));
        assert_eq!(gb_id("https://example.com"), None);
    }

    /// Reads real mod pages and downloads (network): what the Mods page
    /// would offer for them.
    #[test]
    #[ignore]
    fn live_gamebanana_choices() {
        for id in [743665u64, 742827, 745602, 743199, 743629, 744008, 739111] {
            let (m, files) = gb_mod(id).unwrap();
            let playable: Vec<&str> = files.iter().filter(|f| f.playable()).map(|f| f.name.as_str()).collect();
            let pick = pick(&files, "").map(|i| files[i].name.clone());
            println!("{id} {}: {} files, current {playable:?}, pick {pick:?}", m.name, files.len());
            if let Some(name) = pick {
                let f = files.iter().find(|f| f.name == name).unwrap();
                let root = temp(&format!("live-{id}"));
                std::fs::write(root.join("a.zip"), fetch(&f.url).unwrap()).unwrap();
                unpack(&root.join("a.zip"), &root.join("u")).unwrap();
                let p = prepare_dir(&root.join("u"), None, Info { title: m.name.clone(), ..Info::default() }).unwrap();
                let labels: Vec<String> = p.choices.iter().map(|c| format!("{}[fit {}]", c.label, c.fit)).collect();
                println!("    choices {labels:?} recommended {}", p.recommended());
                let _ = std::fs::remove_dir_all(root);
            }
        }
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
