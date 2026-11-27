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

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

pub const REPO: &str = "XiONDiGiTaL/EmblemWeave";
/// The asset name a Windows build is published under.
pub const ASSET_SUFFIX: &str = "-windows-x64.zip";

/// One file attached to a release, with the size and SHA-256 GitHub records for it when the response carries them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

/// A published release, as much of it as the launcher shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Option<(u32, u32, u32)>,
    pub notes: String,
    pub page: String,
    /// The Windows zip: its name, download URL and published hash.
    pub asset: Option<Asset>,
    /// The release's `release-manifest.json`, which also pins the zip's SHA-256 and size.
    pub manifest: Option<Asset>,
}

/// Whether RecompLauncher started this copy. It sets `RECOMPLAUNCHER=1` on the programs it starts and installs every
/// release itself, so such a copy never checks for, downloads or swaps in a release of its own: two updaters on one
/// folder fight.
pub fn managed_by_recomplauncher() -> bool {
    managed_by_recomplauncher_from(&|k| std::env::var(k).ok())
}

pub fn managed_by_recomplauncher_from(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env("RECOMPLAUNCHER").is_some_and(|v| v.trim() == "1")
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

/// One attached file from a GitHub release object. A missing download URL or name makes it unusable.
fn asset_from_json(v: &serde_json::Value) -> Option<Asset> {
    Some(Asset {
        name: v.get("name")?.as_str()?.to_string(),
        url: v.get("browser_download_url")?.as_str()?.to_string(),
        size: v.get("size").and_then(|s| s.as_u64()),
        sha256: v
            .get("digest")
            .and_then(|d| d.as_str())
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase)
            .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())),
    })
}

/// The first 64-character hex run after `marker`. The release notes quote the zip's hash as
/// `` `sha256` of the zip: `<hash>` ``, which older releases publish even when GitHub records no asset digest.
fn sha256_after(text: &str, marker: &str) -> Option<String> {
    let start = text.to_ascii_lowercase().find(&marker.to_ascii_lowercase())? + marker.len();
    let bytes = text.as_bytes();
    let mut i = start;
    while i + 64 <= bytes.len() {
        if bytes[i..i + 64].iter().all(u8::is_ascii_hexdigit) {
            return Some(String::from_utf8_lossy(&bytes[i..i + 64]).to_ascii_lowercase());
        }
        i += 1;
    }
    None
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
    let body_text = v.get("body").and_then(|b| b.as_str()).unwrap_or_default();
    let assets = v.get("assets").and_then(|a| a.as_array());
    let zip_sha = sha256_after(body_text, "of the zip:");
    let mut asset = assets.and_then(|a| {
        a.iter()
            .filter_map(asset_from_json)
            .find(|a| a.name.ends_with(ASSET_SUFFIX))
    });
    if let Some(a) = asset.as_mut() {
        if a.sha256.is_none() {
            a.sha256 = zip_sha;
        }
    }
    let manifest = assets.and_then(|a| {
        a.iter()
            .filter_map(asset_from_json)
            .find(|a| a.name.eq_ignore_ascii_case("release-manifest.json"))
    });
    Ok(Release {
        version: parse_version(&tag),
        notes: body_text.to_string(),
        page: v
            .get("html_url")
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string(),
        asset,
        manifest,
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

/// Where builds replaced by updates are kept: `Old versions\EmblemWeave.exe.old*`
/// beside the launcher, so an update never leaves an `.old` file in the install root.
///
/// Multiple previous builds are kept, with incrementing extensions: .old, .old.1, .old.2, etc.
pub fn backup_exes() -> Vec<PathBuf> {
    current_exe()
        .map(|exe| {
            let dir = old_versions_dir(&exe);
            let mut exes: Vec<_> = std::fs::read_dir(&dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    let p = e.ok()?.path();
                    let is_old = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("EmblemWeave.exe.old"));
                    is_old.then_some(p)
                })
                .collect();
            exes.sort_by(|a, b| {
                // Sort by extension: .old < .old.1 < .old.2 ...
                let a_ext = a.extension().and_then(|s| s.to_str()).unwrap_or("");
                let b_ext = b.extension().and_then(|s| s.to_str()).unwrap_or("");
                a_ext.cmp(b_ext)
            });
            exes
        })
        .unwrap_or_default()
}

/// The folder beside the launcher that keeps previous builds.
fn old_versions_dir(exe: &Path) -> PathBuf {
    exe.with_file_name("Old versions")
}

/// Move the `EmblemWeave.exe.old` an earlier version wrote beside the launcher into `Old versions\`, so the install root is
/// clear again after the first start of this build.
pub fn migrate_legacy_backup() {
    if let Some(exe) = current_exe() {
        migrate_backup(&exe);
        // The previous launcher a headless update moved aside (see `swap_now`); it is no longer running by now.
        let _ = std::fs::remove_file(replaced_exe(&exe));
    }
}

fn migrate_backup(exe: &Path) {
    // EmblemWeave.exe becomes EmblemWeave.exe.old.
    let legacy = exe.with_extension("exe.old");
    if !legacy.is_file() {
        return;
    }
    let dir = old_versions_dir(exe);
    // Use the new naming scheme: find the next available version number
    let backup_name = next_old_version_name(&dir);
    let kept = dir.join(&backup_name);
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
    !backup_exes().is_empty()
}

/// The SHA-256 of a file, read in chunks so a release zip does not have to fit in memory.
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The zip hash and size `release-manifest.json` pins, checked against the file the updater is about to install.
fn expected_from_manifest(body: &[u8], zip_name: &str) -> Result<(String, Option<u64>), String> {
    let v: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("release-manifest.json is not JSON: {e}"))?;
    let archive = v
        .get("archive")
        .ok_or("release-manifest.json has no archive entry")?;
    let name = archive
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("release-manifest.json does not name its archive")?;
    if !name.eq_ignore_ascii_case(zip_name) {
        return Err(format!(
            "release-manifest.json describes {name}, not {zip_name}"
        ));
    }
    let sha = archive
        .get("sha256")
        .and_then(|s| s.as_str())
        .ok_or("release-manifest.json has no archive SHA-256")?;
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "release-manifest.json's archive SHA-256 is malformed: {sha}"
        ));
    }
    Ok((
        sha.to_ascii_lowercase(),
        archive.get("size").and_then(|s| s.as_u64()),
    ))
}

