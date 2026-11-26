//! The Trinity code mod that ships with EmblemWeave.
//!
//! The runtime loads every DLL in an installed mod's `exefs/` folder, so this
//! module keeps `data/play/mods/TrinityCore` present without changing the mod
//! loader or touching any other mod. The payload is embedded in the launcher:
//! a portable copy needs no separate download or support folder. An installed
//! copy newer than the bundled one is left alone; the plugin's own settings
//! file is never removed.

use std::path::{Path, PathBuf};

/// Kept in step with `assets/trinity/plugin.json` and the Trinity release.
pub const BUNDLED_VERSION: &str = "0.2.0";
pub const MOD_FOLDER: &str = "TrinityCore";
pub const DLL_NAME: &str = "Trinity.dll";

const PLUGIN_JSON: &str = include_str!("../../../assets/trinity/plugin.json");
const EMBLEMWEAVE_JSON: &str = include_str!("../../../assets/trinity/emblemweave.json");
const DLL: &[u8] = include_bytes!("../../../assets/trinity/Trinity.dll");

/// Install the bundled copy into the launcher's mods folder. `Ok(Some(message))`
/// means a file changed; `Ok(None)` means the installed version is current or
/// newer. Errors are startup diagnostics, not a reason to stop the launcher.
pub fn ensure_installed() -> Result<Option<String>, String> {
    let mods = crate::engine::run_dir()
        .ok_or_else(|| "the launcher data folder is unavailable".to_string())?
        .join("mods");
    // A package dropped beside EmblemWeave.exe or under plugins/ is an explicit
    // request to use that payload instead of the embedded release. The runtime
    // still loads it from the mods folder, so no loader change is needed.
    if let Some(dll) = external_dll() {
        return ensure_external_in(&mods, &dll);
    }
    ensure_in(&mods)
}

const EXTERNAL_PATHS: [&str; 6] = [
    "plugins/TrinityCore/exefs/Trinity.dll",
    "plugins/TrinityCore/Trinity.dll",
    "plugins/Trinity.dll",
    "TrinityCore/exefs/Trinity.dll",
    "TrinityCore/Trinity.dll",
    "Trinity.dll",
];

/// The first `Trinity.dll` beside the launcher or under its `plugins/` folder.
fn external_dll() -> Option<PathBuf> {
    let root = crate::platform::exe_dir()?;
    EXTERNAL_PATHS
        .iter()
        .map(|rel| root.join(rel))
        .find(|path| path.is_file())
}

/// Stage an externally supplied DLL, keeping a newer installed copy rather than
/// downgrading it. The manifest beside the DLL wins when present; otherwise the
/// embedded manifest and version describe the staged payload.
fn ensure_external_in(mods: &Path, dll: &Path) -> Result<Option<String>, String> {
    let package = match dll.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some("exefs") | Some("lib") => dll.parent().and_then(|p| p.parent()).unwrap_or(Path::new(".")),
        _ => dll.parent().unwrap_or(Path::new(".")),
    };
    let external_manifest = package.join("plugin.json");
    let external_version = manifest_version(&external_manifest);
    let folder = mods.join(MOD_FOLDER);
    let exefs = folder.join("exefs");
    let target = exefs.join(DLL_NAME);
    let installed = manifest_version(&folder.join("plugin.json"));

    if let (Some(ext), Some(inst)) = (&external_version, &installed) {
        if version_tuple(inst) > version_tuple(ext) {
            return Ok(None);
        }
    }

    std::fs::create_dir_all(&exefs)
        .map_err(|e| format!("creating {}: {e}", exefs.display()))?;
    write_atomic(&target, &std::fs::read(dll).map_err(|e| format!("reading {}: {e}", dll.display()))?)?;
    if external_manifest.is_file() {
        write_atomic(&folder.join("plugin.json"), &std::fs::read(&external_manifest).map_err(|e| format!("reading {}: {e}", external_manifest.display()))?)?;
    } else {
        write_atomic(&folder.join("plugin.json"), PLUGIN_JSON.as_bytes())?;
    }
    write_atomic(&folder.join("emblemweave.json"), EMBLEMWEAVE_JSON.as_bytes())?;

    let version = external_version.as_deref().unwrap_or(BUNDLED_VERSION);
    Ok(Some(format!(
        "Trinity {version} installed from {} for code mods",
        dll.display()
    )))
}

