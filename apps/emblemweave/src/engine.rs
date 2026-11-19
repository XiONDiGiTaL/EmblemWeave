//! The game engine: fefw-recomp (Goldoire and its contributors, GPL-3.0),
//! included in this repository at `engine/fefw-recomp` so a clone needs no
//! submodule fetch. See docs/UPSTREAM.md.
//!
//! The launcher builds the game with the engine's own steps (its
//! `docs/BUILDING.md`), from the user's dump, and runs it:
//!
//! 1. the engine's Python environment (`tools/.venv`, made with uv);
//! 2. the dump's ExeFS prepared for the lifter (`main` written uncompressed);
//! 3. the game code lifted and compiled (`recomp/scripts/build_game.cmd`);
//! 4. the texture transcoder sources;
//! 5. `fefw.exe`;
//! 6. the shader translator (MIT-licensed shader code, at the engine's
//!    pinned revision);
//! 7. the game's shaders translated to SPIR-V;
//! 8. `fefw.ini` in the launcher's run folder.
//!
//! Steps whose output is already there are skipped, and every command's
//! output goes to the job log. The dump is only read.

use crate::config::Config;
use crate::jobs::{Ctx, JobResult, StageStatus};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Pinned revision the engine's shader translator is built against.
const SHADER_TRANSLATOR_URL: &str = "https://git.ryujinx.app/ryubing/ryujinx.git";
const SHADER_TRANSLATOR_REV: &str = "a82350bb774f70fcbd41c9987bf67a3775409963";

pub const STAGES: [&str; 8] = [
    "Python environment",
    "Prepare the ExeFS",
    "Recompile the game code",
    "Texture transcoders",
    "Build fefw.exe",
    "Shader translator",
    "Translate the shaders",
    "Write fefw.ini",
];

/// The file in the settings folder that remembers where the engine is, so a
/// launcher copied anywhere (a release download, say) finds a checkout that
/// is already built.
fn remembered() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("engine.txt"))
}

/// The engine checkout. Candidates, in order: `EW_ENGINE`, the remembered
/// folder, then `engine/fefw-recomp` above the launcher or the working
/// folder. One whose game is already built wins; otherwise the first that
/// exists. The choice is remembered for next time.
pub fn root() -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    if let Some(e) = std::env::var_os("EW_ENGINE") {
        found.push(PathBuf::from(e));
    }
    if let Some(text) = remembered().and_then(|f| std::fs::read_to_string(f).ok()) {
        found.push(PathBuf::from(text.trim()));
    }
    for start in [std::env::current_exe().ok(), std::env::current_dir().ok()]
        .into_iter()
        .flatten()
    {
        found.extend(
            start
                .ancestors()
                .map(|a| a.join("engine").join("fefw-recomp")),
        );
    }
    found.retain(|e| e.join("game-build").is_dir());
    let pick = found
        .iter()
        .find(|e| game_exe(e).is_file())
        .or(found.first())
        .cloned()?;
    if let Some(f) = remembered() {
        if std::fs::read_to_string(&f)
            .ok()
            .map(|t| t.trim().to_string())
            != Some(pick.display().to_string())
        {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(&f, pick.display().to_string());
        }
    }
    Some(pick)
}

/// Remember `picked` as the engine checkout, so every later lookup uses it instead of looking beside the launcher. Accepts the
/// checkout (`…\engine\fefw-recomp`) or a folder that holds it, such as an older EmblemWeave copy. Returns the checkout.
///
/// The build is the expensive part of an install and the one thing a new release zip cannot carry, so keeping one engine folder
/// anywhere on disk and pointing a fresh launcher at it is what spares a full rebuild when updating (issue #56).
pub fn remember_root(picked: &Path) -> Result<PathBuf, String> {
    let file = remembered().ok_or("the settings folder is unavailable")?;
    remember_in(&file, picked)
}

/// The write behind [`remember_root`], with the file named so it can be tested: creates its folder, writes the checkout and
/// returns it. A folder that holds no engine is refused before anything is written.
pub(crate) fn remember_in(file: &Path, picked: &Path) -> Result<PathBuf, String> {
    let candidates = [
        picked.to_path_buf(),
        picked.join("engine").join("fefw-recomp"),
    ];
    let root = candidates
        .into_iter()
        .find(|c| c.join("game-build").is_dir())
        .ok_or_else(|| {
            format!(
                "{} does not hold an engine (expected engine\\fefw-recomp with game-build)",
                picked.display()
            )
        })?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(&file, root.display().to_string())
        .map_err(|e| format!("writing {}: {e}", file.display()))?;
    Ok(root)
}

/// A built engine found elsewhere on this PC, offered when this copy has none of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Existing {
    /// The checkout, `…/engine/fefw-recomp`.
    pub root: PathBuf,
    /// The EmblemWeave folder it belongs to, for the offer's text.
    pub install: PathBuf,
    /// When the built game was written, for the offer's text.
    pub built: Option<std::time::SystemTime>,
}

/// The built engine this copy could reuse instead of compiling one.
///
/// A new release is mostly a new launcher, so a copy that has no build of its own looks for one before asking for a rebuild:
/// `engine.txt` in this copy's settings, in the profile settings folder, and in the `data` folder of every older copy beside
/// it (a portable install writes it there), plus an `engine/fefw-recomp` under any sibling EmblemWeave folder. Nothing is
/// changed until the player accepts the offer on the Overview page.
pub fn existing_build() -> Option<Existing> {
    let here = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let mut settings: Vec<PathBuf> = Vec::new();
    if let Some(dir) = Config::dir() {
        settings.push(dir);
    }
    if let Some(dir) = crate::platform::data_root_from(&|k| std::env::var(k).ok()) {
        settings.push(dir);
    }
    // A sibling's remembered engine is in its own data folder, which is where a portable copy keeps it.
    if let Some(parent) = here.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() && p != here {
                    settings.push(p.join("data"));
                }
            }
        }
    }
    find_existing(&here, &settings, root().as_deref())
}

/// The detection behind [`existing_build`], with the folders passed in so it can be tested without touching a real install.
/// `current` is the checkout this copy would use; when it already holds a built game there is nothing to offer.
pub(crate) fn find_existing(
    here: &Path,
    settings: &[PathBuf],
    current: Option<&Path>,
) -> Option<Existing> {
    if current.is_some_and(|c| game_exe(c).is_file()) {
        return None;
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for dir in settings {
        let remembered = std::fs::read_to_string(dir.join("engine.txt")).unwrap_or_default();
        let remembered = remembered.trim();
        if !remembered.is_empty() {
            roots.push(PathBuf::from(remembered));
        }
    }
    if let Some(parent) = here.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() && p != here {
                    roots.push(p.join("engine").join("fefw-recomp"));
                }
            }
        }
    }
    let mut best: Option<Existing> = None;
    let mut seen: Vec<PathBuf> = Vec::new();
    for root in roots {
        if current.is_some_and(|c| c == root) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(game_exe(&root)) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical);
        let built = meta.modified().ok();
        let newer = best
            .as_ref()
            .is_none_or(|b| b.built.is_none_or(|t| built.is_some_and(|at| at > t)));
        if newer {
            let install = root
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.clone());
            best = Some(Existing {
                root,
                install,
                built,
            });
        }
    }
    best
}

/// The built game, if it exists.
pub fn game_exe(root: &Path) -> PathBuf {
    root.join("game-build")
        .join("fefw")
        .join("build")
        .join("release")
        .join("fefw.exe")
}

/// Where the game runs: its `fefw.ini`, log, mods and album live here.
pub fn run_dir() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("play"))
}

/// The folder that holds the built `fefw.exe`. ReShade installs its per-game
/// Vulkan configuration beside the executable it is given.
pub fn game_folder() -> Option<PathBuf> {
    let root = root()?;
    game_exe(&root).parent().map(Path::to_path_buf)
}

/// What ReShade's Vulkan install looks like for this game. The layer itself is
/// global; `ReShadeApps.ini` is the allowlist that decides which executables it
/// loads for, and `ReShade.ini` beside the executable holds that game's shader
/// paths and preset. None of this is bundled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReshadeStatus {
    /// The 64-bit layer DLL and its manifest are in `%ProgramData%\ReShade`.
    pub installed: bool,
    /// This build's executable is in `ReShadeApps.ini`.
    pub enabled: bool,
    /// `ReShade.ini` exists beside the built executable.
    pub configured: bool,
    /// ReShade has written a log for this executable (it loaded at least once).
    pub logged: bool,
}

fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

/// The path ReShade's setup tool writes its layer to.
pub fn reshade_layer_dir() -> PathBuf {
    program_data().join("ReShade")
}

/// Read ReShade's current state without changing anything. Missing files mean
/// "not installed", not an error: ReShade is optional.
pub fn reshade_status() -> ReshadeStatus {
    let common = reshade_layer_dir();
    // 6.8's setup writes the layer straight into `%ProgramData%\ReShade`; the
    // older layout kept the DLL one folder down. Both are a working install.
    let installed = (common.join("ReShade64.dll").is_file()
        || common.join("ReShade64").join("ReShade64.dll").is_file())
        && common.join("ReShade64.json").is_file();
    let game = root().map(|r| game_exe(&r));
    let enabled = installed
        && game
            .as_deref()
            .is_some_and(|exe| reshade_apps_contains(&common.join("ReShadeApps.ini"), exe));
    let configured = game
        .as_deref()
        .and_then(Path::parent)
        .is_some_and(|dir| dir.join("ReShade.ini").is_file());
    let logged = game
        .as_deref()
        .and_then(Path::parent)
        .is_some_and(|dir| dir.join("ReShade.log").is_file());
    ReshadeStatus {
        installed,
        enabled,
        configured,
        logged,
    }
}

/// One preset the release ships, in `engine/fefw-recomp/reshade/Presets`.
pub struct ShipPreset {
    pub name: String,
}

/// The order the presets are offered and cycled in: the general-purpose looks first, the strong stylisations last, so F3/F4
/// walk from a subtle tweak towards a full restyle instead of jumping alphabetically. Names not in this list follow, sorted.
const PRESET_ORDER: [&str; 4] = ["Vivid", "Bright", "RetroCRT", "GameBoy"];

fn preset_rank(name: &str) -> usize {
    PRESET_ORDER
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
        .unwrap_or(PRESET_ORDER.len())
}

/// The presets that travel with the engine, in [`PRESET_ORDER`], from the folder they are kept in.
pub fn ship_presets() -> Vec<ShipPreset> {
    let Some(root) = root() else {
        return Vec::new();
    };
    let dir = root.join("reshade").join("Presets");
    let mut out: Vec<ShipPreset> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("ini")))
            .then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .flatten()
            .map(|name| ShipPreset { name })
        })
        .collect();
    out.sort_by(|a, b| {
        preset_rank(&a.name)
            .cmp(&preset_rank(&b.name))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// The preset `ReShade.ini` names now, without its folder and extension.
pub fn current_reshade_preset() -> Option<String> {
    let dir = game_folder()?;
    let text = std::fs::read_to_string(dir.join("ReShade.ini")).ok()?;
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("PresetPath="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let file = Path::new(value).file_stem()?.to_string_lossy().into_owned();
    (!file.is_empty()).then_some(file)
}

/// Copy the shipped shaders and one preset beside `fefw.exe` and make ReShade load it: `PresetPath` points at the copy,
/// `EffectSearchPaths` at the shader folder, and F3/F4 step through every shipped preset in game (ReShade's own
/// `PresetShortcutPaths`), so a player can compare looks without opening the overlay.
pub fn apply_reshade_preset(name: &str) -> Result<String, String> {
    let root = root().ok_or("the engine folder engine/fefw-recomp was not found")?;
    let dir = game_folder().ok_or("the game is not built yet")?;
    if !dir.join("ReShade.ini").is_file() {
        return Err("ReShade.ini is not beside fefw.exe yet: run Set up ReShade first, and start the game once".into());
    }
    let shipped = root.join("reshade");
    let preset = shipped.join("Presets").join(format!("{name}.ini"));
    if !preset.is_file() {
        return Err(format!("{name} is not one of the shipped presets"));
    }
    let shaders = dir.join("reshade-shaders").join("Shaders");
    std::fs::create_dir_all(&shaders)
        .map_err(|e| format!("creating {}: {e}", shaders.display()))?;
    let mut copied = 0;
    for e in std::fs::read_dir(shipped.join("Shaders"))
        .map_err(|e| format!("reading the shipped shaders: {e}"))?
        .flatten()
    {
        let to = shaders.join(e.file_name());
        std::fs::copy(e.path(), &to).map_err(|e| format!("copying {}: {e}", to.display()))?;
        copied += 1;
    }
    let target = dir.join(format!("{name}.ini"));
    std::fs::copy(&preset, &target).map_err(|e| format!("copying {}: {e}", target.display()))?;

    // The preset list for F3/F4, in folder order, so the keys walk the same sequence the picker shows.
    let mut shortcuts: Vec<String> = Vec::new();
    for p in ship_presets() {
        let copy = dir.join(format!("{}.ini", p.name));
        if !copy.is_file() {
            let _ = std::fs::copy(
                shipped.join("Presets").join(format!("{}.ini", p.name)),
                &copy,
            );
        }
        if copy.is_file() {
            shortcuts.push(format!(
                ".\\{}",
                copy.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
    }

    let ini = dir.join("ReShade.ini");
    let text =
        std::fs::read_to_string(&ini).map_err(|e| format!("reading {}: {e}", ini.display()))?;
    let mut set = |key: &str, value: String, text: &str| -> String {
        let prefix = format!("{key}=");
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        if let Some(line) = lines
            .iter_mut()
            .find(|l| l.trim_start().starts_with(&prefix))
        {
            *line = format!("{prefix}{value}");
        } else {
            // Every key used here lives under [GENERAL] or [INPUT]; place it before the next section.
            let after = lines
                .iter()
                .position(|l| l.trim() == "[INPUT]")
                .unwrap_or(lines.len());
            lines.insert(after, format!("{prefix}{value}"));
        }
        lines.join("\r\n")
    };
    let text = set("PresetPath", format!(".\\{name}.ini"), &text);
    let text = set(
        "EffectSearchPaths",
        r".\reshade-shaders\Shaders".into(),
        &text,
    );
    let text = set("PresetShortcutPaths", shortcuts.join(","), &text);
    // ReShade.ini stores virtual-key codes, not scancodes: its own KeyOverlay=36 is VK_HOME, so F3 is 0x72 (114) and
    // F4 is 0x73 (115).
    let text = set("KeyNextPreset", "114,0,0,0".into(), &text); // F3
    let text = set("KeyPreviousPreset", "115,0,0,0".into(), &text); // F4
    std::fs::write(&ini, text).map_err(|e| format!("writing {}: {e}", ini.display()))?;
    Ok(format!(
        "{name} is loaded: {copied} shaders beside the game, F3 and F4 step through the presets"
    ))
}

/// Whether an INI `Apps=` list contains `game`. Windows paths compare without
/// case and with either slash; the setup tool writes them absolute.
fn reshade_apps_contains(path: &Path, game: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    // The setup writes this file as UTF-8 with a byte order mark, so the first
    // key arrives as `\u{feff}Apps` and would never match.
    let text = text.trim_start_matches('\u{feff}');
    let wanted = game
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("Apps") {
            continue;
        }
        for entry in value.split(',') {
            let entry = entry
                .trim()
                .trim_matches('"')
                .replace('/', "\\")
                .to_ascii_lowercase();
            if entry == wanted {
                return true;
            }
        }
    }
    false
}

/// Find a ReShade setup executable the user already downloaded. This is a
/// convenience only: the Graphics page also opens a file picker, because the
/// user may keep installers on another drive.
pub fn find_reshade_setup() -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Downloads"));
        dirs.push(home.join("Desktop"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if name.starts_with("reshade_setup") && name.ends_with(".exe") {
                return Some(path);
            }
        }
    }
    None
}

/// Launch ReShade's official setup tool for this game's Vulkan executable. The
/// tool asks for administrator approval itself when it registers the Vulkan
/// layer; `--api vulkan` preselects the API so the user only chooses effects and
/// add-ons. No ReShade binary travels with EmblemWeave.
pub fn launch_reshade_setup(setup: &Path) -> Result<(), String> {
    if !setup.is_file() {
        return Err(format!("{} is not there", setup.display()));
    }
    let root = root().ok_or("the engine was not found")?;
    let exe = game_exe(&root);
    if !exe.is_file() {
        return Err("the game is not built yet: build it before setting up ReShade".into());
    }
    Command::new(setup)
        .arg(&exe)
        .args(["--api", "vulkan"])
        .spawn()
        .map_err(|e| format!("starting {}: {e}", setup.display()))?;
    // The setup writes ReShade.ini when it finishes, with the overlay key set to
    // Home and `ForceShortcutModifiers=1` beside it, which makes that key do
    // nothing. Catch the file as it appears and put the modifier rule back, so the
    // first launch already answers Home.
    if let Some(dir) = exe.parent() {
        let ini = dir.join("ReShade.ini");
        std::thread::spawn(move || {
            for _ in 0..240 {
                if ini.is_file() && allow_plain_shortcuts(&ini) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
    }
    Ok(())
}

/// ReShade 6.8 starts a new `ReShade.ini` with `ForceShortcutModifiers=1`, which
/// requires a modifier for every shortcut: the Home key it also writes as the
/// overlay key then opens nothing, and the overlay looks broken. Setting the key
/// back to 0 makes the key the Graphics page names work. Returns whether the file
/// had to change; a missing file or another setting is left alone.
fn allow_plain_shortcuts(ini: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(ini) else {
        return false;
    };
    let mut out = String::with_capacity(text.len() + 2);
    let mut input_section = false;
    let mut changed = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            input_section = trimmed.eq_ignore_ascii_case("[INPUT]");
        }
        if input_section && trimmed.eq_ignore_ascii_case("ForceShortcutModifiers=1") {
            out.push_str("ForceShortcutModifiers=0");
            changed = true;
        } else {
            out.push_str(line);
        }
        out.push_str("\r\n");
    }
    if changed {
        let _ = std::fs::write(ini, out);
    }
    changed
}

/// Whether the game has been built and configured, ready to play.
pub fn ready() -> bool {
    match (root(), run_dir()) {
        (Some(r), Some(d)) => game_exe(&r).is_file() && d.join("fefw.ini").is_file(),
        _ => false,
    }
}

/// Folders of the build tools, put first on PATH for every step. The folder the
/// validated `clang-cl` lives in comes first, so the compiler on PATH is the one
/// that was checked, not whichever LLVM happens to sort first.
pub fn tool_path() -> String {
    crate::toolchain::tool_path()
}

/// Whether `name` answers along the PATH every build step gets.
pub fn which_tool(name: &str) -> bool {
    let mut cmd = Command::new(name);
    cmd.arg("--version")
        .env("PATH", tool_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::toolchain::hidden(&mut cmd)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The PowerShell the engine's scripts run in: PowerShell 7 when it is installed, otherwise the Windows PowerShell that ships with
/// Windows. The scripts use no syntax that only 7 understands, so the second is a working fallback rather than a degraded one.
pub fn shell() -> &'static str {
    if which_tool("pwsh") {
        "pwsh"
    } else {
        "powershell"
    }
}

/// The toolchain, probed once per process: `vswhere`, `clang-cl --version` and
/// `cmake --version` are three processes, and several pages ask for the answer.
fn probe() -> &'static crate::toolchain::Toolchain {
    static CACHE: std::sync::OnceLock<crate::toolchain::Toolchain> = std::sync::OnceLock::new();
    CACHE.get_or_init(crate::toolchain::probe)
}

/// The Visual Studio C++ tools. Every engine script calls `vcvars64.bat`,
/// found through vswhere; without it the whole build dies with a bare exit
/// code, which is what made it look like the launcher's fault.
/// The `vcvars64.bat` of the install the build will use (2022 or newer), or
/// nothing when the newest one is too old. The probe holds every install it
/// found, so an old one can be named instead of reported as absent.
fn vc_tools() -> Option<PathBuf> {
    probe().vs.as_ref().map(|v| v.vcvars())
}

/// The newest C++ tools whatever their version, so a too-old install can be named rather than reported as absent.
pub fn vc_tools_any() -> Option<(PathBuf, u32)> {
    probe().vs_all.first().map(|v| (v.vcvars(), v.major))
}

/// The highest .NET SDK major version installed, from `dotnet --list-sdks`.
///
/// A .NET *runtime* also ships `dotnet.exe`, so its presence says nothing about
/// whether a project can be built. Without this check a PC with only the
/// runtime got as far as the shader harness and died with
/// `0x8000809b` / "sdk-not-found", which names neither the missing piece nor
/// what to install (issue 10).
pub fn dotnet_sdk_major() -> Option<u32> {
    let mut cmd = Command::new("dotnet");
    cmd.args(["--list-sdks"])
        .env("PATH", tool_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    sdk_major_from(&String::from_utf8_lossy(&out.stdout))
}

/// Reads the newest major version out of the `dotnet --list-sdks` listing:
/// one `MAJOR.MINOR.PATCH [folder]` line per SDK.
fn sdk_major_from(listing: &str) -> Option<u32> {
    listing
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|v| v.split('.').next())
        .filter_map(|major| major.parse::<u32>().ok())
        .max()
}

/// The dependencies that stop the build, by registry name. It reads the
/// report the background thread warms at start-up instead of running the checks
/// itself: the Overview draws this every frame, and `vswhere`, `dotnet` and
/// `vcvars64.bat` are not free. Until the first report exists this is empty,
/// which claims nothing.
pub fn missing_tools() -> Vec<&'static str> {
    let rows = crate::deps::cached().unwrap_or_default();
    crate::deps::blocking_for(&rows, crate::deps::Stage::Build)
        .into_iter()
        .map(|r| r.dep.name)
        .collect()
}

/// The winget line that installs Visual Studio's C++ tools. It is the one tool
/// the build cannot do without, the largest download, and the hardest for a
/// player to find, so the launcher can run it instead of describing it.
pub const VS_INSTALL: &str = r#"winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" --accept-package-agreements --accept-source-agreements"#;

/// Whether winget is there to run that line at all.
pub fn have_winget() -> bool {
    tool_path()
        .split(';')
        .any(|d| Path::new(d).join("winget.exe").is_file())
}

/// The runnable install lines for a list of registry names.
pub fn install_lines_for(tools: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in tools {
        let Some(install) = crate::deps::DEPS
            .iter()
            .find(|d| d.name == *tool)
            .and_then(|d| d.install)
        else {
            continue;
        };
        if !install.runnable() {
            continue;
        }
        let line = match install {
            crate::deps::Install::Winget(l) if !l.contains("--accept-package-agreements") => {
                format!("{l} --accept-package-agreements --accept-source-agreements")
            }
            other => other.line().to_string(),
        };
        if !out.contains(&line) {
            out.push(line);
        }
    }
    out
}

/// A script that runs the given lines one after another in one console, so the
/// user answers one permission prompt per installer instead of hunting for each
/// command. A failing line does not stop the rest.
pub fn install_script_lines(lines: &[String]) -> String {
    let mut s = String::from("@echo off\r\ntitle EmblemWeave: build tools\r\n");
    s.push_str("echo Installing what EmblemWeave needs. Accept the administrator prompt when Windows shows it.\r\n");
    s.push_str("echo.\r\n");
    for line in lines {
        s.push_str(&format!("echo === {line} ===\r\n{line}\r\nif errorlevel 1 echo That one did not finish.\r\necho.\r\n"));
    }
    s.push_str(
        "echo Done. Close this window, then press Check again on Requirements.\r\npause\r\n",
    );
    s
}

/// Write a one-off installer script for these lines and run it in its own
/// console, where winget can show progress and raise its prompts.
pub fn start_install_lines(lines: &[String]) -> Result<PathBuf, String> {
    if lines.is_empty() {
        return Ok(PathBuf::new());
    }
    // The lines are winget commands and a .cmd script. Off Windows the launcher refuses instead of
    // running them, so a build for another platform cannot install anything by accident.
    if !crate::platform::install_supported() {
        return Err(crate::platform::install_unsupported_reason());
    }
    let dir = Config::work_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let script = dir.join("install-build-tools.cmd");
    std::fs::write(&script, install_script_lines(lines))
        .map_err(|e| format!("writing {}: {e}", script.display()))?;
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "start", "", &script.display().to_string()])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, so only the installer console shows
    }
    cmd.spawn()
        .map_err(|e| format!("starting the installer: {e}"))?;
    Ok(script)
}

/// What the button on Overview runs: the registry's install lines for every
/// dependency the preflight says is absent or too old.
pub fn start_install() -> Result<PathBuf, String> {
    if !have_winget() {
        return Err("winget is not available on this PC, so the installer cannot be started. Install App Installer from the Microsoft Store, or follow the install lines in docs/TROUBLESHOOTING.md.".into());
    }
    let lines = install_lines_for(&missing_tools());
    if lines.is_empty() {
        return Err("The remaining build tool needs a manual install. Follow the steps on Requirements, then press Check again.".into());
    }
    start_install_lines(&lines)
}

/// The CMake version this PC has, as (major, minor). The engine's projects ask
/// for 3.25; an older one stops with "CMake Generate step failed. Build files
/// cannot be regenerated correctly", which says nothing about the cause.
pub fn cmake_version() -> Option<(u32, u32)> {
    let version = probe().cmake.as_deref()?;
    let parts = crate::toolchain::version_parts(version)?;
    Some((*parts.first()?, *parts.get(1)?))
}

/// What installs a dependency, by registry name: the same line the Requirements
/// page shows, so the message in a log and the button in the interface agree.
pub fn install_hint(tool: &str) -> &'static str {
    crate::deps::DEPS
        .iter()
        .find(|d| d.name == tool)
        .and_then(|d| d.install)
        .map(|i| i.line())
        .unwrap_or("see docs/TROUBLESHOOTING.md")
}

/// What the engine's Python probe found. The executable path is not enough: a
/// `tools/.venv` left by another release or Python install can still contain
/// `python.exe` while that interpreter cannot start (the `python311.dll`
/// conflict in issue #43), silently runs a different interpreter, or has lost
/// some of its pinned packages.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PythonEnv {
    Healthy { version: String, executable: String },
    Missing,
    Unusable { why: String },
}

impl PythonEnv {
    fn reason(&self) -> String {
        match self {
            PythonEnv::Healthy { .. } => "ready".into(),
            PythonEnv::Missing => "the interpreter is missing".into(),
            PythonEnv::Unusable { why } => why.clone(),
        }
    }
}

/// The probe prints one machine-readable line after checking the interpreter
/// identity, its virtual-environment root, its version and every lifter import.
/// Python computes `identity` itself so Windows path normalisation (case,
/// short names, junctions) is the platform's own rather than a string compare
/// here. `sys.prefix != sys.base_prefix` rejects a global interpreter that was
/// copied into `tools/.venv`; `sys.executable` names what actually ran. `socket`
/// is the extension that exposed a Python 3.11 `_socket.pyd` from PYTHONPATH
/// only when the build ran (issue 43); importing it here checks the venv itself.
const PYTHON_PROBE: &str = r#"import json, os, sys
modules = ("capstone", "lz4", "zstandard", "zbic", "elftools", "unicorn", "socket")
errors = []
for name in modules:
    try:
        __import__(name)
    except BaseException as exc:
        errors.append("{}: {}: {}".format(name, type(exc).__name__, exc))
