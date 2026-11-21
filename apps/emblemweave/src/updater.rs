//! Updates from GitHub Releases: asking whether a newer release exists,
//! downloading its zip and swapping the new launcher in for this one.
//!
//! Downloads go through Windows' own `curl.exe` and `tar.exe`, as the mods page
//! does, so the launcher carries no HTTP client of its own.
//!
//! An update is the whole release, not just the executable: the launcher, its
//! docs and the engine sources are written over the install, while the player's
//! data, built game and fetched tools stay where they are. A running executable
//! cannot be overwritten on Windows, but it can be replaced after exit: the new
//! binary is staged as `EmblemWeave.exe.new`, the current one is kept as
//! `backups\EmblemWeave.exe.old`, and a short script swaps them and starts the new one.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "XiONDiGiTaL/EmblemWeave";
/// The asset name a Windows build is published under.
pub const ASSET_SUFFIX: &str = "-windows-x64.zip";

/// A published release, as much of it as the launcher shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Option<(u32, u32, u32)>,
    pub notes: String,
    pub page: String,
    /// The Windows zip: its name and download URL.
    pub asset: Option<(String, String)>,
}

pub fn version_now() -> (u32, u32, u32) {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

/// `v0.1.4`, `0.1.4` and `0.1.4-rc1` all give `(0, 1, 4)`.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let t = text.trim().trim_start_matches(['v', 'V']);
    let mut parts = t.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// Whether `tag` names a release newer than `now`.
pub fn is_newer(tag: &str, now: (u32, u32, u32)) -> bool {
    parse_version(tag).is_some_and(|v| v > now)
}

/// A release tag in the same `major.minor.patch` form as `CARGO_PKG_VERSION`, which is how the carried-build stamp is
/// compared. A tag that is not a version is kept as written.
fn version_text(tag: &str) -> String {
    parse_version(tag)
        .map(|(a, b, c)| format!("{a}.{b}.{c}"))
        .unwrap_or_else(|| tag.trim().to_string())
}

fn system_tool(name: &str) -> PathBuf {
    let sys = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let p = sys.join("System32").join(name);
    if p.is_file() {
        p
    } else {
        PathBuf::from(name)
    }
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// `curl` with the launcher's user agent; the output is the response body.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let out = hidden(Command::new(system_tool("curl.exe")).args([
        "-sSfL",
        "--max-time",
        "120",
        "-A",
        concat!("EmblemWeave/", env!("CARGO_PKG_VERSION")),
        "-H",
        "Accept: application/vnd.github+json",
        url,
    ]))
    .output()
    .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.contains("404") {
            "no release was found for this repository".into()
        } else {
            format!("GitHub answered: {err}")
        });
    }
    Ok(out.stdout)
}

/// The newest published release.
pub fn latest() -> Result<Release, String> {
    let body = fetch(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?;
    release_from_json(&body)
}

/// Reads a GitHub release object. Separated from the request so it can be
/// tested against recorded responses.
pub fn release_from_json(body: &[u8]) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("GitHub answered with something unexpected: {e}"))?;
    let tag = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_string();
    if tag.is_empty() {
        return Err("the release has no tag".into());
    }
    let asset = v.get("assets").and_then(|a| a.as_array()).and_then(|a| {
        a.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            if name.ends_with(ASSET_SUFFIX) {
                Some((
                    name.to_string(),
                    a.get("browser_download_url")?.as_str()?.to_string(),
                ))
            } else {
                None
            }
        })
    });
    Ok(Release {
        version: parse_version(&tag),
        notes: v
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or_default()
            .to_string(),
        page: v
            .get("html_url")
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string(),
        asset,
        tag,
    })
}

/// Where a downloaded update is unpacked. It is disposable: the new launcher
/// is moved out of it before the swap script starts.
pub fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("emblemweave-update")
}

fn tree_bytes(dir: &Path) -> u64 {
    let mut total = 0;
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
                total += m.len();
            }
        }
    }
    total
}

