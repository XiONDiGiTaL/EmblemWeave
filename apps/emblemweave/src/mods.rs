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
    /// The GameBanana file this copy came from, so an update check can tell
    /// a newer upload from the one installed (authors often re-upload under
    /// the same version number).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<u64>,
    /// That file's name as uploaded. A page can hold two mods; their files'
    /// names keep them apart (`family`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_name: String,
    /// When GameBanana received that file (Unix seconds).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub file_date: u64,
    /// When the launcher installed this copy (Unix seconds).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub installed: u64,
    /// The variant picked when the download offered several (one per platform, say).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub variant: String,
    /// The newest upload the user chose to skip (its GameBanana date): no
    /// update is offered until a newer one appears.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skip_update: u64,
    /// Never offer updates for this copy: a personal port or a mod edited
    /// by hand that shares a GameBanana page with the original.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_updates: bool,
    /// The settings-looking files the download shipped (path in lower case →
    /// SHA-256), so an update can tell a default the user never touched from
    /// one they changed. `None` for a copy installed before this was
    /// recorded; an empty map for a download that shipped no settings files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shipped: Option<std::collections::BTreeMap<String, String>>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Write `info` into a mod's `emblemweave.json`, keeping any keys an author
/// or another tool put there that this launcher does not know.
pub fn write_info(folder: &Path, info: &Info) -> Result<(), String> {
    let file = folder.join("emblemweave.json");
    let mut v: serde_json::Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    let ours = serde_json::to_value(info).map_err(|e| e.to_string())?;
    let obj = v.as_object_mut().expect("an object");
    // Fields that are now empty are dropped, so a cleared skip really clears.
    for k in ["file_id", "file_name", "file_date", "installed", "variant", "skip_update", "no_updates", "shipped"] {
        obj.remove(k);
    }
    if let Some(o) = ours.as_object() {
        for (k, x) in o {
            obj.insert(k.clone(), x.clone());
        }
    }
    std::fs::write(&file, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| format!("writing {}: {e}", file.display()))
}

/// A `plugin.json`, as Trinity's plugin layer and the NXP loaders write it.
/// Only the fields the Mods page can state as fact are kept: an unknown key
/// is ignored rather than guessed at.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// Present when the plugin is built against the NXP ABI (Trinity's plugin
    /// layer): `{"min": 1, "max": 4}`.
    pub abi: Option<Abi>,
    pub hosts: Vec<String>,
    pub capabilities: Vec<String>,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Abi {
    pub min: u32,
    pub max: u32,
}

/// What a folder under `mods/` actually holds. Derived from the files present,
/// never from the folder's name or a GameBanana title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// RomFS content the engine lays over the game's files.
    RomFs,
    /// An ExeFS code mod: a DLL (and optionally `.sites`) the runtime loads.
    Code,
    /// `.ips` / `.ips32` / `.pchtxt` patches, applied when the game is built.
    Patch,
    /// More than one of the above in one folder.
    Combined,
    /// A component the launcher itself ships and the runtime needs: Trinity Core.
    Runtime,
    /// Manifests or loose files but no RomFS, code or patch payload.
    MetadataOnly,
    /// No files at all.
    Empty,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::RomFs => "RomFS",
            Kind::Code => "Code mod",
            Kind::Patch => "Patch",
            Kind::Combined => "RomFS + code",
            Kind::Runtime => "Runtime component",
            Kind::MetadataOnly => "Metadata only",
            Kind::Empty => "Empty folder",
        }
    }

    /// Whether the engine can load something from this folder.
    pub fn playable(self) -> bool {
        matches!(self, Kind::RomFs | Kind::Code | Kind::Patch | Kind::Combined | Kind::Runtime)
    }

    /// What a folder holds, from what the scanner found in it. A folder with
    /// only manifests is metadata, not a broken mod: authors ship one beside a
    /// RomFS tree, and an interrupted download can leave one behind.
    pub fn of(romfs: bool, code: bool, patches: bool, manifests: bool) -> Kind {
        match (romfs, code, patches) {
            (true, false, false) => Kind::RomFs,
            (false, true, false) => Kind::Code,
            (false, false, true) => Kind::Patch,
            (true, true, true) | (true, true, false) | (true, false, true) | (false, true, true) => Kind::Combined,
            (false, false, false) if manifests => Kind::MetadataOnly,
            (false, false, false) => Kind::Empty,
        }
    }
}

/// How a code mod talks to the runtime, when its own files say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// The NXP layer: a `plugin.json` with an ABI, Trinity identities or `nxp.*` interfaces.
    Trinity,
    /// The runtime's own NxModApi: the DLL exports `nxmod_init` / `nxmod_early`.
    NxMod,
    /// A DLL whose API nothing in the folder declares.
    Unknown,
}

impl Api {
    pub fn label(self) -> &'static str {
        match self {
            Api::Trinity => "Trinity / NXP",
            Api::NxMod => "NxModApi",
            Api::Unknown => "API not declared",
        }
    }
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
    /// The `plugin.json` found at the folder's top, when it has one.
    pub plugin: Option<Plugin>,
    /// What the folder contains, from its files.
    pub kind: Kind,
    /// How its DLL talks to the runtime, when it has one.
    pub api: Api,
    /// `plugin.json` at the top of the folder (metadata, not necessarily a plugin).
    pub has_plugin_manifest: bool,
    /// `emblemweave.json` at the top of the folder.
    pub has_mod_manifest: bool,
    /// When this copy was installed: the recorded time, else the time its
    /// `emblemweave.json` was written (mods installed before it was recorded).
    pub installed_at: u64,
}

impl Mod {
    /// Trinity Core itself, recognised by the manifest identity the bundle
    /// installs (`org.xion.trinity.core`) or, for an older copy without the id,
    /// by the name it declares. A third-party mod is never matched by its
    /// folder name alone.
    pub fn is_trinity_core(&self) -> bool {
        let Some(p) = &self.plugin else { return false };
        p.id.eq_ignore_ascii_case("org.xion.trinity.core")
            || (p.name.eq_ignore_ascii_case("Trinity Core") && self.has_code)
    }

    /// A code mod that depends on Trinity Core's bridge rather than being it.
    pub fn uses_trinity(&self) -> bool {
        !self.is_trinity_core()
            && self.has_code
            && (self.api == Api::Trinity
                || self
                    .plugin
                    .as_ref()
                    .is_some_and(|p| p.dependencies.iter().any(|d| {
                        let d = d.to_ascii_lowercase();
                        d.contains("trinity") || d == "nxp" || d == "nxp.nxmod"
                    })))
    }

    /// The version to show: the plugin manifest's, else the launcher's record.
    pub fn version(&self) -> &str {
        self.plugin
            .as_ref()
            .map(|p| p.version.as_str())
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.info.version)
    }
}

/// Whether the bytes of a DLL name the runtime's NxModApi entry points or an
/// `nxp.*` interface. PE export names are stored as plain ASCII in the image,
/// so this is evidence from the file itself, not a guess from its name.
pub fn dll_api(bytes: &[u8]) -> (bool, bool) {
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    (
        has(b"nxmod_init") || has(b"nxmod_early"),
        has(b"nxp.") || has(b"trinity_plugin"),
    )
}