expected = os.path.normcase(os.path.realpath(sys.argv[1]))
actual = os.path.normcase(os.path.realpath(sys.executable))
print("EW_PY_PROBE " + json.dumps({
    "version": "{}.{}.{}".format(*sys.version_info[:3]),
    "executable": sys.executable,
    "venv": sys.prefix != sys.base_prefix,
    "identity": actual == expected,
    "imports": errors,
}))
"#;

/// Parse the probe's stdout without starting Python, so healthy and unusable
/// environments can be tested on every platform.
fn parse_python_probe(stdout: &str, stderr: &str) -> PythonEnv {
    let Some(line) = stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("EW_PY_PROBE "))
    else {
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .or_else(|| stdout.lines().rev().find(|l| !l.trim().is_empty()))
            .unwrap_or("the interpreter printed nothing");
        return PythonEnv::Unusable {
            why: format!(
                "the interpreter did not report its version: {}",
                detail.trim()
            ),
        };
    };
    let Ok(report) = serde_json::from_str::<serde_json::Value>(line) else {
        return PythonEnv::Unusable {
            why: "the interpreter's probe report is not valid JSON".into(),
        };
    };
    let version = report
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let executable = report
        .get("executable")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if version.is_empty() || executable.is_empty() {
        return PythonEnv::Unusable {
            why: "the interpreter's probe report does not name its version and executable".into(),
        };
    }
    let parts = crate::toolchain::version_parts(&version).unwrap_or_default();
    if parts.len() < 2 || (parts[0], parts[1]) < (3, 9) {
        return PythonEnv::Unusable {
            why: format!("Python {version} is older than the engine's 3.9 minimum"),
        };
    }
    if !report
        .get("venv")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return PythonEnv::Unusable {
            why: format!("Python {version} at {executable} is not a virtual environment"),
        };
    }
    if !report
        .get("identity")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return PythonEnv::Unusable {
            why: format!("the interpreter reports {executable}, not the tools\\.venv python.exe the launcher started"),
        };
    }
    let imports: Vec<String> = report
        .get("imports")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !imports.is_empty() {
        return PythonEnv::Unusable {
            why: format!(
                "required Python module(s) do not import: {}",
                imports.join("; ")
            ),
        };
    }
    PythonEnv::Healthy {
        version,
        executable,
    }
}

/// Probe the existing interpreter before reusing it. A missing file is not an
/// error: it is the normal first build.
fn probe_python_env(python: &Path) -> PythonEnv {
    if !python.is_file() {
        return PythonEnv::Missing;
    }
    let output = Command::new(python)
        .args(["-I", "-c", PYTHON_PROBE])
        .arg(python)
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) => parse_python_probe(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(e) => PythonEnv::Unusable {
            why: format!("could not start {}: {e}", python.display()),
        },
    }
}

/// Remove only the generated engine environment, never a global Python or an
/// arbitrary directory. The path is built from the engine root, and a link is
/// refused because deleting it could mean deleting its target.
fn remove_engine_venv(root: &Path) -> Result<(), String> {
    let tools = root.join("tools");
    let venv = tools.join(".venv");
    match std::fs::symlink_metadata(&venv) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("checking {}: {e}", venv.display())),
        Ok(m) if m.file_type().is_symlink() => Err(format!(
            "{} is a link, not the engine's generated environment; remove the link yourself and press Build the game again",
            venv.display()
        )),
        Ok(m) if !m.is_dir() => Err(format!("{} is not a directory; remove it yourself and press Build the game again", venv.display())),
        Ok(_) => std::fs::remove_dir_all(&venv).map_err(|e| format!("removing the unusable {}: {e}", venv.display())),
    }
}

/// Run one command in `cwd`, its output (stdout and stderr) into the job
/// log, with no console window. Cancelling the job stops it.
fn run(ctx: &Ctx, cwd: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    run_env(ctx, cwd, program, args, &[])
}

/// The line of a failed run that explains it. Build tools end with a summary
/// ("CMake Generate step failed. Build files cannot be regenerated correctly")
/// that hides the error several lines above it.
/// Whether a line of build output is an error rather than a progress note.
fn error_line(l: &str) -> bool {
    // "-- Configuring incomplete, errors occurred!" mentions errors without
    // being one, so a plain substring search finds the wrong line.
    let t = l.trim();
    let lower = t.to_lowercase();
    // cmd wraps "'git' is not recognized …" onto a second line, so the last
    // two lines of such a run are "…subcommand failed. | operable program
    // or batch file." — the name of the missing tool would be lost.
    !t.starts_with("--")
        && (lower.starts_with("error")
            || lower.contains("cmake error")
            || lower.contains("error:")
            || lower.contains(": error")
            || lower.contains("fatal error")
            || lower.contains("is not recognized as an internal or external command")
            || lower.contains("is not recognized as the name of a cmdlet"))
}

fn reason_from(lines: &[String]) -> String {
    match lines.iter().find(|l| error_line(l)) {
        Some(err) => err.clone(),
        None => lines
            .iter()
            .rev()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// The first error in a whole log, with the lines that follow it. CMake keeps
/// configuring after an error in one directory and prints "Configuring
/// incomplete" thousands of lines later, so the tail of a failed build often
/// names the summary and nothing else. This finds the message itself.
pub fn first_error_block(text: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| !error_line(l));
    let head = lines.next()?.trim().to_string();
    let mut block = vec![head];
    for l in lines.take(6) {
        // Stop at the next progress line: the error's own text never starts with `--`.
        if l.trim_start().starts_with("--") {
            break;
        }
        if !l.trim().is_empty() && block.len() < 7 {
            block.push(l.trim_end().to_string());
        }
        if block.len() >= 7 {
            break;
        }
    }
    Some(block.join("\n"))
}

/// What `--verify-log` prints for one log: how long it is, the first error with
/// its context, the summary it ended with, and which repair the launcher would
/// try for it. Text in, text out, so it can be checked without a build.
pub fn verify_log_text(text: &str) -> String {
    let mut out = format!("{} lines\n", text.lines().count());
    match first_error_block(text) {
        Some(block) => {
            out.push_str("\nfirst error:\n");
            out.push_str(&block);
            out.push('\n');
        }
        None => out.push_str("\nno error line in this log: either it finished, or the step was stopped before it printed anything\n"),
    }
    let all: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    if !all.is_empty() {
        out.push_str(&format!("\nlast line: {}\n", all[all.len() - 1].trim()));
    }
    let mut repairs: Vec<&str> = Vec::new();
    if half_written_deps(text) {
        repairs.push("a fetched graphics dependency is half written; the launcher clears _deps and fetches again");
    }
    if stale_build_cache(text) {
        repairs.push("the build folder holds a CMake cache from another toolchain; the launcher clears it and configures again");
    }
    if repairs.is_empty() {
        out.push_str("\nno automatic repair matches this log\n");
    } else {
        out.push_str("\nrepair:\n");
        for r in repairs {
            out.push_str(&format!("- {r}\n"));
        }
    }
    out
}

/// Lines from a child's pipe, tolerating bytes that are not UTF-8. `BufRead::lines`
/// stops at the first bad byte and every line after it is lost — which is how a
/// CMake error block disappears from a log whose summary line still arrives.
fn stream_lines(reader: impl std::io::Read, mut each: impl FnMut(String)) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match std::io::BufRead::read_until(&mut reader, b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
                    buf.pop();
                }
                each(String::from_utf8_lossy(&buf).into_owned());
            }
        }
    }
}

/// How a step is named in a failure: the script it ran, not the flag before
/// it (`pwsh -NoProfile` says nothing about which step died).
#[cfg(test)]
fn step_name(program: &str, args: &[&str]) -> String {
    if let Some(i) = args.iter().position(|a| *a == "-File") {
        if let Some(script) = args.get(i + 1) {
            return (*script).to_string();
        }
    }
    args.first()
        .map(|a| (*a).to_string())
        .unwrap_or_else(|| program.to_string())
}

/// The program cmd says it could not find: "'git' is not recognized as an
/// internal or external command".
fn missing_program(reason: &str) -> Option<String> {
    let lower = reason.to_lowercase();
    if !lower.contains("is not recognized") {
        return None;
    }
    let start = reason.find('\'')? + 1;
    let end = reason[start..].find('\'')? + start;
    Some(reason[start..end].to_string())
}

/// A failed build that a cache clear can repair: a cache made by another compiler or CMake, or
/// Ninja's "manifest 'build.ninja' still dirty after 100 tries" loop (issue #34). The latter is
/// normally prevented by CMAKE_SUPPRESS_REGENERATION in build_game.cmd; the retry keeps an old
/// extracted folder usable when its build.ninja is already trapped in that loop.
fn needs_cache_reset(error: &str) -> bool {
    let lower = error.to_lowercase();
    [
        "cmakecache.txt",
        "cache was made by another compiler",
        "cmake generate step failed",
        "build files cannot be regenerated",
        "still dirty after",
        "manifest 'build.ninja'",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Characters cmd.exe splits a command line at, quotes or no quotes. A project folder that contains one truncates the
/// `cmd /C "cd . && llvm-lib … object …"` rule CMake generates, and ninja then reports a path cut at the character
/// (`CMakeFiles\G_\Games\PC_Ports_`) instead of naming the folder.
const CMD_BREAKERS: [char; 8] = ['&', '|', '<', '>', '^', '(', ')', '%'];

/// The first character in `path` that cmd.exe cannot carry through a build, if any.
pub fn cmd_breaker(path: &Path) -> Option<char> {
    path.to_string_lossy()
        .chars()
        .find(|c| CMD_BREAKERS.contains(c))
}

/// The folder the game is built from, as the build's commands spell it.
///
/// A path containing a character cmd.exe splits a command line at is reached through a directory junction in the temporary
/// folder, whose spelling has none of those characters. An 8.3 short name is not enough on its own: CMake writes a custom
/// command's `WORKING_DIRECTORY` (the engine's lift command sets it to the checkout root, NxRecomp.cmake) and a target's
/// output folder as absolute paths, and it turns a short name back into the real long spelling there. The junction gives
/// the same files another name, so the working directory the build actually runs in has no separator in it, and every
/// path CMake generates from it stays that way. It is removed when the build is over; the checkout, saves and build
/// outputs never move.
#[derive(Debug)]
struct BuildDir {
    /// What the build commands run in.
    path: PathBuf,
    /// The junction to remove, when one was made.
    link: Option<PathBuf>,
}

impl Drop for BuildDir {
    fn drop(&mut self) {
        if let Some(link) = self.link.take() {
            // Removing a junction removes the name, not the folder it points at: the build outputs stay where they are.
            let _ = std::fs::remove_dir(link);
        }
    }
}

fn build_dir(root: &Path) -> Result<BuildDir, String> {
    if cmd_breaker(root).is_none() {
        return Ok(BuildDir {
            path: root.to_path_buf(),
            link: None,
        });
    }
    #[cfg(windows)]
    if let Some(link) = junction_view(root) {
        return Ok(BuildDir {
            path: link.clone(),
            link: Some(link),
        });
    }
    let bad = cmd_breaker(root).unwrap_or('?');
    Err(format!(
        "the project is in a folder whose path contains '{bad}':\n\n{}\\n\ncmd.exe splits its command line at & | < > ^ ( ) %, even inside quotes, so the linker and lift steps CMake generates \
         for the lifted code would be cut in half and ninja would report a path that does not exist (\"CMakeFiles\\G_\\Games\\
         PC_Ports_\").\n\nEmblemWeave builds such a folder through a temporary junction, \
         but one could not be made here (the temporary folder is unavailable, or creating the junction was refused). Move EmblemWeave to a folder whose path has none of & | < > ^ ( ) %, for example \
         C:\\EmblemWeave, then press Build the game again. An existing game folder can \
         be moved as it is; only paths outside it are written into its ini.",
        root.display()
    ))
}

/// A junction in the temporary folder that names `root` without the characters cmd.exe splits at, or nothing when one
/// cannot be made. The name is derived from the real path, so the same checkout reuses the same junction across builds;
/// a leftover from a killed build is replaced.
#[cfg(windows)]
fn junction_view(root: &Path) -> Option<PathBuf> {
    let temp = std::env::temp_dir();
    if !temp.is_dir() || cmd_breaker(&temp).is_some() {
        return None;
    }
    let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // FNV-1a over the real path: stable across runs, and only a naming hint, never a security decision.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in real.to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for attempt in 0..4 {
        let name = if attempt == 0 {
            format!("emblemweave-build-{hash:016x}")
        } else {
            format!("emblemweave-build-{hash:016x}-{attempt}")
        };
        let link = temp.join(name);
        match std::fs::symlink_metadata(&link) {
            // Only a link is removed: an unrelated real folder with this name is left alone, and the next name is tried.
            Ok(meta) if meta.is_dir() && meta.file_type().is_symlink() => {
                let _ = std::fs::remove_dir(&link);
            }
            Ok(_) => continue,
            Err(_) => {}
        }
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::toolchain::hidden(&mut cmd);
        if cmd.status().is_ok_and(|s| s.success()) && link.is_dir() {
            return Some(link);
        }
    }
    None
}

fn run_env(
    ctx: &Ctx,
    cwd: &Path,
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        // `-ExecutionPolicy Bypass` covers the script this call starts, not a
        // `pwsh` that script starts in turn: PowerShell children read this
        // preference from the environment, so the whole chain runs.
        .env("PSExecutionPolicyPreference", "Bypass")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The environment the toolchain was validated with, not the one the
    // launcher inherited: `CC`, `CXX`, `INCLUDE`, `LIB` and `VCINSTALLDIR` from
    // an older shell would point the build at headers nobody checked. The
    // Python flags keep user-site packages out of the build's interpreter too;
    // the probe runs isolated, so this makes the build match it.
    crate::toolchain::scrub(&mut cmd);
    cmd.env("PYTHONNOUSERSITE", "1");
    match crate::toolchain::init() {
        Ok(resolved) => {
            for (k, v) in &resolved.env {
                cmd.env(k, v);
            }
        }
        Err(_) => {
            cmd.env("PATH", tool_path());
        }
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let command = format!("{program} {}", args.join(" "));
    let started = std::time::Instant::now();
    ctx.log(format!("> {command}"));
    ctx.log(format!("  in {}", cwd.display()));
    // The last lines a failed step printed: the log has everything, but the
    // message is what the user reads, and "exit code 1" alone is not a reason.
    let tail: Tail = Default::default();
    // The first error anywhere in the run, with its context. A configure that
    // fails in one directory and prints the summary at the end hides the message
    // far above the tail, which is exactly how "Configuring incomplete, errors
    // occurred!" ends up being the only thing a report says.
    let first_error: FirstError = Default::default();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("starting {command}: {e}"))?;
    // Both streams are drained on their own threads, so the wait below can
    // notice a cancel while a long configure is still printing: reading stdout
    // inline would block until the process ends, which is how Cancel used to do
    // nothing until the step it was cancelling had finished.
    let mut readers = Vec::new();
    for pipe in [
        child.stdout.take().map(Either::Out),
        child.stderr.take().map(Either::Err),
    ]
    .into_iter()
    .flatten()
    {
        let ctx = ctx.clone();
        let tail = tail.clone();
        let first = first_error.clone();
        readers.push(std::thread::spawn(move || match pipe {
            Either::Out(out) => stream_lines(out, |line| {
                ctx.log(line.clone());
                remember(&tail, &first, &line);
            }),
            Either::Err(err) => stream_lines(err, |line| {
                ctx.log(line.clone());
                remember(&tail, &first, &line);
            }),
        }));
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if ctx.cancelled() {
                    // The whole tree: cmd starts cmake, which starts ninja,
                    // which starts one clang-cl per core. Killing the child
                    // alone leaves those compiling into the build folder after
                    // the launcher has moved on.
                    kill_tree(child.id(), &mut |m| ctx.log(m));
                    let _ = child.wait();
                    for r in readers {
                        let _ = r.join();
                    }
                    return Err("cancelled".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            Err(e) => return Err(format!("waiting for {command}: {e}")),
        }
    };
    for r in readers {
        let _ = r.join();
    }
    ctx.log(format!(
        "< exit {} in {:.1}s: {command}",
        status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "(no code)".into()),
        started.elapsed().as_secs_f32()
    ));
    if ctx.cancelled() {
        return Err("cancelled".into());
    }
    if status.success() {
        Ok(())
    } else {
        // The reason, not the last line: build tools end a failed run with a
        // summary ("Build files cannot be regenerated correctly") that hides the
        // error several lines above it.
        let lines: Vec<String> = tail
            .lock()
            .map(|t| t.iter().filter(|l| !l.trim().is_empty()).cloned().collect())
            .unwrap_or_default();
        let said = reason_from(&lines);
        let block: Vec<String> = first_error
            .lock()
            .ok()
            .and_then(|b| b.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        if said.is_empty() {
            // Nothing at all was printed. A script that security software or a
            // machine policy kills exits exactly like this, so saying "the log
            // has the output" would point at an empty file.
            Err(format!(
                "`{command}` failed ({status}) and printed nothing at all. That is what a script looks like when machine \
                 policy or security software stops it before its first line; the log has the command and the lines around it"
            ))
        } else if let Some(missing) =
            missing_program(&said).or_else(|| missing_program(&block.join(" ")))
        {
            // The reason first: a step can print "'vswhere.exe' is not recognized" as a warning near the top and still
            // fail later for another reason, and naming vswhere as the cause sent a real build failure (the SDK's
            // rc.exe hanging) to an install instruction that changed nothing.
            Err(format!(
                "`{command}` called `{missing}`, which is not installed or not on PATH (exit {status}). Install it, restart \
                 the launcher, then press Build the game again"
            ))
        } else if !block.is_empty() {
            // The error that was printed, wherever in the run it was printed,
            // under the command that printed it.
            Err(format!(
                "`{command}` failed ({status}): {}",
                block.join(" | ")
            ))
        } else {
            Err(format!("`{command}` failed ({status}): {said}"))
        }
    }
}

/// A held build lock: the file names the process that owns the build folder, so
/// a second launcher can tell a live build from one that was killed.
struct BuildLock {
    path: PathBuf,
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Take the workspace build lock, or say who holds it. Two builds at once write
/// the same object files and unpack the same dependencies into the same folders;
/// the second is refused instead of being allowed to corrupt the first.
fn lock_build(root: &Path) -> Result<BuildLock, String> {
    let dir = root.join("work").join("build");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join(".build.lock");
    let held = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| t.split_whitespace().next()?.parse::<u32>().ok());
    if let Some(pid) = held {
        if pid == std::process::id() {
            // A previous run of this same process that never released it.
        } else if process_running(pid) {
            return Err(format!(
                "another EmblemWeave build is already running (process {pid}) and is using this engine folder. Wait for it \
                 to finish; if no build window is open, end that process or delete {}",
                path.display()
            ));
        } else {
            // The owner is gone: a killed launcher leaves its lock behind.
        }
    }
    std::fs::write(
        &path,
        format!(
            "{} {} {}\n",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ),
    )
    .map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(BuildLock { path })
}

/// Whether a pid is alive, asked of Windows rather than guessed.
fn process_running(pid: u32) -> bool {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()),
        // No tasklist: assume it is running rather than stomp on it.
        Err(_) => true,
    }
}