/// Reject a download that is not the exact file the release published: the wrong size (an interrupted download) or the
/// wrong SHA-256 (a corrupted or substituted zip). The hash comes from GitHub's asset digest, the release notes, or the
/// release's `release-manifest.json`. When more than one is published they must agree.
fn verify_download(zip: &Path, release: &Release) -> Result<(), String> {
    let asset = release
        .asset
        .as_ref()
        .ok_or("that release has no Windows download")?;
    let size = std::fs::metadata(zip)
        .map_err(|e| format!("reading the downloaded zip: {e}"))?
        .len();
    if let Some(want) = asset.size {
        if size != want {
            return Err(format!(
                "the download is incomplete: {size} bytes arrived, the release publishes {want}. Nothing was installed; try the update again."
            ));
        }
    }
    let mut sha = asset.sha256.clone();
    let mut published_size = asset.size;
    if let Some(manifest) = &release.manifest {
        if !manifest.url.starts_with("https://") {
            return Err("the release manifest address is not https".into());
        }
        match fetch(&manifest.url) {
            Ok(body) => {
                let (manifest_sha, manifest_size) = expected_from_manifest(&body, &asset.name)?;
                if let Some(known) = &sha {
                    if !known.eq_ignore_ascii_case(&manifest_sha) {
                        return Err(format!(
                            "the release's SHA-256 ({known}) and release-manifest.json ({manifest_sha}) disagree; nothing was installed"
                        ));
                    }
                }
                if let (Some(a), Some(b)) = (published_size, manifest_size) {
                    if a != b {
                        return Err(format!(
                            "the release's zip size ({a}) and release-manifest.json ({b}) disagree; nothing was installed"
                        ));
                    }
                }
                sha = Some(manifest_sha);
                if manifest_size.is_some() {
                    published_size = manifest_size;
                }
            }
            // The manifest is a second source, not the only one: a digest or the hash in the notes still verifies the zip.
            Err(e) if sha.is_none() => {
                return Err(format!(
                    "the release's SHA-256 could not be read from release-manifest.json ({e}), and it publishes no other one; nothing was installed"
                ));
            }
            Err(_) => {}
        }
    }
    let Some(expected) = sha else {
        return Err(
            "the release publishes no SHA-256 for its Windows zip, so EmblemWeave cannot verify the download. Install it from the release page and check SHA256SUMS."
                .into(),
        );
    };
    let got = sha256_file(zip)?;
    if got != expected {
        return Err(format!(
            "the download does not match the SHA-256 the release publishes: got {got}, expected {expected}. Nothing was installed; try the update again."
        ));
    }
    if let Some(want) = published_size {
        if size != want {
            return Err(format!(
                "the download is incomplete: {size} bytes arrived, release-manifest.json publishes {want}. Nothing was installed; try the update again."
            ));
        }
    }
    Ok(())
}

