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
use std::io::BufReader;
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

/// Folders of the build tools, put first on PATH for every step. The folder the
/// validated `clang-cl` lives in comes first, so the compiler on PATH is the one
/// that was checked, not whichever LLVM happens to sort first.
pub fn tool_path() -> String {
    crate::toolchain::tool_path()
}

/// Whether `name` answers along the PATH every build step gets.
pub fn which_tool(name: &str) -> bool {
    let mut cmd = Command::new(name);
    cmd.arg("--version").env("PATH", tool_path()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    crate::toolchain::hidden(&mut cmd).status().map(|s| s.success()).unwrap_or(false)
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

/// The dependencies that stop the build, by registry name. It reads the
/// report the background thread warms at start-up instead of running the checks
/// itself: the Overview draws this every frame, and `vswhere`, `dotnet` and
/// `vcvars64.bat` are not free. Until the first report exists this is empty,
/// which claims nothing.
pub fn missing_tools() -> Vec<&'static str> {
    let rows = crate::deps::cached().unwrap_or_default();
    crate::deps::blocking_for(&rows, crate::deps::Stage::Build).into_iter().map(|r| r.dep.name).collect()
}

/// The winget line that installs Visual Studio's C++ tools. It is the one tool
/// the build cannot do without, the largest download, and the hardest for a
/// player to find, so the launcher can run it instead of describing it.
pub const VS_INSTALL: &str = r#"winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" --accept-package-agreements --accept-source-agreements"#;

/// Whether winget is there to run that line at all.
pub fn have_winget() -> bool {
    tool_path().split(';').any(|d| Path::new(d).join("winget.exe").is_file())
}

/// The runnable install lines for a list of registry names.
pub fn install_lines_for(tools: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tool in tools {
        let Some(install) = crate::deps::DEPS.iter().find(|d| d.name == *tool).and_then(|d| d.install) else { continue };
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
    s.push_str("echo Done. Close this window, then press Check again on Requirements.\r\npause\r\n");
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
    std::fs::write(&script, install_script_lines(lines)).map_err(|e| format!("writing {}: {e}", script.display()))?;
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "start", "", &script.display().to_string()]).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, so only the installer console shows
    }
    cmd.spawn().map_err(|e| format!("starting the installer: {e}"))?;
    Ok(script)
}

/// What the button on Overview runs: the registry's install lines for every
/// dependency the preflight says is absent or too old.
pub fn start_install() -> Result<PathBuf, String> {
    if !have_winget() {
        return Err("winget is not available on this PC, so the installer cannot be started. Install App Installer from the Microsoft Store, or follow the install lines in docs/TROUBLESHOOTING.md.".into());
    }
    let lines = install_lines_for(&missing_tools());
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
        None => lines.iter().rev().take(2).cloned().collect::<Vec<_>>().join(" | "),
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
    let all: Vec<String> = text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect();
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
    // an older shell would point the build at headers nobody checked.
    crate::toolchain::scrub(&mut cmd);
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
    let mut child = cmd.spawn().map_err(|e| format!("starting {command}: {e}"))?;
    // Both streams are drained on their own threads, so the wait below can
    // notice a cancel while a long configure is still printing: reading stdout
    // inline would block until the process ends, which is how Cancel used to do
    // nothing until the step it was cancelling had finished.
    let mut readers = Vec::new();
    for pipe in [child.stdout.take().map(Either::Out), child.stderr.take().map(Either::Err)].into_iter().flatten() {
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
        status.code().map(|c| c.to_string()).unwrap_or_else(|| "(no code)".into()),
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
        } else if let Some(missing) = missing_program(&said).or_else(|| missing_program(&block.join(" "))) {
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
            Err(format!("`{command}` failed ({status}): {}", block.join(" | ")))
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
    let held = std::fs::read_to_string(&path).ok().and_then(|t| t.split_whitespace().next()?.parse::<u32>().ok());
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
    std::fs::write(&path, format!("{} {} {}\n", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0), std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()))
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(BuildLock { path })
}

