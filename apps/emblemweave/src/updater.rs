//! Updates from GitHub Releases: asking whether a newer release exists,
//! downloading its zip and swapping the new launcher in for this one.
//!
//! Downloads go through Windows' own `curl.exe` and `tar.exe`, as the mods page
//! does, so the launcher carries no HTTP client of its own.
//!
//! A running executable cannot be overwritten on Windows, but it can be
//! renamed. The new build is unpacked beside this one, the current binary is
//! kept as `EmblemWeave.exe.old`, and a short script replaces the file and
//! starts it again once this process has exited.

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

fn system_tool(name: &str) -> PathBuf {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
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
        return Err(if err.contains("404") { "no release was found for this repository".into() } else { format!("GitHub answered: {err}") });
    }
    Ok(out.stdout)
}

/// The newest published release.
pub fn latest() -> Result<Release, String> {
    let body = fetch(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    release_from_json(&body)
}

/// Reads a GitHub release object. Separated from the request so it can be
/// tested against recorded responses.
pub fn release_from_json(body: &[u8]) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("GitHub answered with something unexpected: {e}"))?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or_default().to_string();
    if tag.is_empty() {
        return Err("the release has no tag".into());
    }
    let asset = v
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|a| {
            a.iter().find_map(|a| {
                let name = a.get("name")?.as_str()?;
                if name.ends_with(ASSET_SUFFIX) {
                    Some((name.to_string(), a.get("browser_download_url")?.as_str()?.to_string()))
                } else {
                    None
                }
            })
        });
    Ok(Release {
        version: parse_version(&tag),
        notes: v.get("body").and_then(|b| b.as_str()).unwrap_or_default().to_string(),
        page: v.get("html_url").and_then(|p| p.as_str()).unwrap_or_default().to_string(),
        asset,
        tag,
    })
}

/// The launcher running now.
pub fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Where the build replaced by the last update is kept.
pub fn backup_exe() -> Option<PathBuf> {
    current_exe().map(|p| PathBuf::from(format!("{}.old", p.display())))
}

pub fn has_backup() -> bool {
    backup_exe().is_some_and(|p| p.is_file())
}

/// Download a release's zip and unpack its executable into `dir`, returning
/// the unpacked `EmblemWeave.exe`.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let (name, url) = release.asset.clone().ok_or("that release has no Windows download")?;
    if !url.starts_with("https://") {
        return Err("the download address is not https".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let zip = dir.join(crate::mods::clean_name(&name));
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
        return Err(format!("the download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let unpacked = dir.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let out = hidden(Command::new(system_tool("tar.exe")).arg("-xf").arg(&zip).arg("-C").arg(&unpacked))
        .output()
        .map_err(|e| format!("running tar: {e}"))?;
    if !out.status.success() {
        return Err(format!("the download could not be unpacked: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    find_exe(&unpacked, 3).ok_or_else(|| "the download has no EmblemWeave.exe in it".to_string())
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
        } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("EmblemWeave.exe")) {
            return Some(p);
        }
    }
    None
}

/// Replace the running launcher with `new_exe` and start it again. The caller
/// closes the window afterwards; the script waits for the file to be free.
pub fn install(new_exe: &Path, keep_previous: bool) -> Result<(), String> {
    let target = current_exe().ok_or("the launcher's own path is unknown")?;
    let backup = backup_exe().ok_or("the launcher's own path is unknown")?;
    if keep_previous {
        // Copying a running executable is allowed; replacing it is not.
        std::fs::copy(&target, &backup).map_err(|e| format!("keeping the current version: {e}"))?;
    }
    swap_script(new_exe, &target)
}

/// Put the build kept from the last update back.
pub fn restore_previous() -> Result<(), String> {
    let backup = backup_exe().filter(|p| p.is_file()).ok_or("there is no previous version to go back to")?;
    let target = current_exe().ok_or("the launcher's own path is unknown")?;
    let tmp = std::env::temp_dir().join(format!("emblemweave-rollback-{}.exe", std::process::id()));
    std::fs::copy(&backup, &tmp).map_err(|e| format!("reading the previous version: {e}"))?;
    swap_script(&tmp, &target)
}

fn swap_script(new_exe: &Path, target: &Path) -> Result<(), String> {
    let script = std::env::temp_dir().join(format!("emblemweave-update-{}.cmd", std::process::id()));
    let text = format!(
        "@echo off\r\n\
         ping -n 3 127.0.0.1 >nul\r\n\
         :again\r\n\
         move /y \"{new}\" \"{target}\" >nul 2>&1\r\n\
         if errorlevel 1 (\r\n\
         \x20 ping -n 2 127.0.0.1 >nul\r\n\
         \x20 goto again\r\n\
         )\r\n\
         start \"\" \"{target}\"\r\n\
         del \"%~f0\"\r\n",
        new = new_exe.display(),
        target = target.display()
    );
    std::fs::write(&script, text).map_err(|e| format!("writing the update script: {e}"))?;
    hidden(Command::new("cmd").arg("/c").arg(&script)).spawn().map_err(|e| format!("starting the update: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_compare_like_versions() {
        assert_eq!(parse_version("v0.1.4"), Some((0, 1, 4)));
        assert_eq!(parse_version("0.2.0"), Some((0, 2, 0)));
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
