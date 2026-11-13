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

/// Start the game from the run folder. It runs on its own; the launcher
/// stays open.
pub fn play() -> Result<(), String> {
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
    }
}