/// Either stream of a child process, so both are drained the same way.
enum Either {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

/// Stop a process and everything it started. `taskkill /T` walks the tree, so a
/// cancelled build does not leave ninja and a core's worth of `clang-cl` writing
/// into the build folder after the launcher has moved on.
fn kill_tree(pid: u32, log: &mut impl FnMut(String)) {
    if pid == 0 {
        return;
    }
    log(format!("cancelling: stopping the process tree of {pid}"));
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd.status();
}

type Tail = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>>;
type FirstError = std::sync::Arc<std::sync::Mutex<Option<Vec<String>>>>;

/// Keep the last lines, and the first error with the lines that follow it.
fn remember(tail: &Tail, first: &FirstError, line: &str) {
    if let Ok(mut t) = tail.lock() {
        t.push_back(line.to_string());
        while t.len() > 4000 {
            t.pop_front();
        }
    }
    if let Ok(mut f) = first.lock() {
        match f.as_mut() {
            None => {
                if error_line(line) {
                    *f = Some(vec![line.to_string()]);
                }
            }
            Some(block) => {
                if block.len() < 8 {
                    block.push(line.to_string());
                }
            }
        }
    }
}

/// Run the fefw build, repairing each of the two failures it can repair once:
/// a dependency fetched half written, and a build folder whose CMake cache names
/// a toolchain that is not installed any more.
fn build_with_repairs(
    builds: &Path,
    mut build: impl FnMut() -> Result<(), String>,
    mut log: impl FnMut(&str),
) -> Result<(), String> {
    let mut repaired_deps = false;
    let mut repaired_cache = false;
    let mut repaired_driver = false;
    loop {
        let e = match build() {
            Ok(()) => break,
            Err(e) => e,
        };
        if !repaired_deps && half_written_deps(&e) {
            repaired_deps = true;
            log("a fetched dependency is incomplete; clearing it and fetching again");
            for config in std::fs::read_dir(builds).into_iter().flatten().flatten() {
                let _ = std::fs::remove_dir_all(config.path().join("_deps"));
            }
            continue;
        }
        if !repaired_cache && stale_build_cache(&e) {
            repaired_cache = true;
            log("the build folder was made by a different toolchain; clearing its CMake cache and configuring again");
            for config in std::fs::read_dir(builds).into_iter().flatten().flatten() {
                let _ = std::fs::remove_file(config.path().join("CMakeCache.txt"));
                let _ = std::fs::remove_dir_all(config.path().join("CMakeFiles"));
            }
            continue;
        }
        // A folder CMake keeps as clang++ (the GNU driver) receives clang-cl flags such as -clang: and /W3 and stops in
        // dav1d (issue 9). The script starts such a folder over, but a configure that swaps the compiler after its first
        // pass can still leave one behind; throw the configuration folders away and let the script configure from nothing.
        if !repaired_driver && driver_flags_rejected(&e) {
            repaired_driver = true;
            log("the build folder is configured for the clang++ driver and rejects clang-cl flags; deleting it and compiling again");
            for config in std::fs::read_dir(builds).into_iter().flatten().flatten() {
                let _ = std::fs::remove_dir_all(config.path());
            }
            continue;
        }
        return Err(e);
    }
    Ok(())
}

/// A fetch that stopped halfway leaves sources CMake then fails on: it names the
/// `_deps` folder, the patch check, or the stale image patch.
fn half_written_deps(reason: &str) -> bool {
    reason.contains("_deps")
        || reason.contains("imgui_checked.cmake")
        || reason.contains("failure patch no longer matches pinned source")
}

/// CMake refusing to work in a build folder it did not create: a compiler or a
/// generator that is gone, or a cache from a different CMake.
fn stale_build_cache(reason: &str) -> bool {
    reason.contains("CMakeCache.txt")
        || reason.contains("does not match the generator used previously")
        || reason.contains("CMAKE_CXX_COMPILER")
        || reason.contains("CMAKE_C_COMPILER")
        || reason.contains("is not a full path")
}

/// The two error lines a clang++ build prints when it is handed the flags this project writes for clang-cl.
fn driver_flags_rejected(reason: &str) -> bool {
    reason.contains("unknown argument: '-clang:")
        || reason.contains("no such file or directory: '/W3'")
        || reason.contains("unknown argument: '/W3'")
        || reason.contains("unknown argument: '-fmacro-prefix-map")
}

/// What the shader translation has actually produced, block by block.
///
/// The corpus lists every distinct code block the game has (12 088 for this
/// dump); the translator writes `<hash>.spv` for each it can translate and one
/// line per block in `results.tsv`, at the end of the run. A folder that holds a
/// few hundred files and no report is a run that was interrupted, not a finished
/// cache, and a game started on it draws nothing.
struct ShaderReport {
    expected: usize,
    translated: usize,
    /// Attempted, and the translator says it cannot handle the block (compute
    /// shaders with instructions it does not cover). These never get a .spv and
    /// are not a reason to translate again.
    failed: usize,
    /// No .spv and no line in results.tsv: the work that is still to do.
    missing: Vec<String>,
}

impl ShaderReport {
    fn failed_summary(&self) -> String {
        if self.failed == 0 {
            String::new()
        } else {
            format!(
                "; {} blocks are beyond the translator (listed in results.tsv)",
                self.failed
            )
        }
    }
}

/// The folder the shader corpus is read from.
///
/// Setup accepts a `.nup` and the game reads its RomFS in place from it, so
/// `romfs` is a file. The extractor walks a folder: given a file it finds nothing
/// and the corpus comes out as only the shaders inside `main`, a few hundred
/// blocks. That is what 0.1.10 shipped to the players in issues #12 and #15, whose
/// logs show 151 cache records and 598 programs with no translated shader. The
/// archives that carry shaders are copied out of the `.nup` instead, once per game
/// version, and the extractor gets a folder.
/// The RomFS the game can actually read. A `.nup` of stored files is served from the package itself; one packed with zstd cannot be,
/// because a zstd entry is a single frame with no random access into it, so its RomFS is written out into `work/romfs` once and the game
/// reads that folder. Without this a packed package built, then died two seconds into play on `main.npdm is compressed in the .nup`
/// (issue #24); the work is done here, where there is a job to report it, instead of being left to the player.
fn romfs_folder_for_play(root: &Path, romfs: &Path, ctx: &Ctx) -> Result<Option<PathBuf>, String> {
    if romfs.is_dir() {
        return Ok(None);
    }
    let Some((count, bytes)) = crate::gamedata::packed_romfs(romfs) else {
        return Ok(None);
    };
    let dest = root.join("work").join("romfs-overlay");
    let stamp = dest.join(".written");
    let key = format!("{}|{count} files|{bytes} bytes", romfs.display());
    if std::fs::read_to_string(&stamp)
        .map(|t| t.trim() == key.trim())
        .unwrap_or(false)
    {
        ctx.log(format!(
            "game files: {count} packed files; using the overlay already written in {}",
            dest.display()
        ));
        return Ok(Some(dest));
    }
    ctx.log(format!(
        "game files: {count} files of this package ({:.1} GB) are packed with zstd, which the engine cannot read in place, so those files are written once into {} and read from there; the package is used as it is for everything else",
        bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        dest.display()
    ));
    std::fs::create_dir_all(&dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    let (written, total) = crate::gamedata::unpack_packed_romfs(romfs, &dest, &mut |m| {
        ctx.log(format!("game files: {m}"))
    })?;
    std::fs::write(&stamp, &key).map_err(|e| format!("writing {}: {e}", stamp.display()))?;
    ctx.log(format!(
        "game files: {written} packed files written ({:.1} GB) to {}; the package supplies the rest",
        total as f64 / (1024.0 * 1024.0 * 1024.0),
        dest.display()
    ));
    Ok(Some(dest))
}

fn shader_romfs(root: &Path, romfs: &Path) -> Result<(PathBuf, Option<(usize, u64)>), String> {
    if romfs.is_dir() {
        return Ok((romfs.to_path_buf(), None));
    }
    if ew_format::nx::is_nx(romfs) {
        return shader_romfs_nx(root, romfs);
    }
    let nup = ew_format::nup::Nup::open(romfs)
        .map_err(|e| format!("reading {} as a .nup: {e}", romfs.display()))?;
    let Some(pkg) = nup.program_package().cloned() else {
        return Err(format!(
            "{} holds no base game or update, so its shaders cannot be read",
            romfs.display()
        ));
    };
    let out = root.join("work").join("shaders").join("romfs");
    let stamp = out.join(".source");
    let want = format!(
        "{}|{} files|v{}",
        romfs.display(),
        nup.files.len(),
        pkg.version
    );
    if stamp.is_file()
        && std::fs::read_to_string(&stamp)
            .map(|t| t.trim().to_string())
            .ok()
            .as_deref()
            == Some(want.as_str())
    {
        return Ok((out, None));
    }
    // The extractor looks for these by name; every other romfs file is skipped, so
    // the copy is 1.4 GiB of archives, not the game's 30 GB.
    const EXTS: [&str; 5] = [".bfsha", ".ptcl", ".arc", ".sarc", ".bnsh"];
    let _ = std::fs::remove_dir_all(&out);
    let mut copied = 0usize;
    let mut bytes = 0u64;
    for (rel, index) in nup.walk(pkg.romfs) {
        let low = rel.to_ascii_lowercase();
        let base = low.strip_suffix(".zst").unwrap_or(&low);
        if !EXTS.iter().any(|e| base.ends_with(e)) {
            continue;
        }
        // `read` decompresses, so the copy keeps the real name and the extractor sees
        // the archive instead of a .zst it would have to decompress again.
        let rel = if low.ends_with(".zst") {
            rel[..rel.len() - 4].to_string()
        } else {
            rel
        };
        let dst = out.join(&rel);
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        let data = nup
            .read(index)
            .map_err(|e| format!("reading {rel} from {}: {e}", romfs.display()))?;
        bytes += data.len() as u64;
        std::fs::write(&dst, &data).map_err(|e| format!("writing {}: {e}", dst.display()))?;
        copied += 1;
    }
    if copied == 0 {
        return Err(format!(
            "{} holds no shader archive ({EXTS:?}), so no shaders can be translated",
            romfs.display()
        ));
    }
    std::fs::write(&stamp, &want).map_err(|e| format!("writing {}: {e}", stamp.display()))?;
    Ok((out, Some((copied, bytes))))
}

/// The shader archives of an .nx RomFS: the same five extensions read straight out of the archive's blocks (zstd entries are
/// decompressed as they are read), so the archive is never copied and the engine's own reader stays the only thing that parses it.
fn shader_romfs_nx(root: &Path, romfs: &Path) -> Result<(PathBuf, Option<(usize, u64)>), String> {
    let nx = ew_format::nx::Nx::open(romfs)
        .map_err(|e| format!("reading {}: {e:#}", romfs.display()))?;
    let app = crate::gamedata::known_program_id()
        .and_then(|id| nx.application(id))
        .or_else(|| (nx.applications().len() == 1).then_some(0))
        .ok_or_else(|| format!("{} holds no application for this game", romfs.display()))?;
    let snap = nx
        .current(app)
        .ok_or_else(|| format!("{} holds no snapshot for this game", romfs.display()))?;
    let version = nx.snapshots()[snap].version;
    let tree = nx.tree(snap).map_err(|e| format!("{e:#}"))?;
    let out = root.join("work").join("shaders").join("romfs");
    let stamp = out.join(".source");
    let want = format!("{}|{} files|v{version}", romfs.display(), tree.len());
    if stamp.is_file()
        && std::fs::read_to_string(&stamp)
            .map(|t| t.trim().to_string())
            .ok()
            .as_deref()
            == Some(want.as_str())
    {
        return Ok((out, None));
    }
    const EXTS: [&str; 5] = [".bfsha", ".ptcl", ".arc", ".sarc", ".bnsh"];
    let _ = std::fs::remove_dir_all(&out);
    let mut copied = 0usize;
    let mut bytes = 0u64;
    for (path, id, _) in &tree {
        let Some(rel) = path.strip_prefix("romfs/") else {
            continue;
        };
        let low = rel.to_ascii_lowercase();
        let base = low.strip_suffix(".zst").unwrap_or(&low);
        if !EXTS.iter().any(|e| base.ends_with(e)) {
            continue;
        }
        let rel = if low.ends_with(".zst") {
            rel[..rel.len() - 4].to_string()
        } else {
            rel.to_string()
        };
        let dst = out.join(&rel);
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        let data = nx
            .read(*id)
            .map_err(|e| format!("reading {rel} from {}: {e:#}", romfs.display()))?;
        bytes += data.len() as u64;
        std::fs::write(&dst, &data).map_err(|e| format!("writing {}: {e}", dst.display()))?;
        copied += 1;
    }
    if copied == 0 {
        return Err(format!(
            "{} holds no shader archive ({EXTS:?}), so no shaders can be translated",
            romfs.display()
        ));
    }
    std::fs::create_dir_all(&out).ok();
    std::fs::write(&stamp, &want).map_err(|e| format!("writing {}: {e}", stamp.display()))?;
    Ok((out, Some((copied, bytes))))
}

fn shader_report(corpus: &Path, spirv: &Path) -> Result<ShaderReport, String> {
    let unique = corpus.join("unique.tsv");
    let text = std::fs::read_to_string(&unique)
        .map_err(|e| format!("reading {}: {e}", unique.display()))?;
    let expected: Vec<&str> = text
        .lines()
        .skip(1)
        .filter_map(|l| l.split('\t').next())
        .filter(|h| !h.is_empty())
        .collect();
    // hash -> status, from the translator's own report.
    let results = std::fs::read_to_string(spirv.join("results.tsv")).unwrap_or_default();
    let mut status: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for line in results.lines().skip(1) {
        let mut f = line.split('\t');
        if let (Some(h), Some(_group), Some(_stage), Some(_uses), Some(st)) =
            (f.next(), f.next(), f.next(), f.next(), f.next())
        {
            status.insert(h, st);
        }
    }
    let (mut translated, mut failed, mut missing) = (0usize, 0usize, Vec::new());
    for hash in &expected {
        // A report line saying ok with no .spv beside it is work to do again: the
        // file was deleted, or the run that wrote the line was cut short before it.
        match status.get(hash) {
            Some(&st) if st == "ok" && spirv.join(format!("{hash}.spv")).is_file() => {
                translated += 1
            }
            Some(&st) if st == "ok" => missing.push((*hash).to_string()),
            Some(_) => failed += 1,
            None => missing.push((*hash).to_string()),
        }
    }
    Ok(ShaderReport {
        expected: expected.len(),
        translated,
        failed,
        missing,
    })
}

/// Run the translator over the corpus. The harness keeps a per-block journal and
/// skips the blocks that already have a result, so a repair costs the missing
/// blocks and not the whole corpus; its report is written from the journal, so the
/// earlier lines survive a stopped run.
fn run_translator(
    ctx: &Ctx,
    root: &Path,
    bundled: &Option<PathBuf>,
    harness: &Path,
) -> Result<(), String> {
    match bundled {
        Some(exe) => run(
            ctx,
            root,
            &exe.display().to_string(),
            &[
                "--batch",
                "work\\shaders\\corpus",
                "--out",
                "work\\shaders\\spirv",
                "--jobs",
                "4",
                "--no-val",
            ],
        ),
        None => {
            let h = harness.display().to_string();
            run(
                ctx,
                root,
                "dotnet",
                &[
                    &h,
                    "--batch",
                    "work\\shaders\\corpus",
                    "--out",
                    "work\\shaders\\spirv",
                    "--jobs",
                    "4",
                    "--no-val",
                ],
            )
        }
    }
}

/// The texture build's sources, each checked by the file the build opens. A
/// folder left behind by an interrupted fetch has some of them and not others.
fn textures_fetched(root: &Path) -> bool {
    let tp = root.join("tools").join("textures").join("third_party");
    [
        tp.join("astc-encoder").join("CMakeLists.txt"),
        tp.join("bc7enc_rdo").join("CMakeLists.txt"),
        tp.join("ispc")
            .join("ispc-v1.32.0-windows")
            .join("bin")
            .join("ispc.exe"),
    ]
    .iter()
    .all(|p| p.is_file())
}

/// The shader translator checkout the build uses. It is cloned into a
/// temporary folder and moved into place only after the pinned revision is
/// checked out and the tree is complete: a clone that a cancel or a dropped
/// connection interrupted otherwise leaves `.git` behind, which is exactly what
/// the old "is it there" test accepted before failing much later in `dotnet`.
fn ensure_translator_refs(ctx: &Ctx, root: &Path) -> Result<(), String> {
    let refs = root.join("work").join("refs").join("ryubing");
    let project = Path::new("src")
        .join("Ryujinx.Graphics.Shader")
        .join("Ryujinx.Graphics.Shader.csproj");
    if refs.join(&project).is_file() {
        return Ok(());
    }
    if refs.exists() {
        ctx.log(format!("the shader translator checkout in {} is incomplete; cloning it again", refs.display()));
        let _ = std::fs::remove_dir_all(&refs);
    }
    let tmp = root.join("work").join("refs").join("ryubing.partial");
    let mut last = String::new();
    for attempt in 1..=2 {
        let _ = std::fs::remove_dir_all(&tmp);
        let result = run(ctx, root, "git", &["clone", "--quiet", "--filter=blob:none", "--no-checkout", SHADER_TRANSLATOR_URL, "work\\refs\\ryubing.partial"])
            .and_then(|()| run(ctx, root, "git", &["-C", "work\\refs\\ryubing.partial", "checkout", "--quiet", SHADER_TRANSLATOR_REV]))
            .and_then(|()| {
                if tmp.join(&project).is_file() {
                    Ok(())
                } else {
                    Err(format!("the checkout has no {}", project.display()))
                }
            });
        match result {
            Ok(()) => {
                std::fs::rename(&tmp, &refs)
                    .map_err(|e| format!("moving the shader translator checkout into {}: {e}", refs.display()))?;
                return Ok(());
            }
            Err(e) => {
                last = e;
                let _ = std::fs::remove_dir_all(&tmp);
                if attempt < 2 {
                    ctx.log(format!("the shader translator checkout failed; trying once more ({last})"));
                }
            }
        }
    }
    Err(format!(
        "cloning the shader translator's sources failed twice: {last}. That is usually the network or a proxy; work\\refs is \
         clean now, so pressing Build the game again starts the clone over"
    ))
}

/// Settings written to `fefw.ini`.
pub struct RunConfig {
    pub exefs: PathBuf,
    pub romfs: PathBuf,
    /// The dumped add-on content (`[aoc] dir`); None leaves the engine without DLC.
    pub dlc: Option<PathBuf>,
    pub save_dir: PathBuf,
    /// The launcher's mods folder, written as an absolute path. The engine accepts a relative `mods`, but then it is
    /// resolved against whatever working directory the game was started from; a shortcut or a direct `fefw.exe` start
    /// made the same mods invisible (#39). Play still starts in the run folder, so this only removes the ambiguity.
    pub mods: PathBuf,
    /// Files the package cannot serve in place (its zstd entries), written out by the build; the engine lays this folder over the package.
    pub romfs_overlay: Option<PathBuf>,
}

/// The engine's configuration for the launcher's run folder.
pub fn ini_text(root: &Path, run: &RunConfig) -> String {
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    // Only written when the package has files the engine cannot read in place: the folder the build wrote them into.
    let overlay = run
        .romfs_overlay
        .as_ref()
        .map(|p| format!("romfs_overlay = {}\n", fwd(p)))
        .unwrap_or_default();
    // An empty dir is what the engine reads as "no add-on content"; it never falls back to scanning the working folder.
    let dlc = run.dlc.as_ref().map(|p| fwd(p)).unwrap_or_default();
    format!(
        "; Written by EmblemWeave. Every key is described in engine/fefw-recomp/runtime/docs/CONFIG.md.\n\
         [log]\npath = fefw.log\n\
         [fs]\nexefs = {}\nromfs = {}\n{}\nsave_dir = {}\nmods = {}\n\
         [aoc]\ndir = {}\n\
         [nvn]\nshader_cache = {}\nbc7_cache = {}\n\
         [upscale]\nmode = off\n\
         [dlss]\nupscaler = auto\n\
         [diag]\nwatchdog_seconds = 30\n\
         [fefw]\nfps60 = 1\n\
         [engine]\nbuilt_by = {}\npacked_nup = 1\n",
        fwd(&run.exefs),
        fwd(&run.romfs),
        overlay.trim_end(),
        fwd(&run.save_dir),
        fwd(&run.mods),
        dlc,
        fwd(&root.join("work").join("shaders").join("spirv")),
        fwd(&root.join("work").join("bc7_cache")),
        env!("CARGO_PKG_VERSION"),
    )
}

/// The launcher version the game folder was built with, from the `[engine] built_by` this launcher writes into `fefw.ini`. A build from an
/// older release keeps its old engine, and an old engine can still refuse what the newer launcher has learned to handle (#24), so the
/// Overview names the version instead of leaving the player to guess why a fresh launcher behaves like the old one.
pub fn built_by() -> Option<String> {
    let dir = run_dir()?;
    ini_map(&dir.join("fefw.ini"))
        .get("engine.built_by")
        .cloned()
}

/// Refresh the `[engine] built_by` stamp when the engine binary is newer than the ini that names it. A `fefw.exe` rebuilt
/// outside the launcher (CMake straight from the engine folder, or a script) left the old stamp behind, and the Overview then
/// called a current folder stale and told the player to rebuild — the different-version notice that reappeared in the
/// screenshots after every rebuild. Only that one line is rewritten, and the rewrite also moves the ini's timestamp forward so
/// the comparison settles; every other key is left exactly as it was.
///
/// Returns the old and new stamp when it changed the file.
pub fn refresh_build_stamp() -> Option<(String, String)> {
    let ini = run_dir()?.join("fefw.ini");
    let exe = root()?
        .join("game-build")
        .join("fefw")
        .join("build")
        .join("release")
        .join("fefw.exe");
    let ini_at = std::fs::metadata(&ini).ok()?.modified().ok()?;
    let exe_at = std::fs::metadata(&exe).ok()?.modified().ok()?;
    if exe_at <= ini_at {
        return None;
    }
    let old = built_by()?;
    let now = env!("CARGO_PKG_VERSION");
    let text = std::fs::read_to_string(&ini).ok()?;
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("built_by") {
            out.push_str(&format!("built_by = {now}"));
        } else {
            out.push_str(line);
        }
        out.push_str("\r\n");
    }
    std::fs::write(&ini, out).ok()?;
    Some((old, now.to_string()))
}

/// A game folder the launcher can play but that was built by another release and was not carried across by this
/// launcher's updater. The Overview offers one press that rebuilds it and writes the packed-file overlay in the same
/// run, so a manually mixed folder is not left behind (#24, #34). `built` is the release that wrote `fefw.ini`, or
/// `None` for a build older than the stamp itself. An in-place update writes `carried_by`, which suppresses this notice
/// because the updater deliberately kept the existing build (issue #56).
pub fn stale_build() -> Option<(Option<String>, &'static str)> {
    if !ready() {
        return None;
    }
    stale_build_in(&run_dir()?, env!("CARGO_PKG_VERSION"))
}

/// The check behind [`stale_build`], with the run folder and version passed in so it can be tested. `carried_by` is
/// written by the updater after an in-place update: the build was deliberately kept, so a newer launcher must not ask for
/// a rebuild merely because `built_by` names the release that compiled it (issue #56). `built_by` itself stays intact for
/// reports and for the packed-package guard, which needs the engine's real age.
fn stale_build_in(dir: &Path, now: &'static str) -> Option<(Option<String>, &'static str)> {
    let ini = ini_map(&dir.join("fefw.ini"));
    if ini
        .get("engine.carried_by")
        .is_some_and(|v| v.trim() == now)
    {
        return None;
    }
    match ini.get("engine.built_by").cloned() {
        Some(v) if v == now => None,
        Some(v) => Some((Some(v), now)),
        None => Some((None, now)),
    }
}

/// Record that `release` carried this folder's existing build across an in-place update. The updater calls this after it
/// has written the release, so the restarted launcher opens ready to play instead of offering Repair/Rebuild for a build
/// that was intentionally kept (issue #56). Does nothing when there is no built game or no run folder.
pub fn mark_build_carried(engine: &Path, release: &str) {
    let Some(dir) = run_dir() else {
        return;
    };
    mark_build_carried_in(engine, &dir, release);
}

/// The write behind [`mark_build_carried`] for an explicit play folder, so the updater's install path can be tested
/// against a temporary install instead of the running launcher's data.
pub(crate) fn mark_build_carried_in(engine: &Path, dir: &Path, release: &str) {
    if !game_exe(engine).is_file() {
        return;
    }
    let _ = carry_build_forward(dir, release);
}

/// The write behind [`mark_build_carried`]: replaces `carried_by` in `dir`'s `fefw.ini`, preserving every other key.
fn carry_build_forward(dir: &Path, release: &str) -> Result<(), String> {
    let ini = dir.join("fefw.ini");
    let text = std::fs::read_to_string(&ini)
        .map_err(|e| format!("reading {}: {e}", ini.display()))?;
    let updated = set_ini_value(&text, "engine", "carried_by", release);
    std::fs::write(&ini, updated).map_err(|e| format!("writing {}: {e}", ini.display()))
}

/// Throw away the game's compiled runtime before a repair. An incremental rebuild can leave the old
/// `fefw.exe` in place when only the launcher was updated, so issue #34's mixed folder survived a
/// rebuild. The fetched dependencies are outside this folder and are kept; the lifted modules under
/// `work/build` are fine to reuse, because the packed-package reader lives in the runtime.
pub fn clean_runtime_build() -> Result<(), String> {
    let root = root().ok_or("the engine folder engine/fefw-recomp was not found")?;
    #[cfg(windows)]
    if crate::cheats::process::find(crate::cheats::GAME_EXE).is_some() {
        return Err("Close the game before repairing the build".into());
    }
    let build = root.join("game-build").join("fefw").join("build");
    if build.is_dir() {
        std::fs::remove_dir_all(&build)
            .map_err(|e| format!("clearing {}: {e}", build.display()))?;
    }
    Ok(())
}

/// Whether `dir` was built by a release older than `major.minor.patch`. A folder with no `built_by` predates the key itself,
/// which every build has written since 0.1.20, so it counts as old.
fn build_is_older_than(dir: &Path, major: u32, minor: u32, patch: u32) -> bool {
    let Some(text) = ini_map(&dir.join("fefw.ini"))
        .get("engine.built_by")
        .cloned()
    else {
        return true;
    };
    let mut parts = text.split('.').filter_map(|p| p.trim().parse::<u32>().ok());
    let found = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    found < (major, minor, patch)
}

/// The refusal Play gives for `dir` when its package keeps files packed and its engine cannot read them (#34). The build
/// writes `[engine] packed_nup = 1` from this release on; a folder without it was built by an engine that predates the
/// reader, or one whose rebuild reused an old `fefw.exe`. `built_by` alone cannot tell those apart: it records the launcher
/// that wrote the ini, not which engine binary is beside it. None means this guard has nothing to say; Play's other checks run.
fn play_blocker_in(dir: &Path) -> Option<String> {
    let ini = ini_map(&dir.join("fefw.ini"));
    let pkg = PathBuf::from(ini.get("fs.romfs")?);
    if !pkg.is_file() || crate::gamedata::packed_romfs(&pkg).is_none() {
        return None;
    }
    // Two ways an engine at or above 0.1.20 can play a packed package: the reader this release has, or the overlay the
    // launcher writes into `work\romfs-overlay` (0.1.20 and newer read `[fs] romfs_overlay`). A 0.1.31 build that rebuilt
    // properly has neither marker nor reason to be refused once Play has written that folder; only one with neither is.
    let marker = ini
        .get("engine.packed_nup")
        .is_some_and(|v| v.trim() == "1");
    let overlay_ready = ini
        .get("fs.romfs_overlay")
        .map(PathBuf::from)
        .is_some_and(|p| p.is_dir());
    if !build_is_older_than(dir, 0, 1, 20) && (marker || overlay_ready) {
        return None;
    }
    let by = ini
        .get("engine.built_by")
        .cloned()
        .unwrap_or_else(|| "0.1.19 or older".into());
    let why = if build_is_older_than(dir, 0, 1, 20) {
        format!("this game folder was built by EmblemWeave {by}, whose engine cannot read a package that keeps some files packed.")
    } else {
        format!(
            "this game folder says it was built by EmblemWeave {by}, but its engine is older than the reader for packages that keep \
             some files packed (the rebuild reused an old fefw.exe)."
        )
    };
    Some(format!(
        "{why} Press Build the game to rebuild it with this release."
    ))
}

/// The package the built game points at, when that package packs files with zstd and the game folder has no overlay for them: Play writes
/// those files first and then starts the game. True for a game folder built before 0.1.20, or one whose write never ran.
pub fn packed_package_without_overlay() -> Option<PathBuf> {
    let dir = run_dir()?;
    let written = ini_map(&dir.join("fefw.ini"));
    // A key is not an overlay: an interrupted write, or a folder the player moved, leaves an ini that points at nothing.
    // Play then tries to write the files again instead of starting an engine that reports main.npdm is compressed.
    if written
        .get("fs.romfs_overlay")
        .map(PathBuf::from)
        .is_some_and(|p| p.is_dir())
    {
        return None;
    }
    let path = PathBuf::from(written.get("fs.romfs")?);
    (path.is_file() && crate::gamedata::packed_romfs(&path).is_some()).then_some(path)
}

/// Game Setup's "Check my game folder": what the build does for a package it cannot read in place, done on its own so a game folder that
/// was built by an older release can be fixed without compiling anything. It reads the package, writes the files that are packed with
/// zstd into `work/romfs-overlay`, and points the existing `fefw.ini` at that folder.
pub const REPAIR_STAGES: &[&'static str] = &[
    "Read the package",
    "Write the files it cannot serve",
    "Point the game at them",
];

pub fn repair(ctx: &Ctx, romfs: PathBuf) -> Result<JobResult, String> {
    let root = root().ok_or("the engine folder engine/fefw-recomp is missing beside the launcher; unpack the repository whole, or set EW_ENGINE to its path")?;
    ctx.start(0);
    let Some((count, bytes)) = crate::gamedata::packed_romfs(&romfs) else {
        let said = if romfs.is_dir() {
            "the game files are a folder, which is read as it is".to_string()
        } else {
            "the package holds only stored files, which are read as it is".to_string()
        };
        ctx.end(0, StageStatus::Done, said);
        ctx.end(1, StageStatus::Skipped, "nothing to write");
        ctx.end(2, StageStatus::Skipped, "nothing to change");
        return Ok(JobResult::Repaired(None));
    };
    ctx.end(
        0,
        StageStatus::Done,
        format!(
            "{count} files are packed with zstd ({:.1} GB)",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0)
        ),
    );

