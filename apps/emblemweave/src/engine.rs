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
pub fn tool_path() -> String {
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

/// The Visual Studio C++ tools. Every engine script calls `vcvars64.bat`,
/// found through vswhere; without it the whole build dies with a bare exit
/// code, which is what made it look like the launcher's fault.
/// The newest Visual Studio C++ tools on this PC and its major version (17 = 2022, 18 = 2026). With `min_major` set, vswhere only
/// reports an install that new or newer, so an old one can tell the user why it cannot be used instead of failing on a header.
fn vc_install(min_major: Option<u32>) -> Option<(PathBuf, u32)> {
    let base = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let vswhere = Path::new(&base).join("Microsoft Visual Studio").join("Installer").join("vswhere.exe");
    let mut extra: Vec<String> = Vec::new();
    if let Some(m) = min_major {
        extra.push("-version".into());
        extra.push(format!("[{m}.0,)"));
    }
    let query = |property: &str| -> Option<String> {
        if !vswhere.is_file() {
            return None;
        }
        let mut cmd = Command::new(&vswhere);
        cmd.args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"]);
        cmd.args(&extra);
        cmd.args(["-property", property]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = cmd.output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().trim().to_string();
        (!text.is_empty()).then_some(text)
    };
    let path = query("installationPath");
    if let Some(path) = path {
        let major = query("installationVersion").as_deref().and_then(vs_major).unwrap_or(0);
        let bat = PathBuf::from(path).join("VC").join("Auxiliary").join("Build").join("vcvars64.bat");
        return bat.is_file().then_some((bat, major));
    }
    if vswhere.is_file() {
        return None;
    }
    // No vswhere at all: the default folders are the only guess left.
    for (dir, major) in [
        (r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools", 17u32),
        (r"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools", 18u32),
    ] {
        if min_major.is_some_and(|m| major < m) {
            continue;
        }
        let bat = Path::new(dir).join("VC").join("Auxiliary").join("Build").join("vcvars64.bat");
        if bat.is_file() {
            return Some((bat, major));
        }
    }
    None
}

/// The major version out of an `installationVersion` such as `17.9.34728.123`.
fn vs_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

/// The Visual Studio build needs: 2022 (17) or newer. An older one has no `<bit>` header in its C++ library, and the engine's
/// `cxx.h` asks for it, so the build died with "fatal error: 'bit' file not found" on a PC whose tools were all reported present
/// (issue 11).
fn vc_tools() -> Option<PathBuf> {
    vc_install(Some(17)).map(|(bat, _)| bat)
}

/// The newest C++ tools whatever their version, so a too-old install can be named rather than reported as absent.
pub fn vc_tools_any() -> Option<(PathBuf, u32)> {
    vc_install(None)
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
    cmd.args(["--list-sdks"]).env("PATH", tool_path()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
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

/// The tools the build needs, and whether each was found.
pub fn missing_tools() -> Vec<&'static str> {
    let path = tool_path();
    let found = |exe: &str| path.split(';').any(|d| Path::new(d).join(exe).is_file());
    // `dotnet.exe` alone is not enough: the harness targets net10.0, and an
    // older or absent SDK fails the build after a long wait.
    let dotnet_ok = found("dotnet.exe") && dotnet_sdk_major().is_some_and(|major| major >= 10);
    let mut missing: Vec<&'static str> = [
        ("clang-cl.exe", "LLVM 23 (clang-cl)"),
        ("cmake.exe", "CMake"),
        ("ninja.exe", "Ninja"),
        ("dotnet.exe", ".NET 10 SDK"),
        ("uv.exe", "uv"),
        ("git.exe", "Git"),
        ("pwsh.exe", "PowerShell 7"),
    ]
    .into_iter()
    .filter(|(exe, name)| if *name == ".NET 10 SDK" { !dotnet_ok } else { !found(exe) })
    .map(|(_, name)| name)
    .collect();
    if vc_tools().is_none() {
        missing.insert(0, "Visual Studio 2022 Build Tools with the C++ workload");
    }
    missing
}

/// The winget line that installs Visual Studio's C++ tools. It is the one tool
/// the build cannot do without, the largest download, and the hardest for a
/// player to find, so the launcher can run it instead of describing it.
pub const VS_INSTALL: &str = r#"winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" --accept-package-agreements --accept-source-agreements"#;

/// Whether winget is there to run that line at all.
pub fn have_winget() -> bool {
    tool_path().split(';').any(|d| Path::new(d).join("winget.exe").is_file())
}

/// The installer script for a list of missing tools, kept separate so a test
/// can read exactly what it will run. Each tool is one winget line, and a tool
/// that fails does not stop the ones after it: the summary at the end names it.
pub fn install_script_for(tools: &[&str]) -> String {
    let mut s = String::from("@echo off\r\ntitle EmblemWeave: build tools\r\n");
    s.push_str("echo Installing the build tools EmblemWeave needs.\r\n");
    s.push_str("echo.\r\n");
    s.push_str("echo Accept the administrator prompt when Windows shows it.\r\n");
    s.push_str("echo Visual Studio is a few GB; the others are small.\r\n");
    s.push_str("echo.\r\n");
    for tool in tools {
        let hint = install_hint(tool);
        if !hint.starts_with("winget ") {
            continue;
        }
        // Non-interactive: every line accepts the source and package
        // agreements, or winget stops on a prompt the script never answers.
        let line = if hint.contains("--accept-package-agreements") {
            hint.to_string()
        } else {
            format!("{hint} --accept-package-agreements --accept-source-agreements")
        };
        s.push_str(&format!("echo === {tool} ===\r\n{line}\r\nif errorlevel 1 echo That one did not finish: {tool}\r\n"));
    }
    s.push_str("echo.\r\necho Done. Close this window, restart EmblemWeave, then press Build the game again.\r\npause\r\n");
    s
}

/// What the button on Overview runs: every tool the preflight says is absent.
pub fn install_script() -> String {
    install_script_for(&missing_tools())
}

/// Write the installer script and run it in its own console, where winget can
/// show progress and raise the permission prompts it needs.
pub fn start_install() -> Result<PathBuf, String> {
    if !have_winget() {
        return Err("winget is not available on this PC, so the installer cannot be started. Install App Installer from the Microsoft Store, or install the tools by hand from https://aka.ms/vs/17/release/vs_buildtools.exe and the links in docs/TROUBLESHOOTING.md.".into());
    }
    let dir = Config::work_dir().ok_or("APPDATA is not set")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let script = dir.join("install-build-tools.cmd");
    std::fs::write(&script, install_script()).map_err(|e| format!("writing {}: {e}", script.display()))?;
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "start", "", &script.display().to_string()]).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, so only winget's console shows
    }
    cmd.spawn().map_err(|e| format!("starting the installer: {e}"))?;
    Ok(script)
}