/// Remove the previous update's download and unpack folders. A failed or
/// interrupted update can leave a partial zip or an old executable here; none
/// of it is needed once this launcher is running. Saves, profiles and the
/// engine build are elsewhere and are never touched.
pub fn clean_cache() -> u64 {
    let dir = cache_dir();
    let freed = tree_bytes(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    // The swap scripts delete themselves, but one interrupted by a reboot can
    // stay behind. They hold no user data either.
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name.starts_with("emblemweave-update-") && name.ends_with(".cmd") {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    freed
}

/// The launcher running now.
pub fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Where the build replaced by the last update is kept: `backups\EmblemWeave.exe.old` beside the launcher, so an
/// update never leaves an `.old` file in the install root.
///
/// Only one previous build is kept, however many times this copy updates.
pub fn backup_exe() -> Option<PathBuf> {
    current_exe().map(|exe| backups_dir(&exe).join(BACKUP_FILE))
}

/// The folder beside the launcher that keeps the previous build.
fn backups_dir(exe: &Path) -> PathBuf {
    exe.with_file_name("backups")
}

/// The one previous build the updater keeps, in `backups\`.
const BACKUP_FILE: &str = "EmblemWeave.exe.old";

/// Move the `EmblemWeave.exe.old` an earlier version wrote beside the launcher into `backups\`, so the install root is
/// clear again after the first start of this build.
pub fn migrate_legacy_backup() {
    if let Some(exe) = current_exe() {
        migrate_backup(&exe);
    }
}

fn migrate_backup(exe: &Path) {
    // EmblemWeave.exe becomes EmblemWeave.exe.old.
    let legacy = exe.with_extension("exe.old");
    if !legacy.is_file() {
        return;
    }
    let dir = backups_dir(exe);
    let kept = dir.join(BACKUP_FILE);
    if kept.is_file() {
        // Restore offers the newest previous build, which `kept` already holds; this older one is only clutter.
        let _ = std::fs::remove_file(&legacy);
        return;
    }
    // A move keeps the install from holding two copies of the same build; if it fails the file stays put and the next
    // start tries again.
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::rename(&legacy, &kept);
    }
}

pub fn has_backup() -> bool {
    backup_exe().is_some_and(|p| p.is_file())
}

/// Download a release's zip and unpack its executable into `dir`, returning
/// the unpacked `EmblemWeave.exe`.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let (name, url) = release
        .asset
        .clone()
        .ok_or("that release has no Windows download")?;
    if !url.starts_with("https://") {
        return Err("the download address is not https".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let zip = dir.join(crate::mods::clean_name(&name));
    // An interrupted download must not be mistaken for the new zip.
    let _ = std::fs::remove_file(&zip);
    let out = hidden(Command::new(system_tool("curl.exe")).args([
        "-sSfL",
        "--max-time",
        "900",
        "-A",
        concat!("EmblemWeave/", env!("CARGO_PKG_VERSION")),
        "-o",
    ]))
    .arg(&zip)
    .arg(&url)
    .output()
    .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the download failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let unpacked = dir.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let out = hidden(
        Command::new(system_tool("tar.exe"))
            .arg("-xf")
            .arg(&zip)
            .arg("-C")
            .arg(&unpacked),
    )
    .output()
    .map_err(|e| format!("running tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the download could not be unpacked: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    find_exe(&unpacked, 3).ok_or_else(|| "the download has no EmblemWeave.exe in it".to_string())
}

/// Finish an update that an older launcher started but could not complete.
///
/// Before this release the updater moved only `EmblemWeave.exe` out of the download cache and left the rest of the
/// unpacked release there. The new executable starts, sees a matching release with no executable of its own, and writes
/// the docs and engine sources from it before [`clean_cache`] removes the folder. A release downloaded by this version
/// still has its executable (it is copied, not moved), so it is skipped: its overlay already ran.
pub fn finish_interrupted_update(install: &Path, engine: &Path) -> Result<usize, String> {
    let cache = cache_dir().join("unpacked");
    let Some(release) = incomplete_release(&cache) else {
        // A launcher from before `carried_by` existed wrote the whole release but left its executable in the cache. Its
        // presence proves this launcher came from an in-place update, so record the carry even though the sources were
        // already written (issue #56). This is idempotent with the marker [`install`] writes.
        if release_of_this_version(&cache).is_some() {
            crate::engine::mark_build_carried(engine, env!("CARGO_PKG_VERSION"));
        }
        return Ok(0);
    };
    let copied = overlay_release(&release, install, engine)?;
    crate::engine::mark_build_carried(engine, env!("CARGO_PKG_VERSION"));
    Ok(copied)
}

/// The unpacked release for this launcher's own version, executable included or not. A matching folder in the update
/// cache is how the first start after an update knows the build beside it was carried forward; whether it still needs
/// its sources written is [`incomplete_release`]'s question.
fn release_of_this_version(cache: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(cache).ok()?.flatten() {
        let root = entry.path();
        if !root.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let tag = name.split_once('-').map(|(_, tag)| tag).unwrap_or(&name);
        if parse_version(tag) != Some(version_now()) {
            continue;
        }
        if root.join("docs").join("PACKAGE.md").is_file()
            && root.join("engine").join("fefw-recomp").is_dir()
        {
            return Some(root);
        }
    }
    None
}

fn incomplete_release(cache: &Path) -> Option<PathBuf> {
    // The old updater's signature: the executable was moved out, but the release and its manifest remain.
    release_of_this_version(cache).filter(|root| !root.join("EmblemWeave.exe").is_file())
}

fn find_exe(dir: &Path, depth: usize) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                if let Some(f) = find_exe(&p, depth - 1) {
                    return Some(f);
                }
            }
        } else if p
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("EmblemWeave.exe"))
        {
            return Some(p);
        }
    }
    None
}

