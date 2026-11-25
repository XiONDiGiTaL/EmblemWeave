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
    ensure_in(&mods)
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