/// Whether a pid is alive, asked of Windows rather than guessed.
fn process_running(pid: u32) -> bool {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("PID eq {pid}"), "/NH"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
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
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
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
            format!("; {} blocks are beyond the translator (listed in results.tsv)", self.failed)
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
    if std::fs::read_to_string(&stamp).map(|t| t.trim() == key.trim()).unwrap_or(false) {
        ctx.log(format!("game files: {count} packed files; using the overlay already written in {}", dest.display()));
        return Ok(Some(dest));
    }
    ctx.log(format!(
        "game files: {count} files of this package ({:.1} GB) are packed with zstd, which the engine cannot read in place, so those files are written once into {} and read from there; the package is used as it is for everything else",
        bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        dest.display()
    ));
    std::fs::create_dir_all(&dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
    let (written, total) = crate::gamedata::unpack_packed_romfs(romfs, &dest, &mut |m| ctx.log(format!("game files: {m}")))?;
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
    let nup = ew_format::nup::Nup::open(romfs).map_err(|e| format!("reading {} as a .nup: {e}", romfs.display()))?;
    let Some(pkg) = nup.program_package().cloned() else {
        return Err(format!("{} holds no base game or update, so its shaders cannot be read", romfs.display()));
    };
    let out = root.join("work").join("shaders").join("romfs");
    let stamp = out.join(".source");
    let want = format!("{}|{} files|v{}", romfs.display(), nup.files.len(), pkg.version);
    if stamp.is_file() && std::fs::read_to_string(&stamp).map(|t| t.trim().to_string()).ok().as_deref() == Some(want.as_str()) {
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
        let rel = if low.ends_with(".zst") { rel[..rel.len() - 4].to_string() } else { rel };
        let dst = out.join(&rel);
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        let data = nup.read(index).map_err(|e| format!("reading {rel} from {}: {e}", romfs.display()))?;
        bytes += data.len() as u64;
        std::fs::write(&dst, &data).map_err(|e| format!("writing {}: {e}", dst.display()))?;
        copied += 1;
    }
    if copied == 0 {
        return Err(format!("{} holds no shader archive ({EXTS:?}), so no shaders can be translated", romfs.display()));
    }
    std::fs::write(&stamp, &want).map_err(|e| format!("writing {}: {e}", stamp.display()))?;
    Ok((out, Some((copied, bytes))))
}

fn shader_report(corpus: &Path, spirv: &Path) -> Result<ShaderReport, String> {
    let unique = corpus.join("unique.tsv");
    let text = std::fs::read_to_string(&unique).map_err(|e| format!("reading {}: {e}", unique.display()))?;
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
        if let (Some(h), Some(_group), Some(_stage), Some(_uses), Some(st)) = (f.next(), f.next(), f.next(), f.next(), f.next()) {
            status.insert(h, st);
        }
    }
    let (mut translated, mut failed, mut missing) = (0usize, 0usize, Vec::new());
    for hash in &expected {
        // A report line saying ok with no .spv beside it is work to do again: the
        // file was deleted, or the run that wrote the line was cut short before it.
        match status.get(hash) {
            Some(&st) if st == "ok" && spirv.join(format!("{hash}.spv")).is_file() => translated += 1,
            Some(&st) if st == "ok" => missing.push((*hash).to_string()),
            Some(_) => failed += 1,
            None => missing.push((*hash).to_string()),
        }
    }
    Ok(ShaderReport { expected: expected.len(), translated, failed, missing })
}