/// Replace the release in this install with the downloaded one, then start the new launcher.
///
/// The launcher, its docs and the engine sources are the release; `data`, the built game, the Python environment and the
/// fetched third-party sources are not in the zip and are left exactly where they are. The engine is written where this
/// copy actually reads it (`engine_root`), so an install that reuses an older checkout updates that checkout rather than
/// creating a second, stale copy beside it. `portable.txt` is never copied: an update must not switch a profile install to
/// portable storage behind the player's back. The caller closes the window afterwards; the script waits for the running
/// executable to be free and starts the new one.
pub fn install(
    new_exe: &Path,
    engine_root: Option<&Path>,
    keep_previous: bool,
    release_tag: &str,
) -> Result<(), String> {
    let target = current_exe().ok_or("the launcher's own path is unknown")?;
    let staged = install_into(
        new_exe,
        &target,
        &backups_dir(&target),
        crate::engine::run_dir().as_deref(),
        engine_root,
        keep_previous,
        release_tag,
    )?;
    swap_script(&staged, &target, &update_log(), failure_marker().as_deref())
}

/// The part of [`install`] that runs before the restart, with the running launcher's paths passed in so a test can drive
/// the same sequence against a temporary install. `play` is the folder holding the kept build's `fefw.ini`; when there is
/// none, the release is still written but no carry is recorded. Returns the staged launcher the swap script replaces
/// `target` with.
fn install_into(
    new_exe: &Path,
    target: &Path,
    backups: &Path,
    play: Option<&Path>,
    engine_root: Option<&Path>,
    keep_previous: bool,
    release_tag: &str,
) -> Result<PathBuf, String> {
    let install = target.parent().ok_or("the launcher's folder is unknown")?;
    let release = new_exe
        .parent()
        .ok_or("the download has no release folder")?;
    let engine = engine_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| install.join("engine").join("fefw-recomp"));
    // The executable is the one file that cannot be written while it runs. Stage it beside the old one and let the
    // script move it once this process has exited. Stage it before the sources: if the overlay then fails, there is no
    // half-written new launcher waiting to be started and the previous version's backup is still intact.
    let staged = PathBuf::from(format!("{}.new", target.display()));
    std::fs::copy(new_exe, &staged).map_err(|e| format!("staging the new launcher: {e}"))?;
    if let Err(e) = overlay_release(release, install, &engine) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    // The compiled game was deliberately kept: say so before the restart, so Overview opens ready to play instead of
    // treating the carried build as one from another release (issue #56).
    if let Some(play) = play {
        crate::engine::mark_build_carried_in(&engine, play, &version_text(release_tag));
    }
    if keep_previous {
        // The backup lives in `backups\`; the folder does not exist yet on an install that has never updated.
        if let Err(e) = std::fs::create_dir_all(backups) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("creating the backups folder: {e}"));
        }
        // Copying a running executable is allowed; replacing it is not. Do it only once the release is known to be
        // complete, so a failed update does not overwrite the version Restore would go back to.
        if let Err(e) = std::fs::copy(&target, backups.join(BACKUP_FILE)) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("keeping the current version: {e}"));
        }
    }
    Ok(staged)
}