/// Download a release's zip, check it against the hash the release publishes, and unpack its executable into `dir`,
/// returning the unpacked `EmblemWeave.exe`. A zip that does not match is deleted and never unpacked.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let asset = release
        .asset
        .as_ref()
        .ok_or("that release has no Windows download")?;
    let (name, url) = (&asset.name, &asset.url);
    if !url.starts_with("https://") {
        return Err("the download address is not https".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let zip = dir.join(crate::mods::clean_name(name));
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
    .arg(url)
    .output()
    .map_err(|e| format!("running curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the download failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if let Err(e) = verify_download(&zip, release) {
        let _ = std::fs::remove_file(&zip);
        return Err(e);
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
        &old_versions_dir(&target),
        crate::engine::run_dir().as_deref(),
        engine_root,
        keep_previous,
        release_tag,
    )?;
    swap_script(&staged, &target, &update_log(), failure_marker().as_deref(), true)
}

/// [`install`] for `--headless update`: the same staging, but the swap is done before this returns instead of by a script
/// after exit, so `Ok` means the new launcher is in place. It is not opened afterwards; whoever ran the command (a Decky
/// plugin, a script) decides what runs next.
pub fn install_without_restart(
    new_exe: &Path,
    engine_root: Option<&Path>,
    keep_previous: bool,
    release_tag: &str,
) -> Result<(), String> {
    let target = current_exe().ok_or("the launcher's own path is unknown")?;
    let staged = install_into(
        new_exe,
        &target,
        &old_versions_dir(&target),
        crate::engine::run_dir().as_deref(),
        engine_root,
        keep_previous,
        release_tag,
    )?;
    swap_now(&staged, &target)?;
    if let Some(marker) = failure_marker() {
        let _ = std::fs::remove_file(marker);
    }
    Ok(())
}

/// The launcher a headless update moved aside: Windows lets a running executable be renamed but not replaced, so the old
/// one waits under this name until the next start removes it.
fn replaced_exe(target: &Path) -> PathBuf {
    PathBuf::from(format!("{}.replaced", target.display()))
}

/// Swap `staged` in for the running launcher at `target` before returning, so the caller learns whether it worked
/// instead of trusting a script that runs after this process has gone. On any failure `target` is left as it was.
fn swap_now(staged: &Path, target: &Path) -> Result<(), String> {
    let aside = replaced_exe(target);
    // A leftover from an earlier update that has not been cleaned up yet; if it cannot go, the rename below says why.
    let _ = std::fs::remove_file(&aside);
    if let Err(e) = std::fs::rename(target, &aside) {
        let _ = std::fs::remove_file(staged);
        return Err(format!("moving the running launcher aside: {e}"));
    }
    if let Err(e) = std::fs::rename(staged, target) {
        let _ = std::fs::rename(&aside, target);
        let _ = std::fs::remove_file(staged);
        return Err(format!("putting the new launcher in place: {e}"));
    }
    Ok(())
}

/// The part of [`install`] that runs before the restart, with the running launcher's paths passed in so a test can drive
/// the same sequence against a temporary install. `play` is the folder holding the kept build's `fefw.ini`; when there is
/// none, the release is still written but no carry is recorded. Returns the staged launcher the swap script replaces
/// `target` with.
fn install_into(
    new_exe: &Path,
    target: &Path,
    old_versions: &Path,
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
        // The backup lives in `Old versions\`; the folder does not exist yet on an install that has never updated.
        if let Err(e) = std::fs::create_dir_all(old_versions) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("creating the Old versions folder: {e}"));
        }
        // Copying a running executable is allowed; replacing it is not. Do it only once the release is known to be
        // complete, so a failed update does not overwrite the version Restore would go back to.
        // Save with incrementing extension: .old, .old.1, .old.2, etc.
        let backup_name = next_old_version_name(old_versions);
        if let Err(e) = std::fs::copy(&target, old_versions.join(&backup_name)) {
            let _ = std::fs::remove_file(&staged);
            return Err(format!("keeping the current version: {e}"));
        }
    }
    Ok(staged)
}