    ctx.start(1);
    let dest = root.join("work").join("romfs-overlay");
    let stamp = dest.join(".written");
    let key = format!("{}|{count} files|{bytes} bytes", romfs.display());
    if std::fs::read_to_string(&stamp)
        .map(|t| t.trim() == key.trim())
        .unwrap_or(false)
    {
        ctx.end(
            1,
            StageStatus::Skipped,
            format!("already written in {}", dest.display()),
        );
    } else {
        std::fs::create_dir_all(&dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
        let (written, total) = crate::gamedata::unpack_packed_romfs(&romfs, &dest, &mut |m| {
            ctx.log(format!("game files: {m}"))
        })?;
        std::fs::write(&stamp, &key).map_err(|e| format!("writing {}: {e}", stamp.display()))?;
        ctx.end(
            1,
            StageStatus::Done,
            format!(
                "{written} files, {:.1} GB in {}",
                total as f64 / (1024.0 * 1024.0 * 1024.0),
                dest.display()
            ),
        );
    }

    ctx.start(2);
    let Some(dir) = run_dir() else {
        ctx.end(2, StageStatus::Skipped, "no run folder yet");
        return Ok(JobResult::Repaired(Some(dest)));
    };
    let ini_path = dir.join("fefw.ini");
    if !ini_path.is_file() {
        ctx.end(
            2,
            StageStatus::Skipped,
            "the game has not been built yet: the next build writes it",
        );
        return Ok(JobResult::Repaired(Some(dest)));
    }
    let text = std::fs::read_to_string(&ini_path)
        .map_err(|e| format!("reading {}: {e}", ini_path.display()))?;
    let updated = set_ini_value(
        &text,
        "fs",
        "romfs_overlay",
        &dest.display().to_string().replace('\\', "/"),
    );
    if updated != text {
        std::fs::write(&ini_path, updated)
            .map_err(|e| format!("writing {}: {e}", ini_path.display()))?;
    }
    ctx.end(2, StageStatus::Done, ini_path.display().to_string());
    Ok(JobResult::Repaired(Some(dest)))
}

/// Sets `key` in `[section]`, replacing the one that is there or adding it under the section's other keys. The ini is a flat text file the
/// engine reads in order, so the section header has to be found before the key can be written into it.
fn set_ini_value(text: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let mut out: Vec<String> = Vec::new();
    let (mut in_section, mut written) = (false, false);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_section && !written {
                out.push(format!("{key} = {value}"));
                written = true;
            }
            in_section = trimmed.eq_ignore_ascii_case(&header);
            out.push(line.to_string());
            continue;
        }
        if in_section {
            let name = line.split('=').next().unwrap_or("").trim();
            if name.eq_ignore_ascii_case(key) {
                if !written {
                    out.push(format!("{key} = {value}"));
                    written = true;
                }
                continue;
            }
        }
        out.push(line.to_string());
    }
    if in_section && !written {
        out.push(format!("{key} = {value}"));
        written = true;
    }
    if !written {
        out.push(header);
        out.push(format!("{key} = {value}"));
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// The build job's work: every step in order.
pub fn build(
    ctx: &Ctx,
    exefs: PathBuf,
    romfs: PathBuf,
    dlc: Option<PathBuf>,
) -> Result<JobResult, String> {
    // The engine is compiled by the Windows toolchain; a platform without it gets a sentence, not
    // ninety steps that fail at CMake configure.
    if !crate::platform::build_supported() {
        return Err(crate::platform::build_unsupported_reason());
    }
    let root = root().ok_or("the engine folder engine/fefw-recomp is missing beside the launcher; unpack the repository whole, or set EW_ENGINE to its path")?;
    // One probe decides whether the build can start, and what it found is what
    // the build uses from here: the compiler it validated, the SDK beside it,
    // and one `vcvars` environment for every step. A too-old Visual Studio, an
    // install without the C++ workload, a missing Windows SDK and an `INCLUDE`
    // left by another shell are all named here, before anything is compiled.
    let resolved = crate::toolchain::init()?;
    ctx.log(format!("toolchain: {}", resolved.toolchain.summary()));
    ctx.log(format!(
        "toolchain fingerprint: {}",
        resolved.toolchain.fingerprint()
    ));
    for (name, value) in crate::toolchain::stale_env() {
        ctx.log(format!("ignoring the inherited {name}={value}: it would point the build at a toolchain or Python environment that was not checked"));
    }
    // Two builds at once write the same object files and fetch the same sources.
    let _build_lock = lock_build(&root)?;
    // The probe that matters: one file, compiled and linked with exactly the
    // environment below. A header from another install fails here in a second.
    let probe_dir = root.join("work").join("build").join("toolchain-probe");
    crate::toolchain::probe_build(&resolved.compiler, &resolved.env, &probe_dir)
        .map_err(|e| format!("the build cannot start. {e}"))?;
    ctx.log(format!(
        "the toolchain compiles and links a test file ({} with its own headers, libraries and SDK)",
        resolved.compiler.display()
    ));
    let missing = missing_tools();
    if !missing.is_empty() {
        let lines: Vec<String> = missing
            .iter()
            .map(|m| format!("{m} (`{}`)", install_hint(m)))
            .collect();
        return Err(format!("the game build needs tools this PC does not have: {}. Each one is a single install command; see docs/TROUBLESHOOTING.md", lines.join(", ")));
    }
    // CMake is present but may be too old for the engine's projects, which is
    // reported by CMake itself as a generate failure with no reason attached.
    if let Some((major, minor)) = cmake_version() {
        if (major, minor) < (3, 25) {
            return Err(format!(
                "CMake {major}.{minor} is too old: the engine's projects need 3.25 or newer, and an older one stops with \"CMake Generate step failed\" instead of naming the version. Update it with `winget install Kitware.CMake` (or `winget upgrade Kitware.CMake`), restart the launcher, then press Build the game again."
            ));
        }
    }
    let venv = root.join("tools").join(".venv");
    let python = venv.join("Scripts").join("python.exe");
    let py = python.display().to_string();

    // 1. Python environment. `uv` is quicker when it is installed, but it is one more thing to install for one virtualenv, so
    // without it the interpreter the build already needs makes the environment and pip installs the same pinned list. The
    // existing environment is reused only after it proves it is the interpreter the launcher starts, is a venv, and can
    // import every lifter package. Its own pip is used, never a global Python's.
    ctx.start(0);
    match probe_python_env(&python) {
        PythonEnv::Healthy {
            version,
            executable,
        } => {
            ctx.log(format!("Python environment: {executable} (Python {version}) passed the interpreter and import probe"));
            ctx.end(0, StageStatus::Skipped, "already set up");
        }
        found => {
            let why = found.reason();
            if matches!(found, PythonEnv::Missing) {
                ctx.log(format!(
                    "Python environment: {} is missing; creating it",
                    venv.display()
                ));
            } else {
                ctx.log(format!(
                    "Python environment: {} failed the probe: {why}",
                    python.display()
                ));
                ctx.log(format!(
                    "removing only the engine's generated {} and recreating it",
                    venv.display()
                ));
            }
            remove_engine_venv(&root)?;
            let used_uv = which_tool("uv");
            if used_uv {
                run(
                    ctx,
                    &root,
                    "uv",
                    &["venv", "tools\\.venv", "--python", "3.13"],
                )?;
                run(
                    ctx,
                    &root,
                    "uv",
                    &[
                        "pip",
                        "install",
                        "--python",
                        "tools\\.venv",
                        "-r",
                        "recomp\\requirements.txt",
                    ],
                )?;
            } else {
                ctx.log("uv is not installed: the Python environment is made with the interpreter on this PC and pip installs its packages");
                if run(ctx, &root, "py", &["-3", "-m", "venv", "tools\\.venv"]).is_err() {
                    run(ctx, &root, "python", &["-m", "venv", "tools\\.venv"]).map_err(|e| {
                        format!("creating tools\\.venv needs a Python 3.9 or newer: {e}. Install Python with `winget install Python.Python.3.12`, restart the launcher, then press Build the game again")
                    })?;
                }
                run(
                    ctx,
                    &root,
                    &py,
                    &[
                        "-m",
                        "pip",
                        "install",
                        "--disable-pip-version-check",
                        "--no-input",
                        "-r",
                        "recomp\\requirements.txt",
                    ],
                )?;
            }
            match probe_python_env(&python) {
                PythonEnv::Healthy {
                    version,
                    executable,
                } => {
                    ctx.log(format!("Python environment verified: {executable} (Python {version}) imports every lifter package"));
                    let detail = if used_uv {
                        "tools\\.venv ready"
                    } else {
                        "tools\\.venv ready (pip)"
                    };
                    ctx.end(0, StageStatus::Done, detail);
                }
                failed => {
                    let why = failed.reason();
                    ctx.end(0, StageStatus::Failed, why.clone());
                    return Err(format!(
                        "tools\\.venv was created but does not pass the Python probe: {why}. Delete {} and press Build the game again; do not uninstall other Python versions, because only this generated environment is replaced.",
                        venv.display()
                    ));
                }
            }
        }
    }

    // 2. ExeFS, including the active IPS/IPS32/pchtxt patches. They are
    // applied to the launcher-owned uncompressed copy here, before the lifter
    // reads it; the user's dump and the built Windows exe are never touched.
    ctx.start(1);
    let ex = exefs.display().to_string();
    run(
        ctx,
        &root,
        &py,
        &["-I", "tools\\nso\\prepare_exefs.py", "--exefs", &ex],
    )?;
    let mods_dir = run_dir().map(|d| d.join("mods")).unwrap_or_default();
    let main = exefs.join("main");
    let patch_plan = crate::patches::inspect(
        &mods_dir,
        crate::patches::main_build_id(&main),
        std::fs::metadata(&main).ok().map(|m| m.len()),
    );
    if !patch_plan.can_build() {
        return Err(format!(
            "the installed ExeFS patches cannot be applied, so the build stopped before lifting any code:\n{}",
            patch_plan.refusal()
        ));
    }
    let patch_applied = crate::patches::apply(&patch_plan, &main)?;
    if patch_applied.count > 0 {
        ctx.log(format!(
            "exefs patches: {} active file(s), {} record(s), main {} -> {}",
            patch_plan
                .patches
                .iter()
                .filter(|p| p.error.is_none() && !p.records.is_empty())
                .count(),
            patch_applied.count,
            &patch_applied.original_sha256[..12],
            &patch_applied.patched_sha256[..12]
        ));
    } else {
        ctx.log("exefs patches: none active");
    }
    ctx.end(
        1,
        StageStatus::Done,
        format!(
            "game\\dump\\exefs ready{}",
            if patch_applied.count > 0 {
                format!(" + {} patch record(s)", patch_applied.count)
            } else {
                String::new()
            }
        ),
    );

    // 3. Game code: ninja rebuilds only what changed.
    ctx.start(2);
    // NASM only adds the AV1 decoder's assembly; the engine's CMake turns that off by itself when nasm is missing, and the
    // build stays valid, so say what the difference is instead of failing the way the requirements page does not.
    if !which_tool("nasm") {
        ctx.log("NASM is not installed: the AV1 decoder is built in C only, so cutscene movies play slower");
    }
    // A build folder made with another compiler, generator or toolchain file
    // cannot be reused, and CMake says only "Build files cannot be regenerated
    // correctly" after a configure pass. Its cache names what it was made with,
    // so compare that with what was validated here and clear it first.
    let game_build = root.join("work").join("build").join("fefw");
    let toolchain_file = root.join("recomp").join("cmake").join("clang-cl.cmake");
    if let Some(why) = crate::toolchain::cache_mismatch_in(
        &game_build,
        &resolved.compiler,
        "Ninja",
        &toolchain_file.display().to_string(),
    ) {
        ctx.log(format!(
            "the recompile build folder cannot be reused: {why}; clearing its CMake cache"
        ));
        crate::toolchain::clear_cache(&game_build);
    }
    let jobs = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(2).max(1))
        .unwrap_or(8)
        .to_string();
    // The engine's scripts look for `vcvars64.bat` through vswhere, and their
    // `%ProgramFiles(x86)%` path breaks inside cmd's `for /f` parentheses, so
    // every build logged "'vswhere.exe' is not recognized". The path is already
    // known here, and the scripts read NX_VCVARS first.
    let vc_env: Option<String> = vc_tools().map(|p| p.display().to_string());
    ctx.log(format!(
        "visual studio: {}",
        match (&vc_env, vc_tools_any()) {
            (Some(path), Some((_, major))) => format!("{path} (tools version {major})"),
            (Some(path), None) => path.clone(),
            _ => "not found; the engine's scripts will look for it themselves".into(),
        }
    ));
    // The build of the lifted code that this processor can run (cpu.rs): FMA3 + AVX where the CPU has it, SSE4.2 + POPCNT
    // otherwise. A processor with neither set is refused here, before anything is compiled, instead of at start with code 4.
    let cpu = crate::cpu::probe();
    let chosen = crate::cpu::choose(&cpu);
    ctx.log(format!("cpu: {}", crate::cpu::summary(&cpu, &chosen)));
    let variant =
        chosen.map_err(|why| format!("{why}\n\nThe game cannot be built for this processor."))?;
    ctx.log(format!(
        "cpu build: {} ({})",
        variant.key(),
        variant.label()
    ));
    let mut envs: Vec<(&str, &str)> = vec![("NX_JOBS", &jobs), ("FEFW_CPU_VARIANT", variant.key())];
    if let Some(vc) = &vc_env {
        envs.push(("NX_VCVARS", vc));
    }
    // Hook sites code mods bring (sites.rs): main is lifted with main.sites plus theirs. The list is only rewritten when it
    // changes, and with no such mods it is main.sites itself, so ninja lifts main again only when the sites really change.
    let (site_list, mod_sites) = crate::sites::prepare(&root, &mods_dir)?;
    if mod_sites.is_empty() {
        ctx.log("hook sites: main.sites (no installed mod adds any)");
    } else {
        let from: std::collections::BTreeSet<&str> = mod_sites
            .iter()
            .map(|s| s.from.split(':').next().unwrap_or(""))
            .collect();
        ctx.log(format!(
            "hook sites: main.sites plus {} from mods ({})",
            mod_sites.len(),
            from.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    // cmd.exe splits its command line at & | < > ^ ( ) % even when the line is quoted, which is how `G:\Games\PC Ports &
    // Recomps` became a command called `CMakeFiles\G_\Games\PC_Ports_`. The build runs through a spelling of the checkout
    // that has none of those characters, or stops with that explanation instead of building.
    let build_view = build_dir(&root)?;
    // CMake copies this path into the generated lift command; it gets the same spelling as the build folder, so the
    // generated `WORKING_DIRECTORY`/output paths and this argument agree and neither is cut.
    let site_path = site_list
        .strip_prefix(&root)
        .map(|rel| build_view.path.join(rel))
        .unwrap_or(site_list);
    let site_list = site_path.display().to_string();
    envs.push(("FEFW_SITES", &site_list));
    let built = run_env(
        ctx,
        &build_view.path,
        "cmd",
        &["/C", "recomp\\scripts\\build_game.cmd", "fefw"],
        &envs,
    );
    if let Err(e) = built {
        // The same old-toolset case, when it slips past the version check: the C++ library of an older Visual Studio has no
        // `<bit>`, and nothing in the message says that is what it means.
        if e.contains("'bit' file not found") {
            return Err(format!(
                "the C++ standard library the build is using has no `<bit>` header. That means the Visual Studio C++ tools in use are older than 2022, or the C++ workload's libraries are missing from the install.\n\nInstall the current tools with:\n{VS_INSTALL}\n\nthen press Build the game again."
            ));
        }
        // A cache left by an older CMake or compiler cannot be reused, and CMake
        // answers with "Build files cannot be regenerated correctly" instead of
        // saying that. Clear it and configure once more before giving up, so an
        // earlier failed attempt does not condemn every one after it.
        let build = root.join("work").join("build").join("fefw");
        let stale = needs_cache_reset(&e) || driver_flags_rejected(&e);
        if !stale {
            return Err(e);
        }
        ctx.log(
            "the build folder holds a cache CMake cannot reuse; clearing it and configuring again",
        );
        let _ = std::fs::remove_dir_all(build.join("CMakeFiles"));
        let _ = std::fs::remove_file(build.join("CMakeCache.txt"));
        run_env(
            ctx,
            &build_view.path,
            "cmd",
            &["/C", "recomp\\scripts\\build_game.cmd", "fefw"],
            &envs,
        )?;
    }
    // A step that reports success without writing its libraries leaves the
    // next one failing over a missing `fefw_main.lib`, which says nothing
    // about why. Name the real problem here instead.
    let libs = root.join("work").join("build").join("fefw");
    let absent: Vec<&str> = ["main", "sdk", "subsdk0", "subsdk1", "subsdk2", "rtld"]
        .into_iter()
        .filter(|m| !libs.join(format!("fefw_{m}.lib")).is_file())
        .collect();
    if !absent.is_empty() {
        return Err(format!(
            "the recompile wrote no libraries for {} in {}; its log above says why (usually a compiler or Visual Studio C++ problem, or a full disk)",
            absent.join(", "),
            libs.display()
        ));
    }
    ctx.end(
        2,
        StageStatus::Done,
        format!("six modules compiled ({jobs} jobs)"),
    );

    // 4. Texture transcoders. `-ExecutionPolicy Bypass` is not optional: on a PC
    // whose policy is RemoteSigned or AllSigned, PowerShell refuses to load
    // these unsigned scripts and the step fails with a SecurityError instead of
    // running, which is what issue 2's second report shows.
    ctx.start(3);
    // Not "the folder has something in it": each pinned source and the ispc
    // binary are checked by name, because a fetch that stopped halfway leaves a
    // folder that exists and cannot be built.
    if textures_fetched(&root) {
        ctx.end(3, StageStatus::Skipped, "already fetched");
    } else {
        ctx.log("the texture sources are missing or incomplete; fetching the pinned versions");
        run(
            ctx,
            &root,
            shell(),
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "tools\\textures\\scripts\\fetch_third_party.ps1",
            ],
        )?;
        if !textures_fetched(&root) {
            return Err("the texture sources were fetched but are still incomplete. tools\\textures\\third_party needs astc-encoder, bc7enc_rdo and ispc; delete that folder and press Build the game again".into());
        }
        ctx.end(
            3,
            StageStatus::Done,
            "astc-encoder, bc7enc_rdo and ispc fetched and checked",
        );
    }

    // 5. fefw.exe.
    ctx.start(4);
    let fefw_envs: Vec<(&str, &str)> = match &vc_env {
        Some(vc) => vec![("NX_VCVARS", vc)],
        None => Vec::new(),
    };
    let fefw = || {
        run_env(
            ctx,
            &root,
            shell(),
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "game-build\\scripts\\build_fefw.ps1",
            ],
            &fefw_envs,
        )
    };
    // Two failures the launcher can repair on its own, each once:
    //
    // - The runtime fetches its graphics dependencies into the build folder, and a
    //   fetch that was interrupted leaves the source folder half written. CMake then
    //   fails on a file that is not there, or says the patch no longer matches the
    //   pinned source, and it never repairs itself: the download stamps are still in
    //   place. Throw the fetched sources away and fetch once more. What was downloaded
    //   is kept in %LOCALAPPDATA%\nxrt\deps, so this costs an unpack, not a download.
    //
    // - A build folder left by an older launcher names a compiler, generator or CMake
    //   that is no longer installed, and CMake refuses to configure in it until its
    //   cache goes. The downloads and the fetched sources stay.
    let builds = root.join("game-build").join("fefw").join("build");
    for dir in std::fs::read_dir(&builds).into_iter().flatten().flatten() {
        if let Some(why) =
            crate::toolchain::cache_mismatch_in(&dir.path(), &resolved.compiler, "Ninja", "")
        {
            ctx.log(format!(
                "{} cannot be reused: {why}; clearing its CMake cache",
                dir.path().display()
            ));
            crate::toolchain::clear_cache(&dir.path());
        }
    }
    build_with_repairs(&builds, fefw, |m| ctx.log(m))?;
    // lld keeps its scratch files when a link is interrupted (a cancelled build, a
    // crash). They are never read again and one build folder had eight of them at
    // 1.2 GB. A successful link means they are dead weight, so take them with it.
    let swept = tidy_link_temps(&builds);
    if swept > 0 {
        ctx.log(format!(
            "removed {swept} leftover linker temp file(s) from the build folder"
        ));
    }
    // The haptics backend is a vendored DLL the runtime loads from beside `fefw.exe` (gfx/haptic.cpp looks
    // nowhere else), so it is copied in after the build rather than linked. Without it the game still vibrates,
    // on the standard two-motor rumble.
    let renhdrum = root
        .join("runtime")
        .join("third_party")
        .join("renhdrum")
        .join("renhdrum.dll");
    if renhdrum.is_file() {
        let beside = builds.join("release").join("renhdrum.dll");
        if let Err(e) = std::fs::copy(&renhdrum, &beside) {
            ctx.log(format!(
                "copying renhdrum.dll beside fefw.exe: {e} (standard rumble will be used)"
            ));
        }
    }
    // The build writes the engine's own icon into fefw.exe; Windows draws the
    // taskbar button from that resource, so ours goes in before anything runs it.
    let _ = brand_game();
    ctx.end(4, StageStatus::Done, "fefw.exe built");

    // 6. Shader translator.
    ctx.start(5);
    let harness = root
        .join("tools")
        .join("shaders")
        .join("build")
        .join("bin")
        .join("Release")
        .join("net10.0")
        .join("spirv_harness.dll");
    let bundled = bundled_harness_in(&root);
    if let Some(exe) = &bundled {
        ctx.end(
            5,
            StageStatus::Skipped,
            format!("using the one in the zip: {}", exe.display()),
        );
    } else if harness.is_file() {
        ctx.end(5, StageStatus::Skipped, "already built");
    } else {
        // Every release zip carries the published translator, so reaching this point means the engine folder came from a
        // different release than the launcher (a 0.1.17 launcher over a 0.1.10 engine folder is what issues #17 and #18
        // reported) or the zip was unpacked without its `prebuilt` folder. Say which folder and which file, instead of
        // silently building a translator the release meant to ship.
        let (pdir, pfile) = harness_platform();
        ctx.log(format!(
            "this engine folder has no translator at {}; building one from source, which needs the .NET 10 SDK. A release \
             zip carries that file already, so an engine folder without it is from another release: unzip the current \
             release into a folder of its own rather than mixing the two.",
            root.join("tools").join("shaders").join("prebuilt").join(pdir).join(pfile).display()
        ));
        // MSBuild reads item transforms written as @(Item->'...'): a path with an apostrophe in it ends the quoted string
        // early and the build fails with the transform printed as a file name (errors MSB3191 and MSB3021), which is what
        // issue #18 hit in `D:\Games\Fire Emblem Fortune's Weave\...`. Non-ASCII characters are handled differently by
        // every tool in the chain, so they take the same detour. Nothing about the translator needs to live under that
        // path: it is built from a copy under the launcher's own work folder and the result is copied back, so a folder
        // named after the game, or after whoever unzipped it, builds like any other without being renamed.
        let shaders = root.join("tools").join("shaders");
        let awkward_path = root
            .display()
            .to_string()
            .chars()
            .any(|c| c == '\'' || !c.is_ascii());
        let staged = if awkward_path {
            let to = crate::config::Config::work_dir()
                .ok_or_else(|| "the launcher's work folder is not set, so the translator cannot be built outside this path".to_string())?
                .join("shaders-src");
            if to.exists() {
                let _ = std::fs::remove_dir_all(&to);
            }
            let files = crate::saves::copy_tree(&shaders, &to)?;
            ctx.log(format!(
                "this folder's path cannot be handed to MSBuild (an apostrophe or non-ASCII character in it), so the \
                 shader translator is built from {} instead ({files} files) and its output is copied back",
                to.display()
            ));
            Some(to)
        } else {
            None
        };
        let build_shaders = staged.clone().unwrap_or_else(|| shaders.clone());
        ensure_translator_refs(ctx, &root)?;
        let setup = build_shaders.join("scripts").join("setup_ryujinx.ps1").display().to_string();
        let refs = root.join("work").join("refs").join("ryubing").display().to_string();
        run(ctx, &root, shell(), &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &setup, "-Ryubing", &refs])?;
        // "SDK not found" comes back as an HRESULT and a documentation link; name the piece that is missing instead, because a
        // .NET runtime alone satisfies `dotnet.exe` but cannot build a net10.0 project.
        let csproj = build_shaders
            .join("harness")
            .join("SpirvHarness.csproj")
            .display()
            .to_string();
        run(ctx, &root, "dotnet", &["build", &csproj, "-c", "Release"]).map_err(|e| {
            let lower = e.to_lowercase();
            if lower.contains("sdk-not-found") || e.contains("0x8000809b") || lower.contains("sdk not found") {
                format!(
                    "the shader translator is a .NET 10 program and this PC has the .NET runtime but no 10.x SDK. Install it with `{}`, then press Build the game again. ({e})",
                    install_hint(".NET 10 SDK")
                )
            } else if lower.contains("msb3021") && lower.contains("debugsymbolsintermediatepath") {
                // The same apostrophe, when it arrives from a path this check could not see (a referenced source under it).
                format!(
                    "the shader translator could not be built: MSBuild received one of its own item transforms as a file \
                     name, which happens when the folder path has an apostrophe in it. Move the unzipped folder to a path \
                     without one and press Build the game again. ({e})"
                )
            } else {
                e
            }
        })?;
        if let Some(staged) = &staged {
            let from = staged
                .join("build")
                .join("bin")
                .join("Release")
                .join("net10.0");
            let to = shaders
                .join("build")
                .join("bin")
                .join("Release")
                .join("net10.0");
            let files = crate::saves::copy_tree(&from, &to)?;
            ctx.log(format!(
                "copied {files} translator files back into {}",
                to.display()
            ));
        }
        ctx.end(5, StageStatus::Done, "spirv_harness built");
    }

    // 7. Shaders.
    //
    // "Some .spv files exist" is not "the shaders are translated": a run that is
    // interrupted (the launcher closed, the machine shut down, the translator
    // killed) leaves a folder with a few hundred of the 12 000 blocks and no
    // results.tsv, because results.tsv is written once, at the end. Treating
    // that as done is what produced the black screens in issues #12 and #15: the
    // game found no translated shader for 598 programs, skipped every draw and
    // presented black frames at 60 fps with the audio playing.
    ctx.start(6);
    let corpus = root.join("work").join("shaders").join("corpus");
    let spirv = root.join("work").join("shaders").join("spirv");
    let overlay = romfs_folder_for_play(&root, &romfs, ctx)?;
    let (romfs_dir, copied) = shader_romfs(&root, &romfs)?;
    if let Some((files, bytes)) = copied {
        ctx.log(format!(
            "copied {files} shader archives ({:.1} GiB) out of the package: the shader extractor walks a folder, and a .nup or .nx is a file",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0)
        ));
    }
    // The corpus remembers which RomFS it came from. One built from a path that is
    // not that folder (a .nup passed as --romfs, an older launcher) is not reused:
    // it holds a handful of blocks and would translate a handful of shaders.
    let corpus_stamp = corpus.join(".romfs");
    let romfs_id = romfs_dir.display().to_string();
    let corpus_current = corpus.join("unique.tsv").is_file()
        && std::fs::read_to_string(&corpus_stamp)
            .map(|t| t.trim().to_string())
            .ok()
            .as_deref()
            == Some(romfs_id.as_str());
    if !corpus_current {
        let rom = romfs_id.clone();
        let main = root
            .join("game")
            .join("dump")
            .join("exefs")
            .join("main")
            .display()
            .to_string();
        let dump = root
            .join("game")
            .join("dump")
            .join("exefs")
            .display()
            .to_string();
        run(
            ctx,
            &root,
            &py,
            &[
                "-I",
                "tools\\shaders\\ampere\\extract_corpus.py",
                "--romfs",
                &rom,
                "--main",
                &main,
                "--exefs",
                &dump,
                "--out",
                "work\\shaders\\corpus",
                "--jobs",
                "4",
            ],
        )?;
        std::fs::write(&corpus_stamp, &romfs_id)
            .map_err(|e| format!("writing {}: {e}", corpus_stamp.display()))?;
    }
    let mut report = shader_report(&corpus, &spirv)?;
    if report.expected == 0 {
        return Err(format!("{} lists no shader blocks, so the dump could not be read for shaders. Send a diagnostics report; the game cannot draw without them.", corpus.join("unique.tsv").display()));
    }
    if report.missing.is_empty() {
        ctx.end(
            6,
            StageStatus::Skipped,
            format!(
                "all {} shaders translated{}",
                report.expected,
                report.failed_summary()
            ),
        );
    } else {
        ctx.log(format!(
            "{} of {} shaders have no SPIR-V ({} translated, {} the translator cannot handle); translating the missing ones",
            report.missing.len(),
            report.expected,
            report.translated,
            report.failed
        ));
        // The translator writes one file per block and its report at the end, so
        // a crash or an interrupt loses only the block it was on. Re-run while
        // each attempt makes progress, and only a run that stops making any is
        // a failure.
        let mut last = report.missing.len();
        for attempt in 1..=3 {
            // --only-missing: the 12 000 blocks already translated are not redone.
            if let Err(e) = run_translator(ctx, &root, &bundled, &harness) {
                ctx.log(format!("translator attempt {attempt} ended early: {e}"));
            }
            report = shader_report(&corpus, &spirv)?;
            if report.missing.is_empty() {
                break;
            }
            if report.missing.len() >= last {
                break; // no progress: another run would fail the same way
            }
            last = report.missing.len();
        }
        if !report.missing.is_empty() {
            let first: Vec<&str> = report.missing.iter().take(5).map(String::as_str).collect();
            return Err(format!(
                "{} of {} shaders still have no SPIR-V (first: {}). Without them the game links no program, skips every draw and presents black frames \
                 with the audio still playing. The translator's report is {}; send it with a diagnostics report.",
                report.missing.len(),
                report.expected,
                first.join(", "),
                spirv.join("results.tsv").display()
            ));
        }
        ctx.end(
            6,
            StageStatus::Done,
            format!(
                "{} shaders translated{}",
                report.expected,
                report.failed_summary()
            ),
        );
    }

    // 8. fefw.ini.
    ctx.start(7);
    let dir = run_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    let save_dir = Config::save_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    std::fs::create_dir_all(dir.join("mods"))
        .map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let ini = ini_text(
        &root,
        &RunConfig {
            exefs: root.join("game").join("dump").join("exefs"),
            romfs,
            dlc,
            save_dir,
            mods: dir.join("mods"),
            romfs_overlay: overlay,
        },
    );
    std::fs::write(dir.join("fefw.ini"), ini).map_err(|e| format!("writing fefw.ini: {e}"))?;
    ctx.end(
        7,
        StageStatus::Done,
        format!("{}", dir.join("fefw.ini").display()),
    );
    // The executable is current only after the whole build succeeded. Recording
    // this identity here is what lets Play tell a fresh build from a stale one.
    if let Err(e) = crate::patches::record(&patch_plan, &patch_applied) {
        ctx.log(format!(
            "the patch state could not be recorded: {e}; Play will ask for a rebuild"
        ));
    }
    Ok(JobResult::Engine(dir))
}

/// The engine's `[upscale] mode` for a label on the Graphics page. The engine
/// has its own names for the modes; this is where the two vocabularies meet.
/// Anything it does not know (an older saved value, "Bilinear") means off.
pub fn upscale_mode(label: &str) -> &'static str {
    match label {
        "FSR 1.0" => "fsr1",
        "FSR 3.1" => "fsr3",
        "NVIDIA Image Scaling" => "nis",
        "DLSS" => "dlss",
        "DLAA" => "dlaa",
        "XeSS" => "xess",
        _ => "off",
    }
}

/// The shader translator published as one self-contained executable under
/// `tools/shaders/prebuilt/<platform>/`, named by `harness_platform`. It carries
/// its own .NET runtime: no SDK, no runtime and no NuGet restore on the player's
/// PC, and the same sources publish for Linux, so a build that runs there
/// translates shaders with the matching binary instead of through `dotnet`.
pub fn bundled_harness() -> Option<PathBuf> {
    bundled_harness_in(&root()?)
}

/// The directory and file name of the translator for the platform this build
/// runs on. Wine runs the Windows launcher as a Windows program and executes the
/// Windows binary; a native build takes the ELF one.
pub fn harness_platform() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("win-x64", "spirv_harness.exe")
    } else {
        ("linux-x64", "spirv_harness")
    }
}

fn bundled_harness_in(root: &Path) -> Option<PathBuf> {
    let (dir, file) = harness_platform();
    let p = root
        .join("tools")
        .join("shaders")
        .join("prebuilt")
        .join(dir)
        .join(file);
    p.is_file().then_some(p)
}