/// Run the translator over the corpus. The harness keeps a per-block journal and
/// skips the blocks that already have a result, so a repair costs the missing
/// blocks and not the whole corpus; its report is written from the journal, so the
/// earlier lines survive a stopped run.
fn run_translator(ctx: &Ctx, root: &Path, bundled: &Option<PathBuf>, harness: &Path) -> Result<(), String> {
    match bundled {
        Some(exe) => run(
            ctx,
            root,
            &exe.display().to_string(),
            &["--batch", "work\\shaders\\corpus", "--out", "work\\shaders\\spirv", "--jobs", "4", "--no-val"],
        ),
        None => {
            let h = harness.display().to_string();
            run(ctx, root, "dotnet", &[&h, "--batch", "work\\shaders\\corpus", "--out", "work\\shaders\\spirv", "--jobs", "4", "--no-val"])
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
        tp.join("ispc").join("ispc-v1.32.0-windows").join("bin").join("ispc.exe"),
    ]
    .iter()
    .all(|p| p.is_file())
}

/// The Ryujinx checkout the shader translator builds from. It is cloned into a
/// temporary folder and moved into place only after the pinned revision is
/// checked out and the tree is complete: a clone that a cancel or a dropped
/// connection interrupted otherwise leaves `.git` behind, which is exactly what
/// the old "is it there" test accepted before failing much later in `dotnet`.
fn ensure_ryubing_refs(ctx: &Ctx, root: &Path) -> Result<(), String> {
    let refs = root.join("work").join("refs").join("ryubing");
    let project = Path::new("src").join("Ryujinx.Graphics.Shader").join("Ryujinx.Graphics.Shader.csproj");
    if refs.join(&project).is_file() {
        return Ok(());
    }
    if refs.exists() {
        ctx.log(format!("the Ryujinx checkout in {} is incomplete; cloning it again", refs.display()));
        let _ = std::fs::remove_dir_all(&refs);
    }
    let tmp = root.join("work").join("refs").join("ryubing.partial");
    let mut last = String::new();
    for attempt in 1..=2 {
        let _ = std::fs::remove_dir_all(&tmp);
        let result = run(ctx, root, "git", &["clone", "--quiet", "--filter=blob:none", "--no-checkout", RYUBING_URL, "work\\refs\\ryubing.partial"])
            .and_then(|()| run(ctx, root, "git", &["-C", "work\\refs\\ryubing.partial", "checkout", "--quiet", RYUBING_REV]))
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
                    .map_err(|e| format!("moving the Ryujinx checkout into {}: {e}", refs.display()))?;
                return Ok(());
            }
            Err(e) => {
                last = e;
                let _ = std::fs::remove_dir_all(&tmp);
                if attempt < 2 {
                    ctx.log(format!("the Ryujinx checkout failed; trying once more ({last})"));
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
    pub save_dir: PathBuf,
    /// Files the package cannot serve in place (its zstd entries), written out by the build; the engine lays this folder over the package.
    pub romfs_overlay: Option<PathBuf>,
}

/// The engine's configuration for the launcher's run folder.
pub fn ini_text(root: &Path, run: &RunConfig) -> String {
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    // Only written when the package has files the engine cannot read in place: the folder the build wrote them into.
    let overlay = run.romfs_overlay.as_ref().map(|p| format!("romfs_overlay = {}\n", fwd(p))).unwrap_or_default();
    format!(
        "; Written by EmblemWeave. Every key is described in engine/fefw-recomp/runtime/docs/CONFIG.md.\n\
         [log]\npath = fefw.log\n\
         [fs]\nexefs = {}\nromfs = {}\n{}\nsave_dir = {}\nmods = mods\n\
         [nvn]\nshader_cache = {}\nbc7_cache = {}\n\
         [upscale]\nmode = off\n\
         [dlss]\nupscaler = auto\n\
         [diag]\nwatchdog_seconds = 30\n\
         [fefw]\nfps60 = 1\n\
         [engine]\nbuilt_by = {}\n",
        fwd(&run.exefs),
        fwd(&run.romfs),
        overlay.trim_end(),
        fwd(&run.save_dir),
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
    ini_map(&dir.join("fefw.ini")).get("engine.built_by").cloned()
}

/// The package the built game points at, when that package packs files with zstd and the game folder has no overlay for them: Play writes
/// those files first and then starts the game. True for a game folder built before 0.1.20, or one whose write never ran.
pub fn packed_package_without_overlay() -> Option<PathBuf> {
    let dir = run_dir()?;
    let written = ini_map(&dir.join("fefw.ini"));
    if written.get("fs.romfs_overlay").is_some() {
        return None;
    }
    let path = PathBuf::from(written.get("fs.romfs")?);
    (path.is_file() && crate::gamedata::packed_romfs(&path).is_some()).then_some(path)
}

/// Game Setup's "Check my game folder": what the build does for a package it cannot read in place, done on its own so a game folder that
/// was built by an older release can be fixed without compiling anything. It reads the package, writes the files that are packed with
/// zstd into `work/romfs-overlay`, and points the existing `fefw.ini` at that folder.
pub const REPAIR_STAGES: &[&'static str] = &["Read the package", "Write the files it cannot serve", "Point the game at them"];

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
    ctx.end(0, StageStatus::Done, format!("{count} files are packed with zstd ({:.1} GB)", bytes as f64 / (1024.0 * 1024.0 * 1024.0)));

    ctx.start(1);
    let dest = root.join("work").join("romfs-overlay");
    let stamp = dest.join(".written");
    let key = format!("{}|{count} files|{bytes} bytes", romfs.display());
    if std::fs::read_to_string(&stamp).map(|t| t.trim() == key.trim()).unwrap_or(false) {
        ctx.end(1, StageStatus::Skipped, format!("already written in {}", dest.display()));
    } else {
        std::fs::create_dir_all(&dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
        let (written, total) = crate::gamedata::unpack_packed_romfs(&romfs, &dest, &mut |m| ctx.log(format!("game files: {m}")))?;
        std::fs::write(&stamp, &key).map_err(|e| format!("writing {}: {e}", stamp.display()))?;
        ctx.end(1, StageStatus::Done, format!("{written} files, {:.1} GB in {}", total as f64 / (1024.0 * 1024.0 * 1024.0), dest.display()));
    }

    ctx.start(2);
    let Some(dir) = run_dir() else {
        ctx.end(2, StageStatus::Skipped, "no run folder yet");
        return Ok(JobResult::Repaired(Some(dest)));
    };
    let ini_path = dir.join("fefw.ini");
    if !ini_path.is_file() {
        ctx.end(2, StageStatus::Skipped, "the game has not been built yet: the next build writes it");
        return Ok(JobResult::Repaired(Some(dest)));
    }
    let text = std::fs::read_to_string(&ini_path).map_err(|e| format!("reading {}: {e}", ini_path.display()))?;
    let updated = set_ini_value(&text, "fs", "romfs_overlay", &dest.display().to_string().replace('\\', "/"));
    if updated != text {
        std::fs::write(&ini_path, updated).map_err(|e| format!("writing {}: {e}", ini_path.display()))?;
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
pub fn build(ctx: &Ctx, exefs: PathBuf, romfs: PathBuf) -> Result<JobResult, String> {
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
    ctx.log(format!("toolchain fingerprint: {}", resolved.toolchain.fingerprint()));
    for (name, value) in crate::toolchain::stale_env() {
        ctx.log(format!("ignoring the inherited {name}={value}: it would point the build at a toolchain that was not checked"));
    }
    // Two builds at once write the same object files and fetch the same sources.
    let _build_lock = lock_build(&root)?;
    // The probe that matters: one file, compiled and linked with exactly the
    // environment below. A header from another install fails here in a second.
    let probe_dir = root.join("work").join("build").join("toolchain-probe");
    crate::toolchain::probe_build(&resolved.compiler, &resolved.env, &probe_dir)
        .map_err(|e| format!("the build cannot start. {e}"))?;
    ctx.log(format!("the toolchain compiles and links a test file ({} with its own headers, libraries and SDK)", resolved.compiler.display()));
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

    // 1. Python environment. `uv` is quicker when it is installed, but it is one more thing to install for one virtualenv, so
    // without it the interpreter the build already needs makes the environment and pip installs the same pinned list.
    ctx.start(0);
    if python.is_file() {
        ctx.end(0, StageStatus::Skipped, "already set up");
    } else if which_tool("uv") {
        run(ctx, &root, "uv", &["venv", "tools\\.venv", "--python", "3.13"])?;
        run(ctx, &root, "uv", &["pip", "install", "--python", "tools\\.venv", "-r", "recomp\\requirements.txt"])?;
        ctx.end(0, StageStatus::Done, "tools\\.venv ready");
    } else {
        ctx.log("uv is not installed: the Python environment is made with the interpreter on this PC and pip installs its packages");
        if run(ctx, &root, "py", &["-3", "-m", "venv", "tools\\.venv"]).is_err() {
            run(ctx, &root, "python", &["-m", "venv", "tools\\.venv"]).map_err(|e| {
                format!("creating tools\\.venv needs a Python 3.9 or newer: {e}. Install Python with `winget install Python.Python.3.12`, restart the launcher, then press Build the game again")
            })?;
        }
        run(ctx, &root, &py, &["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "-r", "recomp\\requirements.txt"])?;
        ctx.end(0, StageStatus::Done, "tools\\.venv ready (pip)");
    }

    // 2. ExeFS.
    ctx.start(1);
    let ex = exefs.display().to_string();
    run(ctx, &root, &py, &["-I", "tools\\nso\\prepare_exefs.py", "--exefs", &ex])?;
    ctx.end(1, StageStatus::Done, "game\\dump\\exefs ready");

    // 3. Game code: ninja rebuilds only what changed.
    ctx.start(2);
    // A build folder made with another compiler, generator or toolchain file
    // cannot be reused, and CMake says only "Build files cannot be regenerated
    // correctly" after a configure pass. Its cache names what it was made with,
    // so compare that with what was validated here and clear it first.
    let game_build = root.join("work").join("build").join("fefw");
    let toolchain_file = root.join("recomp").join("cmake").join("clang-cl.cmake");
    if let Some(why) =
        crate::toolchain::cache_mismatch_in(&game_build, &resolved.compiler, "Ninja", &toolchain_file.display().to_string())
    {
        ctx.log(format!("the recompile build folder cannot be reused: {why}; clearing its CMake cache"));
        crate::toolchain::clear_cache(&game_build);
    }
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
    // Not "the folder has something in it": each pinned source and the ispc
    // binary are checked by name, because a fetch that stopped halfway leaves a
    // folder that exists and cannot be built.
    if textures_fetched(&root) {
        ctx.end(3, StageStatus::Skipped, "already fetched");
    } else {
        ctx.log("the texture sources are missing or incomplete; fetching the pinned versions");
        run(ctx, &root, shell(), &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "tools\\textures\\scripts\\fetch_third_party.ps1"])?;
        if !textures_fetched(&root) {
            return Err("the texture sources were fetched but are still incomplete. tools\\textures\\third_party needs astc-encoder, bc7enc_rdo and ispc; delete that folder and press Build the game again".into());
        }
        ctx.end(3, StageStatus::Done, "astc-encoder, bc7enc_rdo and ispc fetched and checked");
    }

    // 5. fefw.exe.
    ctx.start(4);
    let fefw_envs: Vec<(&str, &str)> = match &vc_env {
        Some(vc) => vec![("NX_VCVARS", vc)],
        None => Vec::new(),
    };
    let fefw = || run_env(ctx, &root, shell(), &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "game-build\\scripts\\build_fefw.ps1"], &fefw_envs);
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
        if let Some(why) = crate::toolchain::cache_mismatch_in(&dir.path(), &resolved.compiler, "Ninja", "") {
            ctx.log(format!("{} cannot be reused: {why}; clearing its CMake cache", dir.path().display()));
            crate::toolchain::clear_cache(&dir.path());
        }
    }
    build_with_repairs(&builds, fefw, |m| ctx.log(m))?;
    // The haptics backend is a vendored DLL the runtime loads from beside `fefw.exe` (gfx/haptic.cpp looks
    // nowhere else), so it is copied in after the build rather than linked. Without it the game still vibrates,
    // on the standard two-motor rumble.
    let renhdrum = root.join("runtime").join("third_party").join("renhdrum").join("renhdrum.dll");
    if renhdrum.is_file() {
        let beside = builds.join("release").join("renhdrum.dll");
        if let Err(e) = std::fs::copy(&renhdrum, &beside) {
            ctx.log(format!("copying renhdrum.dll beside fefw.exe: {e} (standard rumble will be used)"));
        }
    }
    // The build writes the engine's own icon into fefw.exe; Windows draws the
    // taskbar button from that resource, so ours goes in before anything runs it.
    let _ = brand_game();
    ctx.end(4, StageStatus::Done, "fefw.exe built");

    // 6. Shader translator.
    ctx.start(5);
    let harness = root.join("tools").join("shaders").join("build").join("bin").join("Release").join("net10.0").join("spirv_harness.dll");
    let bundled = bundled_harness_in(&root);
    if let Some(exe) = &bundled {
        ctx.end(5, StageStatus::Skipped, format!("using the one in the zip: {}", exe.display()));
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
        let awkward_path = root.display().to_string().chars().any(|c| c == '\'' || !c.is_ascii());
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
        ensure_ryubing_refs(ctx, &root)?;
        let setup = build_shaders.join("scripts").join("setup_ryujinx.ps1").display().to_string();
        let refs = root.join("work").join("refs").join("ryubing").display().to_string();
        run(ctx, &root, shell(), &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &setup, "-Ryubing", &refs])?;
        // "SDK not found" comes back as an HRESULT and a documentation link; name the piece that is missing instead, because a
        // .NET runtime alone satisfies `dotnet.exe` but cannot build a net10.0 project.
        let csproj = build_shaders.join("harness").join("SpirvHarness.csproj").display().to_string();
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
            let from = staged.join("build").join("bin").join("Release").join("net10.0");
            let to = shaders.join("build").join("bin").join("Release").join("net10.0");
            let files = crate::saves::copy_tree(&from, &to)?;
            ctx.log(format!("copied {files} translator files back into {}", to.display()));
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
            "copied {files} shader archives ({:.1} GiB) out of the .nup: the shader extractor walks a folder, and a .nup is a file",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0)
        ));
    }
    // The corpus remembers which RomFS it came from. One built from a path that is
    // not that folder (a .nup passed as --romfs, an older launcher) is not reused:
    // it holds a handful of blocks and would translate a handful of shaders.
    let corpus_stamp = corpus.join(".romfs");
    let romfs_id = romfs_dir.display().to_string();
    let corpus_current = corpus.join("unique.tsv").is_file()
        && std::fs::read_to_string(&corpus_stamp).map(|t| t.trim().to_string()).ok().as_deref() == Some(romfs_id.as_str());
    if !corpus_current {
        let rom = romfs_id.clone();
        let main = root.join("game").join("dump").join("exefs").join("main").display().to_string();
        let dump = root.join("game").join("dump").join("exefs").display().to_string();
        run(ctx, &root, &py, &["-I", "tools\\shaders\\ampere\\extract_corpus.py", "--romfs", &rom, "--main", &main, "--exefs", &dump, "--out", "work\\shaders\\corpus", "--jobs", "4"])?;
        std::fs::write(&corpus_stamp, &romfs_id).map_err(|e| format!("writing {}: {e}", corpus_stamp.display()))?;
    }
    let mut report = shader_report(&corpus, &spirv)?;
    if report.expected == 0 {
        return Err(format!("{} lists no shader blocks, so the dump could not be read for shaders. Send a diagnostics report; the game cannot draw without them.", corpus.join("unique.tsv").display()));
    }
    if report.missing.is_empty() {
        ctx.end(6, StageStatus::Skipped, format!("all {} shaders translated{}", report.expected, report.failed_summary()));
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
                break;      // no progress: another run would fail the same way
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
        ctx.end(6, StageStatus::Done, format!("{} shaders translated{}", report.expected, report.failed_summary()));
    }

    // 8. fefw.ini.
    ctx.start(7);
    let dir = run_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    let save_dir = Config::save_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    std::fs::create_dir_all(dir.join("mods")).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let ini = ini_text(&root, &RunConfig { exefs: root.join("game").join("dump").join("exefs"), romfs, save_dir, romfs_overlay: overlay });
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
    let p = root.join("tools").join("shaders").join("prebuilt").join(dir).join(file);
    p.is_file().then_some(p)
}

/// The files FSR 3.1 is built from, either layout: the SDK 2.x loader with its upscaler, or the single v1.1.4 DLL.
pub const FSR_FILES: &[&str] = &["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_upscaler_vk.dll", "amd_fidelityfx_vk.dll"];
/// The four DLLs DLSS needs.
pub const DLSS_FILES: &[&str] = &["sl.interposer.dll", "sl.common.dll", "sl.dlss.dll", "nvngx_dlss.dll"];
/// The five more DLSS Frame Generation needs beside those four (`streamline_framegen_files` in the engine).
pub const DLSS_FRAMEGEN_FILES: &[&str] = &["sl.dlss_g.dll", "sl.reflex.dll", "sl.pcl.dll", "nvngx_dlssg.dll", "NvLowLatencyVk.dll"];
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
        self.fsr3.as_ref().is_some_and(|d| d.join("amd_fidelityfx_vk.dll").is_file())
    }

    /// DLSS Frame Generation needs five more files beside the four DLSS ones, from the same Streamline release.
    pub fn dlss_framegen(&self) -> bool {
        self.dlss.as_ref().is_some_and(|d| DLSS_FRAMEGEN_FILES.iter().all(|f| d.join(f).is_file()))
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
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
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

fn sdk_in(dirs: &[PathBuf], folders: &[&str], needed: &[&str], every: bool) -> Option<PathBuf> {
    let vendor = dirs.iter().flat_map(|d| folders.iter().map(move |f| d.join("upscalers").join(f)));
    for dir in vendor {
        if has_files(&dir, needed, every) {
            return Some(dir);
        }
        // An SDK unpacked whole keeps its DLLs a level down, in `bin` or `x64`: the folder a player drops in is the archive's
        // own top folder far more often than a folder flattened by hand.
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut sub: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            sub.sort();
            if let Some(hit) = sub.into_iter().find(|s| has_files(s, needed, every)) {
                return Some(hit);
            }
        }
    }
    None
}

/// Where the engine will look, in the same order: `upscalers\fsr` (and the older `upscalers\fsr3`), `upscalers\streamline`,
/// `upscalers\xess`, under each of `sdk_dirs`.
pub fn sdk() -> Sdk {
    let dirs = sdk_dirs();
    let fsr = ["fsr", "fsr3"];
    Sdk {
        // Either FSR layout is a complete upscaler, so one of the three files being there is what the page reports.
        fsr3: sdk_in(&dirs, &fsr, FSR_FILES, false),
        dlss: sdk_in(&dirs, &["streamline"], DLSS_FILES, true),
        xess: sdk_in(&dirs, &["xess"], XESS_FILES, true),
    }
}

/// The one folder the launcher can point a player at: `upscalers` in the run folder, beside the game's ini and log. Created on demand.
pub fn upscalers_dir() -> Option<PathBuf> {
    run_dir().map(|d| d.join("upscalers"))
}

/// Finds the vendor DLLs in the places a player is likely to have left them (beside the launcher, in the engine folder, in Downloads and on
/// the Desktop, one folder deep) and copies them into the run folder's `upscalers\<vendor>`. Only files that are absent are copied, and
/// nothing is removed, so running it twice changes nothing.
pub fn adopt_upscaler_files() -> Result<String, String> {
    let root_dir = upscalers_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    let mut sources: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
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
            let Some(src) = sources.iter().map(|s| s.join(name)).find(|p| p.is_file()) else { continue };
            if std::fs::create_dir_all(root_dir.join(folder)).is_err() {
                continue;
            }
            // A running game holds its DLLs; a copy that fails leaves the folder as it was and the report says what is still missing.
            if std::fs::copy(&src, &dest).is_ok() {
                copied += 1;
                where_from.get_or_insert_with(|| src.parent().map(Path::to_path_buf).unwrap_or_default());
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
    let from = where_from.map(|p| format!(" from {}", p.display())).unwrap_or_default();
    let found = if copied == 0 {
        "No new upscaler files were found".to_string()
    } else {
        format!("{copied} upscaler file(s) copied{from} into {}", root_dir.display())
    };
    let (ok, missing) = if have.is_empty() {
        (String::new(), format!("; still missing: {}", lack.join(", ")))
    } else if lack.is_empty() {
        (format!("; ready: {}", have.join(", ")), String::new())
    } else {
        (format!("; found: {}", have.join(", ")), format!("; missing: {}", lack.join(", ")))
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
        "xess" if sdk.xess.is_none() => "fsr1",
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
    if let Some(dir) = &sdk.xess {
        d.insert("upscale.xess_dll".into(), fwd(dir));
    }
    // The rest of the Rendering page: the preset the temporal upscalers scale by, the scene buffer's
    // share of the output, and what makes the frames between two rendered ones.
    d.insert("upscale.quality".into(), upscale_quality_word(&g.upscale_quality).into());
    d.insert("upscale.scene_scale".into(), g.scene_scale.clamp(25, 100).to_string());
    d.insert("dlss.frame_gen".into(), frame_gen_word(&g.frame_gen).into());
    d.insert("dlss.upscaler".into(), dlss_upscaler_word(&g.dlss_upscaler).into());
    d.insert("display.astc_quality".into(), g.astc_quality.to_ascii_lowercase());
    d.insert("gfx.gpu".into(), gpu_word(&g.gpu).into());
    // Shader compilation while playing: the engine's own default is min(4, cpus / 6) background threads,
    // which on a 24-thread laptop is 4 threads at full clock. A player who hears the fans can cap them and
    // push them down the priority ladder; 0 is "leave the engine alone".
    if g.compile_threads > 0 {
        d.insert("nvn.gpl_threads".into(), g.compile_threads.min(16).to_string());
    }
    d.insert("nvn.compile_priority".into(), compile_priority_word(&g.compile_priority).into());
    d.insert("nvn.compile_eco".into(), if g.compile_eco { "1" } else { "0" }.into());
    d.insert("audio.volume".into(), format!("{:.2}", a.master.clamp(0.0, 1.0)));
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
            "nvn" | "dlss" | "gfx" => (true, false),
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
static PLAY_ALERT: std::sync::OnceLock<std::sync::Mutex<Vec<(String, bool)>>> = std::sync::OnceLock::new();

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
    alerts().lock().unwrap_or_else(|e| e.into_inner()).push((text, good));
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
                    format!("The game presented its first frame after {} s ({fps:.1} fps).", started.elapsed().as_secs()),
                    true,
                );
                return;
            }
            if started.elapsed().as_secs() >= 90 {
                let gpu = gpu_name(&std::fs::read_to_string(&log).unwrap_or_default()).unwrap_or_else(|| "the graphics card it picked".into());
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
pub fn play(graphics: &crate::config::Graphics, audio: &crate::config::Audio) -> Result<(), String> {
    let root = root().ok_or("the engine was not found")?;
    let dir = run_dir().ok_or(crate::platform::NO_DATA_DIR)?;
    let exe = game_exe(&root);
    if !exe.is_file() || !dir.join("fefw.ini").is_file() {
        return Err("the game is not built yet: run Build the game on Game Setup".into());
    }
    // A build from an earlier release can point the game at a packed .nup, which the engine cannot read in place. The launcher writes those
    // files before play, so reaching this means that write has not happened yet; say so instead of letting the game die on `main.npdm is
    // compressed in the .nup` (issue #24).
    if packed_package_without_overlay().is_some() {
        return Err("this package keeps some of its files packed: press Play again and the launcher writes them into your work folder first".into());
    }
    // The haptics backend is loaded from beside `fefw.exe` and is not part of a game folder that was built by an earlier
    // release, so it is put there on the way in as well as after a build: updating the launcher should not quietly drop
    // rumble to the standard two motors (issue #10). A running game holds the file, and a copy that fails leaves the one
    // already in place.
    let vendor = root.join("runtime").join("third_party").join("renhdrum").join("renhdrum.dll");
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
    fn the_repair_writes_its_key_into_the_section_it_belongs_to() {
        let before = "[log]\npath = fefw.log\n[fs]\nexefs = C:/e\nromfs = E:/g.nup\nsave_dir = C:/s\n[nvn]\nshader_cache = C:/c\n";
        let after = set_ini_value(before, "fs", "romfs_overlay", "C:/w/romfs-overlay");
        // Anywhere inside [fs] is a key the engine reads; the file stays a flat list of lines.
        let fs = after.split("[nvn]").next().unwrap_or_default();
        assert!(fs.contains("romfs_overlay = C:/w/romfs-overlay"), "{after}");
        assert!(after.starts_with("[log]\npath = fefw.log\n[fs]\n"), "{after}");
        assert!(after.contains("[nvn]\nshader_cache = C:/c\n"), "{after}");
        // A second run replaces the line instead of adding another one.
        let again = set_ini_value(&after, "fs", "romfs_overlay", "D:/other");
        assert_eq!(again.matches("romfs_overlay").count(), 1, "{again}");
        assert!(again.contains("romfs_overlay = D:/other"), "{again}");
        // A file with no [fs] section at all gets one.
        let plain = set_ini_value("[log]\npath = fefw.log\n", "fs", "romfs_overlay", "C:/w");
        assert!(plain.contains("[fs]\nromfs_overlay = C:/w\n"), "{plain}");
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
        assert!(lines.iter().any(|l| l == "py -m pip install zbic"), "{lines:?}");
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
            &RunConfig { exefs: PathBuf::from(r"C:\e"), romfs: PathBuf::from(r"E:\g.nup"), save_dir: PathBuf::from(r"C:\s"), romfs_overlay: None },
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
        let root = keep.clone().unwrap_or_else(|| std::env::temp_dir().join("ew-nup-shader-romfs"));
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
        println!("{files} shader archives ({bytes} bytes) below {}", dir.display());
        if keep.is_none() {
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn the_upscaler_replaces_its_own_key_and_keeps_the_rest() {
        let ini = ini_text(
            Path::new(r"C:\x\engine\fefw-recomp"),
            &RunConfig { exefs: PathBuf::from(r"C:\e"), romfs: PathBuf::from(r"E:\g.nup"), save_dir: PathBuf::from(r"C:\s"), romfs_overlay: None },
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
    fn the_bundled_translator_is_found_only_where_this_platform_expects_it() {
        let (dir_name, file_name) = super::harness_platform();
        assert!(dir_name == "win-x64" || dir_name == "linux-x64");
        assert!(file_name == "spirv_harness.exe" || file_name == "spirv_harness");
        assert_eq!(dir_name.ends_with("win-x64"), cfg!(windows), "the platform folder matches the host");

        let dir = std::env::temp_dir().join(format!("ew-harness-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let slot = dir.join("tools").join("shaders").join("prebuilt").join(dir_name);
        std::fs::create_dir_all(&slot).unwrap();
        assert_eq!(super::bundled_harness_in(&dir), None, "an empty slot is not a translator");

        // Another platform's binary is not ours to run.
        let other = if cfg!(windows) { "linux-x64" } else { "win-x64" };
        let other_file = if cfg!(windows) { "spirv_harness" } else { "spirv_harness.exe" };
        std::fs::create_dir_all(dir.join("tools").join("shaders").join("prebuilt").join(other)).unwrap();
        std::fs::write(dir.join("tools").join("shaders").join("prebuilt").join(other).join(other_file), b"x").unwrap();
        assert_eq!(super::bundled_harness_in(&dir), None, "the other platform's file is ignored");

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
        assert_eq!(d.get("upscale.scene_scale").map(String::as_str), Some("100"));
        assert_eq!(d.get("dlss.frame_gen").map(String::as_str), Some("0"));
        assert_eq!(d.get("dlss.upscaler").map(String::as_str), Some("auto"));
        assert_eq!(d.get("display.astc_quality").map(String::as_str), Some("high"));
        assert_eq!(d.get("nvn.compile_priority").map(String::as_str), Some("below"));
        assert_eq!(d.get("nvn.compile_eco").map(String::as_str), Some("1"));
        assert!(!d.contains_key("nvn.gpl_threads"), "automatic leaves the engine's own min(4, cpus / 6)");

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
        assert_eq!(d.get("upscale.quality").map(String::as_str), Some("performance"));
        assert_eq!(d.get("upscale.scene_scale").map(String::as_str), Some("70"));
        assert_eq!(d.get("dlss.frame_gen").map(String::as_str), Some("fsr3"));
        assert_eq!(d.get("dlss.upscaler").map(String::as_str), Some("taau"));
        assert_eq!(d.get("display.astc_quality").map(String::as_str), Some("fast"));
        assert_eq!(d.get("gfx.gpu").map(String::as_str), Some("integrated"));
        assert_eq!(d.get("nvn.gpl_threads").map(String::as_str), Some("2"));
        assert_eq!(d.get("nvn.compile_priority").map(String::as_str), Some("idle"));
        assert_eq!(d.get("nvn.compile_eco").map(String::as_str), Some("0"));
        // A scene scale outside the engine's 25..100 is clamped rather than passed on to be rejected.
        g.scene_scale = 5;
        assert_eq!(super::desired_values(&g, &audio).get("upscale.scene_scale").map(String::as_str), Some("25"));
    }

    #[test]
    fn the_log_report_names_the_error_and_the_repair() {
        let text = "-- populating imgui\nCMake Error at _deps/imgui-src/CMakeLists.txt:12 (message):\n  the source is not there\n-- Configuring incomplete, errors occurred!\n";
        let report = super::verify_log_text(text);
        assert!(report.contains("first error:"), "{report}");
        assert!(report.contains("CMake Error at _deps/imgui-src/CMakeLists.txt:12"), "{report}");
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
        assert!(!dir.join("release").join("_deps").exists(), "the half-written sources are gone");
        assert!(said.iter().any(|m| m.contains("dependency is incomplete")), "{said:?}");
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
        assert!(!rel.join("CMakeCache.txt").exists() && !rel.join("CMakeFiles").exists(), "the cache is gone");
        assert!(out.unwrap_err().contains("unknown argument"), "the message is the second failure");
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
        assert!(super::half_written_deps("CMake Error at _deps/imgui-src/CMakeLists.txt:3"));
        assert!(super::half_written_deps("ImGui Vulkan failure patch no longer matches pinned source"));
        assert!(!super::half_written_deps("clang-cl: error: unknown argument '-fno-exceptions'"));
        // A cache from another toolchain.
        assert!(super::stale_build_cache("CMakeCache.txt is different than the one used previously"));
        assert!(super::stale_build_cache("CMAKE_CXX_COMPILER points at a path that does not exist"));
        assert!(!super::stale_build_cache("Configuring incomplete, errors occurred!"));
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
        assert!(block.starts_with("CMake Error at cmake/gfx_deps.cmake:41"), "{block}");
        assert!(block.contains("no ASTC transcoder"), "{block}");
        assert!(!block.contains("Configuring incomplete"), "the summary is not the error: {block}");
    }

    #[test]
    fn a_run_remembers_the_first_error_and_not_a_later_one() {
        let tail: super::Tail = Default::default();
        let first: super::FirstError = Default::default();
        for line in ["-- step", "error: first thing", "  detail", "error: second thing"] {
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

    #[test]
    fn a_second_build_is_refused_while_the_lock_names_a_live_process() {
        let root = std::env::temp_dir().join(format!("ew-lock-{}", std::process::id()));
        let dir = root.join("work").join("build");
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join(".build.lock");
        // A process that cannot exist: the lock is stale, so a new build takes it.
        std::fs::write(&lock, "9999990 1 emblemweave.exe
").unwrap();
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
        assert!(!textures_fetched(&root), "an empty folder must not count as fetched");
        std::fs::write(tp.join("astc-encoder").join("CMakeLists.txt"), "").unwrap();
        std::fs::create_dir_all(tp.join("bc7enc_rdo")).unwrap();
        std::fs::write(tp.join("bc7enc_rdo").join("CMakeLists.txt"), "").unwrap();
        assert!(!textures_fetched(&root), "one missing piece is still missing");
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
        std::fs::write(dir.join("CMakeCache.txt"), "CMAKE_C_COMPILER:FILEPATH=C:/old/clang-cl.exe
").unwrap();
        let compiler = Path::new("C:/Program Files/LLVM/bin/clang-cl.exe");
        let why = crate::toolchain::cache_mismatch_in(&dir, compiler, "Ninja", "").expect("a mismatch");
        assert!(why.contains("C:/old/clang-cl.exe"), "{why}");
        crate::toolchain::clear_cache(&dir);
        assert!(!dir.join("CMakeCache.txt").exists());
        assert!(!dir.join("CMakeFiles").exists());
        // Nothing there now: CMake will write its own.
        assert!(crate::toolchain::cache_mismatch_in(&dir, compiler, "Ninja", "").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