/// The CMake version this PC has, as (major, minor). The engine's projects ask
/// for 3.25; an older one stops with "CMake Generate step failed. Build files
/// cannot be regenerated correctly", which says nothing about the cause.
pub fn cmake_version() -> Option<(u32, u32)> {
    let mut cmd = Command::new("cmake");
    cmd.arg("--version").stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.lines().next()?.split_whitespace().nth(2)?.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// What to install for a tool the build did not find: an error that only says
/// something is missing sends the user to a search engine (or to the issue
/// tracker, as it did).
pub fn install_hint(tool: &str) -> &'static str {
    match tool {
        "Visual Studio 2022 Build Tools with the C++ workload" => VS_INSTALL,
        "LLVM 23 (clang-cl)" => "winget install LLVM.LLVM",
        "CMake" => "winget install Kitware.CMake",
        "Ninja" => "winget install Ninja-build.Ninja",
        ".NET 10 SDK" => "winget install Microsoft.DotNet.SDK.10",
        "uv" => "winget install astral-sh.uv",
        "Git" => "winget install Git.Git",
        "PowerShell 7" => "winget install Microsoft.PowerShell",
        _ => "see docs/TROUBLESHOOTING.md",
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
fn reason_from(lines: &[String]) -> String {
    // "-- Configuring incomplete, errors occurred!" mentions errors without
    // being one, so a plain substring search finds the wrong line.
    let is_error = |l: &&String| {
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
                || lower.contains("is not recognized as an internal or external command")
                || lower.contains("is not recognized as the name of a cmdlet"))
    };
    match lines.iter().find(is_error) {
        Some(err) => err.clone(),
        None => lines.iter().rev().take(2).cloned().collect::<Vec<_>>().join(" | "),
    }
}

/// How a step is named in a failure: the script it ran, not the flag before
/// it (`pwsh -NoProfile` says nothing about which step died).
fn step_name(program: &str, args: &[&str]) -> String {
    if let Some(i) = args.iter().position(|a| *a == "-File") {
        if let Some(script) = args.get(i + 1) {
            return (*script).to_string();
        }
    }
    args.first().map(|a| (*a).to_string()).unwrap_or_else(|| program.to_string())
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

fn run_env(ctx: &Ctx, cwd: &Path, program: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(envs.iter().copied())
        .current_dir(cwd)
        .env("PATH", tool_path())
        // `-ExecutionPolicy Bypass` covers the script this call starts, not a
        // `pwsh` that script starts in turn: PowerShell children read this
        // preference from the environment, so the whole chain runs.
        .env("PSExecutionPolicyPreference", "Bypass")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    ctx.log(format!("> {program} {}", args.join(" ")));
    // The last lines a failed step printed: the log has everything, but the
    // message is what the user reads, and "exit code 1" alone is not a reason.
    let tail: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>> = Default::default();
    let mut child = cmd.spawn().map_err(|e| format!("starting {program}: {e}"))?;
    let err = child.stderr.take().map(|e| {
        let ctx = ctx.clone();
        let tail = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                ctx.log(line.clone());
                if let Ok(mut t) = tail.lock() {
                    t.push_back(line);
                    while t.len() > 400 {
                        t.pop_front();
                    }
                }
            }
        })
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            ctx.log(line.clone());
            if let Ok(mut t) = tail.lock() {
                t.push_back(line);
                while t.len() > 400 {
                    t.pop_front();
                }
            }
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
        // The reason, not the last line: build tools end a failed run with a
        // summary ("Build files cannot be regenerated correctly") that hides the
        // error several lines above it.
        let lines: Vec<String> = tail
            .lock()
            .map(|t| t.iter().filter(|l| !l.trim().is_empty()).cloned().collect())
            .unwrap_or_default();
        let said = reason_from(&lines);
        let name = step_name(program, args);
        if said.is_empty() {
            // Nothing at all was printed. A script that security software or a
            // machine policy kills exits exactly like this, so saying "the log
            // has the output" would point at an empty file.
            Err(format!(
                "{program} {name} failed ({status}) and printed nothing at all. That is what a script looks like when \
                 machine policy or security software stops it before its first line; the log has the command and the \
                 lines around it"
            ))
        } else if let Some(missing) = missing_program(&said) {
            Err(format!(
                "the build called `{missing}`, which is not installed or not on PATH ({program} {name} failed ({status})). \
                 Install it, restart the launcher, then press Build the game again"
            ))
        } else {
            Err(format!("{program} {name} failed ({status}): {said}"))
        }
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
    let root = root().ok_or("the engine folder engine/fefw-recomp is missing beside the launcher; unpack the repository whole, or set EW_ENGINE to its path")?;
    // An old Visual Studio passes every file-existence check and then dies on a C++20 header. Name the install and the version
    // that is needed, instead of "missing tools" or a header name the user cannot act on.
    if vc_tools().is_none() {
        if let Some((_, major)) = vc_tools_any() {
            return Err(format!(
                "the newest Visual Studio C++ tools on this PC are version {major}, and the engine needs Visual Studio 2022 (17) or newer. An older C++ library has no `<bit>` header, which is the `cxx.h: 'bit' file not found` error the build stops on.\n\nInstall the current tools with:\n{VS_INSTALL}\n\nthen press Build the game again."
            ));
        }
    }
    let missing = missing_tools();
    if !missing.is_empty() {
        let lines: Vec<String> = missing.iter().map(|m| format!("{m} (`{}`)", install_hint(m))).collect();
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
    let mut envs: Vec<(&str, &str)> = vec![("NX_JOBS", &jobs)];
    if let Some(vc) = &vc_env {
        envs.push(("NX_VCVARS", vc));
    }
    let built = run_env(ctx, &root, "cmd", &["/C", "recomp\\scripts\\build_game.cmd", "fefw"], &envs);
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
        let stale = e.to_lowercase().contains("cache") || e.contains("CMake Generate step failed") || e.contains("regenerated");
        if !stale {
            return Err(e);
        }
        ctx.log("the build folder holds a cache CMake cannot reuse; clearing it and configuring again");
        let _ = std::fs::remove_dir_all(build.join("CMakeFiles"));
        let _ = std::fs::remove_file(build.join("CMakeCache.txt"));
        run_env(ctx, &root, "cmd", &["/C", "recomp\\scripts\\build_game.cmd", "fefw"], &envs)?;
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
    ctx.end(2, StageStatus::Done, format!("six modules compiled ({jobs} jobs)"));

    // 4. Texture transcoders. `-ExecutionPolicy Bypass` is not optional: on a PC
    // whose policy is RemoteSigned or AllSigned, PowerShell refuses to load
    // these unsigned scripts and the step fails with a SecurityError instead of
    // running, which is what issue 2's second report shows.
    ctx.start(3);
    if has_files(&root.join("tools").join("textures").join("third_party")) {
        ctx.end(3, StageStatus::Skipped, "already fetched");
    } else {
        run(ctx, &root, "pwsh", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "tools\\textures\\scripts\\fetch_third_party.ps1"])?;
        ctx.end(3, StageStatus::Done, "astc-encoder and bc7enc_rdo fetched");
    }

    // 5. fefw.exe.
    ctx.start(4);
    let fefw_envs: Vec<(&str, &str)> = match &vc_env {
        Some(vc) => vec![("NX_VCVARS", vc)],
        None => Vec::new(),
    };
    let fefw = || run_env(ctx, &root, "pwsh", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "game-build\\scripts\\build_fefw.ps1"], &fefw_envs);
    if let Err(e) = fefw() {
        // The runtime fetches its graphics dependencies into the build folder, and
        // a fetch that was interrupted leaves the source folder half written. CMake
        // then fails on a file that is not there, or says the patch no longer
        // matches the pinned source, and it never repairs itself: the download
        // stamps are still in place. Throw the fetched sources away and fetch once
        // more before giving up. What was downloaded is kept in %LOCALAPPDATA%\nxrt
        // \deps, so this costs an unpack, not a download.
        // The message names the CMake file that tripped, not the missing source,
        // so the check has to look for the patch check and the imgui file too.
        let half = e.contains("_deps")
            || e.contains("imgui_checked.cmake")
            || e.contains("failure patch no longer matches pinned source");
        if !half {
            return Err(e);
        }
        ctx.log("a fetched dependency is incomplete; clearing it and fetching again");
        let builds = root.join("game-build").join("fefw").join("build");
        for config in std::fs::read_dir(&builds).into_iter().flatten().flatten() {
            let _ = std::fs::remove_dir_all(config.path().join("_deps"));
        }
        fefw()?;
    }
    // The build writes the engine's own icon into fefw.exe; Windows draws the
    // taskbar button from that resource, so ours goes in before anything runs it.
    let _ = brand_game();
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
        run(ctx, &root, "pwsh", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "tools\\shaders\\scripts\\setup_ryujinx.ps1", "-Ryubing", "work\\refs\\ryubing"])?;
        // "SDK not found" comes back as an HRESULT and a documentation link; name the piece that is missing instead, because a
        // .NET runtime alone satisfies `dotnet.exe` but cannot build a net10.0 project.
        run(ctx, &root, "dotnet", &["build", "tools\\shaders\\harness\\SpirvHarness.csproj", "-c", "Release"]).map_err(|e| {
            let lower = e.to_lowercase();
            if lower.contains("sdk-not-found") || e.contains("0x8000809b") || lower.contains("sdk not found") {
                format!(
                    "the shader translator is a .NET 10 program and this PC has the .NET runtime but no 10.x SDK. Install it with `{}`, then press Build the game again. ({e})",
                    install_hint(".NET 10 SDK")
                )
            } else {
                e
            }
        })?;
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
            out.insert(format!("{section}.{}", k.trim().to_ascii_lowercase()), v.trim().to_string());
        }
    }
    out
}

