//! The game engine: fefw-recomp (Goldoire, GPL-3.0), included as the git
//! submodule `engine/fefw-recomp`.
//!
//! The launcher builds the game with the engine's own steps (its
//! `docs/BUILDING.md`), from the user's dump, and runs it:
//!
//! 1. the engine's Python environment (`tools/.venv`, made with uv);
//! 2. the dump's ExeFS prepared for the lifter (`main` written uncompressed);
//! 3. the game code lifted and compiled (`recomp/scripts/build_game.cmd`);
//! 4. the texture transcoder sources;
//! 5. `fefw.exe`;
//! 6. the shader translator (Ryujinx's shader code, MIT, at the engine's
//!    pinned revision);
//! 7. the game's shaders translated to SPIR-V;
//! 8. `fefw.ini` in the launcher's run folder.
//!
//! Steps whose output is already there are skipped, and every command's
//! output goes to the job log. The dump is only read.

use crate::config::Config;
use crate::jobs::{Ctx, JobResult, StageStatus};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Ryujinx revision the engine's shader translator is built against.
const RYUBING_URL: &str = "https://git.ryujinx.app/ryubing/ryujinx.git";
const RYUBING_REV: &str = "a82350bb774f70fcbd41c9987bf67a3775409963";

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
    for start in [std::env::current_exe().ok(), std::env::current_dir().ok()].into_iter().flatten() {
        found.extend(start.ancestors().map(|a| a.join("engine").join("fefw-recomp")));
    }
    found.retain(|e| e.join("game-build").is_dir());
    let pick = found.iter().find(|e| game_exe(e).is_file()).or(found.first()).cloned()?;
    if let Some(f) = remembered() {
        if std::fs::read_to_string(&f).ok().map(|t| t.trim().to_string()) != Some(pick.display().to_string()) {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(&f, pick.display().to_string());
        }
    }
    Some(pick)
}

/// The built game, if it exists.
pub fn game_exe(root: &Path) -> PathBuf {
    root.join("game-build").join("fefw").join("build").join("release").join("fefw.exe")
}

/// Where the game runs: its `fefw.ini`, log, mods and album live here.
pub fn run_dir() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("play"))
}

/// Whether the game has been built and configured, ready to play.
pub fn ready() -> bool {
    match (root(), run_dir()) {
        (Some(r), Some(d)) => game_exe(&r).is_file() && d.join("fefw.ini").is_file(),
        _ => false,
    }
}

/// Folders of the build tools, put first on PATH for every step: winget's
/// default locations for LLVM, CMake, Ninja, NASM and .NET.
fn tool_path() -> String {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let dirs = [
        r"C:\Program Files\LLVM\bin".to_string(),
        r"C:\Program Files\CMake\bin".to_string(),
        r"C:\Program Files\dotnet".to_string(),
        format!(r"{local}\Microsoft\WinGet\Links"),
        format!(r"{local}\bin\NASM"),
    ];
    let mut path: Vec<String> = dirs.into_iter().filter(|d| Path::new(d).is_dir()).collect();
    path.push(std::env::var("PATH").unwrap_or_default());
    path.join(";")
}

/// The tools the build needs, and whether each was found.
pub fn missing_tools() -> Vec<&'static str> {
    let path = tool_path();
    let found = |exe: &str| path.split(';').any(|d| Path::new(d).join(exe).is_file());
    [
        ("clang-cl.exe", "LLVM 23 (clang-cl)"),
        ("cmake.exe", "CMake"),
        ("ninja.exe", "Ninja"),
        ("dotnet.exe", ".NET 10 SDK"),
        ("uv.exe", "uv"),
        ("git.exe", "Git"),
        ("pwsh.exe", "PowerShell 7"),
    ]
    .into_iter()
    .filter(|(exe, _)| !found(exe))
    .map(|(_, name)| name)
    .collect()
}

/// Run one command in `cwd`, its output (stdout and stderr) into the job
/// log, with no console window. Cancelling the job stops it.
fn run(ctx: &Ctx, cwd: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    run_env(ctx, cwd, program, args, &[])
}