/// Put the build kept from the last update back.
pub fn restore_previous() -> Result<(), String> {
    let backup = backup_exe()
        .filter(|p| p.is_file())
        .ok_or("there is no previous version to go back to")?;
    let target = current_exe().ok_or("the launcher's own path is unknown")?;
    let tmp = std::env::temp_dir().join(format!("emblemweave-rollback-{}.exe", std::process::id()));
    std::fs::copy(&backup, &tmp).map_err(|e| format!("reading the previous version: {e}"))?;
    swap_script(&tmp, &target, &update_log(), failure_marker().as_deref())
}

/// The release's files that are not generated on this PC. The build folder, the engine's work folder, the Python
/// environment and the fetched third-party trees are outputs, not release files: a new zip never contains them, and an
/// update that replaced them would throw away the expensive parts of the install.
const KEEP_ENGINE: [&str; 7] = [
    "game-build/fefw/build",
    "work",
    "tools/.venv",
    "tools/bin",
    "tools/textures/third_party",
    "tools/shaders/third_party",
    "tools/shaders/build",
];

/// Copy one directory over another, skipping the generated paths in `keep`. Files are replaced, never deleted: a stale
/// file the new release no longer ships is harmless, while deleting the wrong folder would take a build with it.
fn overlay_tree(from: &Path, to: &Path, rel: &Path, keep: &[&str]) -> Result<usize, String> {
    if keep
        .iter()
        .any(|k| rel == Path::new(k) || rel.starts_with(k))
    {
        return Ok(0);
    }
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from)
        .map_err(|e| format!("reading {}: {e}", from.display()))?
        .flatten()
    {
        let src = entry.path();
        let name = entry.file_name();
        let dst = to.join(&name);
        if src.is_dir() {
            copied += overlay_tree(&src, &dst, &rel.join(&name), keep)?;
        } else {
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("copying {} to {}: {e}", src.display(), dst.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Write the release's launcher-side files into `install` and its engine sources into `engine`.
///
/// The release folder is the one `EmblemWeave.exe` sits in. `data` and `portable.txt` are deliberately not copied. The
/// engine is handled separately so it can live outside this install: `engine_root` is where this copy reads it, which is
/// the folder a reused build is in.
fn overlay_release(release: &Path, install: &Path, engine: &Path) -> Result<usize, String> {
    let from_engine = release.join("engine").join("fefw-recomp");
    if !from_engine.is_dir() {
        // A release without engine sources would leave a new launcher beside old sources: the exact mixed install this
        // updater exists to prevent. Refuse before changing the install.
        return Err("the release has no engine/fefw-recomp folder".into());
    }
    std::fs::create_dir_all(install).map_err(|e| format!("creating {}: {e}", install.display()))?;
    let mut copied = 0;
    for entry in std::fs::read_dir(release)
        .map_err(|e| format!("reading {}: {e}", release.display()))?
        .flatten()
    {
        let name = entry.file_name();
        let lower = name.to_string_lossy().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "emblemweave.exe" | "portable.txt" | "data" | "engine"
        ) {
            continue;
        }
        let src = entry.path();
        let dst = install.join(&name);
        if src.is_dir() {
            copied += overlay_tree(&src, &dst, Path::new(""), &[])?;
        } else {
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("copying {} to {}: {e}", src.display(), dst.display()))?;
            copied += 1;
        }
    }
    copied += overlay_tree(&from_engine, engine, Path::new(""), &KEEP_ENGINE)?;
    // The release just rewrote the install and the checkout beside it, and may have brought engine sources this
    // copy had none of, so a folder resolved earlier in the session no longer describes what is on disk.
    crate::platform::forget_resolved_folders();
    Ok(copied)
}

/// The update script's output. It lives with the other launcher logs, so a failed swap can be diagnosed without asking the
/// player to find a hidden temporary file.
fn update_log() -> PathBuf {
    if let Some(dir) = crate::config::Config::dir().map(|d| d.join("logs")) {
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir.join("update.log");
        }
    }
    // The script redirects into this path; a directory that was not created would turn a working move into a failure.
    std::env::temp_dir().join("emblemweave-update.log")
}