fn read_dlls(exefs: &[PathBuf]) -> (bool, bool) {
    const CAP: u64 = 8 * 1024 * 1024;
    let mut nxmod = false;
    let mut nxp = false;
    for dir in exefs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if !kind(&e).is_some_and(|t| t.is_file())
                || !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll"))
            {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&p) else { continue };
            if meta.len() > CAP {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&p) {
                let (n, x) = dll_api(&bytes);
                nxmod |= n;
                nxp |= x;
            }
        }
    }
    (nxmod, nxp)
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
        let has_plugin_manifest = path.join("plugin.json").is_file();
        let has_mod_manifest = path.join("emblemweave.json").is_file();
        let plugin = std::fs::read_to_string(path.join("plugin.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Plugin>(&t).ok())
            .filter(|p| !p.id.is_empty() || !p.name.is_empty() || !p.version.is_empty());
        let (nxmod, nxp) = if has_code { read_dlls(&x) } else { (false, false) };
        let api = if plugin.as_ref().is_some_and(|p| {
            p.abi.is_some()
                || p.id.to_ascii_lowercase().contains("trinity")
                || p.capabilities.iter().any(|c| c.to_ascii_lowercase().starts_with("nxp."))
        }) || nxp
        {
            Api::Trinity
        } else if nxmod {
            Api::NxMod
        } else {
            Api::Unknown
        };
        let mut kind = Kind::of(!r.is_empty(), has_code, has_patches, has_plugin_manifest || has_mod_manifest);
        if plugin.as_ref().is_some_and(|p| {
            p.id.eq_ignore_ascii_case("org.xion.trinity.core")
                || (p.name.eq_ignore_ascii_case("Trinity Core") && has_code)
        }) {
            kind = Kind::Runtime;
        }
        let written = std::fs::metadata(path.join("emblemweave.json"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let installed_at = if info.installed > 0 { info.installed } else { written };
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
            plugin,
            kind,
            api,
            has_plugin_manifest,
            has_mod_manifest,
            installed_at,
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
    move_aside(&m.path, &bin, &m.name)
}

/// Move `folder` into `bin` as `name`, or `name (2)`, `name (3)`… when taken.
fn move_aside(folder: &Path, bin: &Path, name: &str) -> Result<PathBuf, String> {
    move_aside_io(folder, bin, name).map_err(|e| format!("moving {}: {e}", folder.display()))
}

fn move_aside_io(folder: &Path, bin: &Path, name: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(bin)?;
    let mut to = bin.join(name);
    let mut n = 1;
    while to.exists() {
        n += 1;
        to = bin.join(format!("{name} ({n})"));
    }
    rename_retry(folder, &to)?;
    Ok(to)
}

/// Another process may have a file in the folder open: Windows' sharing
/// violation (32), or access denied (5), which a rename of a folder whose
/// files are open also gives.
fn in_use(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32) | Some(5))
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
/// without the `_` a switched-off one carries). One that cannot be renamed
/// does not stop the rest. Returns how many changed and what failed.
pub fn apply_profile(list: &[Mod], want: &[String]) -> (usize, Vec<String>) {
    let mut changed = 0;
    let mut failed = Vec::new();
    for m in list {
        // The bundled runtime component is not the player's to switch: a profile
        // that does not name it (every profile saved before it shipped) would
        // otherwise switch Trinity Core off and break the mods that need it.
        if m.kind == Kind::Runtime {
            continue;
        }
        let on = want.iter().any(|w| w.eq_ignore_ascii_case(&m.name));
        if on != m.enabled {
            match set_enabled(m, on) {
                Ok(_) => changed += 1,
                Err(e) => failed.push(e),
            }
        }
    }
    (changed, failed)
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
    /// An update: where the copy it replaced was moved, and how many of
    /// that copy's settings files were carried over.
    pub replaced: Option<(PathBuf, usize)>,
    /// `.ini` settings files kept as the user had them, with the sections
    /// and keys the new version added merged in with their defaults: the
    /// file (path inside the mod) and what was added (`Section/key`).
    pub merged: Vec<(String, Vec<String>)>,
    /// Other settings files kept as the user had them where the new version
    /// ships its own: its default is beside the user's as `<name>.new`.
    pub new_beside: Vec<String>,
    /// Settings files the old download shipped and the user changed that
    /// the new version no longer ships (renamed or dropped): the user's
    /// copy was kept in the mod folder, where the mod may not read it.
    pub dropped: Vec<String>,
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

    /// The choice an update should take: the variant installed before, or
    /// the only one. A copy installed before variants were recorded is
    /// matched by its name, which carried the variant's folder. `None` means
    /// the user has to choose.
    pub fn same_variant(&self, info: &Info) -> Option<usize> {
        if self.choices.len() == 1 {
            return Some(0);
        }
        if !info.variant.is_empty() {
            return self.choices.iter().position(|c| c.label.eq_ignore_ascii_case(&info.variant));
        }
        // Installed by this launcher with no variant: the download had one, now several. An
        // install time alone does not say so: Skip records one on a copy from an older launcher.
        if info.installed > 0 && (info.file_id.is_some() || info.shipped.is_some()) {
            return None;
        }
        let t = info.title.to_lowercase();
        let hits: Vec<usize> = (0..self.choices.len()).filter(|&i| !self.choices[i].label.is_empty() && t.contains(&self.choices[i].label.to_lowercase())).collect();
        if hits.len() == 1 { Some(hits[0]) } else { None }
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

/// Installs and updates run on worker threads; this keeps two of them from
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
        return Ok(Installed { mods: Vec::new(), exefs_only: true, patch_only: false, needs_build: false, parts: false, replaced: None, merged: Vec::new(), new_beside: Vec::new(), dropped: Vec::new(), off_for: None });
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
        info.installed = now();
        info.shipped = Some(shipped_settings(&partial));
        let json = partial.join("emblemweave.json");
        std::fs::write(&json, serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?).map_err(|e| format!("writing {}: {e}", json.display()))?;
        rename_retry(&partial, &folder).map_err(|e| format!("moving {} into place: {e}", folder.display()))?;
        partials.retain(|x| x != &partial);
        placed.push(folder);
        names.push(name);
    }
    Ok(Installed { mods: names, exefs_only: p.exefs_only, patch_only: p.patch_only, needs_build, parts, replaced: None, merged: Vec::new(), new_beside: Vec::new(), dropped: Vec::new(), off_for })
}

/// Files a mod keeps its settings in. An update carries them over from the
/// copy it replaces, so the user's choices survive. Code, game data and
/// manifests never count, whatever they are called.
fn is_settings(rel: &str) -> bool {
    let l = rel.to_lowercase();
    let name = l.rsplit('/').next().unwrap_or(&l);
    // A mod's description of itself, in whatever format: the new version's is the right one. A
    // `.new` / `.old` copy beside a settings file is not the one the mod reads.
    if name.ends_with(".new") || name.ends_with(".old") {
        return false;
    }
    if matches!(name, "emblemweave.json" | "mod.toml") || name.starts_with("plugin.") || name.starts_with("manifest.") {
        return false;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if matches!(ext, "dll" | "sites" | "ips" | "ips32" | "pchtxt" | "nro" | "nso" | "exe" | "pdb" | "bin" | "zst" | "msbt" | "bfres" | "zip" | "md" | "lua" | "py" | "js" | "dat" | "pak" | "so" | "wasm" | "cpp" | "h") {
        return false;
    }
    let named = name.contains("settings") || name.contains("config") || name.contains("favorite");
    match ext {
        "ini" | "cfg" | "toml" | "conf" | "settings" => true,
        "json" | "txt" | "yaml" | "yml" | "tsv" | "csv" => named,
        // `native_appearance.settings.favorite-characters` and the like.
        _ => named && (ext.contains('-') || ext.len() > 5),
    }
}

/// A mod's settings files, for the Mods page's Settings button: the files an
/// update carries over, sorted, each followed by the new version's default
/// an update put beside it (`<file>.new`), if there is one. Never an `.old`
/// copy, a `.new` one of no settings file, or a manifest.
pub fn settings_of(folder: &Path) -> Vec<String> {
    let mut files = settings_files(folder);
    files.sort_by_key(|f| f.to_lowercase());
    files
        .into_iter()
        .flat_map(|f| {
            let new = format!("{f}.new");
            let beside = folder.join(&new).is_file().then_some(new);
            std::iter::once(f).chain(beside)
        })
        .collect()
}

/// The settings files a mod folder could carry over: settings-looking, and
/// not game data (`romfs/`) or source code (`src/`). Paths as written.
fn settings_files(folder: &Path) -> Vec<String> {
    let mut files = Vec::new();
    // Game data and source code are never walked: a RomFS tree can hold thousands of files.
    let Ok(entries) = std::fs::read_dir(folder) else { return files };
    for e in entries.flatten() {
        let Some(t) = kind(&e) else { continue };
        let name = e.file_name().to_string_lossy().to_lowercase();
        if t.is_dir() && name != "romfs" && name != "src" {
            walk_rel_cased(&e.path(), folder, &mut files);
        } else if t.is_file() {
            files.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    files
        .into_iter()
        .filter(|rel| {
            let lower = rel.to_lowercase();
            !lower.starts_with("romfs/") && !lower.starts_with("src/") && is_settings(rel)
        })
        .collect()
}

fn sha256_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| sha256_bytes(&b))
}

fn sha256_bytes(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

/// What a freshly built copy shipped of its settings files, for `Info::shipped`.
fn shipped_settings(folder: &Path) -> std::collections::BTreeMap<String, String> {
    settings_files(folder).into_iter().filter_map(|rel| Some((rel.to_lowercase(), sha256_file(&folder.join(&rel))?))).collect()
}

/// Carry the old copy's settings into the new one. `shipped` is what the
/// old copy's download shipped (`Info::shipped`): a file is carried when the
/// download did not ship it (the mod wrote it at run time) or the user
/// changed it; an untouched default gives way to the new version's. With no
/// record (a copy installed before they were kept) every settings file is
/// carried. A carried file the new version ships with other contents is
/// never just copied over it, or its new options would be lost: an `.ini` is
/// merged (`merge_ini`), any other file stays the user's with the new
/// default beside it as `<name>.new`. A file the old download shipped, the
/// user changed and the new one no longer ships (renamed or dropped) is
/// carried too, but listed apart: the new version may not read it.
fn carry_settings(old: &Path, new: &Path, shipped: Option<&std::collections::BTreeMap<String, String>>) -> Result<Carried, String> {
    let mut c = Carried::default();
    for rel in settings_files(old) {
        let mine = std::fs::read(old.join(&rel)).map_err(|e| format!("keeping {rel}: {e}"))?;
        let to = new.join(&rel);
        let theirs = std::fs::read(&to).ok();
        if theirs.as_ref() == Some(&mine) {
            continue;
        }
        let was = shipped.and_then(|s| s.get(&rel.to_lowercase()));
        if was.is_some_and(|h| sha256_bytes(&mine) == *h) {
            continue;
        }
        let write = |to: &Path, bytes: &[u8]| -> Result<(), String> {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(to, bytes).map_err(|e| format!("keeping {rel}: {e}"))
        };
        let Some(theirs) = theirs else {
            if was.is_some() {
                c.dropped.push(rel.clone());
            } else {
                c.kept += 1;
            }
            write(&to, &mine)?;
            continue;
        };
        c.kept += 1;
        let ini = rel.to_lowercase().ends_with(".ini");
        match ini.then(|| merge_ini(&mine, &theirs)).flatten() {
            Some((text, added)) => {
                write(&to, &text)?;
                if !added.is_empty() {
                    c.merged.push((rel.clone(), added));
                }
            }
            None => {
                write(&new.join(format!("{rel}.new")), &theirs)?;
                write(&to, &mine)?;
                c.new_beside.push(rel.clone());
            }
        }
    }
    Ok(c)
}

/// What `carry_settings` carried over.
#[derive(Debug, Default)]
struct Carried {
    kept: usize,
    merged: Vec<(String, Vec<String>)>,
    new_beside: Vec<String>,
    dropped: Vec<String>,
}

/// One line of an `.ini`, as `merge_ini` reads it.
#[derive(Debug, PartialEq, Eq)]
enum IniLine {
    /// `[Name]`: the name as written (never empty).
    Section(String),
    /// `key = value`: the key as written.
    Key(String),
    /// `;` or `#`.
    Comment,
    Blank,
    /// Anything else, kept as it is.
    Other,
}

fn ini_line(line: &str) -> IniLine {
    let t = line.trim_start_matches('\u{feff}').trim();
    if t.is_empty() {
        IniLine::Blank
    } else if t.starts_with(';') || t.starts_with('#') {
        IniLine::Comment
    } else if let Some(name) = t.strip_prefix('[').and_then(|r| r.find(']').map(|i| r[..i].trim())) {
        // `[]` names no section: a malformed line, kept as it is.
        if name.is_empty() {
            IniLine::Other
        } else {
            IniLine::Section(name.to_string())
        }
    } else if let Some((k, _)) = t.split_once('=').filter(|(k, _)| !k.trim().is_empty()) {
        IniLine::Key(k.trim().to_string())
    } else {
        IniLine::Other
    }
}

/// A section of the new version's `.ini`: its name as written ("" before the
/// first header), the comment lines above its header and the header, and the
/// lines after the header.
struct IniSection {
    name: String,
    head: Vec<String>,
    body: Vec<String>,
}

/// Merge the new version's `.ini` into the user's. The user's file stays as
/// it is (values, comments, order, keys they added or the new version
/// removed, each line's own line ending); every key only the new file has is
/// added with its default at the end of its section, with the comment lines
/// just above it in the new file, and every section only the new file has
/// goes at the end of the file with the comment lines just above its header.
/// Names match in any case (Unicode lower case), keys before the first
/// header form a section of their own, and `[]` is no header. Added lines
/// take the line ending most of the user's file uses; the user's byte order
/// mark stays at the start, the new file's is dropped. Returns the merged
/// file and what was added (`Section/key`, or `Section` for a section with
/// no keys), or `None` when either file is not UTF-8 text or ends a line
/// with a carriage return alone.
fn merge_ini(mine: &[u8], theirs: &[u8]) -> Option<(Vec<u8>, Vec<String>)> {
    let utf16 = |b: &[u8]| b.starts_with(&[0xff, 0xfe]) || b.starts_with(&[0xfe, 0xff]);
    if utf16(mine) || utf16(theirs) {
        return None;
    }
    let original = mine;
    let (mine, theirs) = (std::str::from_utf8(mine).ok()?, std::str::from_utf8(theirs).ok()?);
    let lone_cr = |t: &str| t.match_indices('\r').any(|(i, _)| !t[i + 1..].starts_with('\n'));
    if lone_cr(mine) || lone_cr(theirs) {
        return None;
    }
    let bom = mine.starts_with('\u{feff}');
    let mine = mine.strip_prefix('\u{feff}').unwrap_or(mine);
    let theirs = theirs.strip_prefix('\u{feff}').unwrap_or(theirs);
    // The user's lines, each with its own line ending ("" for a last line without one).
    let lines: Vec<(&str, &str)> = mine
        .split_inclusive('\n')
        .map(|l| match l.strip_suffix("\r\n") {
            Some(t) => (t, "\r\n"),
            None => l.strip_suffix('\n').map_or((l, ""), |t| (t, "\n")),
        })
        .collect();
    let crlf = lines.iter().filter(|(_, e)| *e == "\r\n").count();
    let eol = if crlf * 2 > lines.iter().filter(|(_, e)| !e.is_empty()).count() { "\r\n" } else { "\n" };

    // The user's sections, by lower-case name: their keys, and the line new keys go after.
    let mut keys: std::collections::HashMap<String, std::collections::HashSet<String>> = Default::default();
    let mut after: std::collections::HashMap<String, usize> = Default::default();
    let mut comments: std::collections::HashSet<&str> = Default::default();
    let mut first_header = None;
    let mut current = String::new();
    keys.insert(String::new(), Default::default());
    for (i, (l, _)) in lines.iter().enumerate() {
        match ini_line(l) {
            IniLine::Section(name) => {
                first_header.get_or_insert(i);
                current = name.to_lowercase();
                keys.entry(current.clone()).or_default();
                after.insert(current.clone(), i);
            }
            IniLine::Key(k) => {
                keys.entry(current.clone()).or_default().insert(k.to_lowercase());
                after.insert(current.clone(), i);
            }
            IniLine::Comment => {
                comments.insert(l.trim());
            }
            _ => {}
        }
    }
    // Keys before the first header with none in the user's file go above the comments over that header.
    let top = {
        let mut at = first_header.unwrap_or(lines.len());
        while at > 0 && ini_line(lines[at - 1].0) == IniLine::Comment {
            at -= 1;
        }
        at
    };

    // The new file, section by section; a name that comes back joins its first section.
    let mut sections: Vec<IniSection> = vec![IniSection { name: String::new(), head: Vec::new(), body: Vec::new() }];
    for l in theirs.lines() {
        let l = l.to_string();
        match ini_line(&l) {
            IniLine::Section(name) => {
                let last = sections.last_mut().expect("the section before the first header");
                let mut head = Vec::new();
                while last.body.last().is_some_and(|b| ini_line(b) == IniLine::Comment) {
                    head.insert(0, last.body.pop().expect("a comment line"));
                }
                head.push(l);
                sections.push(IniSection { name, head, body: Vec::new() });
            }
            _ => sections.last_mut().expect("a section").body.push(l),
        }
    }
    let mut merged: Vec<IniSection> = Vec::new();
    for s in sections {
        match merged.iter_mut().find(|m| m.name.to_lowercase() == s.name.to_lowercase()) {
            Some(m) => m.body.extend(s.body),
            None => merged.push(s),
        }
    }

    let mut added = Vec::new();
    // Lines to add before user line `i` (`lines.len()`: at the end of the file).
    let mut inserts: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
    let mut tail: Vec<String> = Vec::new();
    let label = |section: &str, key: &str| if section.is_empty() { key.to_string() } else { format!("{section}/{key}") };
    for s in merged {
        let lower = s.name.to_lowercase();
        if let Some(have) = keys.get_mut(&lower) {
            for (j, l) in s.body.iter().enumerate() {
                if let IniLine::Key(k) = ini_line(l) {
                    if have.insert(k.to_lowercase()) {
                        added.push(label(&s.name, &k));
                        let at = after.get(&lower).map(|a| a + 1).unwrap_or(top);
                        let to = inserts.entry(at).or_default();
                        // The comment lines right above the key describe it: they come along,
                        // unless the user's file has them all already (they deleted only the key).
                        let mut from = j;
                        while from > 0 && ini_line(&s.body[from - 1]) == IniLine::Comment {
                            from -= 1;
                        }
                        if !s.body[from..j].iter().all(|c| comments.contains(c.trim())) {
                            to.extend(s.body[from..j].iter().cloned());
                        }
                        to.push(l.clone());
                    }
                }
            }
            continue;
        }
        // A section the user's file does not have: as the new file writes it, each key once.
        let mut seen = std::collections::HashSet::new();
        let mut body: Vec<String> = s
            .body
            .into_iter()
            .filter(|l| match ini_line(l) {
                IniLine::Key(k) => seen.insert(k.to_lowercase()),
                _ => true,
            })
            .collect();
        while body.last().is_some_and(|l| ini_line(l) == IniLine::Blank) {
            body.pop();
        }
        let named: Vec<String> = body.iter().filter_map(|l| if let IniLine::Key(k) = ini_line(l) { Some(label(&s.name, &k)) } else { None }).collect();
        if named.is_empty() {
            added.push(s.name.clone());
        }
        added.extend(named);
        let last = tail.last().map(String::as_str).or(lines.last().map(|l| l.0));
        if last.is_some_and(|l| ini_line(l) != IniLine::Blank) {
            tail.push(String::new());
        }
        tail.extend(s.head);
        tail.extend(body);
    }
    if added.is_empty() {
        return Some((original.to_vec(), added));
    }
    let mut out: Vec<(String, &str)> = Vec::with_capacity(lines.len() + tail.len() + added.len());
    for (i, (l, e)) in lines.iter().enumerate() {
        if let Some(v) = inserts.remove(&i) {
            out.extend(v.into_iter().map(|x| (x, eol)));
        }
        out.push((l.to_string(), e));
    }
    if let Some(v) = inserts.remove(&lines.len()) {
        out.extend(v.into_iter().map(|x| (x, eol)));
    }
    out.extend(tail.into_iter().map(|x| (x, eol)));
    // A last line without a line ending gets one once lines follow it; the file still ends as the user's did.
    let open_end = !mine.is_empty() && !mine.ends_with('\n');
    let mut text = String::from(if bom { "\u{feff}" } else { "" });
    let n = out.len();
    for (i, (l, e)) in out.into_iter().enumerate() {
        text.push_str(&l);
        if i + 1 < n || !open_end {
            text.push_str(if e.is_empty() { eol } else { e });
        }
    }
    Some((text.into_bytes(), added))
}

/// The game is running: its mods' DLLs are locked and their files in use.
/// Tests never ask the real process list.
fn game_running() -> bool {
    !cfg!(test) && crate::platform::game_running()
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

fn walk_rel_cased(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let Some(t) = kind(&e) else { continue };
        if t.is_dir() {
            walk_rel_cased(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Replace an installed mod with choice `pick` of a newer download. The
/// folder keeps its name (and so its place in the load order and whether it
/// is on), the mod's settings files are carried over, and the old copy is
/// moved to `mods-removed` rather than deleted.
pub fn finish_update(p: &Prepared, pick: usize, old: &Mod) -> Result<Installed, String> {
    let bin = removed_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    finish_update_into(p, pick, old, &bin)
}

fn finish_update_into(p: &Prepared, pick: usize, old: &Mod, bin: &Path) -> Result<Installed, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if p.unusable_exefs {
        return Err("the new download has nothing this engine can use".into());
    }
    let c = p.choices.get(pick).ok_or("that choice is not in the download")?;
    let mods = old.path.parent().ok_or("no parent folder")?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Built in the mods folder beside the copy it replaces, so the final rename stays in one folder;
    // the engine and `list` skip dot folders.
    let fresh = mods.join(format!(".partial-update-{stamp}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&fresh);
    let mut keep = false;
    let r = place_update(p, c, old, bin, &fresh, &mut keep);
    if r.is_err() && !keep {
        let _ = std::fs::remove_dir_all(&fresh);
    }
    r
}

/// Where the installed copy is now: it may have been switched on or off since the update started.
fn current_path(old: &Mod) -> Result<PathBuf, String> {
    [old.name.clone(), format!("_{}", old.name), format!("-{}", old.name)]
        .iter()
        .map(|n| old.path.with_file_name(n))
        .find(|p| p.is_dir())
        .ok_or_else(|| format!("{} is no longer in the mods folder", old.name))
}

/// The installed copy is still at `was`, where its settings and
/// emblemweave.json were read from: switched on or off in the meantime, the
/// new copy would carry what it read from a folder that has moved.
fn still_at(old: &Mod, was: &Path) -> Result<PathBuf, String> {
    let now = current_path(old)?;
    if now != was {
        return Err(format!("{} was switched on or off during the update; try again", old.name));
    }
    Ok(now)
}

#[cfg(test)]
thread_local! {
    /// Run by `place_update` just before it takes WRITING, so a test can change the mods folder there.
    static BEFORE_LOCK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}

fn place_update(p: &Prepared, c: &Choice, old: &Mod, bin: &Path, fresh: &Path, keep: &mut bool) -> Result<Installed, String> {
    let needs_build = build_folder(c, fresh)?;
    // Recorded before the old settings come over: what this download itself ships.
    let shipped = shipped_settings(fresh);
    let path = current_path(old)?;
    let carried = carry_settings(&path, fresh, old.info.shipped.as_ref())?;
    // Keys an author or the user put in emblemweave.json survive.
    let _ = std::fs::copy(path.join("emblemweave.json"), fresh.join("emblemweave.json"));
    let mut info = p.info.clone();
    // The installed name stays: profiles and the load order refer to it.
    info.title = old.info.title.clone();
    info.variant = c.label.clone();
    info.installed = now();
    info.skip_update = 0;
    info.no_updates = false;
    info.shipped = Some(shipped);
    write_info(fresh, &info)?;
    // The game may have started while the download ran: its DLLs would be locked half way through the move.
    if game_running() {
        return Err("Close the game to update".into());
    }
    let hint = |e: &std::io::Error| {
        if in_use(e) || game_running() { ". If the game is running, close it first: Windows keeps a running mod's DLL locked" } else { "" }
    };
    #[cfg(test)]
    if let Some(f) = BEFORE_LOCK.with(|h| h.borrow_mut().take()) {
        f();
    }
    // Only finding the folder and the renames need the lock: two jobs never move one folder at once.
    let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    // It may have started while this job waited for the lock, too.
    if game_running() {
        return Err("Close the game to update".into());
    }
    let path = still_at(old, &path)?;
    let label = if old.info.version.is_empty() { format!("{} (before update)", old.name) } else { format!("{} {} (before update)", old.name, clean_name(&old.info.version)) };
    let backup = move_aside_io(&path, bin, &label).map_err(|e| format!("moving {}: {e}{}", path.display(), hint(&e)))?;
    if let Err(e) = rename_retry(fresh, &path) {
        // Put the old copy back so the mod is not lost.
        return Err(match rename_retry(&backup, &path) {
            Ok(()) => format!("could not put the new copy in place ({e}); the old copy is back{}", hint(&e)),
            Err(_) => {
                // Neither copy is in place: keep both where the user can find them, not in a hidden folder.
                let label = if p.info.version.is_empty() { format!("{} (new, not installed)", old.name) } else { format!("{} {} (new, not installed)", old.name, clean_name(&p.info.version)) };
                let new = match move_aside_io(fresh, bin, &label) {
                    Ok(to) => to,
                    Err(_) => {
                        *keep = true;
                        fresh.to_path_buf()
                    }
                };
                format!("could not put the new copy in place ({e}); the new copy is in {} and the old copy is in {}{}", new.display(), backup.display(), hint(&e))
            }
        });
    }
    Ok(Installed {
        mods: vec![old.info.title.clone()],
        exefs_only: p.exefs_only,
        patch_only: p.patch_only,
        needs_build,
        parts: false,
        replaced: Some((backup, carried.kept)),
        merged: carried.merged,
        new_beside: carried.new_beside,
        dropped: carried.dropped,
        off_for: None,
    })
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
    // (Update all) or by two copies of the launcher.
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
/// left alone; a `.partial-…` install or update is kept for a day. A folder whose
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
/// are `<stamp>-<pid>-<n>` (`.partial-update-…` for an update).
fn stage_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(".partial-").unwrap_or(name);
    let rest = rest.strip_prefix("update-").unwrap_or(rest);
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

/// A newer upload of an installed mod, if GameBanana has one: a current
/// download uploaded after the installed one (or, for a mod installed
/// before files were recorded, after it was installed), from the same file
/// family when that is known, and not skipped by the user. Console and
/// emulator builds are only offered when there is nothing else.
pub fn update_for(m: &Mod, files: &[GbFile]) -> Option<GbFile> {
    newer_uploads(m, files).into_iter().max_by_key(|f| (f.fit().signum(), f.date))
}

/// The current uploads newer than the installed copy that could be its
/// update. Only the installed file's family while that family is still on
/// the page (a page can hold two mods), and no console / emulator build in
/// place of a PC one.
fn newer_uploads(m: &Mod, files: &[GbFile]) -> Vec<GbFile> {
    let since = if m.info.file_date > 0 { m.info.file_date } else { m.installed_at };
    if since == 0 || m.info.no_updates {
        return Vec::new();
    }
    let fam = family(&m.info.file_name);
    let fam_alive = !fam.is_empty() && files.iter().any(|f| f.playable() && family(&f.name) == fam);
    let pc = m.info.file_name.is_empty() || fit(&m.info.file_name) >= 0;
    let any_pc = files.iter().any(|g| g.playable() && g.fit() >= 0);
    files
        .iter()
        .filter(|f| f.playable() && f.date > since && Some(f.id) != m.info.file_id && f.date > m.info.skip_update)
        .filter(|f| !fam_alive || family(&f.name) == fam)
        .filter(|f| !(pc && any_pc && f.fit() < 0))
        .cloned()
        .collect()
}

/// What sets a file apart from others of its family once the version is
/// gone: `mymod-v2-optional-addon.zip` is `optional addon`, the main file
/// `main mod`, `quicksave-100_33510.zip` nothing.
fn file_kind(file_name: &str) -> String {
    let l = file_name.to_lowercase();
    let mut stem = l.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(l.clone());
    if let Some((head, tail)) = stem.rsplit_once('_') {
        if tail.len() == 5 && tail.chars().all(|c| c.is_ascii_hexdigit()) && tail.chars().any(|c| c.is_ascii_digit()) {
            stem = head.to_string();
        }
    }
    let fam = family(file_name);
    let rest = stem.strip_prefix(&fam).unwrap_or(&stem);
    rest.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()) && !(w.starts_with('v') && w[1..].chars().all(|c| c.is_ascii_digit())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The launcher can't tell which newer upload is this mod's update, so the
/// user picks the file: the installed file is unknown (installed before
/// files were recorded) or gone from the page while the page holds more
/// than one mod, or several newer uploads of the family differ in what they
/// are (the main mod and an add-on).
pub fn update_needs_choice(m: &Mod, files: &[GbFile]) -> bool {
    let newer = newer_uploads(m, files);
    if newer.is_empty() {
        return false;
    }
    let fam = family(&m.info.file_name);
    let fam_alive = !fam.is_empty() && files.iter().any(|f| f.playable() && family(&f.name) == fam);
    if !fam_alive {
        let mut fams: Vec<String> = files.iter().filter(|f| f.playable()).map(|f| family(&f.name)).collect();
        fams.sort();
        fams.dedup();
        if fams.len() > 1 {
            return true;
        }
    }
    let mut kinds: Vec<String> = newer.iter().map(|f| file_kind(&f.name)).collect();
    kinds.sort();
    kinds.dedup();
    kinds.len() > 1
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
            file_id: Some(file.id).filter(|&i| i > 0),
            file_name: file.name.clone(),
            file_date: file.date,
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
        let mut installed = list(&mods);
        // A mod removed since the list was read fails; the rest still switch.
        let mut gone = installed[0].clone();
        gone.name = "Aardvark".into();
        gone.path = mods.join("Aardvark");
        installed.insert(0, gone);
        let (changed, failed) = apply_profile(&installed, &["Beta".to_string()]);
        assert_eq!(changed, 3, "Alpha and Gamma off, Beta on");
        assert_eq!(failed.len(), 1, "{failed:?}");
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
        let was = |variant: &str, title: &str, installed: u64, file_id: Option<u64>| Info { variant: variant.into(), title: title.into(), installed, file_id, ..Info::default() };
        assert_eq!(p.same_variant(&was("obelisk chamber (pc recomp)", "", 1, Some(3))), Some(p.recommended()));
        assert_eq!(p.same_variant(&was("", "Gift Guide", 0, None)), None);
        // Installed by an older launcher, the variant only in the name.
        assert_eq!(p.same_variant(&was("", "Gift Guide - Obelisk Chamber (PC recomp)_mods", 0, None)), Some(p.recommended()));
        assert_eq!(p.same_variant(&was("", "Gift Guide - Obelisk Chamber (PC recomp)_mods", 5, Some(3))), None, "a recorded install without a variant asks");
        // Skip wrote an install time on an older copy: still matched by its name.
        assert_eq!(p.same_variant(&was("", "Gift Guide - Obelisk Chamber (PC recomp)_mods", 5, None)), Some(p.recommended()));
        // Only the picked one is installed, and it is on.
        let mods = root.join("mods");
        let r = finish(&p, &[p.recommended()], &mods).unwrap();
        assert_eq!(r.mods, vec!["Gift Guide - Obelisk Chamber (PC recomp)".to_string()]);
        let l = list(&mods);
        assert_eq!(l.len(), 1);
        assert!(l[0].enabled);
        assert_eq!(l[0].info.variant, "Obelisk Chamber (PC recomp)");
        assert!(l[0].installed_at > 0);
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

    /// Settings travel with an update; code never does.
    #[test]
    fn only_settings_files_are_carried() {
        for keep in ["settings.ini", "exefs/turn.cfg", "native_appearance.settings.favorite-characters", "config.json", "exefs/x.toml"] {
            assert!(is_settings(keep), "{keep}");
        }
        for skip in ["plugin.toml", "exefs/plugin.ini", "manifest.cfg", "exefs/manifest.toml", "mod.toml", "plugin.json", "exefs/config_loader.lua", "config.dat", "settings.py", "exefs/config_menu.dll", "exefs/settings_ui.sites", "configpatch.ips", "README.md", "emblemweave.json", "manifest.json", "names.tsv", "LICENSE", "settings.ini.new", "exefs/turn.cfg.old", "config.json.new"] {
            assert!(!is_settings(skip), "{skip}");
        }
    }

    /// The Settings button lists a mod's settings files, each with the new
    /// default an update put beside it, never other copies or manifests.
    #[test]
    fn the_settings_button_lists_only_settings_files() {
        let root = temp("settings-of");
        let m = root.join("M");
        for f in ["exefs/settings.ini", "exefs/settings.ini.new", "exefs/gone.cfg.new", "exefs/turn.cfg.old", "exefs/m.dll", "exefs/plugin.toml", "emblemweave.json", "manifest.json", "romfs/config.ini", "src/config.toml", "Config.json", "README.md"] {
            write(&m.join(f));
        }
        assert_eq!(settings_of(&m), vec!["Config.json".to_string(), "exefs/settings.ini".to_string(), "exefs/settings.ini.new".to_string()]);
        let none = root.join("N");
        write(&none.join("exefs/n.dll"));
        write(&none.join("romfs/GameData/settings.ini"));
        assert!(settings_of(&none).is_empty(), "no Settings button for a mod without settings files");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A newer upload of the other mod on a shared page is not an update.
    #[test]
    fn an_update_stays_in_its_family() {
        let files = vec![
            file(1, "pleasenomoreclicking-v200-main-mod.zip", "MAIN MOD", 100, false),
            file(2, "manualstacking-v120-main-mod.zip", "MAIN MOD", 300, false),
            file(3, "thirdmod-v1.zip", "MAIN MOD", 300, false),
            file(4, "gg-v2-switch.zip", "atmosphere for emus", 400, false),
        ];
        let nmc = installed(Some(1), "pleasenomoreclicking-v200-main-mod.zip", 100, 150);
        assert_eq!(update_for(&nmc, &files), None);
        // Installed before files were recorded on a page with several mods: the user picks.
        let legacy = installed(None, "", 0, 150);
        assert!(update_needs_choice(&legacy, &files));
        assert!(!update_needs_choice(&nmc, &files));
        let only_a = vec![file(1, "amod-v1.zip", "", 100, false), file(2, "bmod-v1.zip", "", 50, false), file(5, "amod-v2.zip", "", 300, false)];
        assert!(update_needs_choice(&installed(None, "", 0, 150), &only_a), "a legacy copy of B must not take A's upload silently");
        // The installed file was renamed away on a page with two mods.
        let renamed = installed(Some(7), "bmod-old-v1.zip", 100, 150);
        assert!(update_needs_choice(&renamed, &only_a));
        // A main file and an add-on of one family.
        let addon = vec![file(1, "mymod-v1-main.zip", "", 100, false), file(2, "mymod-v2-main.zip", "", 300, false), file(3, "mymod-v2-optional-addon.zip", "", 301, false)];
        assert!(update_needs_choice(&installed(Some(1), "mymod-v1-main.zip", 100, 150), &addon));
        assert_eq!(file_kind("quicksave-100_33510.zip"), "");
        // One family, one kind: no question.
        let qs = vec![file(1, "quicksave-100_6e177.zip", "", 100, true), file(2, "quicksave-100_33510.zip", "", 200, false)];
        assert!(!update_needs_choice(&installed(Some(1), "quicksave-100_6e177.zip", 100, 150), &qs));
        assert!(fit("Hotkey for switching units") == 0 && fit("(Weapon Switch)") == 0 && fit("New (windowskin") == 0);
        assert!(!file(9, "x.zip", "Recompiled from source; only works on PC", 1, false).source_only());
        assert_eq!(family("facade_added.zip"), "facade_added");
        // A console build is never offered in place of a PC one.
        let gg = installed(Some(9), "gg-v1.zip", 100, 150);
        assert_eq!(update_for(&gg, &files), None);
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

    fn installed(file_id: Option<u64>, file_name: &str, file_date: u64, installed_at: u64) -> Mod {
        Mod {
            path: PathBuf::new(),
            name: "x".into(),
            enabled: true,
            info: Info { title: "x".into(), file_id, file_name: file_name.into(), file_date, gamebanana_id: Some(1), ..Info::default() },
            files: 1,
            has_romfs: false,
            has_exefs: true,
            has_code: true,
            has_patches: false,
            plugin: None,
            kind: Kind::Code,
            api: Api::Unknown,
            has_plugin_manifest: false,
            has_mod_manifest: false,
            installed_at,
        }
    }

    /// An update is a newer current upload, even under the same version.
    #[test]
    fn finds_updates() {
        let files = vec![
            file(1, "quicksave-100_6e177.zip", "for recomps only", 100, true),
            file(2, "quicksave-100_33510.zip", "recomps only", 200, false),
        ];
        let old = installed(Some(1), "quicksave-100_6e177.zip", 100, 150);
        assert_eq!(update_for(&old, &files).map(|f| f.id), Some(2));
        let current = installed(Some(2), "quicksave-100_33510.zip", 200, 250);
        assert_eq!(update_for(&current, &files), None);
        // Installed before files were recorded: newer than the install time.
        assert_eq!(update_for(&installed(None, "", 0, 150), &files).map(|f| f.id), Some(2));
        assert_eq!(update_for(&installed(None, "", 0, 250), &files), None);
        // Skipped.
        let mut skipped = installed(Some(1), "quicksave-100_6e177.zip", 100, 150);
        skipped.info.skip_update = 200;
        assert_eq!(update_for(&skipped, &files), None);
        let mut port = installed(None, "", 0, 150);
        port.info.no_updates = true;
        assert_eq!(update_for(&port, &files), None);
        // Source code uploads are not updates.
        let src = vec![file(3, "x-v2-source-only.zip", "SOURCE CODE ONLY", 900, false)];
        assert_eq!(update_for(&old, &src), None);
    }

    /// An update keeps the folder (and so on/off and the load order), keeps
    /// the user's settings, and moves the old copy aside.
    #[test]
    fn an_update_replaces_in_place_and_keeps_settings() {
        let root = temp("update");
        let mods = root.join("mods");
        write(&mods.join("_Quick Save/exefs/quick_save.dll"));
        std::fs::write(mods.join("_Quick Save/settings.ini"), "slot=3").unwrap();
        std::fs::write(mods.join("_Quick Save/exefs/turn.cfg"), "t=5").unwrap();
        std::fs::write(mods.join("_Quick Save/exefs/quick.ini"), "a=5\n").unwrap();
        write(&mods.join("_Quick Save/exefs/old_only.dll"));
        write_info(&mods.join("_Quick Save"), &Info { title: "Quick Save".into(), version: "1.0.0".into(), ..Info::default() }).unwrap();
        let old = list(&mods).remove(0);
        let new = root.join("new");
        std::fs::create_dir_all(new.join("QuickSave/exefs")).unwrap();
        std::fs::write(new.join("QuickSave/exefs/quick_save.dll"), "new").unwrap();
        std::fs::write(new.join("QuickSave/settings.ini"), "slot=1").unwrap();
        std::fs::write(new.join("QuickSave/exefs/turn.cfg"), "t=2").unwrap();
        std::fs::write(new.join("QuickSave/exefs/quick.ini"), "a=1\nb=2\n").unwrap();
        let p = prepare_dir(&new, None, Info { title: "Quicksave".into(), version: "1.0.1".into(), file_id: Some(7), ..Info::default() }).unwrap();
        let r = finish_update_into(&p, 0, &old, &root.join("bin")).unwrap();
        let (backup, kept) = r.replaced.clone().unwrap();
        // No record of what 1.0.0 shipped: every settings file is the user's. The .ini 1.0.1 ships too
        // is merged; turn.cfg keeps the user's copy, with the new default beside it.
        assert_eq!(kept, 3);
        assert_eq!(r.merged, vec![("exefs/quick.ini".to_string(), vec!["b".to_string()])]);
        assert_eq!(r.new_beside, vec!["exefs/turn.cfg".to_string()]);
        assert!(backup.join("exefs/old_only.dll").is_file(), "the old copy is kept aside");
        let l = list(&mods);
        assert_eq!(l.len(), 1);
        assert!(!l[0].enabled, "a switched-off mod stays off");
        assert_eq!(l[0].info.title, "Quick Save", "the installed name stays");
        assert_eq!(l[0].info.version, "1.0.1");
        assert_eq!(l[0].info.file_id, Some(7));
        assert_eq!(std::fs::read_to_string(mods.join("_Quick Save/exefs/quick_save.dll")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(mods.join("_Quick Save/settings.ini")).unwrap(), "slot=3", "the user's settings survive");
        assert_eq!(std::fs::read_to_string(mods.join("_Quick Save/exefs/turn.cfg")).unwrap(), "t=5", "the user's copy stays");
        assert_eq!(std::fs::read_to_string(mods.join("_Quick Save/exefs/turn.cfg.new")).unwrap(), "t=2", "the new default is beside it");
        assert!(!mods.join("_Quick Save/exefs/turn.cfg.old").exists());
        assert_eq!(std::fs::read_to_string(mods.join("_Quick Save/exefs/quick.ini")).unwrap(), "a=5\nb=2\n", "a legacy .ini is merged");
        assert_eq!(l[0].info.shipped.as_ref().unwrap().keys().cloned().collect::<Vec<_>>(), vec!["exefs/quick.ini".to_string(), "exefs/turn.cfg".to_string()], "the update records what it shipped");
        assert!(!mods.join("_Quick Save/exefs/old_only.dll").exists(), "files the new version dropped go");
        let _ = std::fs::remove_dir_all(root);
    }

    /// With a record of what the download shipped, an update keeps what the
    /// user changed or the mod wrote, and lets an untouched default go.
    #[test]
    fn an_update_carries_only_changed_or_created_settings() {
        let root = temp("carry");
        let mods = root.join("mods");
        let v1 = root.join("v1");
        std::fs::create_dir_all(v1.join("M/exefs")).unwrap();
        std::fs::write(v1.join("M/exefs/m.dll"), "1").unwrap();
        std::fs::write(v1.join("M/exefs/settings.ini"), "a=1").unwrap();
        std::fs::write(v1.join("M/exefs/turn.cfg"), "t=1").unwrap();
        let p = prepare_dir(&v1, None, Info { title: "M".into(), ..Info::default() }).unwrap();
        finish(&p, &[0], &mods).unwrap();
        let old = list(&mods).remove(0);
        assert_eq!(old.info.shipped.as_ref().map(|s| s.len()), Some(2), "{:?}", old.info.shipped);
        // The user edits one file and the mod writes another at run time; settings.ini stays as shipped.
        std::fs::write(mods.join("M/exefs/turn.cfg"), "t=9").unwrap();
        std::fs::write(mods.join("M/exefs/native_appearance.settings.favorite-characters"), "lyn").unwrap();
        let v2 = root.join("v2");
        std::fs::create_dir_all(v2.join("M/exefs")).unwrap();
        std::fs::write(v2.join("M/exefs/m.dll"), "2").unwrap();
        std::fs::write(v2.join("M/exefs/settings.ini"), "a=2").unwrap();
        std::fs::write(v2.join("M/exefs/turn.cfg"), "t=2").unwrap();
        let p = prepare_dir(&v2, None, Info { title: "M".into(), version: "2".into(), ..Info::default() }).unwrap();
        let r = finish_update_into(&p, 0, &old, &root.join("bin")).unwrap();
        assert_eq!(r.replaced.as_ref().unwrap().1, 2);
        assert_eq!(r.new_beside, vec!["exefs/turn.cfg".to_string()], "the changed file the new version ships differently");
        assert!(r.merged.is_empty());
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/settings.ini")).unwrap(), "a=2", "an untouched default gives way to the new one");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/turn.cfg")).unwrap(), "t=9", "the user's edit is carried");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/turn.cfg.new")).unwrap(), "t=2", "with the new default beside it");
        assert!(!mods.join("M/exefs/settings.ini.new").exists(), "an untouched default has nothing beside it");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/native_appearance.settings.favorite-characters")).unwrap(), "lyn", "a file made at run time is carried");
        assert!(!mods.join("M/exefs/settings.ini.old").exists());
        let now = list(&mods).remove(0);
        assert_eq!(now.info.shipped.as_ref().and_then(|s| s.get("exefs/settings.ini")), sha256_file(&v2.join("M/exefs/settings.ini")).as_ref(), "the record is the new download's");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A download that shipped no settings files still records that (an empty
    /// record, not none): a settings.ini the mod wrote at run time is the
    /// user's, kept over the one the next version ships.
    #[test]
    fn an_empty_record_still_counts_as_a_record() {
        let root = temp("empty-record");
        let mods = root.join("mods");
        let v1 = root.join("v1");
        write(&v1.join("M/exefs/m.dll"));
        let p = prepare_dir(&v1, None, Info { title: "M".into(), ..Info::default() }).unwrap();
        finish(&p, &[0], &mods).unwrap();
        let old = list(&mods).remove(0);
        assert_eq!(old.info.shipped, Some(Default::default()), "an empty record is written");
        std::fs::write(mods.join("M/exefs/settings.ini"), "mine").unwrap();
        let v2 = root.join("v2");
        write(&v2.join("M/exefs/m.dll"));
        std::fs::write(v2.join("M/exefs/settings.ini"), "default").unwrap();
        let p = prepare_dir(&v2, None, Info { title: "M".into(), version: "2".into(), ..Info::default() }).unwrap();
        let r = finish_update_into(&p, 0, &old, &root.join("bin")).unwrap();
        assert_eq!(r.replaced.as_ref().unwrap().1, 1);
        assert!(r.merged.is_empty() && r.new_beside.is_empty() && r.dropped.is_empty(), "{r:?}");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/settings.ini")).unwrap(), "mine", "no key to add: the user's file as it was");
        assert!(!mods.join("M/exefs/settings.ini.old").exists() && !mods.join("M/exefs/settings.ini.new").exists());
        // Nothing of the update is left beside the mod, and a failed one leaves nothing either.
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 1);
        let mut gone = list(&mods).remove(0);
        gone.name = "Elsewhere".into();
        gone.path = mods.join("Elsewhere");
        assert!(finish_update_into(&p, 0, &gone, &root.join("bin")).is_err());
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 1, "no .partial-update folder is left");
        let _ = std::fs::remove_dir_all(root);
    }

    /// The new version's `.ini` is merged into the user's: their values,
    /// comments, order and extra keys stay, new keys and sections are added.
    #[test]
    fn an_ini_merge_adds_what_the_new_version_has() {
        let mine = "; my comment\r\ntop=1\r\n[Controls]\r\nHold_Key = F5   ; mine\r\nmy_key=1\r\nold_key=2\r\n[UI]\r\n; ui comment\r\ntheme=dark\r\n";
        let theirs = "top=0\nglob_new=7\n[controls]\nhold_key=F1\nnew_key=3\n[Ui]\ntheme=light\nscale=2\n; Audio settings\n[Audio]\nvolume=5\nvolume=6\n[Empty]\n";
        let (text, added) = merge_ini(mine.as_bytes(), theirs.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(text).unwrap(),
            "; my comment\r\ntop=1\r\nglob_new=7\r\n[Controls]\r\nHold_Key = F5   ; mine\r\nmy_key=1\r\nold_key=2\r\nnew_key=3\r\n[UI]\r\n; ui comment\r\ntheme=dark\r\nscale=2\r\n\r\n; Audio settings\r\n[Audio]\r\nvolume=5\r\n\r\n[Empty]\r\n"
        );
        assert_eq!(added, vec!["glob_new", "controls/new_key", "Ui/scale", "Audio/volume", "Empty"]);
        // Nothing new: the user's file, byte for byte.
        let (same, none) = merge_ini(mine.as_bytes(), "[CONTROLS]\nHOLD_KEY=1\n".as_bytes()).unwrap();
        assert_eq!((same.as_slice(), none.len()), (mine.as_bytes(), 0));
        // A byte order mark and a missing last line ending stay; a global key goes above the comments over the first section.
        let (text, _) = merge_ini("\u{feff}a=1".as_bytes(), b"a=2\nb=3\n").unwrap();
        assert_eq!(String::from_utf8(text).unwrap(), "\u{feff}a=1\nb=3");
        let (text, _) = merge_ini(b"; head\n[S]\nx=1\nodd line\n", b"g=1\n[S]\nx=2\ny=2\n").unwrap();
        assert_eq!(String::from_utf8(text).unwrap(), "g=1\n; head\n[S]\nx=1\ny=2\nodd line\n");
        // Not UTF-8, or UTF-16: no merge.
        assert!(merge_ini(b"a=\xff\n", b"a=1\nb=2\n").is_none());
        assert!(merge_ini(b"\xff\xfea\x00=\x001\x00", b"a=1\nb=2\n").is_none());
        assert!(merge_ini(b"a=1\n", b"\xfe\xff\x00a").is_none());
    }

    /// Merging keeps the user's byte order mark first and drops the new
    /// file's, keeps each line's own line ending (added lines take the one
    /// most of the file uses), matches section names in any case and brings a
    /// new key's comment lines along.
    #[test]
    fn an_ini_merge_keeps_boms_line_endings_and_comments() {
        let merge = |mine: &str, theirs: &str| merge_ini(mine.as_bytes(), theirs.as_bytes()).map(|(t, a)| (String::from_utf8(t).unwrap(), a));
        // The user's byte order mark stays first, above a global key added at the top.
        assert_eq!(merge("\u{feff}[S]\nk=1\n", "g=1\n[S]\nk=1\n"), Some(("\u{feff}g=1\n[S]\nk=1\n".to_string(), vec!["g".to_string()])));
        // The new file's goes nowhere, whether its first line is a new key or a new section.
        assert_eq!(merge("a=1\n", "\u{feff}b=2\na=1\n").unwrap().0, "a=1\nb=2\n");
        assert_eq!(merge("a=1\n", "\u{feff}[New]\nx=1\n"), Some(("a=1\n\n[New]\nx=1\n".to_string(), vec!["New/x".to_string()])));
        assert_eq!(merge("\u{feff}a=1\n", "\u{feff}[New]\nx=1\n").unwrap().0, "\u{feff}a=1\n\n[New]\nx=1\n");
        assert_eq!(merge("\u{feff}[S]\na=1\n", "\u{feff}[S]\na=1\nb=2\n").unwrap().0, "\u{feff}[S]\na=1\nb=2\n");
        // Mixed line endings: each line keeps its own, added ones take the most used.
        assert_eq!(merge("a=1\r\n[S]\nx=1\r\ny=1\n[T]\r\nz=1\r\n", "a=1\nb=2\n[S]\nx=1\nw=2\n[T]\nz=1\n").unwrap().0, "a=1\r\nb=2\r\n[S]\nx=1\r\ny=1\nw=2\r\n[T]\r\nz=1\r\n");
        assert_eq!(merge("a=1\n[S]\nx=1\r\n", "[S]\nx=1\r\ny=2\r\n").unwrap().0, "a=1\n[S]\nx=1\r\ny=2\n");
        // A carriage return alone: not merged (the new default goes beside it).
        assert_eq!(merge("a=1\rb=1\n", "a=1\nc=2\n"), None);
        assert_eq!(merge("a=1\n", "a=1\rc=2\n"), None);
        // Section names in any case, beyond ASCII too; `[]` is no header.
        assert_eq!(merge("[Ö]\na=1\n", "[ö]\na=1\nb=2\n"), Some(("[Ö]\na=1\nb=2\n".to_string(), vec!["ö/b".to_string()])));
        assert_eq!(merge("x=1\n", "[Ö]\na=1\n[ö]\nb=2\n"), Some(("x=1\n\n[Ö]\na=1\nb=2\n".to_string(), vec!["Ö/a".to_string(), "Ö/b".to_string()])));
        assert_eq!(merge("[S]\nx=1\n[]\ny=1\n", "z=1\n[S]\nx=1\ny=1\n").unwrap().0, "z=1\n[S]\nx=1\n[]\ny=1\n");
        // The comment lines right above a new key come with it; not across a blank line, and not twice.
        assert_eq!(
            merge("[S]\nx=1\n", "[S]\nx=1\n; speed, 1 to 4\n; 2 is the default\nspeed=2\nother=1\n").unwrap().0,
            "[S]\nx=1\n; speed, 1 to 4\n; 2 is the default\nspeed=2\nother=1\n"
        );
        assert_eq!(merge("[S]\nx=1\n", "[S]\nx=1\n; about x\n\nfar=1\n").unwrap().0, "[S]\nx=1\nfar=1\n");
        assert_eq!(merge("[S]\n; speed, 1 to 4\nx=1\n", "[S]\n; speed, 1 to 4\nspeed=2\nx=1\n").unwrap().0, "[S]\n; speed, 1 to 4\nx=1\nspeed=2\n");
        assert_eq!(merge("a=1\n[S]\n", "# b help\nb=2\n[S]\n").unwrap().0, "a=1\n# b help\nb=2\n[S]\n");
    }

    /// An update merges a changed `.ini`, puts the new default of any other
    /// changed file beside it, and records the new version's shipped files,
    /// so the next update still sees the user's changes.
    #[test]
    fn an_update_merges_ini_and_puts_other_defaults_beside() {
        let root = temp("merge");
        let mods = root.join("mods");
        let version = |n: &str, ini: &str, json: &str| {
            let d = root.join(format!("v{n}"));
            std::fs::create_dir_all(d.join("M/exefs")).unwrap();
            std::fs::write(d.join("M/exefs/m.dll"), n).unwrap();
            std::fs::write(d.join("M/exefs/settings.ini"), ini).unwrap();
            std::fs::write(d.join("M/exefs/config.json"), json).unwrap();
            prepare_dir(&d, None, Info { title: "M".into(), version: n.into(), ..Info::default() }).unwrap()
        };
        finish(&version("1", "[A]\nk=1\n", "{\"a\":1}"), &[0], &mods).unwrap();
        std::fs::write(mods.join("M/exefs/settings.ini"), "[A]\nk=9\nmy_key=1\n").unwrap();
        std::fs::write(mods.join("M/exefs/config.json"), "{\"a\":5}").unwrap();
        let v2 = version("2", "[A]\nk=1\nnew=2\n[B]\nz=0\n", "{\"a\":2}");
        let r = finish_update_into(&v2, 0, &list(&mods).remove(0), &root.join("bin")).unwrap();
        assert_eq!(r.replaced.as_ref().unwrap().1, 2);
        assert_eq!(r.merged, vec![("exefs/settings.ini".to_string(), vec!["A/new".to_string(), "B/z".to_string()])]);
        assert_eq!(r.new_beside, vec!["exefs/config.json".to_string()]);
        let ini = mods.join("M/exefs/settings.ini");
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "[A]\nk=9\nmy_key=1\nnew=2\n\n[B]\nz=0\n");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/config.json")).unwrap(), "{\"a\":5}");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/config.json.new")).unwrap(), "{\"a\":2}");
        let now = list(&mods).remove(0);
        let shipped = now.info.shipped.clone().unwrap();
        assert_eq!(shipped.get("exefs/settings.ini"), sha256_file(&root.join("v2/M/exefs/settings.ini")).as_ref(), "the record is the new download's");
        assert_ne!(shipped.get("exefs/settings.ini"), sha256_file(&ini).as_ref());
        assert!(!shipped.contains_key("exefs/config.json.new"), "a .new copy is not a settings file");
        // The same files again: the merged file is still the user's.
        let r = finish_update_into(&version("3", "[A]\nk=1\nnew=2\n[B]\nz=0\n", "{\"a\":2}"), 0, &now, &root.join("bin")).unwrap();
        assert!(r.merged.is_empty(), "nothing new to add");
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "[A]\nk=9\nmy_key=1\nnew=2\n\n[B]\nz=0\n");
        // An .ini that is not UTF-8 is not merged: the new default goes beside it.
        std::fs::write(&ini, b"[A]\nk=\xff\n").unwrap();
        let r = finish_update_into(&version("4", "[A]\nk=1\nnew=3\n", "{\"a\":2}"), 0, &list(&mods).remove(0), &root.join("bin")).unwrap();
        assert_eq!(r.new_beside, vec!["exefs/config.json".to_string(), "exefs/settings.ini".to_string()]);
        assert_eq!(std::fs::read(&ini).unwrap(), b"[A]\nk=\xff\n");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/settings.ini.new")).unwrap(), "[A]\nk=1\nnew=3\n");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mod switched off while its update was being built is not replaced:
    /// the settings came from the folder it had before.
    #[test]
    fn an_update_stops_when_the_mod_was_switched_during_it() {
        let root = temp("switched");
        let mods = root.join("mods");
        let v1 = root.join("v1");
        write(&v1.join("M/exefs/m.dll"));
        let p = prepare_dir(&v1, None, Info { title: "M".into(), version: "1".into(), ..Info::default() }).unwrap();
        finish(&p, &[0], &mods).unwrap();
        let old = list(&mods).remove(0);
        let v2 = root.join("v2");
        write(&v2.join("M/exefs/m.dll"));
        let p = prepare_dir(&v2, None, Info { title: "M".into(), version: "2".into(), ..Info::default() }).unwrap();
        let (from, to) = (mods.join("M"), mods.join("_M"));
        BEFORE_LOCK.with(|h| *h.borrow_mut() = Some(Box::new(move || std::fs::rename(from, to).unwrap())));
        let e = finish_update_into(&p, 0, &old, &root.join("bin")).unwrap_err();
        assert_eq!(e, "M was switched on or off during the update; try again");
        assert!(mods.join("_M/exefs/m.dll").is_file(), "the switched-off copy stays");
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 1, "the new copy is cleaned up");
        assert!(!root.join("bin").exists(), "nothing was set aside");
        assert!(still_at(&old, &mods.join("_M")).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A settings file the user changed that the new version no longer ships
    /// is kept, and named apart from the settings carried over.
    #[test]
    fn a_settings_file_the_new_version_dropped_is_named() {
        let root = temp("dropped");
        let mods = root.join("mods");
        let v1 = root.join("v1");
        write(&v1.join("M/exefs/m.dll"));
        std::fs::write(v1.join("M/exefs/turn.cfg"), "t=1").unwrap();
        let p = prepare_dir(&v1, None, Info { title: "M".into(), ..Info::default() }).unwrap();
        finish(&p, &[0], &mods).unwrap();
        let old = list(&mods).remove(0);
        std::fs::write(mods.join("M/exefs/turn.cfg"), "t=9").unwrap();
        std::fs::write(mods.join("M/exefs/favorites.settings"), "lyn").unwrap();
        let v2 = root.join("v2");
        write(&v2.join("M/exefs/m.dll"));
        std::fs::write(v2.join("M/exefs/turn_control.toml"), "t=2").unwrap();
        let p = prepare_dir(&v2, None, Info { title: "M".into(), version: "2".into(), ..Info::default() }).unwrap();
        let r = finish_update_into(&p, 0, &old, &root.join("bin")).unwrap();
        assert_eq!(r.dropped, vec!["exefs/turn.cfg".to_string()]);
        assert_eq!(r.replaced.as_ref().unwrap().1, 1, "only the file the mod wrote counts as kept settings");
        assert_eq!(std::fs::read_to_string(mods.join("M/exefs/turn.cfg")).unwrap(), "t=9", "the user's copy stays in the mod folder");
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
        assert_eq!(stage_pid(".partial-update-100-4242-3"), Some(4242));
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

    /// Unknown keys in emblemweave.json survive a rewrite.
    #[test]
    fn write_info_keeps_unknown_keys() {
        let root = temp("info");
        std::fs::write(root.join("emblemweave.json"), r#"{"title":"T","description":"keep me","skip_update":5}"#).unwrap();
        write_info(&root, &Info { title: "T".into(), ..Info::default() }).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("emblemweave.json")).unwrap()).unwrap();
        assert_eq!(v["description"], "keep me");
        assert!(v.get("skip_update").is_none());
        let _ = std::fs::remove_dir_all(root);
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

    /// A real update (network): an old Quick Save upload installed, the
    /// newer one found and put in its place with the settings kept.
    #[test]
    #[ignore]
    fn live_gamebanana_update() {
        let root = temp("live-update");
        let mods = root.join("mods");
        let (m, files) = gb_mod(745602).unwrap();
        let old = files.iter().filter(|f| f.archived).min_by_key(|f| f.date).unwrap().clone();
        let stage = root.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("a.zip"), fetch(&old.url).unwrap()).unwrap();
        unpack(&stage.join("a.zip"), &stage.join("u")).unwrap();
        let info = Info { title: m.name.clone(), gamebanana_id: Some(m.id), file_id: Some(old.id), file_name: old.name.clone(), file_date: old.date, version: old.version.clone(), ..Info::default() };
        let p = prepare_dir(&stage.join("u"), None, info).unwrap();
        finish(&p, &[0], &mods).unwrap();
        let installed = list(&mods).remove(0);
        std::fs::write(installed.path.join("settings.ini"), "mine").unwrap();
        let f = update_for(&installed, &files).expect("a newer upload");
        println!("installed {} -> update {}", old.name, f.name);
        let root2 = root.join("stage2");
        std::fs::create_dir_all(&root2).unwrap();
        std::fs::write(root2.join("b.zip"), fetch(&f.url).unwrap()).unwrap();
        unpack(&root2.join("b.zip"), &root2.join("u")).unwrap();
        let info = Info { title: m.name.clone(), gamebanana_id: Some(m.id), file_id: Some(f.id), file_name: f.name.clone(), file_date: f.date, ..Info::default() };
        let p = prepare_dir(&root2.join("u"), Some(root2.clone()), info).unwrap();
        let i = p.same_variant(&installed.info).unwrap();
        let r = finish_update_into(&p, i, &installed, &root.join("bin")).unwrap();
        println!("{r:?}");
        let after = list(&mods).remove(0);
        assert_eq!(after.info.file_id, Some(f.id));
        assert_eq!(std::fs::read_to_string(after.path.join("settings.ini")).unwrap(), "mine");
        assert_eq!(update_for(&after, &files), None);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The reported ultrawide mod (GameBanana 737498) is four `.pchtxt` code
    /// patches, not RomFS content: EmblemWeave must install each pack as a patch
    /// mod that applies when the game is built, not call it an empty folder.
    #[test]
    #[ignore]
    fn live_gamebanana_ultrawide_patch_pack() {
        let root = temp("uw-pack");
        let mods = root.join("mods");
        let (m, files) = gb_mod(737498).unwrap();
        let f = files
            .iter()
            .find(|f| f.name.contains("43-18") && !f.archived)
            .or_else(|| files.iter().find(|f| f.name.contains("43-18")))
            .expect("the 21:9 pack")
            .clone();
        println!("{} file {} ({:?})", m.name, f.name, f.version);
        let p = gb_prepare(&m, &f).unwrap();
        assert_eq!(p.choices.len(), 4, "four folders: required, HUD, edges, dialogue");
        let picks: Vec<usize> = (0..p.choices.len()).collect();
        finish(&p, &picks, &mods).unwrap();
        p.discard();
        let l = list(&mods);
        assert_eq!(l.len(), 4);
        for m in &l {
            assert!(m.has_patches, "{} holds only .pchtxt patches", m.name);
            assert!(!m.has_romfs);
            assert_eq!(m.kind, Kind::Patch, "{}: {:?}", m.name, m.kind);
            assert!(m.kind.playable());
        }
        let _ = std::fs::remove_dir_all(root);
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

    // --- classification ---------------------------------------------------------------------------------------------------------

    /// Every layout the Mods page must tell apart, built with the real folder
    /// shape each mod ships.
    #[test]
    fn classifies_real_package_layouts() {
        let root = temp("kinds");
        let mods = root.join("mods");
        let put = |rel: &str| {
            let p = mods.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        put("RomFS Mod/romfs/GameData/Job.bin");
        put("Code Mod/exefs/quick_save.dll");
        put("Patch Mod/exefs/patches/patch.pchtxt");
        put("Both/romfs/a.bin");
        put("Both/exefs/x.dll");
        put("Manifest Only/plugin.json");
        std::fs::create_dir_all(mods.join("Blank")).unwrap();
        // A nested wrapper folder (the shape an archive with one top folder makes).
        put("Wrapped/Wrapped Mod/romfs/a.bin");

        let l = list(&mods);
        let kind = |n: &str| l.iter().find(|m| m.name == n).unwrap().kind;
        assert_eq!(kind("RomFS Mod"), Kind::RomFs);
        assert_eq!(kind("Code Mod"), Kind::Code);
        assert_eq!(kind("Patch Mod"), Kind::Patch);
        assert_eq!(kind("Both"), Kind::Combined);
        assert_eq!(kind("Manifest Only"), Kind::MetadataOnly);
        assert_eq!(kind("Blank"), Kind::Empty);
        assert_eq!(kind("Wrapped"), Kind::RomFs);
        assert!(l.iter().all(|m| m.kind.playable() == !matches!(m.kind, Kind::MetadataOnly | Kind::Empty)));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A download with only notes in it is refused and says why; one whose
    /// `exefs` holds nothing the engine can load is marked unusable instead of
    /// being installed as an empty mod.
    #[test]
    fn refuses_a_download_with_nothing_to_install() {
        let root = temp("incomplete");
        let notes = root.join("Notes");
        std::fs::create_dir_all(notes.join("Ultrawide 3440")).unwrap();
        std::fs::write(notes.join("Ultrawide 3440/read me.txt"), b"hello").unwrap();
        let e = prepare_dir(&notes, None, Info::default()).unwrap_err();
        assert!(e.contains("no romfs folder, patches or game files"), "{e}");

        let exefs = root.join("Exefs");
        std::fs::create_dir_all(exefs.join("exefs")).unwrap();
        std::fs::write(exefs.join("exefs/read me.txt"), b"hello").unwrap();
        let p = prepare_dir(&exefs, None, Info::default()).unwrap();
        assert!(p.unusable_exefs);
        assert!(p.choices.is_empty(), "nothing to offer, not an empty mod");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A DLL with no manifest still counts as a code mod, and its API is
    /// unknown rather than assumed.
    #[test]
    fn a_bare_dll_is_code_with_an_unknown_api() {
        let root = temp("bare-dll");
        let mods = root.join("mods");
        std::fs::create_dir_all(mods.join("Bare/exefs")).unwrap();
        std::fs::write(mods.join("Bare/exefs/bare.dll"), b"MZ not really a dll").unwrap();
        let m = list(&mods).remove(0);
        assert_eq!(m.kind, Kind::Code);
        assert_eq!(m.api, Api::Unknown);
        assert!(!m.uses_trinity());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The bundled Trinity Core, recognised by the identity its manifest
    /// carries, is a runtime component and not an ordinary content mod.
    #[test]
    fn recognises_the_bundled_trinity_core() {
        let root = temp("trinity-core");
        let mods = root.join("mods");
        let folder = mods.join("TrinityCore");
        std::fs::create_dir_all(folder.join("exefs")).unwrap();
        std::fs::write(folder.join("exefs/Trinity.dll"), b"MZ").unwrap();
        std::fs::write(
            folder.join("plugin.json"),
            r#"{"schema":1,"id":"org.xion.trinity.core","name":"Trinity Core","version":"0.2.0",
                "abi":{"min":1,"max":1},"hosts":["emblemweave"],"capabilities":["nxp.ui"]}"#,
        )
        .unwrap();
        let m = list(&mods).remove(0);
        assert!(m.is_trinity_core());
        assert_eq!(m.kind, Kind::Runtime);
        assert_eq!(m.kind.label(), "Runtime component");
        assert_eq!(m.version(), "0.2.0");
        assert!(!m.uses_trinity(), "the core is not a dependent mod");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A third-party plugin on the NXP layer is told apart from the core, and
    /// from a mod that merely sits in a folder called TrinityCore.
    #[test]
    fn a_third_party_nxp_plugin_is_not_the_core() {
        let root = temp("nxp-mod");
        let mods = root.join("mods");
        let folder = mods.join("Turn Control");
        std::fs::create_dir_all(folder.join("exefs")).unwrap();
        std::fs::write(folder.join("exefs/turn.dll"), b"MZ").unwrap();
        std::fs::write(
            folder.join("plugin.json"),
            r#"{"id":"org.example.turn","name":"Turn Control","version":"2.0.0","abi":{"min":1,"max":2},
                "capabilities":["nxp.settings"],"dependencies":["trinity"]}"#,
        )
        .unwrap();
        let m = list(&mods).remove(0);
        assert!(!m.is_trinity_core());
        assert!(m.uses_trinity());
        assert_eq!(m.api, Api::Trinity);
        assert_eq!(m.kind, Kind::Code);
        // A folder named like the core, with a DLL, but no manifest identity is
        // not the core: the name alone proves nothing.
        let fake = mods.join("TrinityCore");
        std::fs::create_dir_all(fake.join("exefs")).unwrap();
        std::fs::write(fake.join("exefs/Trinity.dll"), b"MZ").unwrap();
        let fake = list(&mods).into_iter().find(|m| m.name == "TrinityCore").unwrap();
        assert!(!fake.is_trinity_core());
        assert_eq!(fake.kind, Kind::Code);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Loading a mod profile never switches the bundled runtime component off.
    #[test]
    fn a_profile_leaves_the_runtime_component_alone() {
        let root = temp("profile-runtime");
        let mods = root.join("mods");
        let core = mods.join("TrinityCore");
        std::fs::create_dir_all(core.join("exefs")).unwrap();
        std::fs::write(core.join("exefs/Trinity.dll"), b"MZ").unwrap();
        std::fs::write(core.join("plugin.json"), r#"{"id":"org.xion.trinity.core","name":"Trinity Core"}"#).unwrap();
        std::fs::create_dir_all(mods.join("Other/romfs")).unwrap();
        std::fs::write(mods.join("Other/romfs/a.bin"), b"x").unwrap();
        let before = list(&mods);
        assert!(before.iter().any(|m| m.is_trinity_core() && m.enabled));
        // A profile that names only the other mod.
        let (changed, failed) = apply_profile(&before, &["Other".to_string()]);
        assert_eq!((changed, failed.len()), (0, 0));
        let after = list(&mods);
        assert!(after.iter().any(|m| m.is_trinity_core() && m.enabled), "Trinity Core stays on");
        assert!(after.iter().any(|m| m.name == "Other" && m.enabled));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A DLL's export names are read from its bytes: NxModApi entry points and
    /// nxp interfaces are evidence, not a guess from the file name.
    #[test]
    fn reads_the_api_from_the_dll() {
        assert_eq!(dll_api(b"text nxmod_init more"), (true, false));
        assert_eq!(dll_api(b"nxp.nxmod"), (false, true));
        assert_eq!(dll_api(b"nxmod_init nxp.ui"), (true, true));
        assert_eq!(dll_api(b"MZ nothing here"), (false, false));
    }
}