fn run_env(ctx: &Ctx, cwd: &Path, program: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args).envs(envs.iter().copied()).current_dir(cwd).env("PATH", tool_path()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    ctx.log(format!("> {program} {}", args.join(" ")));
    let mut child = cmd.spawn().map_err(|e| format!("starting {program}: {e}"))?;
    let err = child.stderr.take().map(|e| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                ctx.log(line);
            }
        })
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            ctx.log(line);
            if ctx.cancelled() {
                let _ = child.kill();
                return Err("cancelled".into());
            }
        }
    }
    if let Some(t) = err {
        let _ = t.join();
    }
    let status = child.wait().map_err(|e| format!("waiting for {program}: {e}"))?;
    if ctx.cancelled() {
        return Err("cancelled".into());
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed ({status})", args.first().copied().unwrap_or("")))
    }
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(false)
}

/// Settings written to `fefw.ini`.
pub struct RunConfig {
    pub exefs: PathBuf,
    pub romfs: PathBuf,
    pub save_dir: PathBuf,
}

/// The engine's configuration for the launcher's run folder.
pub fn ini_text(root: &Path, run: &RunConfig) -> String {
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    format!(
        "; Written by EmblemWeave. Every key is described in engine/fefw-recomp/runtime/docs/CONFIG.md.\n\
         [log]\npath = fefw.log\n\
         [fs]\nexefs = {}\nromfs = {}\nsave_dir = {}\nmods = mods\n\
         [nvn]\nshader_cache = {}\nbc7_cache = {}\n\
         [upscale]\nmode = off\n\
         [dlss]\nupscaler = auto\n\
         [diag]\nwatchdog_seconds = 30\n\
         [fefw]\nfps60 = 1\n",
        fwd(&run.exefs),
        fwd(&run.romfs),
        fwd(&run.save_dir),
        fwd(&root.join("work").join("shaders").join("spirv")),
        fwd(&root.join("work").join("bc7_cache")),
    )
}