/// A note the swap script leaves when it cannot replace the launcher; the next start shows it instead of failing silently.
fn failure_marker() -> Option<PathBuf> {
    crate::config::Config::dir().map(|d| d.join("update-failed.txt"))
}

/// Read and clear the note a failed swap left behind.
pub fn take_failure() -> Option<String> {
    let path = failure_marker()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn swap_script(
    new_exe: &Path,
    target: &Path,
    log: &Path,
    failed: Option<&Path>,
) -> Result<(), String> {
    let script =
        std::env::temp_dir().join(format!("emblemweave-update-{}.cmd", std::process::id()));
    let marker = failed.map(|p| p.display().to_string()).unwrap_or_default();
    let text = format!(
        "@echo off\r\n\
         setlocal\r\n\
         set \"NEW={new}\"\r\n\
         set \"TARGET={target}\"\r\n\
         set \"LOG={log}\"\r\n\
         set \"FAIL={marker}\"\r\n\
         ping -n 3 127.0.0.1 >nul\r\n\
         set tries=0\r\n\
         :again\r\n\
         move /y \"%NEW%\" \"%TARGET%\" >> \"%LOG%\" 2>&1\r\n\
         if not errorlevel 1 goto installed\r\n\
         set /a tries+=1\r\n\
         if %tries% geq 30 goto failed\r\n\
         ping -n 2 127.0.0.1 >nul\r\n\
         goto again\r\n\
         :installed\r\n\
         if not \"%FAIL%\"==\"\" if exist \"%FAIL%\" del \"%FAIL%\" >nul 2>&1\r\n\
         start \"\" \"%TARGET%\"\r\n\
         goto done\r\n\
         :failed\r\n\
         if not \"%FAIL%\"==\"\" echo The update could not replace EmblemWeave.exe. The previous launcher was started. See \"%LOG%\". > \"%FAIL%\"\r\n\
         start \"\" \"%TARGET%\"\r\n\
         :done\r\n\
         del \"%~f0\"\r\n",
        new = new_exe.display(),
        target = target.display(),
        log = log.display(),
    );
    std::fs::write(&script, text).map_err(|e| format!("writing the update script: {e}"))?;
    hidden(Command::new("cmd").arg("/c").arg(&script))
        .spawn()
        .map_err(|e| format!("starting the update: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn an_update_replaces_the_release_and_keeps_the_build_and_data() {
        let root = std::env::temp_dir().join(format!("ew-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let release = root.join("EmblemWeave-v0.9.0");
        let install = root.join("EmblemWeave-v0.1.39");
        let engine = install.join("engine/fefw-recomp");

        write(&release.join("EmblemWeave.exe"), "new launcher");
        write(&release.join("portable.txt"), "release marker");
        write(&release.join("data/save/Other"), "release save");
        write(&release.join("README.md"), "new readme");
        write(&release.join("docs/USAGE.md"), "new docs");
        write(
            &release.join("engine/fefw-recomp/README.md"),
            "new engine readme",
        );
        write(
            &release.join("engine/fefw-recomp/recomp/new.txt"),
            "new engine source",
        );
        // Generated folders must never be taken from the release, even if a malformed zip carries them.
        write(
            &release.join("engine/fefw-recomp/game-build/fefw/build/release/fefw.exe"),
            "release build",
        );
        write(
            &release.join("engine/fefw-recomp/tools/.venv/pyvenv.cfg"),
            "release venv",
        );
        write(
            &release.join("engine/fefw-recomp/work/cache"),
            "release work",
        );
        write(
            &release.join("engine/fefw-recomp/tools/textures/third_party/lib.txt"),
            "release fetched",
        );

        write(&install.join("EmblemWeave.exe"), "old launcher");
        write(&install.join("data/save/Auto"), "save");
        write(
            &install.join("engine/fefw-recomp/game-build/fefw/build/release/fefw.exe"),
            "built game",
        );
        write(
            &install.join("engine/fefw-recomp/tools/.venv/pyvenv.cfg"),
            "keep venv",
        );
        write(
            &install.join("engine/fefw-recomp/tools/textures/third_party/lib.txt"),
            "keep fetched",
        );

        overlay_release(&release, &install, &engine).unwrap();

        // The release's launcher-side files are written over the install.
        assert_eq!(read(&install.join("README.md")), "new readme");
        assert_eq!(read(&install.join("docs/USAGE.md")), "new docs");
        assert_eq!(
            read(&install.join("engine/fefw-recomp/README.md")),
            "new engine readme"
        );
        assert_eq!(
            read(&install.join("engine/fefw-recomp/recomp/new.txt")),
            "new engine source"
        );

        // The executable is swapped by the script after the process exits, not by the overlay; the storage marker is
        // never added by an update.
        assert_eq!(read(&install.join("EmblemWeave.exe")), "old launcher");
        assert!(!install.join("portable.txt").exists());

        // Everything generated on this PC survives.
        assert_eq!(read(&install.join("data/save/Auto")), "save");
        assert!(!install.join("data/save/Other").exists());
        assert_eq!(
            read(&install.join("engine/fefw-recomp/game-build/fefw/build/release/fefw.exe")),
            "built game"
        );
        assert_eq!(
            read(&install.join("engine/fefw-recomp/tools/.venv/pyvenv.cfg")),
            "keep venv"
        );
        assert_eq!(
            read(&install.join("engine/fefw-recomp/tools/textures/third_party/lib.txt")),
            "keep fetched"
        );
        assert!(!install.join("engine/fefw-recomp/work/cache").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    /// An in-place update keeps the compiled game, so the restarted launcher must not treat it as a build from another
    /// release: the install path records `carried_by` in `fefw.ini` while never writing the build folder (issue #56).
    #[test]
    fn an_in_place_update_records_the_carried_build_and_keeps_the_compiled_game() {
        let root = std::env::temp_dir().join(format!("ew-update-carried-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let release = root.join("EmblemWeave-v0.1.40");
        let install = root.join("EmblemWeave");
        let engine = install.join("engine/fefw-recomp");
        let play = root.join("data/play");

        write(&release.join("EmblemWeave.exe"), "new launcher");
        write(&release.join("docs/PACKAGE.md"), "release manifest");
        // A malformed zip's generated folder must never replace the player's build.
        write(
            &release.join("engine/fefw-recomp/game-build/fefw/build/release/fefw.exe"),
            "release build",
        );

        write(&install.join("EmblemWeave.exe"), "old launcher");
        write(&install.join("data/config.json"), "player settings");
        let built = engine.join("game-build/fefw/build/release/fefw.exe");
        write(&built, "compiled game");
        write(
            &play.join("fefw.ini"),
            "[engine]\nbuilt_by = 0.1.39\npacked_nup = 1\n[fs]\nromfs = game.nup\n",
        );

        let staged = install_into(
            &release.join("EmblemWeave.exe"),
            &install.join("EmblemWeave.exe"),
            &install.join("backups"),
            Some(&play),
            Some(&engine),
            true,
            "v0.1.40",
        )
        .unwrap();

        // The build was deliberately carried: its own stamp stays and the update is recorded.
        let ini = read(&play.join("fefw.ini"));
        assert!(
            ini.contains("built_by = 0.1.39"),
            "the build's own stamp is kept: {ini}"
        );
        assert!(
            ini.contains("carried_by = 0.1.40"),
            "the carry is recorded: {ini}"
        );
        // The compiled game is exactly the file that was already there.
        assert_eq!(read(&built), "compiled game");
        // The release is written, while the running launcher waits for the swap script and is kept as the backup.
        assert_eq!(read(&install.join("docs/PACKAGE.md")), "release manifest");
        assert_eq!(read(&staged), "new launcher");
        assert_eq!(read(&install.join("EmblemWeave.exe")), "old launcher");
        // The backup goes in `backups\`, which the update creates; nothing is written beside the launcher.
        assert_eq!(
            read(&install.join("backups/EmblemWeave.exe.old")),
            "old launcher"
        );
        assert!(!install.join("EmblemWeave.exe.old").exists());
        // The player's data is not part of the release overlay.
        assert_eq!(read(&install.join("data/config.json")), "player settings");

        let _ = std::fs::remove_dir_all(root);
    }

    /// The build an older version kept beside the launcher is moved into `backups\`, so upgrading does not leave the
    /// install root with an `.old` file after the update that starts using the folder.
    #[test]
    fn a_backup_beside_the_launcher_moves_into_the_backups_folder() {
        let root = std::env::temp_dir().join(format!("ew-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let exe = root.join("EmblemWeave.exe");
        write(&exe, "current launcher");
        write(&root.join("EmblemWeave.exe.old"), "previous launcher");

        migrate_backup(&exe);

        assert!(!root.join("EmblemWeave.exe.old").exists());
        assert_eq!(
            read(&root.join("backups/EmblemWeave.exe.old")),
            "previous launcher"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The root copy an older version left behind is older than the one the last update kept: the kept backup wins and
    /// the root file is removed instead of replacing it.
    #[test]
    fn an_older_backup_beside_the_launcher_does_not_replace_the_kept_one() {
        let root = std::env::temp_dir().join(format!("ew-migrate-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let exe = root.join("EmblemWeave.exe");
        write(&exe, "current launcher");
        write(&root.join("EmblemWeave.exe.old"), "older");
        write(&root.join("backups/EmblemWeave.exe.old"), "kept");

        migrate_backup(&exe);

        assert!(!root.join("EmblemWeave.exe.old").exists());
        assert_eq!(read(&root.join("backups/EmblemWeave.exe.old")), "kept");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_update_keeps_an_existing_portable_marker() {
        let root = std::env::temp_dir().join(format!("ew-update-portable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let release = root.join("EmblemWeave-v0.9.0");
        let install = root.join("EmblemWeave");
        write(&release.join("EmblemWeave.exe"), "new");
        write(&release.join("portable.txt"), "release marker");
        write(
            &release.join("engine/fefw-recomp/README.md"),
            "new engine readme",
        );
        write(&install.join("portable.txt"), "this install is portable");

        overlay_release(&release, &install, &install.join("engine/fefw-recomp")).unwrap();
        assert_eq!(
            read(&install.join("portable.txt")),
            "this install is portable"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_update_writes_the_engine_where_this_copy_reads_it() {
        // A launcher that reused a build elsewhere must update that checkout, not just the unused copy beside the exe.
        let root = std::env::temp_dir().join(format!("ew-update-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let release = root.join("EmblemWeave-v0.9.0");
        let install = root.join("EmblemWeave");
        let shared = root.join("old-install/engine/fefw-recomp");
        write(&release.join("EmblemWeave.exe"), "new");
        write(
            &release.join("engine/fefw-recomp/README.md"),
            "new engine readme",
        );
        write(
            &release.join("engine/fefw-recomp/recomp/new.txt"),
            "new source",
        );
        write(
            &shared.join("game-build/fefw/build/release/fefw.exe"),
            "existing build",
        );
        write(&shared.join("recomp/old.txt"), "old source");

        overlay_release(&release, &install, &shared).unwrap();
        assert_eq!(read(&shared.join("README.md")), "new engine readme");
        assert_eq!(read(&shared.join("recomp/new.txt")), "new source");
        assert_eq!(
            read(&shared.join("game-build/fefw/build/release/fefw.exe")),
            "existing build"
        );
        assert!(!install.join("engine/fefw-recomp/README.md").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_release_without_engine_sources_is_refused_before_anything_is_written() {
        let root = std::env::temp_dir().join(format!("ew-update-no-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let release = root.join("EmblemWeave-v0.9.0");
        let install = root.join("EmblemWeave");
        write(&release.join("README.md"), "new readme");
        write(&install.join("README.md"), "old readme");

        assert!(overlay_release(&release, &install, &install.join("engine/fefw-recomp")).is_err());
        assert_eq!(read(&install.join("README.md")), "old readme");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_old_updaters_cache_is_finished_once() {
        // Releases before this one moved the executable out and left the rest of the zip behind. That release is the
        // only one an interrupted update should finish; a release this version downloaded keeps its executable.
        let cache = std::env::temp_dir().join(format!("ew-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let root = cache.join(format!("EmblemWeave-v{}", env!("CARGO_PKG_VERSION")));
        write(&root.join("docs/PACKAGE.md"), "release manifest");
        write(&root.join("engine/fefw-recomp/README.md"), "engine");
        assert_eq!(incomplete_release(&cache), Some(root.clone()));

        write(&root.join("EmblemWeave.exe"), "new launcher");
        assert_eq!(incomplete_release(&cache), None);
        // A completed release still identifies the update to the first start of this launcher.
        assert_eq!(release_of_this_version(&cache), Some(root.clone()));

        // A newer release left in the cache is not this build's unfinished update.
        let other = cache.join("EmblemWeave-v9.9.9");
        write(&other.join("docs/PACKAGE.md"), "release manifest");
        write(&other.join("engine/fefw-recomp/README.md"), "engine");
        assert_eq!(incomplete_release(&cache), None);
        let _ = std::fs::remove_dir_all(cache);
    }

    #[test]
    fn tags_compare_like_versions() {
        assert_eq!(parse_version("v0.1.4"), Some((0, 1, 4)));
        assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
        assert_eq!(version_text("v0.1.4-rc1"), "0.1.4");
        assert_eq!(parse_version("v0.1.4-rc1"), Some((0, 1, 4)));
        assert!(is_newer("v0.1.5", (0, 1, 4)));
        assert!(is_newer("v0.2.0", (0, 1, 9)));
        assert!(!is_newer("v0.1.4", (0, 1, 4)));
        assert!(!is_newer("v0.1.3", (0, 1, 4)));
        assert!(!is_newer("nightly", (0, 1, 4)));
    }

    /// The Windows zip is picked out of a release's assets, and the notes and
    /// page come along.
    #[test]
    fn reads_a_release() {
        let body = br#"{
            "tag_name": "v0.9.0",
            "body": "Big changes.",
            "html_url": "https://github.com/XiONDiGiTaL/EmblemWeave/releases/tag/v0.9.0",
            "assets": [
                {"name": "source.tar.gz", "browser_download_url": "https://example.com/source.tar.gz"},
                {"name": "EmblemWeave-v0.9.0-windows-x64.zip", "browser_download_url": "https://example.com/app.zip"}
            ]
        }"#;
        let r = release_from_json(body).unwrap();
        assert_eq!(r.tag, "v0.9.0");
        assert_eq!(r.version, Some((0, 9, 0)));
        assert_eq!(r.notes, "Big changes.");
        assert!(r.page.ends_with("v0.9.0"));
        assert_eq!(r.asset.unwrap().0, "EmblemWeave-v0.9.0-windows-x64.zip");
    }

    #[test]
    fn a_release_without_a_windows_build_says_so() {
        let body = br#"{"tag_name": "v1.0.0", "assets": []}"#;
        let r = release_from_json(body).unwrap();
        assert!(r.asset.is_none());
        assert!(download(&r, &std::env::temp_dir()).is_err());
        assert!(release_from_json(b"{}").is_err());
        assert!(release_from_json(b"not json").is_err());
    }
}