fn ini_map(path: &Path) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(path).map(|t| ini_parse(&t)).unwrap_or_default()
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
    let window = (num("display.width").unwrap_or(0), num("display.height").unwrap_or(0));
    // play_display.cpp's internal_size(): a fixed render size wins, else the
    // window at render_scale (100 = the window's own size).
    let scale = (num("display.render_scale").unwrap_or(100) as f32 / 100.0).clamp(0.25, 4.0);
    let picture = match (num("display.render_width"), num("display.render_height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => ((window.0 as f32 * scale) as u32, (window.1 as f32 * scale) as u32),
    };
    let mut values: std::collections::BTreeMap<String, String> = controls.clone().into_iter().collect();
    values.extend(game.clone());
    Some(Live {
        output: game.get("fefw.output").cloned().unwrap_or_else(|| "console".into()),
        // merge_env(): fefw.ini's [upscale] wins over the F1 menu's own copy.
        upscaling: game.get("upscale.mode").or_else(|| controls.get("upscale.mode")).cloned().unwrap_or_else(|| "off".into()),
        window_mode: get("display.window_mode").unwrap_or_else(|| "windowed".into()),
        vsync: matches!(get("display.vsync").as_deref(), Some("1") | Some("true") | Some("on")),
        aspect: get("display.aspect").unwrap_or_else(|| "auto".into()),
        fps_limit: get("display.fps_limit").unwrap_or_else(|| "auto".into()),
        window,
        picture,
        volume: get("audio.volume").and_then(|v| v.parse().ok()).unwrap_or(1.0),
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
    ["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_vk.dll"].iter().map(|f| d.join(f)).find(|p| p.is_file())
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
        other => other,
    };
    d.insert("upscale.mode".into(), mode.into());
    d.insert("upscale.sharpness".into(), sharpen_value(&g.sharpening).into());
    d.insert("display.window_mode".into(), window_mode(&g.display_mode).into());
    d.insert("display.vsync".into(), if g.vsync { "1" } else { "0" }.into());
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
        d.insert("display.render_scale".into(), (scale * 100.0).round().to_string());
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
    d.insert("audio.volume".into(), format!("{:.2}", a.master.clamp(0.0, 1.0)));
    d
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
fn take_back<'a>(desired: &Values, applied: &Values, on_file: &'a Values, id: &str) -> Option<&'a str> {
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
        // list of ratios intact.
        if !v.is_empty() && !v.eq_ignore_ascii_case("auto") {
            changed |= put(&mut g.aspect, v.to_string());
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
    if !render_keys.iter().any(|id| applied.get(*id) != desired.get(*id)) {
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
    let mut fefw = std::fs::read_to_string(&fefw_path).map_err(|e| format!("reading {}: {e}", fefw_path.display()))?;
    // The game writes controls.ini on its first start; a file made here is
    // filled with the factory settings by play mode, which keeps these keys.
    let ctl_path = dir.join("controls.ini");
    let mut ctl = std::fs::read_to_string(&ctl_path).unwrap_or_else(|_| "[meta]\ndefaults_rev = 1\n".to_string());
    let on_disk = (ini_parse(&fefw), ini_parse(&ctl));

    let mut touched = false;
    for (id, value) in desired_values(g, a) {
        let (section, key) = id.split_once('.').unwrap_or(("", ""));
        let (in_fefw, in_ctl) = match section {
            "fefw" => (true, false),
            "display" | "audio" => (false, true),
            _ => (true, true),
        };
        let current = if in_fefw { on_disk.0.get(&id) } else { on_disk.1.get(&id) };
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
    std::fs::write(&fefw_path, fefw).map_err(|e| format!("writing {}: {e}", fefw_path.display()))?;
    std::fs::write(&ctl_path, ctl).map_err(|e| format!("writing {}: {e}", ctl_path.display()))?;
    Ok(true)
}

// One line the launcher shows after a launch that did not draw anything. The
// engine's own log is written by another process, so a watcher thread fills
// this and the app drains it once per frame.
static PLAY_ALERT: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();

fn alerts() -> &'static std::sync::Mutex<Vec<String>> {
    PLAY_ALERT.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// The next launch finding, if the watcher made one.
pub fn take_play_alert() -> Option<String> {
    let mut a = alerts().lock().unwrap_or_else(|e| e.into_inner());
    if a.is_empty() {
        None
    } else {
        Some(a.remove(0))
    }
}

fn alert(text: String) {
    alerts().lock().unwrap_or_else(|e| e.into_inner()).push(text);
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
    let Ok(meta) = std::fs::metadata(path) else { return String::new() };
    if meta.len() < *from {
        *from = 0;
    }
    let Ok(mut file) = std::fs::File::open(path) else { return String::new() };
    use std::io::{Read, Seek, SeekFrom};
    if file.seek(SeekFrom::Start(*from)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if file.take(meta.len().saturating_sub(*from).min(1 << 20)).read_to_end(&mut buf).is_err() {
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
    text.lines().filter(|l| l.contains("present pass")).find_map(|l| {
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
        .filter(|l| l.contains("error") || l.contains("ERR") || l.contains("failed") || l.contains("vulkan"))
        .next_back()
        .map(|l| l.trim().to_string())
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
                    let why = last_error(&log).map(|e| format!(" Last line: {e}")).unwrap_or_default();
                    alert(format!(
                        "The game exited after {} s without finishing (code {:?}).{why} The Game Log has everything it printed.",
                        started.elapsed().as_secs(),
                        status.code()
                    ));
                }
                return;
            }
            let text = new_log_text(&log, &mut offset);
            if presented_fps(&text).is_some() {
                return; // it draws: nothing to report
            }
            if started.elapsed().as_secs() >= 90 {
                let gpu = gpu_name(&std::fs::read_to_string(&log).unwrap_or_default()).unwrap_or_else(|| "the graphics card it picked".into());
                alert(format!(
                    "The game has been running for 90 s and has not presented a single frame: the window looks black while the sound plays. \
It is running on {gpu}. Try Windowed or Borderless on Graphics, update that card's Vulkan driver, and if it stays black send the Game Log."
                ));
                return;
            }
        }
    });
}

/// Start the game from the run folder. It runs on its own; the launcher
/// stays open.
pub fn play(graphics: &crate::config::Graphics, audio: &crate::config::Audio) -> Result<(), String> {
    let root = root().ok_or("the engine was not found")?;
    let dir = run_dir().ok_or("APPDATA is not set")?;
    let exe = game_exe(&root);
    if !exe.is_file() || !dir.join("fefw.ini").is_file() {
        return Err("the game is not built yet: run Build the game on Game Setup".into());
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
    let child = Command::new(&exe)
        .current_dir(&dir)
        .env("NXRT_DIAG_WATCHDOG_SECONDS", "30")
        // One line per second while frames are presented: the evidence the
        // watcher needs to tell a slow start from a black window.
        .env("NXRT_GFX_LOG_FPS", "1")
        .spawn()
        .map_err(|e| format!("starting {}: {e}", exe.display()))?;
    watch_first_frame(child, log, from);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            super::step_name("pwsh", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "tools\\textures\\scripts\\fetch_third_party.ps1"]),
            "tools\\textures\\scripts\\fetch_third_party.ps1"
        );
        assert_eq!(super::step_name("cmd", &["/C", "recomp\\scripts\\build_game.cmd", "fefw"]), "/C");
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
        let lines: Vec<String> = ["ninja: build stopped: subcommand failed.", "Done building project"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(reason_from(&lines), "Done building project | ninja: build stopped: subcommand failed.");
    }

    #[test]
    fn the_installer_script_installs_every_missing_tool() {
        let s = install_script_for(&[
            "Visual Studio 2022 Build Tools with the C++ workload",
            "LLVM 23 (clang-cl)",
            "CMake",
            "PowerShell 7",
        ]);
        // A plain cmd script, or the console closes before anything is read.
        assert!(s.starts_with("@echo off"));
        assert!(s.contains("Microsoft.VisualStudio.2022.BuildTools"), "{s}");
        assert!(s.contains("Microsoft.VisualStudio.Workload.VCTools"), "{s}");
        // Without these winget stops at a prompt the console cannot answer.
        assert!(s.contains("--accept-package-agreements"));
        assert!(s.contains("--accept-source-agreements"));
        // Every tool in the list is run, not just the first.
        assert!(s.contains("winget install LLVM.LLVM"), "{s}");
        assert!(s.contains("winget install Kitware.CMake"), "{s}");
        assert!(s.contains("winget install Microsoft.PowerShell"), "{s}");
        assert!(s.trim_end().ends_with("pause"));
    }

    #[test]
    fn a_tool_without_an_install_line_is_left_out() {
        let s = install_script_for(&["Some tool nobody knows"]);
        assert!(!s.contains("winget "), "{s}");
        assert!(s.contains("Done."));
    }

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
    fn the_engine_log_says_whether_a_frame_was_presented() {
        // The line the renderer writes once a second, and the one it writes
        // when it picks a card. Both are read, not invented.
        let text = "I gfx: using GPU 0: NVIDIA GeForce RTX 4070 (discrete, Vulkan 1.3.280, driver 0x1234, 12.0 GiB)\n\
I gfx: fps 59.9, present pass GPU 1.204 ms avg\n\
I fefw: gameplay at 60 fps ([fefw] fps60 = 1, fps120 = 0), game clock log every 10 s\n";
        assert_eq!(super::presented_fps(text), Some(59.9));
        assert_eq!(super::gpu_name(text).as_deref(), Some("NVIDIA GeForce RTX 4070"));
        // A start-up line that names a frame rate is not evidence of a frame.
        assert_eq!(
            super::presented_fps("I gfx: using GPU 0: a card\nI fefw: gameplay at 60 fps ([fefw] fps60 = 1)\n"),
            None,
            "a run that never presented has no present line"
        );
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
        assert_eq!(super::vs_major("17.9.34728.123"), Some(17));
        assert_eq!(super::vs_major("18.0.0"), Some(18));
        assert_eq!(super::vs_major(""), None);
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
        std::fs::write(&game, "[log]\npath = fefw.log\n[fefw]\nfps60 = 1\n[upscale]\nmode = off\n").unwrap();
        let controls = dir.join("controls.ini");
        std::fs::write(&controls, "[meta]\ndefaults_rev = 1\n[display]\nwindow_mode = windowed\nfps_limit = 60\n").unwrap();

        let g = crate::config::Graphics {
            resolution: "2560 × 1440".into(),
            display_mode: "Borderless window".into(),
            frame_limit: 120,
            // A mode whose DLLs this machine may not have: FSR 1 needs none,
            // so what the page selects is what both files must carry.
            upscaling: "FSR 1.0".into(),
            ..crate::config::Graphics::default()
        };
        let a = crate::config::Audio { master: 0.4, ..crate::config::Audio::default() };
        let mut applied = Values::new();
        assert!(write_settings(&dir, &g, &a, &mut applied).unwrap(), "the first start writes");

        let text = std::fs::read_to_string(&game).unwrap();
        assert!(text.contains("output = 1440p"));
        assert!(text.contains("mode = fsr1"));
        assert!(text.contains("fps60 = 1"), "the rest of the file stays");
        assert!(text.contains("sharpness = 0.8"), "the sharpening the page shows");

        let text = std::fs::read_to_string(&controls).unwrap();
        assert!(text.contains("window_mode = borderless"));
        assert!(text.contains("fps_limit = 120"));
        assert!(text.contains("vsync = 1"));
        assert!(text.contains("aspect = 16:9"));
        assert!(text.contains("width = 2560"));
        assert!(text.contains("height = 1440"));
        assert!(text.contains("render_width = 2560"), "at 1.00x the picture is that size");
        assert!(text.contains("render_height = 1440"));
        assert!(text.contains("[audio]\nvolume = 0.40"), "the master volume play mode mixes at");
        assert!(text.contains("[meta]") && text.contains("defaults_rev = 1"), "the file's own sections stay");
        // The sharpening sits in the same section, so look for the keys rather than for the section header followed by one of
        // them: either order in the section is fine, a missing key is not.
        let upscale = text.split("[upscale]").nth(1).unwrap_or_default();
        assert!(upscale.lines().any(|l| l.trim() == "mode = fsr1"), "the F1 menu shows the same choice");
        assert!(upscale.lines().any(|l| l.trim() == "sharpness = 0.8"), "and the same sharpening");

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
        assert!(text.contains("render_width = 0"), "no fixed size pins the scaled picture");
        assert!(text.contains("render_scale = 50"));
        assert!(text.contains("window_mode = borderless"), "the default mode is still written");
        assert!(text.contains("fps_limit = 60"));
        assert!(text.contains("volume = 1.00"), "the volume goes back up too");

        // The game's F1 menu is the other place these settings are changed: its
        // values come back onto the pages, but only where the page itself has
        // not moved on.
        let mut adopt_g = crate::config::Graphics::default();
        let mut adopt_au = crate::config::Audio { master: 1.0, ..crate::config::Audio::default() };
        let desired = desired_values(&adopt_g, &adopt_au);
        let mut file = desired.clone();
        file.insert("upscale.mode".into(), "fsr1".into());
        file.insert("fefw.output".into(), "1080p".into());
        file.insert("display.window_mode".into(), "exclusive".into());
        file.insert("display.vsync".into(), "0".into());
        file.insert("display.fps_limit".into(), "120".into());
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
        assert!((adopt_au.master - 0.35).abs() < 0.001);
        let second = desired_values(&adopt_g, &adopt_au);
        assert!(!adopt_from(&mut adopt_g, &mut adopt_au, &second, &second, &file), "nothing left to take");

        // The render size is pixels in the file: a window of 1920 wide rendering
        // 1440 comes back as 0.75x.
        let mut rg = crate::config::Graphics { resolution: "1920 \u{00d7} 1080".into(), ..crate::config::Graphics::default() };
        let mut ra = crate::config::Audio::default();
        let asked = desired_values(&rg, &ra);
        let mut file = asked.clone();
        file.insert("display.width".into(), "1920".into());
        file.insert("display.render_width".into(), "1440".into());
        file.insert("display.render_height".into(), "810".into());
        assert!(adopt_from(&mut rg, &mut ra, &asked, &asked, &file));
        assert!((rg.render_scale - 0.75).abs() < 0.001, "{}", rg.render_scale);

        // A page setting changed here and not yet written outranks the file.
        let mut pending = crate::config::Graphics::default();
        let mut pending_audio = crate::config::Audio::default();
        let written = desired_values(&pending, &pending_audio);
        pending.display_mode = "Exclusive fullscreen".into();
        let asked = desired_values(&pending, &pending_audio);
        assert!(!adopt_from(&mut pending, &mut pending_audio, &asked, &written, &written));
        assert_eq!(pending.display_mode, "Exclusive fullscreen", "the page keeps its own change");
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
        std::fs::write(dir.join("fefw.ini"), "[fefw]\nfps60 = 1\noutput = 1080p\n[upscale]\nmode = fsr3\n").unwrap();
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
}