/// Determine the next old version filename: EmblemWeave.exe.old, EmblemWeave.exe.old.1, EmblemWeave.exe.old.2, etc.
fn next_old_version_name(old_versions: &Path) -> String {
    let existing = backup_exes();
    if existing.is_empty() {
        "EmblemWeave.exe.old".to_string()
    } else {
        // Find the highest numbered extension
        let mut max_num = 0;
        for p in &existing {
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext == "old" {
                max_num = max_num.max(1);
            } else if let Some(num_str) = ext.strip_prefix("old.") {
                if let Ok(num) = num_str.parse::<u32>() {
                    max_num = max_num.max(num + 1);
                }
            }
        }
        if max_num == 0 {
            "EmblemWeave.exe.old".to_string()
        } else if max_num == 1 {
            "EmblemWeave.exe.old.1".to_string()
        } else {
            format!("EmblemWeave.exe.old.{max_num}")
        }
    }
}

/// Put the most recent kept build back (the one with the highest version number).
pub fn restore_previous() -> Result<(), String> {
    let exes = backup_exes();
    let backup = exes.last().ok_or("there is no previous version to go back to")?;
    let target = current_exe().ok_or("the launcher's own path is unknown")?;
    let tmp = std::env::temp_dir().join(format!("emblemweave-rollback-{}.exe", std::process::id()));
    std::fs::copy(backup, &tmp).map_err(|e| format!("reading the previous version: {e}"))?;
    swap_script(&tmp, &target, &update_log(), failure_marker().as_deref(), true)
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
    restart: bool,
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
         {start}\
         goto done\r\n\
         :failed\r\n\
         if not \"%FAIL%\"==\"\" echo The update could not replace EmblemWeave.exe. The previous launcher was started. See \"%LOG%\". > \"%FAIL%\"\r\n\
         {start}\
         :done\r\n\
         del \"%~f0\"\r\n",
        new = new_exe.display(),
        target = target.display(),
        log = log.display(),
        start = if restart { "start \"\" \"%TARGET%\"\r\n" } else { "" },
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
    fn only_recomplauncher_one_leaves_updates_to_recomplauncher() {
        let with = |value: Option<&str>| {
            let value = value.map(str::to_string);
            managed_by_recomplauncher_from(&move |k| if k == "RECOMPLAUNCHER" { value.clone() } else { None })
        };
        assert!(with(Some("1")));
        assert!(!with(None));
        assert!(!with(Some("")));
        assert!(!with(Some("0")));
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
            &install.join("Old versions"),
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
        // The backup goes in `Old versions\`, which the update creates; nothing is written beside the launcher.
        assert_eq!(
            read(&install.join("Old versions/EmblemWeave.exe.old")),
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
    fn a_backup_beside_the_launcher_moves_into_the_old_versions_folder() {
        let root = std::env::temp_dir().join(format!("ew-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let exe = root.join("EmblemWeave.exe");
        write(&exe, "current launcher");
        write(&root.join("EmblemWeave.exe.old"), "previous launcher");

        migrate_backup(&exe);

        assert!(!root.join("EmblemWeave.exe.old").exists());
        assert_eq!(
            read(&root.join("Old versions/EmblemWeave.exe.old")),
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
        write(&root.join("Old versions/EmblemWeave.exe.old"), "kept");

        migrate_backup(&exe);

        assert!(!root.join("EmblemWeave.exe.old").exists());
        assert_eq!(read(&root.join("Old versions/EmblemWeave.exe.old")), "kept");
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
        assert_eq!(
            r.asset.unwrap().name,
            "EmblemWeave-v0.9.0-windows-x64.zip"
        );
    }

    /// GitHub records a digest and size for each asset; the release's manifest is found beside the zip, and the notes'
    /// quoted hash is the fallback for releases whose API response carries no digest.
    #[test]
    fn reads_the_published_hash_size_and_manifest() {
        let body = br#"{
            "tag_name": "v0.9.0",
            "body": "`sha256` of the zip: `1111111111111111111111111111111111111111111111111111111111111111`",
            "assets": [
                {
                    "name": "EmblemWeave-v0.9.0-windows-x64.zip",
                    "browser_download_url": "https://example.com/app.zip",
                    "size": 42,
                    "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                },
                {
                    "name": "release-manifest.json",
                    "browser_download_url": "https://example.com/release-manifest.json",
                    "size": 7
                }
            ]
        }"#;
        let r = release_from_json(body).unwrap();
        let asset = r.asset.unwrap();
        assert_eq!(asset.size, Some(42));
        assert_eq!(
            asset.sha256.as_deref(),
            Some("2222222222222222222222222222222222222222222222222222222222222222")
        );
        assert_eq!(
            r.manifest.unwrap().name,
            "release-manifest.json",
            "the manifest is kept as its own asset"
        );

        let no_digest = br#"{
            "tag_name": "v0.9.0",
            "body": "`sha256` of the zip: `3333333333333333333333333333333333333333333333333333333333333333`",
            "assets": [{"name": "EmblemWeave-v0.9.0-windows-x64.zip", "browser_download_url": "https://example.com/app.zip"}]
        }"#;
        let r = release_from_json(no_digest).unwrap();
        assert_eq!(
            r.asset.unwrap().sha256.as_deref(),
            Some("3333333333333333333333333333333333333333333333333333333333333333")
        );
        assert!(r.manifest.is_none());
    }

    #[test]
    fn release_manifest_pins_the_zip_it_describes() {
        let sha = "a".repeat(64);
        let body = serde_json::json!({
            "schema": 1,
            "archive": {
                "name": "EmblemWeave-v0.9.0-windows-x64.zip",
                "sha256": sha,
                "size": 1234
            }
        });
        let text = body.to_string();
        let (got, size) = expected_from_manifest(
            text.as_bytes(),
            "EmblemWeave-v0.9.0-windows-x64.zip",
        )
        .unwrap();
        assert_eq!(got, sha);
        assert_eq!(size, Some(1234));
        assert!(
            expected_from_manifest(text.as_bytes(), "EmblemWeave-v0.9.0-other.zip").is_err(),
            "a manifest for another archive cannot verify this download"
        );
        assert!(expected_from_manifest(b"{}", "EmblemWeave-v0.9.0-windows-x64.zip").is_err());
        assert!(expected_from_manifest(b"not json", "EmblemWeave-v0.9.0-windows-x64.zip").is_err());
    }

    fn zip_with_launcher() -> Vec<u8> {
        use std::io::Write;
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut bytes);
            zw.start_file(
                "EmblemWeave-v0.9.0/EmblemWeave.exe",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zw.write_all(b"new launcher").unwrap();
            zw.finish().unwrap();
        }
        bytes.into_inner()
    }

    fn release_for_zip(name: &str, sha: String, size: u64) -> Release {
        Release {
            tag: "v0.9.0".into(),
            version: Some((0, 9, 0)),
            notes: String::new(),
            page: String::new(),
            asset: Some(Asset {
                name: name.into(),
                url: "https://example.com/app.zip".into(),
                size: Some(size),
                sha256: Some(sha),
            }),
            manifest: None,
        }
    }

    /// A zip that bit rot, a truncated transfer or a substituted file changed must never reach the installer.
    #[test]
    fn a_corrupted_or_incomplete_download_is_rejected() {
        let dir = std::env::temp_dir().join(format!("ew-update-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = "EmblemWeave-v0.9.0-windows-x64.zip";
        let path = dir.join(name);

        let good = zip_with_launcher();
        std::fs::write(&path, &good).unwrap();
        let release = release_for_zip(name, sha256_file(&path).unwrap(), good.len() as u64);
        assert!(verify_download(&path, &release).is_ok(), "the intact download passes");

        let mut corrupted = good.clone();
        let at = corrupted.len() / 2;
        corrupted[at] ^= 0xff;
        std::fs::write(&path, &corrupted).unwrap();
        let err = verify_download(&path, &release).unwrap_err();
        assert!(err.contains("does not match the SHA-256"), "{err}");

        std::fs::write(&path, &good[..good.len() / 2]).unwrap();
        let err = verify_download(&path, &release).unwrap_err();
        assert!(err.contains("incomplete"), "{err}");

        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn a_headless_swap_is_done_before_it_reports() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("EmblemWeave.exe");
        let staged = dir.path().join("EmblemWeave.exe.new");
        write(&target, "old");
        write(&staged, "new");
        swap_now(&staged, &target).unwrap();
        assert_eq!(read(&target), "new");
        assert_eq!(read(&replaced_exe(&target)), "old");
        assert!(!staged.exists());

        // A staged file that is missing fails, and the launcher in place is untouched.
        write(&target, "current");
        assert!(swap_now(&staged, &target).is_err());
        assert_eq!(read(&target), "current");
    }
}