/// The build job's work: every step in order.
pub fn build(ctx: &Ctx, exefs: PathBuf, romfs: PathBuf) -> Result<JobResult, String> {
    let root = root().ok_or("the engine (engine/fefw-recomp) was not found; run `git submodule update --init`")?;
    let missing = missing_tools();
    if !missing.is_empty() {
        return Err(format!("build tools missing: {}", missing.join(", ")));
    }
    let python = root.join("tools").join(".venv").join("Scripts").join("python.exe");
    let py = python.display().to_string();

    // 1. Python environment.
    ctx.start(0);
    if python.is_file() {
        ctx.end(0, StageStatus::Skipped, "already set up");
    } else {
        run(ctx, &root, "uv", &["venv", "tools\\.venv", "--python", "3.13"])?;
        run(ctx, &root, "uv", &["pip", "install", "--python", "tools\\.venv", "-r", "recomp\\requirements.txt"])?;
        ctx.end(0, StageStatus::Done, "tools\\.venv ready");
    }

    // 2. ExeFS.
    ctx.start(1);
    let ex = exefs.display().to_string();
    run(ctx, &root, &py, &["-I", "tools\\nso\\prepare_exefs.py", "--exefs", &ex])?;
    ctx.end(1, StageStatus::Done, "game\\dump\\exefs ready");

    // 3. Game code: ninja rebuilds only what changed.
    ctx.start(2);
    let jobs = std::thread::available_parallelism().map(|n| n.get().saturating_sub(2).max(1)).unwrap_or(8).to_string();
    run_env(ctx, &root, "cmd", &["/C", "recomp\\scripts\\build_game.cmd", "fefw"], &[("NX_JOBS", &jobs)])?;
    ctx.end(2, StageStatus::Done, format!("six modules compiled ({jobs} jobs)"));

    // 4. Texture transcoders.
    ctx.start(3);
    if has_files(&root.join("tools").join("textures").join("third_party")) {
        ctx.end(3, StageStatus::Skipped, "already fetched");
    } else {
        run(ctx, &root, "pwsh", &["-NoProfile", "-File", "tools\\textures\\scripts\\fetch_third_party.ps1"])?;
        ctx.end(3, StageStatus::Done, "astc-encoder and bc7enc_rdo fetched");
    }

    // 5. fefw.exe.
    ctx.start(4);
    run(ctx, &root, "pwsh", &["-NoProfile", "-File", "game-build\\scripts\\build_fefw.ps1"])?;
    ctx.end(4, StageStatus::Done, "fefw.exe built");

    // 6. Shader translator.
    ctx.start(5);
    let harness = root.join("tools").join("shaders").join("build").join("bin").join("Release").join("net10.0").join("spirv_harness.dll");
    if harness.is_file() {
        ctx.end(5, StageStatus::Skipped, "already built");
    } else {
        let refs = root.join("work").join("refs").join("ryubing");
        if !refs.join(".git").is_dir() {
            run(ctx, &root, "git", &["clone", "--filter=blob:none", "--no-checkout", RYUBING_URL, "work\\refs\\ryubing"])?;
        }
        run(ctx, &root, "git", &["-C", "work\\refs\\ryubing", "checkout", RYUBING_REV])?;
        run(ctx, &root, "pwsh", &["-NoProfile", "-File", "tools\\shaders\\scripts\\setup_ryujinx.ps1", "-Ryubing", "work\\refs\\ryubing"])?;
        run(ctx, &root, "dotnet", &["build", "tools\\shaders\\harness\\SpirvHarness.csproj", "-c", "Release"])?;
        ctx.end(5, StageStatus::Done, "spirv_harness built");
    }

    // 7. Shaders.
    ctx.start(6);
    let spirv = root.join("work").join("shaders").join("spirv");
    if has_files(&spirv) {
        ctx.end(6, StageStatus::Skipped, "already translated");
    } else {
        let rom = romfs.display().to_string();
        let main = root.join("game").join("dump").join("exefs").join("main").display().to_string();
        let dump = root.join("game").join("dump").join("exefs").display().to_string();
        run(ctx, &root, &py, &["-I", "tools\\shaders\\ampere\\extract_corpus.py", "--romfs", &rom, "--main", &main, "--exefs", &dump, "--out", "work\\shaders\\corpus", "--jobs", "4"])?;
        // Without the Vulkan SDK there is no spirv-val; the shaders are used unvalidated.
        let h = harness.display().to_string();
        run(ctx, &root, "dotnet", &[&h, "--batch", "work\\shaders\\corpus", "--out", "work\\shaders\\spirv", "--jobs", "4", "--no-val"])?;
        ctx.end(6, StageStatus::Done, "shaders translated to SPIR-V");
    }

    // 8. fefw.ini.
    ctx.start(7);
    let dir = run_dir().ok_or("APPDATA is not set")?;
    let save_dir = Config::save_dir().ok_or("APPDATA is not set")?;
    std::fs::create_dir_all(dir.join("mods")).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let ini = ini_text(&root, &RunConfig { exefs: root.join("game").join("dump").join("exefs"), romfs, save_dir });
    std::fs::write(dir.join("fefw.ini"), ini).map_err(|e| format!("writing fefw.ini: {e}"))?;
    ctx.end(7, StageStatus::Done, format!("{}", dir.join("fefw.ini").display()));
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
        _ => "off",
    }
}

/// The SDK folders the engine looks for the upscaler DLLs in, and which of them
/// are actually there. FSR 3.1 and DLSS cannot be shipped with the launcher, so
/// the Graphics page says where to put them instead of pretending they work.
#[derive(Debug, Default, Clone)]
pub struct Sdk {
    /// `upscalers\fsr3`: AMD's FidelityFX Vulkan DLLs.
    pub fsr3: Option<PathBuf>,
    /// `upscalers\streamline`: NVIDIA's Streamline DLLs.
    pub dlss: Option<PathBuf>,
}

fn sdk_in(dirs: &[PathBuf], folder: &str, needed: &[&str], every: bool) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join("upscalers").join(folder)).find(|d| {
        if every {
            needed.iter().all(|f| d.join(f).is_file())
        } else {
            needed.iter().any(|f| d.join(f).is_file())
        }
    })
}