/// The testable part: install under a supplied mods folder.
pub fn ensure_in(mods: &Path) -> Result<Option<String>, String> {
    let folder = mods.join(MOD_FOLDER);
    let exefs = folder.join("exefs");
    let dll = exefs.join(DLL_NAME);
    let manifest = folder.join("plugin.json");
    let already = installed_version(&manifest);
    let current = already
        .as_deref()
        .is_some_and(|v| version_tuple(v) >= version_tuple(BUNDLED_VERSION));

    if current && dll.is_file() {
        return Ok(None);
    }
    // A newer install wins even if its DLL is missing: do not overwrite a
    // version this launcher does not know how to replace.
    if already
        .as_deref()
        .is_some_and(|v| version_tuple(v) > version_tuple(BUNDLED_VERSION))
    {
        return Ok(None);
    }

    std::fs::create_dir_all(&exefs).map_err(|e| format!("creating {}: {e}", exefs.display()))?;
    // Write the DLL first, so a manifest claiming the new version never points
    // at an old or half-written payload.
    write_atomic(&dll, DLL)?;
    write_atomic(&manifest, PLUGIN_JSON.as_bytes())?;
    write_atomic(
        &folder.join("emblemweave.json"),
        EMBLEMWEAVE_JSON.as_bytes(),
    )?;

    // The old release used a lower-case name. Windows treats the two as one
    // file; on a case-sensitive filesystem both would be loaded.
    #[cfg(not(windows))]
    {
        let legacy = exefs.join("trinity.dll");
        if legacy != dll && legacy.is_file() {
            let _ = std::fs::remove_file(legacy);
        }
    }

    let action = if already.is_some() {
        "updated"
    } else {
        "installed"
    };
    Ok(Some(format!(
        "Trinity {BUNDLED_VERSION} {action} for code mods ({})",
        folder.display()
    )))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let temporary = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("writing {}: {e}", temporary.display()))?;
    // `rename` replaces an existing file on Unix. Windows refuses, so remove
    // the destination first; the temporary file is already complete.
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("replacing {}: {e}", path.display()))?;
    }
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("moving {} to {}: {e}", temporary.display(), path.display()))
}

fn manifest_version(manifest: &Path) -> Option<String> {
    installed_version(manifest)
}

fn installed_version(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("version")?.as_str().map(ToOwned::to_owned)
}

fn version_tuple(v: &str) -> (u64, u64, u64) {
    let mut parts = v.split('.');
    let mut next = || {
        parts
            .next()
            .and_then(|p| p.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (next(), next(), next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ew-trinity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn installs_the_embedded_payload_once() {
        let mods = temp("install");
        let msg = ensure_in(&mods).unwrap();
        assert!(msg.is_some());
        let folder = mods.join(MOD_FOLDER);
        assert_eq!(
            std::fs::read(folder.join("exefs").join(DLL_NAME)).unwrap(),
            DLL
        );
        assert_eq!(
            installed_version(&folder.join("plugin.json")).as_deref(),
            Some(BUNDLED_VERSION)
        );
        assert!(ensure_in(&mods).unwrap().is_none());
        let _ = std::fs::remove_dir_all(mods);
    }

    #[test]
    fn does_not_downgrade_a_newer_install() {
        let mods = temp("newer");
        let folder = mods.join(MOD_FOLDER);
        std::fs::create_dir_all(folder.join("exefs")).unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"version":"9.9.9"}"#).unwrap();
        std::fs::write(folder.join("exefs").join(DLL_NAME), b"newer").unwrap();
        assert!(ensure_in(&mods).unwrap().is_none());
        assert_eq!(
            std::fs::read(folder.join("exefs").join(DLL_NAME)).unwrap(),
            b"newer"
        );
        let _ = std::fs::remove_dir_all(mods);
    }

    #[test]
    fn installs_a_root_dll_with_the_embedded_manifest() {
        let root = temp("external");
        let mods = root.join("mods");
        std::fs::write(root.join(DLL_NAME), b"external payload").unwrap();
        let message = ensure_external_in(&mods, &root.join(DLL_NAME)).unwrap().unwrap();
        assert!(message.contains("installed from"));
        assert_eq!(
            std::fs::read(mods.join(MOD_FOLDER).join("exefs").join(DLL_NAME)).unwrap(),
            b"external payload"
        );
        assert_eq!(
            installed_version(&mods.join(MOD_FOLDER).join("plugin.json")).as_deref(),
            Some(BUNDLED_VERSION)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_a_newer_install_over_an_external_dll() {
        let root = temp("external-newer");
        let mods = root.join("mods");
        let folder = mods.join(MOD_FOLDER);
        std::fs::create_dir_all(folder.join("exefs")).unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"version":"9.9.9"}"#).unwrap();
        std::fs::write(folder.join("exefs").join(DLL_NAME), b"installed").unwrap();
        std::fs::write(root.join(DLL_NAME), b"older external").unwrap();
        std::fs::write(root.join("plugin.json"), r#"{"version":"0.2.0"}"#).unwrap();
        assert!(ensure_external_in(&mods, &root.join(DLL_NAME)).unwrap().is_none());
        assert_eq!(
            std::fs::read(folder.join("exefs").join(DLL_NAME)).unwrap(),
            b"installed"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_the_plugin_settings() {
        let mods = temp("settings");
        ensure_in(&mods).unwrap();
        let settings = mods.join(MOD_FOLDER).join(".nxp-settings.ini");
        std::fs::write(&settings, "trinity.enabled=0\n").unwrap();
        ensure_in(&mods).unwrap();
        assert_eq!(
            std::fs::read_to_string(settings).unwrap(),
            "trinity.enabled=0\n"
        );
        let _ = std::fs::remove_dir_all(mods);
    }
}