/// The files FSR 3.1 is built from, either layout: the SDK 2.x loader with its upscaler, or the single v1.1.4 DLL.
pub const FSR_FILES: &[&str] = &[
    "amd_fidelityfx_loader_vk.dll",
    "amd_fidelityfx_upscaler_vk.dll",
    "amd_fidelityfx_vk.dll",
];
/// The four DLLs DLSS needs.
pub const DLSS_FILES: &[&str] = &[
    "sl.interposer.dll",
    "sl.common.dll",
    "sl.dlss.dll",
    "nvngx_dlss.dll",
];
/// The five more DLSS Frame Generation needs beside those four (`streamline_framegen_files` in the engine).
pub const DLSS_FRAMEGEN_FILES: &[&str] = &[
    "sl.dlss_g.dll",
    "sl.reflex.dll",
    "sl.pcl.dll",
    "nvngx_dlssg.dll",
    "NvLowLatencyVk.dll",
];
/// Intel's XeSS is one DLL.
pub const XESS_FILES: &[&str] = &["libxess.dll"];

/// The SDK folders the engine looks for the upscaler DLLs in, and which of them
/// are actually there. FSR 3.1, DLSS and XeSS cannot be shipped with the launcher, so
/// the Graphics page says where to put them instead of pretending they work.
#[derive(Debug, Default, Clone)]
pub struct Sdk {
    /// `upscalers\fsr`: AMD's FidelityFX Vulkan DLLs.
    pub fsr3: Option<PathBuf>,
    /// `upscalers\streamline`: NVIDIA's Streamline DLLs.
    pub dlss: Option<PathBuf>,
    /// `upscalers\xess`: Intel's `libxess.dll`.
    pub xess: Option<PathBuf>,
}

impl Sdk {
    /// FSR 3 frame generation goes through the v1.1.4 DLL, so the v2.4 loader pair alone cannot make frames.
    pub fn fsr_framegen(&self) -> bool {
        self.fsr3
            .as_ref()
            .is_some_and(|d| d.join("amd_fidelityfx_vk.dll").is_file())
    }

    /// DLSS Frame Generation needs five more files beside the four DLSS ones, from the same Streamline release.
    pub fn dlss_framegen(&self) -> bool {
        self.dlss
            .as_ref()
            .is_some_and(|d| DLSS_FRAMEGEN_FILES.iter().all(|f| d.join(f).is_file()))
    }
}

/// Every root the vendor folders are looked under: the run folder, the game's own folder, the engine folder in the package (where the
/// original project's `upscalers` folder sits) and the launcher's folder.
pub fn sdk_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut add = |d: PathBuf| {
        if !dirs.iter().any(|o| o == &d) {
            dirs.push(d);
        }
    };
    if let Some(d) = run_dir() {
        add(d);
    }
    if let Some(r) = root() {
        add(r.clone());
        if let Some(d) = game_exe(&r).parent() {
            add(d.to_path_buf());
        }
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        add(d);
    }
    dirs
}

fn has_files(dir: &Path, needed: &[&str], every: bool) -> bool {
    if every {
        needed.iter().all(|f| dir.join(f).is_file())
    } else {
        needed.iter().any(|f| dir.join(f).is_file())
    }
}

/// Look for the DLLs in `dir`, then in the folders below it, up to `depth` levels: an SDK unpacked whole keeps them in
/// `bin`, `bin\x64` or `x64\bin` rather than at its top. The original project also calls this folder `upscaler`, so both
/// spellings are looked under every root (issues #26 and #29).
fn sdk_folder(dir: &Path, needed: &[&str], every: bool, depth: usize) -> Option<PathBuf> {
    if has_files(dir, needed, every) {
        return Some(dir.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    let mut sub: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    sub.sort();
    for s in sub {
        if let Some(hit) = sdk_folder(&s, needed, every, depth - 1) {
            return Some(hit);
        }
    }
    None
}

fn sdk_in(dirs: &[PathBuf], folders: &[&str], needed: &[&str], every: bool) -> Option<PathBuf> {
    for d in dirs {
        for root in ["upscalers", "upscaler"] {
            for folder in folders {
                if let Some(hit) = sdk_folder(&d.join(root).join(folder), needed, every, 3) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

/// A complete FSR layout: the SDK 2.x loader and upscaler, or the single v1.1.4 DLL. A lone loader is not enough.
fn sdk_fsr(dirs: &[PathBuf]) -> Option<PathBuf> {
    let folders = ["fsr", "fsr3"];
    sdk_in(
        dirs,
        &folders,
        &[
            "amd_fidelityfx_loader_vk.dll",
            "amd_fidelityfx_upscaler_vk.dll",
        ],
        true,
    )
    .or_else(|| sdk_in(dirs, &folders, &["amd_fidelityfx_vk.dll"], true))
}

/// Where the engine will look, in the same order: `upscalers\fsr` (and the older `upscalers\fsr3`), `upscalers\streamline`,
/// `upscalers\xess`, under each of `sdk_dirs`.
pub fn sdk() -> Sdk {
    let dirs = sdk_dirs();
    Sdk {
        fsr3: sdk_fsr(&dirs),
        dlss: sdk_in(&dirs, &["streamline"], DLSS_FILES, true),
        xess: sdk_in(&dirs, &["xess"], XESS_FILES, true),
    }
}

/// The one folder the launcher can point a player at: `upscalers` in the run folder, beside the game's ini and log. Created on demand.
pub fn upscalers_dir() -> Option<PathBuf> {
    run_dir().map(|d| d.join("upscalers"))
}

/// A file by name below `dir`, comparing names without case, up to `depth` folders deep. An SDK archive extracted as it
/// came keeps its DLLs in `bin` or `bin\x64`, not loose in the folder the player unzipped.
fn find_in_tree(dir: &Path, wanted: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let mut sub: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_file()
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(wanted))
        {
            return Some(p);
        }
        if p.is_dir() {
            sub.push(p);
        }
    }
    sub.sort();
    sub.into_iter()
        .find_map(|s| find_in_tree(&s, wanted, depth - 1))
}

/// The vendor DLLs, wherever an extracted SDK left them. Only folders that name a vendor are walked deeply: searching all
/// of Downloads, the engine tree or the launcher folder four levels down would cost more than it finds.
fn find_upscaler_file(sources: &[PathBuf], name: &str) -> Option<PathBuf> {
    for s in sources {
        let p = s.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    const HINTS: [&str; 10] = [
        "upscal",
        "fsr",
        "fidelity",
        "streamline",
        "dlss",
        "xess",
        "intel",
        "nvidia",
        "amd",
        "sdk",
    ];
    for s in sources {
        for e in std::fs::read_dir(s).into_iter().flatten().flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let named = p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if HINTS.iter().any(|h| named.contains(h)) {
                if let Some(hit) = find_in_tree(&p, name, 4) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

/// Finds the vendor DLLs in the places a player is likely to have left them (beside the launcher, in the engine folder, in Downloads and on
/// the Desktop, one folder deep) and copies them into the run folder's `upscalers\<vendor>`. Only files that are absent are copied, and
/// nothing is removed, so running it twice changes nothing.
pub fn adopt_upscaler_files() -> Result<String, String> {
    let root_dir = upscalers_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    let mut sources: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        sources.push(d);
    }
    if let Some(r) = root() {
        sources.push(r.clone());
    }
    for key in ["USERPROFILE", "HOME"] {
        if let Ok(home) = std::env::var(key) {
            if home.is_empty() {
                continue;
            }
            let home = PathBuf::from(home);
            sources.push(home.join("Downloads"));
            sources.push(home.join("Desktop"));
        }
    }
    // One level deeper: the DLLs arrive inside a folder named after the SDK, not loose in Downloads.
    let shallow = sources.clone();
    for s in shallow {
        if let Ok(entries) = std::fs::read_dir(&s) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    sources.push(p);
                }
            }
        }
    }
    sources.dedup();
    let mut copied = 0usize;
    let mut where_from: Option<PathBuf> = None;
    for (folder, files) in [
        ("fsr", FSR_FILES),
        ("streamline", DLSS_FILES),
        ("streamline", DLSS_FRAMEGEN_FILES),
        ("xess", XESS_FILES),
    ] {
        for name in files {
            let dest = root_dir.join(folder).join(name);
            if dest.is_file() {
                continue;
            }
            let Some(src) = find_upscaler_file(&sources, name) else {
                continue;
            };
            if std::fs::create_dir_all(root_dir.join(folder)).is_err() {
                continue;
            }
            // A running game holds its DLLs; a copy that fails leaves the folder as it was and the report says what is still missing.
            if std::fs::copy(&src, &dest).is_ok() {
                copied += 1;
                where_from
                    .get_or_insert_with(|| src.parent().map(Path::to_path_buf).unwrap_or_default());
            }
        }
    }
    let sdk = sdk();
    let mut have: Vec<&str> = Vec::new();
    let mut lack: Vec<String> = Vec::new();
    if sdk.fsr3.is_some() {
        have.push("FSR 3.1");
        if sdk.fsr_framegen() {
            have.push("FSR 3 frame generation");
        }
    } else {
        lack.push("AMD's FidelityFX DLLs".into());
    }
    match &sdk.dlss {
        Some(_) => {
            have.push("DLSS");
            if sdk.dlss_framegen() {
                have.push("DLSS Frame Generation");
            }
        }
        None => lack.push("NVIDIA's Streamline DLLs".into()),
    }
    if sdk.xess.is_some() {
        have.push("XeSS");
    }
    let from = where_from
        .map(|p| format!(" from {}", p.display()))
        .unwrap_or_default();
    let found = if copied == 0 {
        "No new upscaler files were found".to_string()
    } else {
        format!(
            "{copied} upscaler file(s) copied{from} into {}",
            root_dir.display()
        )
    };
    let (ok, missing) = if have.is_empty() {
        (
            String::new(),
            format!("; still missing: {}", lack.join(", ")),
        )
    } else if lack.is_empty() {
        (format!("; ready: {}", have.join(", ")), String::new())
    } else {
        (
            format!("; found: {}", have.join(", ")),
            format!("; missing: {}", lack.join(", ")),
        )
    };
    Ok(format!("{found}{ok}{missing}"))
}

/// Set one key of an ini in place, adding its section when the file has none.
/// Comments, blank lines and every other key stay as they are.
pub fn set_ini_key(text: &str, section: &str, key: &str, value: &str) -> String {
    let section = section.to_ascii_lowercase();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut current = String::new();
    let mut replaced = false;
    for line in lines.iter_mut() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            current = t[1..t.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if current == section {
            if let Some((k, _)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    *line = format!("{key} = {value}");
                    replaced = true;
                    break;
                }
            }
        }
    }
    if !replaced {
        let header = format!("[{section}]");
        match lines
            .iter()
            .position(|l| l.trim().eq_ignore_ascii_case(&header))
        {
            Some(i) => lines.insert(i + 1, format!("{key} = {value}")),
            None => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(header);
                lines.push(format!("{key} = {value}"));
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The engine's `[fefw] output` for a resolution label: the size the game
/// renders its picture, UI and post effects at. "Native display" leaves that to
/// the game, whose own config is `console` (docked 2560×1440, handheld 1080p).
pub fn output_mode(label: &str) -> &'static str {
    match label {
        "1280 × 720" => "720p",
        "1920 × 1080" => "1080p",
        "2560 × 1440" => "1440p",
        "3840 × 2160" => "2160p",
        _ => "console",
    }
}

/// The size a resolution label names, when it names one.
pub fn resolution_size(label: &str) -> Option<(u32, u32)> {
    match label {
        "1280 × 720" => Some((1280, 720)),
        "1920 × 1080" => Some((1920, 1080)),
        "2560 × 1440" => Some((2560, 1440)),
        "3840 × 2160" => Some((3840, 2160)),
        _ => None,
    }
}

/// The engine's `[display] window_mode` for a display-mode label.
pub fn window_mode(label: &str) -> &'static str {
    match label {
        "Borderless window" => "borderless",
        "Exclusive fullscreen" => "exclusive",
        _ => "windowed",
    }
}

/// The engine's `[upscale] sharpness` (0 to 1, applied live) for a label on the Graphics page. The four steps are the ones the
/// game's own menu offers: FSR 1, NIS and FSR 3's RCAS all sharpen, and 0.8 is the engine's own default.
pub fn sharpen_value(label: &str) -> &'static str {
    match label {
        "Off" => "0",
        "Low" => "0.45",
        "High" => "1",
        _ => "0.8",
    }
}

/// The page's label for a sharpness the game's own menu set: the nearest step, so a hand-edited value still shows something
/// sensible.
fn sharpen_label(value: &str) -> Option<&'static str> {
    let n: f32 = value.trim().parse().ok()?;
    Some(if n <= 0.01 {
        "Off"
    } else if n < 0.6 {
        "Low"
    } else if n < 0.95 {
        "Default"
    } else {
        "High"
    })
}

/// The Graphics page's label for one of the engine's upscale modes.
pub fn upscale_label(mode: &str) -> &'static str {
    match mode {
        "fsr1" | "spatial" => "FSR 1.0",
        "fsr3" => "FSR 3.1",
        "nis" => "NVIDIA Image Scaling",
        "dlss" => "DLSS",
        "dlaa" => "DLAA",
        _ => "Off",
    }
}

/// What the game's own ini files say it will start with. The launcher writes
/// them before each start, and the game and its F1 menu write them too, so
/// reading them back is the only way a page can show the truth rather than the
/// wish.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Live {
    /// `[fefw] output`, how the engine names the picture size.
    pub output: String,
    /// The effective `[upscale] mode`, after play mode's merge.
    pub upscaling: String,
    pub window_mode: String,
    pub vsync: bool,
    pub aspect: String,
    /// A number, or `auto` for the display's refresh rate.
    pub fps_limit: String,
    pub window: (u32, u32),
    /// The size the 3D picture is rendered at.
    pub picture: (u32, u32),
    pub volume: f32,
    /// Every key the two files carry, by `section.key` (fefw.ini wins where
    /// both have one, as play mode's merge does).
    pub values: std::collections::BTreeMap<String, String>,
}

fn ini_parse(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with(';') || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_ascii_lowercase();
        } else if let Some((k, v)) = t.split_once('=') {
            out.insert(
                format!("{section}.{}", k.trim().to_ascii_lowercase()),
                v.trim().to_string(),
            );
        }
    }
    out
}

fn ini_map(path: &Path) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(path)
        .map(|t| ini_parse(&t))
        .unwrap_or_default()
}

/// Read the two files in `dir`; `None` when neither of them is there yet.
pub fn live_in(dir: &Path) -> Option<Live> {
    let game = ini_map(&dir.join("fefw.ini"));
    let controls = ini_map(&dir.join("controls.ini"));
    if game.is_empty() && controls.is_empty() {
        return None;
    }
    let get = |k: &str| controls.get(k).or_else(|| game.get(k)).cloned();
    let num = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok());
    let window = (
        num("display.width").unwrap_or(0),
        num("display.height").unwrap_or(0),
    );
    // play_display.cpp's internal_size(): a fixed render size wins, else the
    // window at render_scale (100 = the window's own size).
    let scale = (num("display.render_scale").unwrap_or(100) as f32 / 100.0).clamp(0.25, 4.0);
    let picture = match (num("display.render_width"), num("display.render_height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => (
            (window.0 as f32 * scale) as u32,
            (window.1 as f32 * scale) as u32,
        ),
    };
    let mut values: std::collections::BTreeMap<String, String> =
        controls.clone().into_iter().collect();
    values.extend(game.clone());
    Some(Live {
        output: game
            .get("fefw.output")
            .cloned()
            .unwrap_or_else(|| "console".into()),
        // merge_env(): fefw.ini's [upscale] wins over the F1 menu's own copy.
        upscaling: game
            .get("upscale.mode")
            .or_else(|| controls.get("upscale.mode"))
            .cloned()
            .unwrap_or_else(|| "off".into()),
        window_mode: get("display.window_mode").unwrap_or_else(|| "windowed".into()),
        vsync: matches!(
            get("display.vsync").as_deref(),
            Some("1") | Some("true") | Some("on")
        ),
        aspect: get("display.aspect").unwrap_or_else(|| "auto".into()),
        fps_limit: get("display.fps_limit").unwrap_or_else(|| "auto".into()),
        window,
        picture,
        volume: get("audio.volume")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0),
        values,
    })
}

/// The run folder's files, cached for a second so a page can ask every frame.
pub fn live() -> Option<Live> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Option<Live>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, live)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(1) {
            return live.clone();
        }
    }
    let fresh = run_dir().and_then(|d| live_in(&d));
    *cache = Some((Instant::now(), fresh.clone()));
    fresh
}

/// Give the built game EmblemWeave's taskbar icon.
///
/// Windows draws a program's taskbar button from the icon stored in its .exe,
/// and every build writes the engine's own into fefw.exe. This runs at start,
/// after a build and before every start, so the game is never seen with the
/// engine's purple icon unless a copy is running and cannot be rewritten.
#[cfg(windows)]
pub fn brand_game() -> Result<bool, String> {
    let root = root().ok_or("the engine was not found")?;
    let exe = game_exe(&root);
    if !exe.is_file() || crate::cheats::process::find(crate::cheats::GAME_EXE).is_some() {
        return Ok(false);
    }
    crate::exe_icon::brand(&exe)
}

#[cfg(not(windows))]
pub fn brand_game() -> Result<bool, String> {
    Ok(false)
}

/// The FSR 3.1 DLL the engine should load, whichever SDK layout is installed.
fn fsr_dll(sdk: &Sdk) -> Option<PathBuf> {
    let d = sdk.fsr3.as_ref()?;
    ["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_vk.dll"]
        .iter()
        .map(|f| d.join(f))
        .find(|p| p.is_file())
}

type Values = std::collections::BTreeMap<String, String>;

/// The values the launcher last put into the game's files. A start writes only
/// what differs from them, so a setting changed in the game's own F1 menu stays
/// as it was instead of being reset by the next launch.
pub fn applied_values() -> Values {
    crate::config::Config::dir()
        .map(|d| d.join("applied.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_applied(v: &Values) {
    if let Some(p) = crate::config::Config::dir().map(|d| d.join("applied.json")) {
        if let Ok(t) = serde_json::to_string_pretty(v) {
            let _ = std::fs::write(p, t);
        }
    }
}

/// Every value the Graphics and Audio pages own, by `section.key`. The upscaler
/// lives in both of the game's files, so it is written to both.
pub fn desired_values(g: &crate::config::Graphics, a: &crate::config::Audio) -> Values {
    let mut d = Values::new();
    let sdk = sdk();
    d.insert("fefw.output".into(), output_mode(&g.resolution).into());
    // Never ask the engine for an upscaler whose DLLs are not there: the run
    // folder is created by this launcher, so it is the one place they can be,
    // and a mode the engine cannot load is a black picture with working sound.
    let mode = match upscale_mode(&g.upscaling) {
        "fsr3" if fsr_dll(&sdk).is_none() => "fsr1",
        "dlss" | "dlaa" if sdk.dlss.is_none() => "off",
        "xess" if sdk.xess.is_none() => "fsr1",
        other => other,
    };
    d.insert("upscale.mode".into(), mode.into());
    d.insert(
        "upscale.sharpness".into(),
        sharpen_value(&g.sharpening).into(),
    );
    d.insert(
        "display.window_mode".into(),
        window_mode(&g.display_mode).into(),
    );
    d.insert(
        "display.vsync".into(),
        if g.vsync { "1" } else { "0" }.into(),
    );
    d.insert("display.aspect".into(), g.aspect.clone());
    d.insert("display.fps_limit".into(), g.frame_limit.to_string());
    let scale = g.render_scale.clamp(0.5, 2.0);
    let even = |v: f32| (((v.round() as i64) & !1).max(320)).to_string();
    if let Some((w, h)) = resolution_size(&g.resolution) {
        d.insert("display.width".into(), w.to_string());
        d.insert("display.height".into(), h.to_string());
        // The picture at the render scale the page shows: 1.00× is the chosen
        // size, below it renders less and lets the upscaler fill the rest.
        d.insert("display.render_width".into(), even(w as f32 * scale));
        d.insert("display.render_height".into(), even(h as f32 * scale));
    } else if (scale - 1.0).abs() > 0.001 {
        // Native display with a render scale: the engine scales its own window
        // sized picture, so no fixed size may pin it.
        d.insert("display.render_width".into(), "0".into());
        d.insert("display.render_height".into(), "0".into());
        d.insert(
            "display.render_scale".into(),
            (scale * 100.0).round().to_string(),
        );
    }
    // Point the engine at the DLLs the Graphics page found. Its own defaults
    // only look beside the game and the launcher, so naming them keeps what the
    // page reports and what the game loads the same file.
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    if let Some(dll) = fsr_dll(&sdk) {
        d.insert("upscale.fsr_dll".into(), fwd(&dll));
    }
    if let Some(dir) = &sdk.dlss {
        d.insert("upscale.dlss_dll".into(), fwd(dir));
    }
    if let Some(dir) = &sdk.xess {
        d.insert("upscale.xess_dll".into(), fwd(dir));
    }
    // The rest of the Rendering page: the preset the temporal upscalers scale by, the scene buffer's
    // share of the output, and what makes the frames between two rendered ones.
    d.insert(
        "upscale.quality".into(),
        upscale_quality_word(&g.upscale_quality).into(),
    );
    d.insert(
        "upscale.scene_scale".into(),
        g.scene_scale.clamp(25, 100).to_string(),
    );
    d.insert("dlss.frame_gen".into(), frame_gen_word(&g.frame_gen).into());
    d.insert(
        "dlss.upscaler".into(),
        dlss_upscaler_word(&g.dlss_upscaler).into(),
    );
    d.insert(
        "display.astc_quality".into(),
        g.astc_quality.to_ascii_lowercase(),
    );
    d.insert("gfx.gpu".into(), gpu_word(&g.gpu).into());
    // Shader work while playing is three pools: whole pipelines built at a draw, recorded pipelines prewarmed at boot, and
    // shader parts compiled in the background. The page's one number caps all three; before this it only reached the parts
    // pool, so a player who chose 2 still had four prewarm workers and six preload readers fighting the game (#10).
    if g.compile_threads > 0 {
        let n = g.compile_threads.min(8).to_string();
        for key in [
            "nvn.gpl_threads",
            "nvn.prewarm_threads",
            "nvn.shader_preload_threads",
        ] {
            d.insert(key.into(), n.clone());
        }
    }
    d.insert(
        "nvn.compile_priority".into(),
        compile_priority_word(&g.compile_priority).into(),
    );
    d.insert(
        "nvn.compile_eco".into(),
        if g.compile_eco { "1" } else { "0" }.into(),
    );
    d.insert(
        "audio.volume".into(),
        format!("{:.2}", a.master.clamp(0.0, 1.0)),
    );
    d
}

/// The page's label for the engine's `[upscale] quality`.
fn upscale_quality_word(label: &str) -> &'static str {
    match label {
        "Quality" => "quality",
        "Balanced" => "balanced",
        "Performance" => "performance",
        "Ultra performance" => "ultra_performance",
        _ => "native",
    }
}

fn upscale_quality_label(word: &str) -> Option<&'static str> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "native" => "Native",
        "quality" => "Quality",
        "balanced" => "Balanced",
        "performance" => "Performance",
        "ultra_performance" | "ultraperformance" => "Ultra performance",
        _ => return None,
    })
}

fn frame_gen_word(label: &str) -> &'static str {
    match label {
        "DLSS Frame Generation" => "1",
        "FSR 3 Frame Generation" => "fsr3",
        _ => "0",
    }
}

fn frame_gen_label(word: &str) -> Option<&'static str> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "0" | "off" | "false" => "Off",
        "1" | "dlss" | "on" | "true" => "DLSS Frame Generation",
        "2" => "DLSS Frame Generation",
        "fsr3" => "FSR 3 Frame Generation",
        _ => return None,
    })
}

fn dlss_upscaler_word(label: &str) -> &'static str {
    match label {
        "TAAU" => "taau",
        "DLSS" => "dlss",
        "FSR 3" => "fsr3",
        "XeSS" => "xess",
        "Bilinear" => "bilinear",
        _ => "auto",
    }
}

fn dlss_upscaler_label(word: &str) -> Option<&'static str> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "auto" | "" => "Automatic",
        "taau" => "TAAU",
        "dlss" => "DLSS",
        "fsr3" => "FSR 3",
        "xess" => "XeSS",
        "bilinear" => "Bilinear",
        _ => return None,
    })
}

fn gpu_word(label: &str) -> &'static str {
    match label {
        "Discrete" => "discrete",
        "Integrated" => "integrated",
        _ => "",
    }
}

fn gpu_label(word: &str) -> Option<&'static str> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "automatic" => "Automatic",
        "discrete" => "Discrete",
        "integrated" => "Integrated",
        _ => return None,
    })
}

fn compile_priority_word(label: &str) -> &'static str {
    match label {
        "Normal" => "normal",
        "Idle" => "idle",
        _ => "below",
    }
}

/// The page's label for the engine's `[fefw] output`.
fn output_label(mode: &str) -> Option<&'static str> {
    Some(match mode {
        "720p" => "1280 \u{00d7} 720",
        "1080p" => "1920 \u{00d7} 1080",
        "1440p" => "2560 \u{00d7} 1440",
        "2160p" => "3840 \u{00d7} 2160",
        "console" => "Native display",
        _ => return None,
    })
}

/// The page's name for an aspect the engine wrote. The engine accepts a `W:H`
/// ratio or a number from 1.0 to 5.0; both arrive here. Standard ratios become
/// the label the combo lists, and any other number keeps its value in a form a
/// person can read instead of the engine's full float expansion.
pub fn aspect_label(value: &str) -> String {
    let t = value.trim();
    if let Ok(n) = t.parse::<f32>() {
        for (label, ratio) in [
            ("16:9", 16.0 / 9.0),
            ("16:10", 16.0 / 10.0),
            ("21:9", 21.0 / 9.0),
            ("32:9", 32.0 / 9.0),
        ] {
            if (n - ratio).abs() < 0.001 {
                return label.to_string();
            }
        }
        if (1.0..=5.0).contains(&n) {
            return format!("{n:.3}:1");
        }
    }
    t.to_string()
}

fn window_mode_label(mode: &str) -> Option<&'static str> {
    Some(match mode {
        "windowed" => "Windowed",
        "borderless" => "Borderless window",
        "exclusive" => "Exclusive fullscreen",
        _ => return None,
    })
}

/// What the game has changed in its own menus: the file's value for a key whose
/// page still asks for the value the launcher last wrote. A setting changed on
/// the page is the player's newer intent, and is left for the next start.
fn take_back<'a>(
    desired: &Values,
    applied: &Values,
    on_file: &'a Values,
    id: &str,
) -> Option<&'a str> {
    let want = desired.get(id)?;
    if applied.get(id) != Some(want) {
        return None;
    }
    let now = on_file.get(id)?;
    (now.as_str() != want.as_str()).then_some(now.as_str())
}

fn put<T: PartialEq>(field: &mut T, value: T) -> bool {
    if *field == value {
        return false;
    }
    *field = value;
    true
}