/// Where the engine will look: the run folder, the game's folder and the
/// launcher's folder.
pub fn sdk() -> Sdk {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = run_dir() {
        dirs.push(d);
    }
    if let Some(r) = root() {
        if let Some(d) = game_exe(&r).parent() {
            dirs.push(d.to_path_buf());
        }
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(d);
    }
    Sdk {
        // The SDK 2.x loader, or the single v1.1.4 DLL: FSR 3.1 runs on either.
        fsr3: sdk_in(&dirs, "fsr3", &["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_vk.dll"], false),
        dlss: sdk_in(&dirs, "streamline", &["sl.interposer.dll", "sl.common.dll", "sl.dlss.dll", "nvngx_dlss.dll"], true),
    }
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
        match lines.iter().position(|l| l.trim().eq_ignore_ascii_case(&header)) {
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

/// The FSR 3.1 DLL the engine should load, whichever SDK layout is installed.
fn fsr_dll(sdk: &Sdk) -> Option<PathBuf> {
    let d = sdk.fsr3.as_ref()?;
    ["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_vk.dll"].iter().map(|f| d.join(f)).find(|p| p.is_file())
}

/// Write the Graphics settings the game can act on before every start.
///
/// The picture size and the upscaler go into `fefw.ini`: play mode merges that
/// file's `[upscale]` keys over its own, and `[fefw] output` is the game's
/// picture size. The window mode and the frame cap go into `controls.ini`,
/// which holds play mode's `[display]` section.
fn write_settings(dir: &Path, g: &crate::config::Graphics) -> Result<(), String> {
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    let sdk = sdk();

    let path = dir.join("fefw.ini");
    let mut text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    text = set_ini_key(&text, "fefw", "output", output_mode(&g.resolution));
    text = set_ini_key(&text, "upscale", "mode", upscale_mode(&g.upscaling));
    // Point the engine at the DLLs the Graphics page found. Its own defaults
    // only look beside the game and the launcher, so naming them keeps what the
    // page reports and what the game loads the same file.
    if let Some(dll) = fsr_dll(&sdk) {
        text = set_ini_key(&text, "upscale", "fsr_dll", &fwd(&dll));
    }
    if let Some(d) = &sdk.dlss {
        text = set_ini_key(&text, "upscale", "dlss_dll", &fwd(d));
    }
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;

    // The game writes controls.ini on its first start; a file made here is
    // filled with the factory settings by play mode, which keeps these keys.
    let path = dir.join("controls.ini");
    let mut text = std::fs::read_to_string(&path).unwrap_or_else(|_| "[meta]\ndefaults_rev = 1\n".to_string());
    text = set_ini_key(&text, "display", "window_mode", window_mode(&g.display_mode));
    text = set_ini_key(&text, "display", "fps_limit", &g.frame_limit.to_string());
    if let Some((w, h)) = resolution_size(&g.resolution) {
        text = set_ini_key(&text, "display", "width", &w.to_string());
        text = set_ini_key(&text, "display", "height", &h.to_string());
        // The picture itself, so the game renders at the size the window shows.
        text = set_ini_key(&text, "display", "render_width", &w.to_string());
        text = set_ini_key(&text, "display", "render_height", &h.to_string());
    }
    // "Native display" leaves the window and its picture as they are: the
    // engine's own defaults are the gold configuration, and nothing set in the
    // game's F1 menu is overwritten by a choice the user did not make.
    // The in-game F1 menu reads its own copy of the upscaler from this file, so
    // it shows the choice the engine is running rather than an older one.
    text = set_ini_key(&text, "upscale", "mode", upscale_mode(&g.upscaling));
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Start the game from the run folder. It runs on its own; the launcher
/// stays open.
pub fn play(graphics: &crate::config::Graphics) -> Result<(), String> {
    let root = root().ok_or("the engine was not found")?;
    let dir = run_dir().ok_or("APPDATA is not set")?;
    let exe = game_exe(&root);
    if !exe.is_file() || !dir.join("fefw.ini").is_file() {
        return Err("the game is not built yet: run Build the game on Game Setup".into());
    }
    // The taskbar shows the icon inside the .exe: make it EmblemWeave's.
    // Not while a copy is running (Windows cannot update a running program).
    #[cfg(windows)]
    if crate::cheats::process::find(crate::cheats::GAME_EXE).is_none() {
        let _ = crate::exe_icon::brand(&exe);
    }
    // The runtime's freeze watchdog counts idle time from the moment it starts,
    // and a cold start on this PC takes longer than its 10 s default to reach the
    // first presented frame (window, then the GPU device, then the swap chain and
    // the shader preload). Left alone every launch logs a FREEZE dump that is
    // not a freeze. 30 s still catches a real stall; a run started outside the
    // launcher gets the same value from the [diag] section of fefw.ini.
    // The picture size, window mode, frame cap and upscaler are the Graphics
    // settings the game can act on today; it reads them from its own ini files
    // at start. A file that cannot be written must not stop the game starting.
    let _ = write_settings(&dir, graphics);
    Command::new(&exe)
        .current_dir(&dir)
        .env("NXRT_DIAG_WATCHDOG_SECONDS", "30")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("starting {}: {e}", exe.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ini_points_at_the_dump_saves_and_shaders() {
        let ini = ini_text(
            Path::new(r"C:\x\engine\fefw-recomp"),
            &RunConfig { exefs: PathBuf::from(r"C:\e"), romfs: PathBuf::from(r"E:\g.nup"), save_dir: PathBuf::from(r"C:\s") },
        );
        assert!(ini.contains("romfs = E:/g.nup"));
        assert!(ini.contains("save_dir = C:/s"));
        assert!(ini.contains("shader_cache = C:/x/engine/fefw-recomp/work/shaders/spirv"));
        assert!(ini.contains("mods = mods"));
        // A cold start takes over 10 s to reach the first frame here, so the
        // watchdog's default would dump a freeze that never happened.
        assert!(ini.contains("watchdog_seconds = 30"));
        // The upscaler section is written up front; Play patches its mode.
        assert!(ini.contains("[upscale]\nmode = off"));
    }

    #[test]
    fn the_upscaler_replaces_its_own_key_and_keeps_the_rest() {
        let ini = ini_text(
            Path::new(r"C:\x\engine\fefw-recomp"),
            &RunConfig { exefs: PathBuf::from(r"C:\e"), romfs: PathBuf::from(r"E:\g.nup"), save_dir: PathBuf::from(r"C:\s") },
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
        let edited = set_ini_key("; mine\n[upscale]\nsharpness = 0.5\n", "upscale", "mode", "nis");
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
    fn play_writes_the_graphics_settings_into_the_run_folders_ini_files() {
        let dir = std::env::temp_dir().join(format!("ew-graphics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("fefw.ini");
        std::fs::write(&game, "[log]\npath = fefw.log\n[fefw]\nfps60 = 1\n[upscale]\nmode = off\n").unwrap();
        let controls = dir.join("controls.ini");
        std::fs::write(&controls, "[meta]\ndefaults_rev = 1\n[display]\nwindow_mode = windowed\nfps_limit = 60\n").unwrap();

        let g = crate::config::Graphics {
            resolution: "2560 × 1440".into(),
            display_mode: "Borderless window".into(),
            frame_limit: 120,
            upscaling: "FSR 3.1".into(),
            ..crate::config::Graphics::default()
        };
        write_settings(&dir, &g).unwrap();

        let text = std::fs::read_to_string(&game).unwrap();
        assert!(text.contains("output = 1440p"));
        assert!(text.contains("mode = fsr3"));
        assert!(text.contains("fps60 = 1"), "the rest of the file stays");

        let text = std::fs::read_to_string(&controls).unwrap();
        assert!(text.contains("window_mode = borderless"));
        assert!(text.contains("fps_limit = 120"));
        assert!(text.contains("width = 2560"));
        assert!(text.contains("height = 1440"));
        assert!(text.contains("render_width = 2560"), "the picture is that size too");
        assert!(text.contains("render_height = 1440"));
        assert!(text.contains("[meta]") && text.contains("defaults_rev = 1"), "the file's own sections stay");
        assert!(text.contains("[upscale]\nmode = fsr3"), "the F1 menu shows the same choice");

        // Back to the defaults: the game's own console size again, and the keys
        // are replaced rather than repeated.
        let g = crate::config::Graphics {
            resolution: "Native display".into(),
            upscaling: "Off".into(),
            ..crate::config::Graphics::default()
        };
        write_settings(&dir, &g).unwrap();
        let text = std::fs::read_to_string(&game).unwrap();
        assert!(text.contains("output = console"));
        assert!(text.contains("mode = off"));
        assert_eq!(text.matches("mode =").count(), 1);
        let text = std::fs::read_to_string(&controls).unwrap();
        assert!(text.contains("render_width = 2560"), "Native display leaves the window's sizes alone");
        assert!(text.contains("window_mode = borderless"), "the default mode is still written");
        assert!(text.contains("fps_limit = 60"));
        let _ = std::fs::remove_dir_all(&dir);
    }

}