fn adopt_from(
    g: &mut crate::config::Graphics,
    a: &mut crate::config::Audio,
    desired: &Values,
    applied: &Values,
    on_file: &Values,
) -> bool {
    let mut changed = false;
    if let Some(v) = take_back(desired, applied, on_file, "upscale.mode") {
        changed |= put(&mut g.upscaling, upscale_label(v).to_string());
    }
    if let Some(v) = take_back(desired, applied, on_file, "upscale.sharpness") {
        if let Some(label) = sharpen_label(v) {
            changed |= put(&mut g.sharpening, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "fefw.output") {
        if let Some(label) = output_label(v) {
            changed |= put(&mut g.resolution, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "display.window_mode") {
        if let Some(label) = window_mode_label(v) {
            changed |= put(&mut g.display_mode, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "display.aspect") {
        // "auto" is the engine's own default; leaving it out keeps the page's
        // list of ratios intact. The engine resolves auto to a number such as
        // 1.7777777777777777, so a numeric value is shown as the ratio it is.
        if !v.is_empty() && !v.eq_ignore_ascii_case("auto") {
            changed |= put(&mut g.aspect, aspect_label(v));
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "upscale.quality") {
        if let Some(label) = upscale_quality_label(v) {
            changed |= put(&mut g.upscale_quality, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "upscale.scene_scale") {
        if let Ok(n) = v.trim().parse::<u32>() {
            changed |= put(&mut g.scene_scale, n.clamp(25, 100));
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "dlss.frame_gen") {
        if let Some(label) = frame_gen_label(v) {
            changed |= put(&mut g.frame_gen, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "dlss.upscaler") {
        if let Some(label) = dlss_upscaler_label(v) {
            changed |= put(&mut g.dlss_upscaler, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "display.astc_quality") {
        // The engine's own words are lower case; the page capitalises them.
        let mut label = v.trim().to_ascii_lowercase();
        if let Some(c) = label.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        if matches!(label.as_str(), "High" | "Fast" | "Uncompressed") {
            changed |= put(&mut g.astc_quality, label);
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "gfx.gpu") {
        if let Some(label) = gpu_label(v) {
            changed |= put(&mut g.gpu, label.to_string());
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "display.vsync") {
        changed |= put(&mut g.vsync, matches!(v, "1" | "true" | "on" | "yes"));
    }
    // The files carry the render size in pixels while the page shows a ratio, so
    // the window (`display.width`) is the reference: a fixed resolution's size or
    // the console's own one. `render_scale` is the engine's percent for the case
    // where it sizes the picture from its window.
    let render_keys = ["display.render_width", "display.render_height"];
    if !render_keys
        .iter()
        .any(|id| applied.get(*id) != desired.get(*id))
    {
        let number = |id: &str| on_file.get(id).and_then(|v| v.trim().parse::<f32>().ok());
        let scale_now = match (number("display.render_width"), number("display.width")) {
            (Some(render), Some(width)) if render > 0.0 && width > 0.0 => Some(render / width),
            _ => number("display.render_scale").map(|percent| percent / 100.0),
        };
        if let Some(scale) = scale_now {
            let scale = (scale * 100.0).round() / 100.0;
            if (0.5..=2.0).contains(&scale) {
                changed |= put(&mut g.render_scale, scale);
            }
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "display.fps_limit") {
        if let Ok(fps) = v.parse::<u32>() {
            if (30..=240).contains(&fps) {
                changed |= put(&mut g.frame_limit, fps);
            }
        }
    }
    if let Some(v) = take_back(desired, applied, on_file, "audio.volume") {
        if let Ok(vol) = v.parse::<f32>() {
            changed |= put(&mut a.master, vol.clamp(0.0, 1.0));
        }
    }
    changed
}

/// Take the values the game's own F1 menu changed back into the pages, so they
/// show what the next start will use instead of only naming it.
pub fn adopt(g: &mut crate::config::Graphics, a: &mut crate::config::Audio) -> bool {
    // The pages are drawn many times a second; the files change at human speed.
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if now.saturating_sub(LAST.load(std::sync::atomic::Ordering::Relaxed)) < 1000 {
        return false;
    }
    LAST.store(now, std::sync::atomic::Ordering::Relaxed);
    let Some(live) = live() else { return false };
    let desired = desired_values(g, a);
    let applied = applied_values();
    adopt_from(g, a, &desired, &applied, &live.values)
}

/// Write the Graphics and Audio values the game can act on, but only the ones
/// that changed since the launcher last wrote them.
///
/// The picture size and the upscaler go into `fefw.ini`: play mode merges that
/// file's `[upscale]` keys over its own, and `[fefw] output` is the game's
/// picture size. The window mode, aspect, vsync, frame cap, render size and
/// volume go into `controls.ini`, which holds play mode's `[display]` and
/// `[audio]` sections. A key the game's own menu has changed since the last
/// write is left alone: the player made that choice in the game.
fn write_settings(
    dir: &Path,
    g: &crate::config::Graphics,
    a: &crate::config::Audio,
    applied: &mut Values,
) -> Result<bool, String> {
    let fefw_path = dir.join("fefw.ini");
    let mut fefw = std::fs::read_to_string(&fefw_path)
        .map_err(|e| format!("reading {}: {e}", fefw_path.display()))?;
    // The game writes controls.ini on its first start; a file made here is
    // filled with the factory settings by play mode, which keeps these keys.
    let ctl_path = dir.join("controls.ini");
    let mut ctl = std::fs::read_to_string(&ctl_path)
        .unwrap_or_else(|_| "[meta]\ndefaults_rev = 1\n".to_string());
    let on_disk = (ini_parse(&fefw), ini_parse(&ctl));

    let mut touched = false;
    // A folder built before the launcher wrote an absolute mods path still says `mods = mods`. Rewrite it at Play so an update
    // fixes an existing game folder too, not only one built from now on (#39).
    let mods = dir.join("mods").display().to_string().replace('\\', "/");
    if ini_parse(&fefw).get("fs.mods") != Some(&mods) {
        fefw = set_ini_key(&fefw, "fs", "mods", &mods);
        touched = true;
    }
    for (id, value) in desired_values(g, a) {
        let (section, key) = id.split_once('.').unwrap_or(("", ""));
        let (in_fefw, in_ctl) = match section {
            "fefw" => (true, false),
            "display" | "audio" => (false, true),
            "nvn" | "dlss" | "gfx" => (true, false),
            _ => (true, true),
        };
        let current = if in_fefw {
            on_disk.0.get(&id)
        } else {
            on_disk.1.get(&id)
        };
        if current.is_some_and(|v| v == &value) {
            // The file already says what the page wants.
            applied.insert(id, value);
            continue;
        }
        // Present but different, and still the value the launcher wrote: the
        // game's F1 menu changed it since, and that choice stands.
        if current.is_some() && applied.get(&id) == Some(&value) {
            continue;
        }
        if in_fefw {
            fefw = set_ini_key(&fefw, section, key, &value);
        }
        if in_ctl {
            ctl = set_ini_key(&ctl, section, key, &value);
        }
        applied.insert(id, value);
        touched = true;
    }
    if !touched {
        return Ok(false);
    }
    std::fs::write(&fefw_path, fefw)
        .map_err(|e| format!("writing {}: {e}", fefw_path.display()))?;
    std::fs::write(&ctl_path, ctl).map_err(|e| format!("writing {}: {e}", ctl_path.display()))?;
    Ok(true)
}

// One line the launcher shows after a launch that did not draw anything. The
// engine's own log is written by another process, so a watcher thread fills
// this and the app drains it once per frame.
static PLAY_ALERT: std::sync::OnceLock<std::sync::Mutex<Vec<(String, bool)>>> =
    std::sync::OnceLock::new();

fn alerts() -> &'static std::sync::Mutex<Vec<(String, bool)>> {
    PLAY_ALERT.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// The next launch finding, if the watcher made one, and whether it is good
/// news. Both kinds are recorded: "it presented a frame at 59.9 fps" is what
/// tells a black-screen report apart from a game that never drew at all.
pub fn take_play_alert() -> Option<(String, bool)> {
    let mut a = alerts().lock().unwrap_or_else(|e| e.into_inner());
    if a.is_empty() {
        None
    } else {
        Some(a.remove(0))
    }
}

fn alert(text: String, good: bool) {
    alerts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((text, good));
}

static WATCHING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether a launch is still being watched, so the window keeps repainting
/// until its finding arrives.
pub fn play_watch_active() -> bool {
    WATCHING.load(std::sync::atomic::Ordering::Relaxed) > 0
}

struct WatchGuard;

impl Drop for WatchGuard {
    fn drop(&mut self) {
        WATCHING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// What the engine logged after `from` bytes. A log that got shorter was
/// started over, so it is read whole.
fn new_log_text(path: &Path, from: &mut u64) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return String::new();
    };
    if meta.len() < *from {
        *from = 0;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    use std::io::{Read, Seek, SeekFrom};
    if file.seek(SeekFrom::Start(*from)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if file
        .take(meta.len().saturating_sub(*from).min(1 << 20))
        .read_to_end(&mut buf)
        .is_err()
    {
        return String::new();
    }
    *from += buf.len() as u64;
    String::from_utf8_lossy(&buf).into_owned()
}

/// The frame rate the engine reports once a second while it presents, and the
/// GPU it named at start. Both are logged by the engine itself.
fn presented_fps(text: &str) -> Option<f32> {
    // Only the renderer's per-second line carries "present pass": other lines
    // mention a frame rate too ("gameplay at 60 fps"), and a run that never
    // drew must not look like one that did.
    text.lines()
        .filter(|l| l.contains("present pass"))
        .find_map(|l| {
            let rest = l.split("fps ").nth(1)?;
            rest.split(',').next()?.trim().parse::<f32>().ok()
        })
}

fn gpu_name(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.split("using GPU ").nth(1)?;
        let name = rest.split(": ").nth(1)?;
        // "Name (discrete, Vulkan …, 12.0 GiB)" and "Name (default rule)":
        // the name alone is what a player recognises.
        Some(name.split(" (").next()?.trim().to_string())
    })
}

/// The last lines that look like a failure, for a message that says why.
fn last_error(log: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log).ok()?;
    text.lines()
        .filter(|l| {
            l.contains("error") || l.contains("ERR") || l.contains("failed") || l.contains("vulkan")
        })
        .next_back()
        .map(|l| l.trim().to_string())
}

/// Delete the `RCX*.tmp` scratch files lld leaves in a release folder after an
/// interrupted link. Only that name is touched: the rest of the folder is the
/// build. Returns how many files went.
fn tidy_link_temps(builds: &Path) -> usize {
    let mut gone = 0;
    let Ok(dirs) = std::fs::read_dir(builds) else {
        return 0;
    };
    for dir in dirs.flatten() {
        let release = dir.path().join("release");
        let Ok(files) = std::fs::read_dir(&release) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().to_string();
            let temp = name.starts_with("RCX") && name.ends_with(".tmp");
            if temp && std::fs::remove_file(file.path()).is_ok() {
                gone += 1;
            }
        }
    }
    gone
}

/// Watch a launch until it draws or gives up. The engine logs "fps N, …" only
/// when a frame is presented, so a window that stays black while the sound
/// plays is exactly a run with no such line: it is named here instead of
/// leaving the player with nothing to report but "black screen".
fn watch_first_frame(mut child: std::process::Child, log: PathBuf, from: u64) {
    WATCHING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        let _done = WatchGuard;
        let started = std::time::Instant::now();
        let mut offset = from;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    let why = last_error(&log)
                        .map(|e| format!(" Last line: {e}"))
                        .unwrap_or_default();
                    alert(
                        format!(
                            "The game exited after {} s without finishing (code {:?}).{why} The Game Log has everything it printed.",
                            started.elapsed().as_secs(),
                            status.code()
                        ),
                        false,
                    );
                }
                return;
            }
            let text = new_log_text(&log, &mut offset);
            if let Some(fps) = presented_fps(&text) {
                alert(
                    format!(
                        "The game presented its first frame after {} s ({fps:.1} fps).",
                        started.elapsed().as_secs()
                    ),
                    true,
                );
                return;
            }
            if started.elapsed().as_secs() >= 90 {
                let gpu = gpu_name(&std::fs::read_to_string(&log).unwrap_or_default())
                    .unwrap_or_else(|| "the graphics card it picked".into());
                alert(
                    format!(
                        "The game has been running for 90 s and has not presented a single frame: the window looks black while the sound plays. \
It is running on {gpu}. Try Windowed or Borderless on Graphics, update that card's Vulkan driver, and if it stays black send the Game Log."
                    ),
                    false,
                );
                return;
            }
        }
    });
}

/// Start the game from the run folder. It runs on its own; the launcher
/// stays open.
pub fn play(
    graphics: &crate::config::Graphics,
    audio: &crate::config::Audio,
) -> Result<(), String> {
    let root = root().ok_or("the engine was not found")?;
    let dir = run_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    let exe = game_exe(&root);
    if !exe.is_file() || !dir.join("fefw.ini").is_file() {
        return Err("the game is not built yet: run Build the game on Game Setup".into());
    }
    // A build from an earlier release can point the game at a packed .nup, which the engine cannot read in place. The launcher writes those
    // files before play, so reaching this means that write has not happened yet; say so instead of letting the game die on `main.npdm is
    // compressed in the .nup` (issue #24).
    // A game folder built by a release before 0.1.20 carries an engine that cannot read a package which keeps some files packed, and a
    // launcher-side overlay does not help it: the reader that decompresses those files arrived with 0.1.20. Mixed folders are the usual
    // cause (a new EmblemWeave.exe dropped into an old extracted folder), and its log says "main.npdm is compressed in the .nup" (#34).
    if let Some(note) = play_blocker_in(&dir) {
        return Err(note);
    }
    if packed_package_without_overlay().is_some() {
        return Err("this package keeps some of its files packed: press Play again and the launcher writes them into your work folder first".into());
    }
    // ExeFS patches are compiled into the built executable. If the active set
    // has changed since the last successful build, Play must not start a game
    // that silently lacks them (or still contains a removed one).
    let main = root.join("game").join("dump").join("exefs").join("main");
    let patch_plan = crate::patches::inspect(
        &dir.join("mods"),
        crate::patches::main_build_id(&main),
        std::fs::metadata(&main).ok().map(|m| m.len()),
    );
    if !patch_plan.can_build() {
        return Err(format!(
            "the installed ExeFS patches cannot be applied: {}",
            patch_plan.refusal()
        ));
    }
    if crate::patches::needs_build(&patch_plan) {
        return Err("the active ExeFS patches have changed since the last build: press Build the game, then Play again".into());
    }
    // The haptics backend is loaded from beside `fefw.exe` and is not part of a game folder that was built by an earlier
    // release, so it is put there on the way in as well as after a build: updating the launcher should not quietly drop
    // rumble to the standard two motors (issue #10). A running game holds the file, and a copy that fails leaves the one
    // already in place.
    let vendor = root
        .join("runtime")
        .join("third_party")
        .join("renhdrum")
        .join("renhdrum.dll");
    if vendor.is_file() && !dir.join("renhdrum.dll").is_file() {
        let _ = std::fs::copy(&vendor, dir.join("renhdrum.dll"));
    }
    // The taskbar shows the icon inside the .exe: make it EmblemWeave's.
    // Not while a copy is running (Windows cannot update a running program).
    let _ = brand_game();
    // The runtime's freeze watchdog counts idle time from the moment it starts,
    // and a cold start on this PC takes longer than its 10 s default to reach the
    // first presented frame (window, then the GPU device, then the swap chain and
    // the shader preload). Left alone every launch logs a FREEZE dump that is
    // not a freeze. 30 s still catches a real stall; a run started outside the
    // launcher gets the same value from the [diag] section of fefw.ini.
    // The picture size, window mode, frame cap and upscaler are the Graphics
    // settings the game can act on today; it reads them from its own ini files
    // at start. A file that cannot be written must not stop the game starting.
    let mut applied = applied_values();
    if write_settings(&dir, graphics, audio, &mut applied).is_ok() {
        save_applied(&applied);
    }
    let log = dir.join("fefw.log");
    let from = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    let mut cmd = Command::new(&exe);
    cmd.current_dir(&dir)
        // The runtime searches its executable, the working folder, and this folder for `upscalers/<vendor>` (#26).
        // The engine checkout is where the issue's FSR files were copied, and its ancestors cover a release layout.
        .env("NXRT_LAUNCHER_DIR", &root)
        .env("NXRT_DIAG_WATCHDOG_SECONDS", "30")
        // One line per second while frames are presented: the evidence the
        // watcher needs to tell a slow start from a black window.
        .env("NXRT_GFX_LOG_FPS", "1");
    if !graphics.re_shade {
        // ReShade's Vulkan layer is implicit and global. The loader's disable
        // switches apply to this child only, so switching ReShade off here does
        // not uninstall it or change any other game.
        cmd.env("DISABLE_VK_LAYER_reshade_1", "1")
            .env("DISABLE_VK_LAYER_reshade", "1");
    } else if let Some(dir) = exe.parent() {
        // A ReShade.ini from before 6.8, or one the setup wrote before the launcher
        // could catch it, still blocks the bare Home key. Every launch checks once.
        let _ = allow_plain_shortcuts(&dir.join("ReShade.ini"));
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("starting {}: {e}", exe.display()))?;
    watch_first_frame(child, log, from);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_an_engine_folder_is_remembered_where_lookups_read_it() {
        let dir = std::env::temp_dir().join(format!("ew-engine-pick-{}", std::process::id()));
        let install = dir.join("EmblemWeave-old");
        let checkout = install.join("engine").join("fefw-recomp");
        std::fs::create_dir_all(checkout.join("game-build")).unwrap();
        let file = dir.join("data").join("engine.txt");

        // The whole older copy is accepted and the checkout inside it is what gets remembered.
        assert_eq!(remember_in(&file, &install).unwrap(), checkout);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap().trim(),
            checkout.display().to_string()
        );
        // The checkout itself works too, as the picker's own help text says.
        assert_eq!(remember_in(&file, &checkout).unwrap(), checkout);

        // A folder without an engine is refused and leaves the remembered one in place.
        std::fs::create_dir_all(dir.join("unrelated")).unwrap();
        assert!(remember_in(&file, &dir.join("unrelated")).is_err());
        assert_eq!(
            std::fs::read_to_string(&file).unwrap().trim(),
            checkout.display().to_string()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_copy_beside_this_one_is_offered_when_nothing_is_built_here() {
        let dir = std::env::temp_dir().join(format!("ew-reuse-{}", std::process::id()));
        let here = dir.join("EmblemWeave-v0.1.40");
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(dir.join("unrelated")).unwrap();
        let old = dir.join("EmblemWeave-v0.1.39");
        let exe = game_exe(&old.join("engine").join("fefw-recomp"));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "built").unwrap();

        let found = find_existing(&here, &[], None).expect("the sibling build is offered");
        assert_eq!(found.root, old.join("engine").join("fefw-recomp"));
        assert_eq!(found.install, old);

        // A copy that already has its own build is not offered another one.
        let mine = here.join("engine").join("fefw-recomp");
        let mine_exe = game_exe(&mine);
        std::fs::create_dir_all(mine_exe.parent().unwrap()).unwrap();
        std::fs::write(&mine_exe, "mine").unwrap();
        assert!(find_existing(&here, &[], Some(&mine)).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_remembered_engine_in_another_settings_folder_is_offered() {
        let dir = std::env::temp_dir().join(format!("ew-reuse-memo-{}", std::process::id()));
        let here = dir.join("fresh");
        std::fs::create_dir_all(&here).unwrap();
        // The old copy lives on another drive: only its engine.txt names the checkout.
        let elsewhere = dir.join("drive");
        let root = elsewhere.join("engine").join("fefw-recomp");
        let exe = game_exe(&root);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "built").unwrap();
        let settings = dir.join("old-data");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("engine.txt"), root.display().to_string()).unwrap();

        let found =
            find_existing(&here, &[settings], None).expect("the remembered build is offered");
        assert_eq!(found.root, root);

        // An engine.txt pointing at this copy's own unbuilt checkout is not an offer. A copy with no siblings is used here, so
        // the built engine above cannot be found instead.
        let lone = dir.join("lone").join("fresh");
        std::fs::create_dir_all(&lone).unwrap();
        let own = lone.join("engine").join("fefw-recomp");
        std::fs::create_dir_all(own.join("game-build")).unwrap();
        let own_settings = dir.join("lone").join("data");
        std::fs::create_dir_all(&own_settings).unwrap();
        std::fs::write(own_settings.join("engine.txt"), own.display().to_string()).unwrap();
        assert!(find_existing(&lone, &[own_settings], Some(&own)).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reshade_apps_matches_the_build_path_without_case_or_slashes() {
        let dir = std::env::temp_dir().join(format!("ew-reshade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let apps = dir.join("ReShadeApps.ini");
        std::fs::write(
            &apps,
            "[Apps]\nApps=C:\\Games\\fefw.exe, D:/Other/game.exe\n",
        )
        .unwrap();

        assert!(reshade_apps_contains(
            &apps,
            Path::new(r"c:\games\FEFW.exe")
        ));
        assert!(reshade_apps_contains(
            &apps,
            Path::new(r"d:\other\GAME.exe")
        ));
        assert!(!reshade_apps_contains(
            &apps,
            Path::new(r"c:\games\other.exe")
        ));

        // The setup tool writes a byte order mark before `Apps` and CRLF line ends.
        std::fs::write(&apps, "\u{feff}Apps=C:\\Games\\fefw.exe\r\n\r\n").unwrap();
        assert!(reshade_apps_contains(
            &apps,
            Path::new(r"c:\games\fefw.exe")
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_overlay_key_is_freed_from_forced_modifiers_in_the_input_section_only() {
        let dir = std::env::temp_dir().join(format!("ew-reshade-ini-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("ReShade.ini");
        // 6.8's own default, with the same key name in another section left alone.
        std::fs::write(
            &ini,
            "[GENERAL]\r\nForceShortcutModifiers=1\r\n[INPUT]\r\nKeyOverlay=36,0,0,0\r\nForceShortcutModifiers=1\r\n[OVERLAY]\r\nShowFPS=2\r\n",
        )
        .unwrap();

        assert!(allow_plain_shortcuts(&ini));
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(
            text.contains("[GENERAL]\r\nForceShortcutModifiers=1\r\n"),
            "the other section is not touched: {text}"
        );
        assert!(
            text.contains("[INPUT]\r\nKeyOverlay=36,0,0,0\r\nForceShortcutModifiers=0\r\n"),
            "{text}"
        );
        assert_eq!(
            text.matches("ForceShortcutModifiers=0").count(),
            1,
            "{text}"
        );
        // Already right: nothing is rewritten, so the overlay keeps its state file.
        assert!(!allow_plain_shortcuts(&ini));
        assert!(!allow_plain_shortcuts(&dir.join("absent.ini")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn probe_line(
        version: &str,
        executable: &str,
        venv: bool,
        identity: bool,
        imports: &[&str],
    ) -> String {
        format!(
            "EW_PY_PROBE {}",
            serde_json::json!({
                "version": version,
                "executable": executable,
                "venv": venv,
                "identity": identity,
                "imports": imports,
            })
        )
    }

    #[test]
    fn a_healthy_python_environment_is_reused() {
        let line = probe_line(
            "3.13.5",
            r"C:\engine\tools\.venv\Scripts\python.exe",
            true,
            true,
            &[],
        );
        assert_eq!(
            super::parse_python_probe(&line, ""),
            PythonEnv::Healthy {
                version: "3.13.5".into(),
                executable: r"C:\engine\tools\.venv\Scripts\python.exe".into(),
            }
        );
    }

    #[test]
    fn an_old_or_foreign_interpreter_is_unusable() {
        let old = probe_line("3.8.10", r"C:\Python38\python.exe", true, true, &[]);
        let said = super::parse_python_probe(&old, "");
        assert!(matches!(said, PythonEnv::Unusable { .. }), "{said:?}");
        assert!(said.reason().contains("older than"), "{}", said.reason());

        // A global interpreter copied into the venv folder reports the same
        // executable but is not a virtual environment.
        let global = probe_line(
            "3.13.5",
            r"C:\engine\tools\.venv\Scripts\python.exe",
            false,
            true,
            &[],
        );
        let said = super::parse_python_probe(&global, "");
        assert!(
            said.reason().contains("not a virtual environment"),
            "{}",
            said.reason()
        );

        // A launcher path that resolves to another executable is not the
        // interpreter that was probed.
        let other = probe_line("3.13.5", r"C:\Python313\python.exe", true, false, &[]);
        let said = super::parse_python_probe(&other, "");
        assert!(said.reason().contains("reports"), "{}", said.reason());
    }

    #[test]
    fn a_missing_lifter_import_is_unusable_and_names_the_module() {
        let line = probe_line(
            "3.13.5",
            r"C:\engine\tools\.venv\Scripts\python.exe",
            true,
            true,
            &["capstone: ImportError: no module named capstone"],
        );
        let said = super::parse_python_probe(&line, "");
        assert!(matches!(said, PythonEnv::Unusable { .. }), "{said:?}");
        assert!(said.reason().contains("capstone"), "{}", said.reason());
    }

    #[test]
    fn an_interpreter_that_cannot_start_reports_the_dll_conflict() {
        let said = super::parse_python_probe(
            "",
            "ImportError: Module use of python311.dll conflicts with this version of Python.\n",
        );
        assert!(matches!(said, PythonEnv::Unusable { .. }), "{said:?}");
        assert!(said.reason().contains("python311.dll"), "{}", said.reason());
        // A report that is not the probe's own JSON is not reused either.
        assert!(matches!(
            super::parse_python_probe("EW_PY_PROBE {oops", ""),
            PythonEnv::Unusable { .. }
        ));
    }

    #[test]
    fn recreation_removes_only_the_engine_venv() {
        let root = std::env::temp_dir().join(format!("ew-venv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let venv = root.join("tools").join(".venv");
        let other = root.join("tools").join("other");
        std::fs::create_dir_all(&venv).unwrap();
        std::fs::write(venv.join("python.exe"), b"stale").unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("keep.txt"), b"keep").unwrap();
        std::fs::write(root.join("python.exe"), b"global").unwrap();

        super::remove_engine_venv(&root).unwrap();
        assert!(!venv.exists(), "the generated environment is gone");
        assert!(
            other.join("keep.txt").is_file(),
            "a sibling under tools is kept"
        );
        assert!(
            root.join("python.exe").is_file(),
            "an interpreter outside tools is kept"
        );
        // Removing it again is fine: a missing environment is the normal first build.
        super::remove_engine_venv(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_venv_is_not_an_error() {
        assert_eq!(
            super::probe_python_env(Path::new(r"C:\no\such\tools\.venv\Scripts\python.exe")),
            PythonEnv::Missing
        );
    }

    #[test]
    fn the_repair_writes_its_key_into_the_section_it_belongs_to() {
        let before = "[log]\npath = fefw.log\n[fs]\nexefs = C:/e\nromfs = E:/g.nup\nsave_dir = C:/s\n[nvn]\nshader_cache = C:/c\n";
        let after = set_ini_value(before, "fs", "romfs_overlay", "C:/w/romfs-overlay");
        // Anywhere inside [fs] is a key the engine reads; the file stays a flat list of lines.
        let fs = after.split("[nvn]").next().unwrap_or_default();
        assert!(fs.contains("romfs_overlay = C:/w/romfs-overlay"), "{after}");
        assert!(
            after.starts_with("[log]\npath = fefw.log\n[fs]\n"),
            "{after}"
        );
        assert!(after.contains("[nvn]\nshader_cache = C:/c\n"), "{after}");
        // A second run replaces the line instead of adding another one.
        let again = set_ini_value(&after, "fs", "romfs_overlay", "D:/other");
        assert_eq!(again.matches("romfs_overlay").count(), 1, "{again}");
        assert!(again.contains("romfs_overlay = D:/other"), "{again}");
        // A file with no [fs] section at all gets one.
        let plain = set_ini_value("[log]\npath = fefw.log\n", "fs", "romfs_overlay", "C:/w");
        assert!(plain.contains("[fs]\nromfs_overlay = C:/w\n"), "{plain}");
    }

    /// A minimal .nup with one base package whose RomFS holds one zstd-marked file, enough for `gamedata::packed_romfs`.
    fn write_packed_nup(path: &Path) {
        use sha2::{Digest, Sha256};
        let names: Vec<u8> = b"romfsdata.bin".to_vec(); // romfs at 0..5, data.bin at 5..13
        let toc_size = 0x30 + 0x40 + 2 * 0x18 + 0x50 + 2 * 4 + names.len();
        let data_off = 0x80 + toc_size;
        let payload: Vec<u8> = vec![0x28, 0xb5, 0x2f, 0xfd]; // zstd magic; the guard never decompresses it
        let mut toc = vec![0u8; toc_size];
        toc[0..4].copy_from_slice(b"TOC\x01");
        toc[4..8].copy_from_slice(&1u32.to_le_bytes());
        toc[8..16].copy_from_slice(&2u64.to_le_bytes());
        toc[16..24].copy_from_slice(&1u64.to_le_bytes());
        let pool_off = toc_size - names.len();
        toc[24..32].copy_from_slice(&(pool_off as u64).to_le_bytes());
        toc[32..40].copy_from_slice(&(names.len() as u64).to_le_bytes());
        let pkg = 0x30;
        toc[pkg..pkg + 8].copy_from_slice(&0x0400_7EF0_0000_0000u64.to_le_bytes());
        toc[pkg + 16..pkg + 20].copy_from_slice(&1u32.to_le_bytes()); // romfs directory
        let dir_at = pkg + 0x40;
        toc[dir_at + 12..dir_at + 16].copy_from_slice(&1u32.to_le_bytes()); // root: one child
        let d1 = dir_at + 0x18;
        toc[d1 + 4..d1 + 8].copy_from_slice(&5u32.to_le_bytes()); // "romfs"
        toc[d1 + 8..d1 + 12].copy_from_slice(&1u32.to_le_bytes());
        toc[d1 + 12..d1 + 16].copy_from_slice(&1u32.to_le_bytes());
        let file_at = dir_at + 2 * 0x18;
        toc[file_at..file_at + 4].copy_from_slice(&5u32.to_le_bytes());
        toc[file_at + 4..file_at + 8].copy_from_slice(&8u32.to_le_bytes());
        toc[file_at + 8..file_at + 16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        toc[file_at + 16..file_at + 24].copy_from_slice(&(data_off as u64).to_le_bytes());
        toc[file_at + 24..file_at + 32].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        toc[file_at + 32..file_at + 64].copy_from_slice(&Sha256::digest(&payload));
        toc[file_at + 64] = 1; // one zstd frame
        let child_at = file_at + 0x50;
        toc[child_at..child_at + 4].copy_from_slice(&1u32.to_le_bytes()); // root -> romfs
        toc[child_at + 4..child_at + 8].copy_from_slice(&2u32.to_le_bytes()); // romfs -> file 0
        toc[pool_off..].copy_from_slice(&names);
        let mut head = vec![0u8; 0x80];
        head[0..4].copy_from_slice(b"NUP\x01");
        head[4..6].copy_from_slice(&0x80u16.to_le_bytes());
        head[6..8].copy_from_slice(&1u16.to_le_bytes());
        head[8..16].copy_from_slice(&0x80u64.to_le_bytes());
        head[16..24].copy_from_slice(&(toc_size as u64).to_le_bytes());
        head[24..56].copy_from_slice(&Sha256::digest(&toc));
        head[56..64].copy_from_slice(&(data_off as u64).to_le_bytes());
        head[0x40..0x48].copy_from_slice(&((data_off + payload.len()) as u64).to_le_bytes());
        head[0x48..0x4C].copy_from_slice(&1u32.to_le_bytes());
        let mut out = head;
        out.extend(toc);
        out.extend(payload);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn an_old_build_pointing_at_a_packed_package_is_refused() {
        let dir = std::env::temp_dir().join(format!("ew-play-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let nup = dir.join("FEFW.nup");
        write_packed_nup(&nup);
        std::fs::write(
            dir.join("fefw.ini"),
            format!(
                "[engine]\nbuilt_by = 0.1.10\n[fs]\nromfs = {}\n",
                nup.display()
            ),
        )
        .unwrap();
        let note = play_blocker_in(&dir).expect("an old build with a packed package is refused");
        assert!(note.contains("0.1.10"), "{note}");
        assert!(note.contains("Press Build the game"), "{note}");
        // The same folder, built by this release, is not refused by this guard.
        std::fs::write(
            dir.join("fefw.ini"),
            format!(
                "[engine]\nbuilt_by = 0.1.33\npacked_nup = 1\n[fs]\nromfs = {}\n",
                nup.display()
            ),
        )
        .unwrap();
        assert_eq!(play_blocker_in(&dir), None);
        // A newer built_by with no marker is a rebuild that reused the old engine; it is refused too (the #34 report).
        std::fs::write(
            dir.join("fefw.ini"),
            format!(
                "[engine]\nbuilt_by = 0.1.32\n[fs]\nromfs = {}\n",
                nup.display()
            ),
        )
        .unwrap();
        let stale = play_blocker_in(&dir)
            .expect("an engine without the packed reader is refused even when built_by is newer");
        assert!(stale.contains("0.1.32"), "{stale}");
        assert!(stale.contains("old fefw.exe"), "{stale}");
        // Once Play has written the overlay (what 0.1.20 through 0.1.33 read), that same folder is allowed.
        let overlay = dir.join("romfs-overlay");
        std::fs::create_dir_all(&overlay).unwrap();
        std::fs::write(
            dir.join("fefw.ini"),
            format!(
                "[engine]\nbuilt_by = 0.1.32\n[fs]\nromfs = {}\nromfs_overlay = {}\n",
                nup.display(),
                overlay.display()
            ),
        )
        .unwrap();
        assert_eq!(play_blocker_in(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_in_place_update_does_not_make_the_carried_build_stale() {
        let dir = std::env::temp_dir().join(format!("ew-carried-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("fefw.ini"),
            "[engine]\nbuilt_by = 0.1.39\npacked_nup = 1\n[fs]\nromfs = game.nup\n",
        )
        .unwrap();
        // A launcher that only moved on is still told the build came from another release...
        assert!(stale_build_in(&dir, "0.1.40").is_some());
        // ...until the updater records that it carried this build forward on purpose.
        carry_build_forward(&dir, "0.1.40").unwrap();
        assert_eq!(stale_build_in(&dir, "0.1.40"), None);
        let text = std::fs::read_to_string(dir.join("fefw.ini")).unwrap();
        assert!(
            text.contains("built_by = 0.1.39"),
            "the build's own stamp is kept for reports: {text}"
        );
        assert!(
            text.contains("carried_by = 0.1.40"),
            "the update is recorded: {text}"
        );
        // A later launcher is stale again until its updater carries the same build.
        assert!(stale_build_in(&dir, "0.1.41").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_clang_driver_failure_is_recognized_and_repaired() {
        // The first line is what the #9 report ends with; the second is the one clang prints for its missing /W3 flag.
        assert!(driver_flags_rejected("clang++: error: unknown argument: '-clang:-fmacro-prefix-map=F:/Games/EmblemWeave-windows-x64=.\'"));
        assert!(driver_flags_rejected(
            "clang: error: no such file or directory: '/W3'"
        ));
        assert!(!driver_flags_rejected(
            "ninja: build stopped: subcommand failed."
        ));
    }

    #[test]
    fn the_build_ninja_regeneration_loop_requests_a_cache_reset() {
        // The end of the #34 log: CMake is re-run until Ninja gives up. Clearing CMakeFiles and the
        // cache lets the next configure write a fresh manifest without the stale dependency.
        let log = "[0/2] Re-checking globbed directories...\n[1/2] Re-running CMake...\nninja: error: manifest 'build.ninja' still dirty after 100 tries, perhaps system time is not set";
        assert!(needs_cache_reset(log));
        assert!(needs_cache_reset(
            "CMake Generate step failed.  Build files cannot be regenerated correctly."
        ));
        assert!(needs_cache_reset("the cache was made by another compiler"));
        assert!(needs_cache_reset(
            "CMake Error: The current CMakeCache.txt directory is different"
        ));
        assert!(!needs_cache_reset(
            "ninja: build stopped: subcommand failed."
        ));
        assert!(!needs_cache_reset("'bit' file not found"));
        // A shader or texture cache message is not a configure cache that needs deleting.
        assert!(!needs_cache_reset("nvn: shader cache: 12088 records"));
        assert!(!needs_cache_reset(
            "nvn.texdata: BC7 data from cache C:/cache/27/x.bc7"
        ));
    }

    #[test]
    fn an_extracted_sdk_in_downloads_is_searched_for_its_dlls() {
        let root = std::env::temp_dir().join(format!("ew-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root
            .join("Downloads")
            .join("fsr-sdk")
            .join("bin")
            .join("x64");
        std::fs::create_dir_all(&bin).unwrap();
        let dll = bin.join("amd_fidelityfx_vk.dll");
        std::fs::write(&dll, "").unwrap();
        let sources = vec![root.join("Downloads")];
        assert_eq!(
            find_upscaler_file(&sources, "amd_fidelityfx_vk.dll"),
            Some(dll)
        );
        assert_eq!(find_upscaler_file(&sources, "libxess.dll"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn upscaler_dlls_are_found_in_an_extracted_sdk_layout() {
        let root = std::env::temp_dir().join(format!("ew-sdk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // The two spellings the original project and the issue reporters used.
        let fsr = root.join("upscalers").join("fsr").join("bin").join("x64");
        std::fs::create_dir_all(&fsr).unwrap();
        std::fs::write(fsr.join("amd_fidelityfx_vk.dll"), "").unwrap();
        let xess = root.join("upscaler").join("xess").join("bin");
        std::fs::create_dir_all(&xess).unwrap();
        std::fs::write(xess.join("libxess.dll"), "").unwrap();
        assert_eq!(sdk_fsr(&[root.clone()]), Some(fsr));
        assert_eq!(
            sdk_in(&[root.clone()], &["xess"], XESS_FILES, true),
            Some(xess)
        );
        assert_eq!(
            sdk_in(&[root.clone()], &["streamline"], DLSS_FILES, true),
            None
        );
        // A half-copied SDK 2.x folder (loader only) must not count as a working FSR layout.
        let partial = root.join("partial").join("upscalers").join("fsr");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("amd_fidelityfx_loader_vk.dll"), "").unwrap();
        assert_eq!(sdk_fsr(&[root.join("partial")]), None);
        std::fs::write(partial.join("amd_fidelityfx_upscaler_vk.dll"), "").unwrap();
        assert_eq!(sdk_fsr(&[root.join("partial")]), Some(partial));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_reason_is_the_error_and_not_the_summary_after_it() {
        let lines: Vec<String> = [
            "-- Configuring incomplete, errors occurred!",
            "CMake Error at CMakeLists.txt:3 (cmake_minimum_required):",
            "  CMake 3.25 or higher is required. You are running version 3.20.5",
            "CMake Generate step failed. Build files cannot be regenerated correctly.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let said = reason_from(&lines);
        assert!(said.contains("CMake Error at CMakeLists.txt"), "{said}");
    }

    #[test]
    fn a_failed_step_is_named_by_the_script_it_ran() {
        assert_eq!(
            super::step_name(
                "pwsh",
                &[
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    "tools\\textures\\scripts\\fetch_third_party.ps1"
                ]
            ),
            "tools\\textures\\scripts\\fetch_third_party.ps1"
        );
        assert_eq!(
            super::step_name("cmd", &["/C", "recomp\\scripts\\build_game.cmd", "fefw"]),
            "/C"
        );
    }

    #[test]
    fn a_project_path_cmd_would_split_is_refused_with_the_reason() {
        use std::path::Path;
        assert_eq!(
            super::cmd_breaker(Path::new(r"G:\Games\PC Ports & Recomps\emblemweave")),
            Some('&')
        );
        assert_eq!(
            super::cmd_breaker(Path::new(r"C:\Users\me\EmblemWeave")),
            None
        );
        assert_eq!(
            super::cmd_breaker(Path::new(r"C:\Games (2)\EmblemWeave")),
            Some('(')
        );
        assert_eq!(
            super::cmd_breaker(Path::new(r"C:\Games\100%\EmblemWeave")),
            Some('%')
        );

        assert!(super::build_dir(Path::new(r"C:\Users\me\EmblemWeave")).is_ok());
        let said =
            super::build_dir(Path::new(r"G:\Games\PC Ports & Recomps\emblemweave")).unwrap_err();
        assert!(said.contains("contains '&'"), "names the character: {said}");
        assert!(
            said.contains("temporary junction"),
            "explains the junction and why it failed: {said}"
        );
        assert!(
            said.contains("PC Ports & Recomps"),
            "names the folder: {said}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_cmake_ninja_build_runs_from_a_folder_cmd_would_split() {
        use std::path::Path;

        // The engine's own build tools. A PC without them cannot compile anything, so the test says what it skipped
        // instead of failing on a missing install.
        if !super::which_tool("cmake") || !super::which_tool("ninja") {
            eprintln!("skipping the end-to-end special-path build: cmake and ninja are not installed");
            return;
        }
        let toolchain = match crate::toolchain::init() {
            Ok(toolchain) => toolchain,
            Err(e) => {
                eprintln!("skipping the end-to-end special-path build: the Windows toolchain is unavailable: {e}");
                return;
            }
        };

        // A folder name with an ampersand, the shape of the reported `G:\Games\PC Ports & Recomps`.
        let root = std::env::temp_dir().join(format!(
            "EmblemWeave & Regression {}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let sites = root.join("hook sites").join("main.sites");
        std::fs::create_dir_all(sites.parent().unwrap()).unwrap();
        std::fs::write(&sites, "fefw:main\n").unwrap();
        std::fs::write(root.join("main.c"), "int main(void) { return 0; }\n").unwrap();
        // The parts of the engine's build this exercises: a Ninja project, the hook-site list embedded in a command
        // ninja runs through cmd.exe, and an absolute path from the folder cmd.exe would split.
        std::fs::write(
            root.join("CMakeLists.txt"),
            r#"cmake_minimum_required(VERSION 3.25)
project(ew_special_path_probe C)
if(DEFINED ENV{FEFW_SITES} AND NOT "$ENV{FEFW_SITES}" STREQUAL "")
    set(FEFW_SITES "$ENV{FEFW_SITES}")
endif()
if(NOT DEFINED FEFW_SITES OR FEFW_SITES STREQUAL "")
    message(FATAL_ERROR "FEFW_SITES is not set")
endif()
configure_file("${FEFW_SITES}" "${CMAKE_BINARY_DIR}/main.sites" COPYONLY)
add_executable(probe main.c)
# The engine's lift rule (NxRecomp.cmake): an absolute working directory and the hook-site list in a command ninja runs
# through cmd.exe. CMake writes both as absolute paths, which is where the long spelling of the checkout used to be cut.
add_custom_command(OUTPUT "${CMAKE_BINARY_DIR}/lift.txt"
    COMMAND ${CMAKE_COMMAND} -E copy_if_different "${FEFW_SITES}" "${CMAKE_BINARY_DIR}/lift.txt"
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    DEPENDS "${FEFW_SITES}")
add_custom_target(lift ALL DEPENDS "${CMAKE_BINARY_DIR}/lift.txt")
add_dependencies(lift probe)
"#,
        )
        .unwrap();

        // The mapping the build applies before it starts: the folder it runs in, and the hook-site path beneath it.
        let view = super::build_dir(&root)
            .unwrap_or_else(|e| panic!("the build folder for a real '&' path was refused: {e}"));
        let build_root = &view.path;
        assert_ne!(
            build_root, &root,
            "the build has to run from a spelling cmd.exe can carry, not the long one"
        );
        assert_eq!(
            super::cmd_breaker(build_root),
            None,
            "the build folder still splits a cmd line: {}",
            build_root.display()
        );
        assert!(
            build_root.is_dir(),
            "the build folder does not name the checkout: {}",
            build_root.display()
        );
        let safe_sites = build_root.join(sites.strip_prefix(&root).unwrap());
        assert_eq!(
            super::cmd_breaker(&safe_sites),
            None,
            "the hook-site path still splits a cmd line: {}",
            safe_sites.display()
        );

        let build = Path::new("work").join("build").join("fefw");
        let mut configure = Command::new("cmake");
        configure
            .current_dir(build_root)
            .args(["-G", "Ninja"])
            .arg("-S")
            .arg(build_root)
            .arg("-B")
            .arg(build_root.join(&build))
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(format!("-DCMAKE_C_COMPILER={}", toolchain.compiler.display()))
            .env("FEFW_SITES", &safe_sites);
        // The environment every build step gets, from the same toolchain probe (INCLUDE, LIB and PATH from vcvars).
        crate::toolchain::scrub(&mut configure);
        for (k, v) in &toolchain.env {
            configure.env(k, v);
        }
        crate::toolchain::hidden(&mut configure);
        let out = configure.output().expect("starting cmake");
        assert!(
            out.status.success(),
            "cmake could not configure in {}:\n{}{}",
            build_root.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        let mut compile = Command::new("cmake");
        compile
            .current_dir(build_root)
            .arg("--build")
            .arg(build_root.join(&build));
        crate::toolchain::scrub(&mut compile);
        for (k, v) in &toolchain.env {
            compile.env(k, v);
        }
        crate::toolchain::hidden(&mut compile);
        let out = compile.output().expect("starting cmake --build");
        assert!(
            out.status.success(),
            "ninja could not build in {}:\n{}{}",
            build_root.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        let exe = build_root.join(&build).join("probe.exe");
        assert!(
            exe.is_file(),
            "the compiler and linker did not write {}; ninja ran, so its command line was cut somewhere",
            exe.display()
        );
        // The lift-shaped command runs with an absolute working directory and carries the hook-site path: the two places
        // the long spelling used to be cut.
        assert_eq!(
            std::fs::read_to_string(build_root.join(&build).join("lift.txt"))
                .expect("the lift-shaped command did not run"),
            "fefw:main\n"
        );
        // The end-to-end point: CMake wrote the whole build with the short spelling. A long path in this file is what
        // cmd.exe splits before the linker runs (`CMakeFiles\G_\Games\PC_Ports_`).
        let ninja = std::fs::read_to_string(build_root.join(&build).join("build.ninja"))
            .expect("reading build.ninja");
        assert!(
            !ninja.to_lowercase().contains("emblemweave & regression"),
            "build.ninja still names the long folder, so cmd.exe can split a generated command:\n{ninja}"
        );

        let junction = view.link.clone();
        drop(view);
        assert!(
            junction.is_none_or(|link| !link.exists()),
            "the junction was left behind after the build"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn a_real_special_path_gets_a_build_folder_cmd_can_carry() {
        let dir = std::env::temp_dir().join(format!("EmblemWeave & path {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let view = super::build_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(view.path, dir, "the build needs another spelling of the folder");
        assert_eq!(
            super::cmd_breaker(&view.path),
            None,
            "the build folder still splits a cmd line: {}",
            view.path.display()
        );
        assert!(view.path.is_dir(), "{} does not name the folder", view.path.display());
        // The same folder: what the build writes lands in the checkout, not in a copy.
        std::fs::write(dir.join("marker.txt"), "kept").unwrap();
        assert_eq!(
            std::fs::read_to_string(view.path.join("marker.txt")).unwrap(),
            "kept"
        );
        let link = view.link.clone().expect("the mapping must use a junction");
        drop(view);
        assert!(!link.exists(), "removing the view must remove only the junction");
        assert!(dir.join("marker.txt").is_file(), "the checkout must be untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_program_is_named_not_just_its_continuation() {
        // What a failed ninja step on Windows actually ends with.
        let lines: Vec<String> = [
            "[9/90] Linking CXX executable fefw.exe",
            "'ispc' is not recognized as an internal or external command,",
            "operable program or batch file.",
            "ninja: build stopped: subcommand failed.",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let said = reason_from(&lines);
        assert!(said.contains("ispc"), "{said}");
        assert_eq!(super::missing_program(&said).as_deref(), Some("ispc"));
        assert_eq!(super::missing_program("CMake Error at x.cmake:2"), None);
    }

    #[test]
    fn a_run_without_the_word_error_keeps_its_last_lines() {
        let lines: Vec<String> = [
            "ninja: build stopped: subcommand failed.",
            "Done building project",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            reason_from(&lines),
            "Done building project | ninja: build stopped: subcommand failed."
        );
    }

    #[test]
    fn the_installer_script_installs_every_missing_dependency() {
        let lines = install_lines_for(&[
            "Visual Studio C++ tools",
            "LLVM (clang-cl and lld-link)",
            "CMake",
            "PowerShell 7",
            "zbic (Python package)",
        ]);
        let s = install_script_lines(&lines);
        // A plain cmd script, or the console closes before anything is read.
        assert!(s.starts_with("@echo off"));
        assert!(s.contains("Microsoft.VisualStudio.2022.BuildTools"), "{s}");
        assert!(s.contains("Microsoft.VisualStudio.Workload.VCTools"), "{s}");
        // Without these winget stops at a prompt the console cannot answer.
        assert!(s.contains("--accept-package-agreements"));
        assert!(s.contains("--accept-source-agreements"));
        // Every tool in the list is run, not just the first.
        assert!(s.contains("--id LLVM.LLVM"), "{s}");
        assert!(s.contains("--id Kitware.CMake"), "{s}");
        assert!(s.contains("--id Microsoft.PowerShell"), "{s}");
        // A dependency installed by a command rather than winget runs too.
        assert!(
            lines.iter().any(|l| l == "py -m pip install zbic"),
            "{lines:?}"
        );
        assert!(s.trim_end().ends_with("pause"));
    }

    #[test]
    fn a_dependency_without_an_install_line_is_left_out() {
        let lines = install_lines_for(&["Shader translator", "Some tool nobody knows"]);
        assert!(lines.is_empty(), "{lines:?}");
        let s = install_script_lines(&lines);
        assert!(!s.contains("winget "), "{s}");
        assert!(s.contains("Done."));
    }

    #[test]
    fn the_ini_points_at_the_dump_saves_and_shaders() {
        let ini = ini_text(
            Path::new(r"C:\x\engine\fefw-recomp"),
            &RunConfig {
                exefs: PathBuf::from(r"C:\e"),
                romfs: PathBuf::from(r"E:\g.nup"),
                dlc: None,
                save_dir: PathBuf::from(r"C:\s"),
                mods: PathBuf::from(r"C:\play\mods"),
                romfs_overlay: None,
            },
        );
        assert!(ini.contains("romfs = E:/g.nup"));
        assert!(ini.contains("save_dir = C:/s"));
        assert!(ini.contains("shader_cache = C:/x/engine/fefw-recomp/work/shaders/spirv"));
        assert!(ini.contains("mods = C:/play/mods"));
        // A cold start takes over 10 s to reach the first frame here, so the
        // watchdog's default would dump a freeze that never happened.
        assert!(ini.contains("watchdog_seconds = 30"));
        // The upscaler section is written up front; Play patches its mode.
        assert!(ini.contains("[upscale]\nmode = off"));
        // A dump with no add-on content must not make the engine scan a default folder.
        assert!(ini.contains("[aoc]\ndir = \n"));
    }

    #[test]
    fn the_ini_points_at_dumped_add_on_content() {
        let ini = ini_text(
            Path::new(r"C:\x\engine\fefw-recomp"),
            &RunConfig {
                exefs: PathBuf::from(r"C:\e"),
                romfs: PathBuf::from(r"E:\g.nup"),
                dlc: Some(PathBuf::from(r"E:\FEFW\dlc")),
                save_dir: PathBuf::from(r"C:\s"),
                mods: PathBuf::from(r"C:\play\mods"),
                romfs_overlay: None,
            },
        );
        assert!(ini.contains("[aoc]\ndir = E:/FEFW/dlc"));
    }

    /// Issues #12 and #15: Setup accepts a `.nup`, the game reads its RomFS in place
    /// from it, and the shader extractor walks a folder and found nothing, so the
    /// corpus held only the shaders inside `main`. Run by hand against a real
    /// package: `EW_TEST_NUP=<game>.nup cargo test -p emblemweave a_nup -- --ignored`.
    #[test]
    #[ignore]
    fn a_nup_hands_the_extractor_a_folder_of_shader_archives() {
        let Some(nup) = std::env::var_os("EW_TEST_NUP").map(PathBuf::from) else {
            panic!("set EW_TEST_NUP to the game's .nup");
        };
        // EW_TEST_ROOT keeps the copy, so the extractor can be run on it by hand.
        let keep = std::env::var_os("EW_TEST_ROOT").map(PathBuf::from);
        let root = keep
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("ew-nup-shader-romfs"));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, copied) = shader_romfs(&root, &nup).expect("the .nup yields a folder");
        let (files, bytes) = copied.expect("a .nup is copied, not used as it is");
        assert!(dir.is_dir(), "{}", dir.display());
        assert!(files > 0 && bytes > 0, "{files} files, {bytes} bytes");
        // A second call reuses the copy instead of reading 1.4 GiB out of the package again.
        let (again, copied_again) = shader_romfs(&root, &nup).expect("the copy is reused");
        assert_eq!(again, dir);
        assert!(copied_again.is_none(), "the copy was made twice");
        let count = std::fs::read_dir(&dir).expect("the folder").count();
        assert!(count > 0, "the folder is empty");
        println!(
            "{files} shader archives ({bytes} bytes) below {}",
            dir.display()
        );
        if keep.is_none() {
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn the_upscaler_replaces_its_own_key_and_keeps_the_rest() {
        let ini = ini_text(
            Path::new(r"C:\x\engine\fefw-recomp"),
            &RunConfig {
                exefs: PathBuf::from(r"C:\e"),
                romfs: PathBuf::from(r"E:\g.nup"),
                dlc: None,
                save_dir: PathBuf::from(r"C:\s"),
                mods: PathBuf::from(r"C:\play\mods"),
                romfs_overlay: None,
            },
        );
        let patched = set_ini_key(&ini, "upscale", "mode", "fsr3");
        assert!(patched.contains("[upscale]\nmode = fsr3"));
        assert!(!patched.contains("mode = off"));
        assert_eq!(patched.matches("mode = fsr3").count(), 1);
        // Every other section survives, including the keys after it.
        assert!(patched.contains("upscaler = auto"));
        assert!(patched.contains("watchdog_seconds = 30"));
        // A file without the section gets one instead of losing the key.
        let bare = set_ini_key("[log]\npath = fefw.log\n", "upscale", "mode", "fsr1");
        assert!(bare.contains("[upscale]\nmode = fsr1"));
        assert!(bare.contains("path = fefw.log"));
        // A hand-edited ini keeps its shape: comments and other keys stay.
        let edited = set_ini_key(
            "; mine\n[upscale]\nsharpness = 0.5\n",
            "upscale",
            "mode",
            "nis",
        );
        assert!(edited.contains("; mine"));
        assert!(edited.contains("sharpness = 0.5"));
        assert!(edited.contains("mode = nis"));
    }

    #[test]
    fn the_labels_name_the_engine_modes() {
        assert_eq!(upscale_mode("FSR 3.1"), "fsr3");
        assert_eq!(upscale_mode("NVIDIA Image Scaling"), "nis");
        assert_eq!(upscale_mode("DLAA"), "dlaa");
        // Values saved by an older build, and anything unknown, mean off.
        assert_eq!(upscale_mode("Bilinear"), "off");
        assert_eq!(upscale_mode(""), "off");
        // The display settings the launcher names map to the engine's own.
        assert_eq!(output_mode("1920 × 1080"), "1080p");
        assert_eq!(output_mode("Native display"), "console");
        assert_eq!(resolution_size("3840 × 2160"), Some((3840, 2160)));
        assert_eq!(resolution_size("Native display"), None);
        assert_eq!(window_mode("Exclusive fullscreen"), "exclusive");
        assert_eq!(window_mode("Windowed"), "windowed");
    }

    #[test]
    fn the_engine_log_says_whether_a_frame_was_presented() {
        // The line the renderer writes once a second, and the one it writes
        // when it picks a card. Both are read, not invented.
        let text = "I gfx: using GPU 0: NVIDIA GeForce RTX 4070 (discrete, Vulkan 1.3.280, driver 0x1234, 12.0 GiB)\n\
I gfx: fps 59.9, present pass GPU 1.204 ms avg\n\
I fefw: gameplay at 60 fps ([fefw] fps60 = 1, fps120 = 0), game clock log every 10 s\n";
        assert_eq!(super::presented_fps(text), Some(59.9));
        assert_eq!(
            super::gpu_name(text).as_deref(),
            Some("NVIDIA GeForce RTX 4070")
        );
        // A start-up line that names a frame rate is not evidence of a frame.
        assert_eq!(
            super::presented_fps(
                "I gfx: using GPU 0: a card\nI fefw: gameplay at 60 fps ([fefw] fps60 = 1)\n"
            ),
            None,
            "a run that never presented has no present line"
        );
    }

    #[test]
    fn the_bundled_translator_is_found_only_where_this_platform_expects_it() {
        let (dir_name, file_name) = super::harness_platform();
        assert!(dir_name == "win-x64" || dir_name == "linux-x64");
        assert!(file_name == "spirv_harness.exe" || file_name == "spirv_harness");
        assert_eq!(
            dir_name.ends_with("win-x64"),
            cfg!(windows),
            "the platform folder matches the host"
        );

        let dir = std::env::temp_dir().join(format!("ew-harness-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let slot = dir
            .join("tools")
            .join("shaders")
            .join("prebuilt")
            .join(dir_name);
        std::fs::create_dir_all(&slot).unwrap();
        assert_eq!(
            super::bundled_harness_in(&dir),
            None,
            "an empty slot is not a translator"
        );

        // Another platform's binary is not ours to run.
        let other = if cfg!(windows) {
            "linux-x64"
        } else {
            "win-x64"
        };
        let other_file = if cfg!(windows) {
            "spirv_harness"
        } else {
            "spirv_harness.exe"
        };
        std::fs::create_dir_all(
            dir.join("tools")
                .join("shaders")
                .join("prebuilt")
                .join(other),
        )
        .unwrap();
        std::fs::write(
            dir.join("tools")
                .join("shaders")
                .join("prebuilt")
                .join(other)
                .join(other_file),
            b"x",
        )
        .unwrap();
        assert_eq!(
            super::bundled_harness_in(&dir),
            None,
            "the other platform's file is ignored"
        );

        std::fs::write(slot.join(file_name), b"x").unwrap();
        assert_eq!(super::bundled_harness_in(&dir), Some(slot.join(file_name)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_rendering_page_reaches_the_engines_own_keys() {
        let audio = crate::config::Audio::default();
        let mut g = crate::config::Graphics::default();
        let d = super::desired_values(&g, &audio);
        assert_eq!(d.get("upscale.quality").map(String::as_str), Some("native"));
        assert_eq!(
            d.get("upscale.scene_scale").map(String::as_str),
            Some("100")
        );
        assert_eq!(d.get("dlss.frame_gen").map(String::as_str), Some("0"));
        assert_eq!(d.get("dlss.upscaler").map(String::as_str), Some("auto"));
        assert_eq!(
            d.get("display.astc_quality").map(String::as_str),
            Some("high")
        );
        assert_eq!(
            d.get("nvn.compile_priority").map(String::as_str),
            Some("idle")
        );
        assert_eq!(d.get("nvn.compile_eco").map(String::as_str), Some("1"));
        assert_eq!(d.get("nvn.gpl_threads").map(String::as_str), Some("2"));
        assert_eq!(d.get("nvn.prewarm_threads").map(String::as_str), Some("2"));
        assert_eq!(
            d.get("nvn.shader_preload_threads").map(String::as_str),
            Some("2")
        );

        g.upscale_quality = "Performance".into();
        g.scene_scale = 70;
        g.frame_gen = "FSR 3 Frame Generation".into();
        g.dlss_upscaler = "TAAU".into();
        g.astc_quality = "Fast".into();
        g.gpu = "Integrated".into();
        g.compile_threads = 2;
        g.compile_priority = "Idle".into();
        g.compile_eco = false;
        let d = super::desired_values(&g, &audio);
        assert_eq!(
            d.get("upscale.quality").map(String::as_str),
            Some("performance")
        );
        assert_eq!(d.get("upscale.scene_scale").map(String::as_str), Some("70"));
        assert_eq!(d.get("dlss.frame_gen").map(String::as_str), Some("fsr3"));
        assert_eq!(d.get("dlss.upscaler").map(String::as_str), Some("taau"));
        assert_eq!(
            d.get("display.astc_quality").map(String::as_str),
            Some("fast")
        );
        assert_eq!(d.get("gfx.gpu").map(String::as_str), Some("integrated"));
        assert_eq!(d.get("nvn.gpl_threads").map(String::as_str), Some("2"));
        assert_eq!(d.get("nvn.prewarm_threads").map(String::as_str), Some("2"));
        assert_eq!(
            d.get("nvn.shader_preload_threads").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            d.get("nvn.compile_priority").map(String::as_str),
            Some("idle")
        );
        assert_eq!(d.get("nvn.compile_eco").map(String::as_str), Some("0"));
        // A scene scale outside the engine's 25..100 is clamped rather than passed on to be rejected.
        g.scene_scale = 5;
        assert_eq!(
            super::desired_values(&g, &audio)
                .get("upscale.scene_scale")
                .map(String::as_str),
            Some("25")
        );
    }

    #[test]
    fn the_log_report_names_the_error_and_the_repair() {
        let text = "-- populating imgui\nCMake Error at _deps/imgui-src/CMakeLists.txt:12 (message):\n  the source is not there\n-- Configuring incomplete, errors occurred!\n";
        let report = super::verify_log_text(text);
        assert!(report.contains("first error:"), "{report}");
        assert!(
            report.contains("CMake Error at _deps/imgui-src/CMakeLists.txt:12"),
            "{report}"
        );
        assert!(report.contains("the source is not there"), "{report}");
        assert!(report.contains("half written"), "{report}");
        assert!(!report.contains("another toolchain"), "{report}");

        // A log with nothing in it that looks like an error says so, instead of quoting a summary.
        let quiet = super::verify_log_text("-- building\n-- done\n");
        assert!(quiet.contains("no error line in this log"), "{quiet}");
        assert!(quiet.contains("no automatic repair"), "{quiet}");
    }

    #[test]
    fn a_half_written_fetch_is_cleared_and_the_build_retried_once() {
        let dir = std::env::temp_dir().join(format!("ew-repair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let deps = dir.join("release").join("_deps").join("imgui-src");
        std::fs::create_dir_all(&deps).unwrap();
        std::fs::write(deps.join("CMakeLists.txt"), b"half").unwrap();

        let mut calls = 0;
        let mut said: Vec<String> = Vec::new();
        let out = super::build_with_repairs(
            &dir,
            || {
                calls += 1;
                if calls == 1 {
                    Err("pwsh build_fefw.ps1 failed (exit code: 1): CMake Error at _deps/imgui-src/CMakeLists.txt:3".to_string())
                } else {
                    Ok(())
                }
            },
            |m| said.push(m.to_string()),
        );
        assert!(out.is_ok(), "the retry succeeds: {out:?}");
        assert_eq!(calls, 2, "exactly one retry");
        assert!(
            !dir.join("release").join("_deps").exists(),
            "the half-written sources are gone"
        );
        assert!(
            said.iter().any(|m| m.contains("dependency is incomplete")),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_cache_is_cleared_and_a_third_failure_is_reported() {
        let dir = std::env::temp_dir().join(format!("ew-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rel = dir.join("release");
        std::fs::create_dir_all(rel.join("CMakeFiles")).unwrap();
        std::fs::write(rel.join("CMakeCache.txt"), b"cache").unwrap();

        let mut calls = 0;
        let out = super::build_with_repairs(
            &dir,
            || {
                calls += 1;
                if calls == 1 {
                    Err("CMakeCache.txt is different than the one used previously".to_string())
                } else {
                    Err("clang-cl: error: unknown argument '-fno-exceptions'".to_string())
                }
            },
            |_| {},
        );
        assert!(out.is_err(), "the second failure is the real one");
        assert_eq!(calls, 2, "one retry, then give up");
        assert!(
            !rel.join("CMakeCache.txt").exists() && !rel.join("CMakeFiles").exists(),
            "the cache is gone"
        );
        assert!(
            out.unwrap_err().contains("unknown argument"),
            "the message is the second failure"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unrelated_failure_is_not_retried() {
        let dir = std::env::temp_dir().join(format!("ew-norepair-{}", std::process::id()));
        let mut calls = 0;
        let out = super::build_with_repairs(
            &dir,
            || {
                calls += 1;
                Err("clang-cl: error: unknown argument '-fno-exceptions'".to_string())
            },
            |_| {},
        );
        assert!(out.is_err());
        assert_eq!(calls, 1, "no retry for a failure that has no repair");
    }

    #[test]
    fn the_two_repairs_recognise_their_own_failures() {
        // Half-written fetch: the CMake file that tripped, or the _deps folder.
        assert!(super::half_written_deps(
            "CMake Error at _deps/imgui-src/CMakeLists.txt:3"
        ));
        assert!(super::half_written_deps(
            "ImGui Vulkan failure patch no longer matches pinned source"
        ));
        assert!(!super::half_written_deps(
            "clang-cl: error: unknown argument '-fno-exceptions'"
        ));
        // A cache from another toolchain.
        assert!(super::stale_build_cache(
            "CMakeCache.txt is different than the one used previously"
        ));
        assert!(super::stale_build_cache(
            "CMAKE_CXX_COMPILER points at a path that does not exist"
        ));
        assert!(!super::stale_build_cache(
            "Configuring incomplete, errors occurred!"
        ));
    }

    #[test]
    fn a_log_reader_keeps_the_lines_after_a_bad_byte() {
        // `BufRead::lines` returns Err at the 0xFF and stops: every line after it, the CMake error block included, was
        // dropped from the log while the summary on stdout still arrived.
        let mut got = Vec::new();
        super::stream_lines(&b"first\n\xff broken\ncmake: last\n"[..], |l| got.push(l));
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], "first");
        assert!(got[1].contains("broken"), "{}", got[1]);
        assert_eq!(got[2], "cmake: last");
    }

    #[test]
    fn the_first_error_is_kept_with_its_context_from_anywhere_in_the_log() {
        let mut text = String::new();
        for i in 0..5000 {
            text.push_str(&format!("-- populating dependency {i}\n"));
        }
        text.push_str("CMake Error at cmake/gfx_deps.cmake:41 (message):\n");
        text.push_str("  no ASTC transcoder: neither the ISPC build\n");
        for i in 0..5000 {
            text.push_str(&format!("-- more output {i}\n"));
        }
        text.push_str("-- Configuring incomplete, errors occurred!\n");
        let block = super::first_error_block(&text).expect("the error is in the log");
        assert!(
            block.starts_with("CMake Error at cmake/gfx_deps.cmake:41"),
            "{block}"
        );
        assert!(block.contains("no ASTC transcoder"), "{block}");
        assert!(
            !block.contains("Configuring incomplete"),
            "the summary is not the error: {block}"
        );
    }

    #[test]
    fn a_run_remembers_the_first_error_and_not_a_later_one() {
        let tail: super::Tail = Default::default();
        let first: super::FirstError = Default::default();
        for line in [
            "-- step",
            "error: first thing",
            "  detail",
            "error: second thing",
        ] {
            super::remember(&tail, &first, line);
        }
        let block = first.lock().unwrap().clone().expect("a block was recorded");
        assert_eq!(block[0], "error: first thing");
        assert_eq!(block.len(), 3);
    }

    #[test]
    fn an_upscaler_without_its_dlls_is_not_requested() {
        let mut g = crate::config::Graphics::default();
        g.upscaling = "FSR 3.1".into();
        let d = super::desired_values(&g, &crate::config::Audio::default());
        assert_eq!(
            d.get("upscale.mode").map(String::as_str),
            Some("fsr1"),
            "FSR 3 without its loader DLL would draw nothing, so the spatial one is requested instead"
        );
        g.upscaling = "DLSS".into();
        let d = super::desired_values(&g, &crate::config::Audio::default());
        assert_eq!(d.get("upscale.mode").map(String::as_str), Some("off"));
        g.upscaling = "FSR 1.0".into();
        let d = super::desired_values(&g, &crate::config::Audio::default());
        assert_eq!(d.get("upscale.mode").map(String::as_str), Some("fsr1"));
    }

    #[test]
    fn sharpening_uses_the_engines_four_steps() {
        assert_eq!(super::sharpen_value("Off"), "0");
        assert_eq!(super::sharpen_value("Low"), "0.45");
        assert_eq!(super::sharpen_value("Default"), "0.8");
        assert_eq!(super::sharpen_value("High"), "1");
        assert_eq!(super::sharpen_label("0"), Some("Off"));
        assert_eq!(super::sharpen_label("0.45"), Some("Low"));
        assert_eq!(super::sharpen_label("0.8"), Some("Default"));
        assert_eq!(super::sharpen_label("1"), Some("High"));
    }

    #[test]
    fn the_sharpening_on_the_page_reaches_the_ini() {
        let mut g = crate::config::Graphics::default();
        g.sharpening = "High".into();
        let d = super::desired_values(&g, &crate::config::Audio::default());
        assert_eq!(d.get("upscale.sharpness").map(String::as_str), Some("1"));
    }

    #[test]
    fn a_visual_studio_version_is_read_by_its_major_number() {
        assert_eq!(crate::toolchain::vs_major("17.9.34728.123"), Some(17));
        assert_eq!(crate::toolchain::vs_major("18.0.0"), Some(18));
        assert_eq!(crate::toolchain::vs_major(""), None);
    }

    #[test]
    fn the_dotnet_sdk_major_is_the_newest_one_listed() {
        let listing = "8.0.206 [C:\\Program Files\\dotnet\\sdk]\r\n10.0.404 [C:\\Program Files\\dotnet\\sdk]\r\n";
        assert_eq!(super::sdk_major_from(listing), Some(10));
        assert_eq!(super::sdk_major_from("6.0.100 [C:\\sdk]"), Some(6));
        assert_eq!(super::sdk_major_from(""), None);
    }

    #[test]
    fn play_writes_the_graphics_settings_into_the_run_folders_ini_files() {
        let dir = std::env::temp_dir().join(format!("ew-graphics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("fefw.ini");
        std::fs::write(
            &game,
            "[log]\npath = fefw.log\n[fefw]\nfps60 = 1\n[upscale]\nmode = off\n",
        )
        .unwrap();
        let controls = dir.join("controls.ini");
        std::fs::write(
            &controls,
            "[meta]\ndefaults_rev = 1\n[display]\nwindow_mode = windowed\nfps_limit = 60\n",
        )
        .unwrap();

        let g = crate::config::Graphics {
            resolution: "2560 × 1440".into(),
            display_mode: "Borderless window".into(),
            frame_limit: 120,
            // A mode whose DLLs this machine may not have: FSR 1 needs none,
            // so what the page selects is what both files must carry.
            upscaling: "FSR 1.0".into(),
            ..crate::config::Graphics::default()
        };
        let a = crate::config::Audio {
            master: 0.4,
            ..crate::config::Audio::default()
        };
        let mut applied = Values::new();
        assert!(
            write_settings(&dir, &g, &a, &mut applied).unwrap(),
            "the first start writes"
        );

        let text = std::fs::read_to_string(&game).unwrap();
        assert!(text.contains("output = 1440p"));
        assert!(text.contains("mode = fsr1"));
        assert!(
            text.contains("mods = "),
            "the run folder's mods are an absolute path after Play"
        );
        assert!(
            !text.contains("mods = mods"),
            "an old relative mods key is replaced"
        );
        assert!(text.contains("fps60 = 1"), "the rest of the file stays");
        assert!(
            text.contains("sharpness = 0.8"),
            "the sharpening the page shows"
        );

        let text = std::fs::read_to_string(&controls).unwrap();
        assert!(text.contains("window_mode = borderless"));
        assert!(text.contains("fps_limit = 120"));
        assert!(text.contains("vsync = 1"));
        assert!(text.contains("aspect = 16:9"));
        assert!(text.contains("width = 2560"));
        assert!(text.contains("height = 1440"));
        assert!(
            text.contains("render_width = 2560"),
            "at 1.00x the picture is that size"
        );
        assert!(text.contains("render_height = 1440"));
        assert!(
            text.contains("[audio]\nvolume = 0.40"),
            "the master volume play mode mixes at"
        );
        assert!(
            text.contains("[meta]") && text.contains("defaults_rev = 1"),
            "the file's own sections stay"
        );
        // The sharpening sits in the same section, so look for the keys rather than for the section header followed by one of
        // them: either order in the section is fine, a missing key is not.
        let upscale = text.split("[upscale]").nth(1).unwrap_or_default();
        assert!(
            upscale.lines().any(|l| l.trim() == "mode = fsr1"),
            "the F1 menu shows the same choice"
        );
        assert!(
            upscale.lines().any(|l| l.trim() == "sharpness = 0.8"),
            "and the same sharpening"
        );

        // Back to the defaults: the game's own console size again, and the keys
        // are replaced rather than repeated.
        let g = crate::config::Graphics {
            resolution: "Native display".into(),
            upscaling: "Off".into(),
            render_scale: 0.5,
            ..crate::config::Graphics::default()
        };
        assert!(
            write_settings(&dir, &g, &crate::config::Audio::default(), &mut applied).unwrap(),
            "a changed setting is written"
        );
        let text = std::fs::read_to_string(&game).unwrap();
        assert!(text.contains("output = console"));
        assert!(text.contains("mode = off"));
        assert_eq!(text.matches("mode =").count(), 1);
        let text = std::fs::read_to_string(&controls).unwrap();
        assert!(
            text.contains("render_width = 0"),
            "no fixed size pins the scaled picture"
        );
        assert!(text.contains("render_scale = 50"));
        assert!(
            text.contains("window_mode = borderless"),
            "the default mode is still written"
        );
        assert!(text.contains("fps_limit = 60"));
        assert!(
            text.contains("volume = 1.00"),
            "the volume goes back up too"
        );

        // The game's F1 menu is the other place these settings are changed: its
        // values come back onto the pages, but only where the page itself has
        // not moved on.
        let mut adopt_g = crate::config::Graphics::default();
        let mut adopt_au = crate::config::Audio {
            master: 1.0,
            ..crate::config::Audio::default()
        };
        let desired = desired_values(&adopt_g, &adopt_au);
        let mut file = desired.clone();
        file.insert("upscale.mode".into(), "fsr1".into());
        file.insert("fefw.output".into(), "1080p".into());
        file.insert("display.window_mode".into(), "exclusive".into());
        file.insert("display.vsync".into(), "0".into());
        file.insert("display.fps_limit".into(), "120".into());
        file.insert("display.aspect".into(), "1.7777777777777777".into());
        file.insert("audio.volume".into(), "0.35".into());
        assert!(
            adopt_from(&mut adopt_g, &mut adopt_au, &desired, &desired, &file),
            "the game's values are taken"
        );
        assert_eq!(adopt_g.upscaling, "FSR 1.0");
        assert_eq!(adopt_g.resolution, "1920 \u{00d7} 1080");
        assert_eq!(adopt_g.display_mode, "Exclusive fullscreen");
        assert!(!adopt_g.vsync);
        assert_eq!(adopt_g.frame_limit, 120);
        assert_eq!(
            adopt_g.aspect, "16:9",
            "the engine's resolved float is shown as the ratio it is"
        );
        assert_eq!(aspect_label("1.600"), "16:10");
        assert_eq!(aspect_label("2.3333333"), "21:9");
        assert_eq!(aspect_label("1.5"), "1.500:1");
        assert_eq!(aspect_label("auto"), "auto");
        assert!((adopt_au.master - 0.35).abs() < 0.001);
        let second = desired_values(&adopt_g, &adopt_au);
        assert!(
            !adopt_from(&mut adopt_g, &mut adopt_au, &second, &second, &file),
            "nothing left to take"
        );

        // The render size is pixels in the file: a window of 1920 wide rendering
        // 1440 comes back as 0.75x.
        let mut rg = crate::config::Graphics {
            resolution: "1920 \u{00d7} 1080".into(),
            ..crate::config::Graphics::default()
        };
        let mut ra = crate::config::Audio::default();
        let asked = desired_values(&rg, &ra);
        let mut file = asked.clone();
        file.insert("display.width".into(), "1920".into());
        file.insert("display.render_width".into(), "1440".into());
        file.insert("display.render_height".into(), "810".into());
        assert!(adopt_from(&mut rg, &mut ra, &asked, &asked, &file));
        assert!(
            (rg.render_scale - 0.75).abs() < 0.001,
            "{}",
            rg.render_scale
        );

        // A page setting changed here and not yet written outranks the file.
        let mut pending = crate::config::Graphics::default();
        let mut pending_audio = crate::config::Audio::default();
        let written = desired_values(&pending, &pending_audio);
        pending.display_mode = "Exclusive fullscreen".into();
        let asked = desired_values(&pending, &pending_audio);
        assert!(!adopt_from(
            &mut pending,
            &mut pending_audio,
            &asked,
            &written,
            &written
        ));
        assert_eq!(
            pending.display_mode, "Exclusive fullscreen",
            "the page keeps its own change"
        );
        // A second start with nothing changed touches no file, so a setting the
        // game's own menu changed in between survives it.
        let original = std::fs::read_to_string(&controls).unwrap();
        let patched = original.replace("window_mode = borderless", "window_mode = exclusive");
        std::fs::write(&controls, &patched).unwrap();
        assert!(
            !write_settings(&dir, &g, &crate::config::Audio::default(), &mut applied).unwrap(),
            "nothing to write"
        );
        let text = std::fs::read_to_string(&controls).unwrap();
        assert_eq!(text, patched, "the F1 menu's choice stands");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_game_files_are_read_back_the_way_the_engine_reads_them() {
        let dir = std::env::temp_dir().join(format!("ew-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("fefw.ini"),
            "[fefw]\nfps60 = 1\noutput = 1080p\n[upscale]\nmode = fsr3\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("controls.ini"),
            "# the game's file\n[meta]\ndefaults_rev = 1\n[display]\nwindow_mode = borderless\nvsync = 0\nfps_limit = 90\n\
             width = 1920\nheight = 1080\nrender_width = 1440\nrender_height = 810\naspect = 21:9\n[upscale]\nmode = fsr1\n[audio]\nvolume = 0.4\n",
        )
        .unwrap();
        let l = live_in(&dir).unwrap();
        assert_eq!(l.output, "1080p");
        // fefw.ini wins over the F1 menu's own copy, as merge_env does.
        assert_eq!(l.upscaling, "fsr3");
        assert_eq!(upscale_label(&l.upscaling), "FSR 3.1");
        assert_eq!(l.window_mode, "borderless");
        assert!(!l.vsync);
        assert_eq!(l.aspect, "21:9");
        assert_eq!(l.fps_limit, "90");
        assert_eq!(l.window, (1920, 1080));
        assert_eq!(l.picture, (1440, 810));
        assert!((l.volume - 0.4).abs() < 1e-6);

        // Without fefw.ini's key the file play mode reads decides, and a picture
        // with no fixed size is the window at render_scale.
        std::fs::write(dir.join("fefw.ini"), "[fefw]\noutput = console\n").unwrap();
        std::fs::write(
            dir.join("controls.ini"),
            "[display]\nwindow_mode = windowed\nvsync = 1\nwidth = 1920\nheight = 1080\nrender_scale = 75\n[upscale]\nmode = nis\n",
        )
        .unwrap();
        let l = live_in(&dir).unwrap();
        assert_eq!(l.upscaling, "nis");
        assert!(l.vsync);
        assert_eq!(l.output, "console");
        assert_eq!(l.picture, (1440, 810));

        // Nothing there at all: no panel rather than a made-up one.
        assert!(live_in(&dir.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_build_is_refused_while_the_lock_names_a_live_process() {
        let root = std::env::temp_dir().join(format!("ew-lock-{}", std::process::id()));
        let dir = root.join("work").join("build");
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join(".build.lock");
        // A process that cannot exist: the lock is stale, so a new build takes it.
        std::fs::write(
            &lock,
            "9999990 1 emblemweave.exe
",
        )
        .unwrap();
        let first = lock_build(&root);
        assert!(first.is_ok(), "{:?}", first.err());
        // The lock is released when it is dropped.
        drop(first);
        assert!(!lock.exists());
        // While a build holds it with this process's id, the same process may take it again (a restart of the job),
        // but another id that is alive must not.
        let held = lock_build(&root).unwrap();
        assert!(lock.exists());
        drop(held);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn texture_sources_are_checked_by_file_not_by_folder() {
        let root = std::env::temp_dir().join(format!("ew-textures-{}", std::process::id()));
        let tp = root.join("tools").join("textures").join("third_party");
        std::fs::create_dir_all(tp.join("astc-encoder")).unwrap();
        assert!(
            !textures_fetched(&root),
            "an empty folder must not count as fetched"
        );
        std::fs::write(tp.join("astc-encoder").join("CMakeLists.txt"), "").unwrap();
        std::fs::create_dir_all(tp.join("bc7enc_rdo")).unwrap();
        std::fs::write(tp.join("bc7enc_rdo").join("CMakeLists.txt"), "").unwrap();
        assert!(
            !textures_fetched(&root),
            "one missing piece is still missing"
        );
        let ispc = tp.join("ispc").join("ispc-v1.32.0-windows").join("bin");
        std::fs::create_dir_all(&ispc).unwrap();
        std::fs::write(ispc.join("ispc.exe"), "").unwrap();
        assert!(textures_fetched(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cache_on_disk_is_read_and_cleared() {
        let dir = std::env::temp_dir().join(format!("ew-cache-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("CMakeFiles")).unwrap();
        std::fs::write(
            dir.join("CMakeCache.txt"),
            "CMAKE_C_COMPILER:FILEPATH=C:/old/clang-cl.exe
",
        )
        .unwrap();
        let compiler = Path::new("C:/Program Files/LLVM/bin/clang-cl.exe");
        let why =
            crate::toolchain::cache_mismatch_in(&dir, compiler, "Ninja", "").expect("a mismatch");
        assert!(why.contains("C:/old/clang-cl.exe"), "{why}");
        crate::toolchain::clear_cache(&dir);
        assert!(!dir.join("CMakeCache.txt").exists());
        assert!(!dir.join("CMakeFiles").exists());
        // Nothing there now: CMake will write its own.
        assert!(crate::toolchain::cache_mismatch_in(&dir, compiler, "Ninja", "").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
